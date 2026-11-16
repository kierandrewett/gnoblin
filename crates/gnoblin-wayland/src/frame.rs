//! The frame hook: one iteration of the compositor's loop.
//!
//! The loop that [`Server::run`](crate::Server::run) drives wakes for a client, a
//! backend event, a control command or the loop's timeout. Every wake ends here, and
//! here is where the Frame path of `design/rust-compositor.md` runs: the host takes
//! the scene's damage once, each output asks for its frame, and an output with no
//! damage draws nothing.
//!
//! The hook is a module of its own because the event loop is not where rendering
//! belongs: `server.rs` owns the socket and the calloop wiring, `state.rs` owns the
//! policy translation, and this file owns "what happens once per iteration". It is
//! also the only place that knows both a [`Backend`] and a `State`, which is what
//! makes it the natural home for the screenshot request beside the render call.
//!
//! # Why an idle iteration draws nothing
//!
//! [`Backend::render`] asks the host for a frame and draws only when the host answers
//! that something changed. An idle iteration therefore costs one call into the
//! compositor and no pixman work at all, which is what makes a software-rendered
//! session usable: the alternative, repainting 1024x768 at 60 Hz on a CPU, is a
//! permanently warm core and an idle fan.
//!
//! # Failure modes
//!
//! 1. **An idle iteration draws a frame.** [`FrameStats::frames`] does not move while
//!    `iterations` does: `an_idle_iteration_draws_no_frame`.
//! 2. **A failed frame stops the session.** A render error is logged and the next
//!    output is rendered; the pixels of the failed output stay as they were:
//!    `a_frame_that_cannot_be_drawn_does_not_stop_the_loop`.
//! 3. **A screenshot waits forever when the loop stops.** The request has a deadline
//!    and answers [`None`]: `a_screenshot_request_is_answered_with_pixels`.
//! 4. **A guarded screenshot reads pixels the lock covered after the caller looked.** The caller checks the lock from a
//!    snapshot of the session, and the loop reads the pixels later, so a lock engaged in between would be missed. A
//!    guarded request ([`Screenshots::request_guarded`]) is asked again at the moment the loop reads, and is answered
//!    [`ShotRefusal::Locked`] with no pixels:
//!    `a_guarded_screenshot_is_refused_when_the_loop_finds_the_session_covered`.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::ping::Ping;

use gnoblin_backend::{Backend, Host};
use gnoblin_render_frame::image::Image;
use gnoblin_types::{OutputId, Rect};

use crate::state::State;

use crate::log;

/// How many screenshot requests may wait for the loop at once. Each caller blocks until its own is answered, so this
/// is the number of callers a stalled loop can hold; past it a request is refused rather than queued behind a loop
/// that is not answering.
const MAX_PENDING_SHOTS: usize = 8;

/// How long a screenshot waits for the loop to take it.
///
/// It is generous because the loop's own timeout is 100 ms and a screenshot request
/// is answered at the end of the next iteration; it is finite because a session that
/// has stopped must not leave a caller blocked for ever.
pub const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(10);

/// What the loop has done, for a test and for a diagnostics reader.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct FrameStats {
    /// How many times the loop ran its per-iteration work.
    pub iterations: u64,
    /// How many iterations drew at least one output.
    ///
    /// A session with nothing to redraw leaves this still while `iterations` climbs,
    /// which is the property a test asserts to prove that idling costs no paint work.
    pub frames: u64,
    /// The damage of the last frame drawn, in the last output's coordinates.
    pub damage: Vec<Rect>,
}

impl FrameStats {
    /// Records one iteration.
    pub fn record(&mut self, drew: bool, damage: &[Rect]) {
        self.iterations = self.iterations.saturating_add(1);
        if drew {
            self.frames = self.frames.saturating_add(1);
            self.damage.clear();
            self.damage.extend_from_slice(damage);
        }
    }
}

/// The timestamp one frame is stamped with, in milliseconds since the first frame.
///
/// A frame callback's payload is a timestamp a client uses to work out its own drawing
/// rate, so anything monotonic and millisecond-scaled serves. It is measured from this
/// process's first frame rather than read from a clock the compositor would have to
/// carry through every seam.
fn frame_time() -> u32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// A screenshot request waiting for the loop.
struct Shot {
    output: OutputId,
    /// Whether the picture is the frame as the display shows it (at the output's mode) rather than the one the
    /// compositor composed, which has more pixels when the output is supersampled.
    display: bool,
    /// Whether the lock is asked again when the loop reads the pixels: a capture a client asked for, as opposed to a
    /// screenshot the harness takes of the session as it is.
    guarded: bool,
    slot: Arc<ShotSlot>,
}

/// Why a guarded screenshot produced no picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotRefusal {
    /// The loop found the session covered by the lock, or capture embargoed, when it came to read the pixels.
    Locked,
    /// The loop did not answer in time, or the output has no frame to read back.
    Unavailable,
}

/// Where a served screenshot lands.
#[derive(Default)]
struct ShotSlot {
    state: Mutex<ShotState>,
    ready: Condvar,
}

/// The pixels, and whether the loop has looked at the request at all.
#[derive(Default)]
struct ShotState {
    image: Option<Image>,
    served: bool,
    /// The loop refused to read: the session was covered when it came to it.
    covered: bool,
}

impl ShotSlot {
    /// Records the answer and wakes the waiter.
    fn serve(&self, image: Option<Image>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.image = image;
        state.served = true;
        self.ready.notify_all();
    }

    /// Records that the loop found the session covered, and wakes the waiter. No pixel was read.
    fn serve_covered(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.covered = true;
        state.served = true;
        self.ready.notify_all();
    }

    /// Waits for the answer, or gives up after `timeout`.
    fn wait(&self, timeout: Duration) -> Option<Image> {
        self.take(timeout).and_then(|state| state.image)
    }

    /// [`ShotSlot::wait`] for a guarded request, which can also be refused because the session was covered.
    fn wait_guarded(&self, timeout: Duration) -> Result<Image, ShotRefusal> {
        match self.take(timeout) {
            Some(state) if state.covered => Err(ShotRefusal::Locked),
            Some(ShotState { image: Some(image), .. }) => Ok(image),
            _ => Err(ShotRefusal::Unavailable),
        }
    }

    /// Waits until the loop has looked at the request and takes what it left, or gives up after `timeout`.
    fn take(&self, timeout: Duration) -> Option<ShotState> {
        let deadline = Instant::now().checked_add(timeout);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while !state.served {
            let remaining = deadline.and_then(|deadline| deadline.checked_duration_since(Instant::now()))?;
            let (guard, outcome) = self
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            state = guard;
            if outcome.timed_out() && !state.served {
                return None;
            }
        }
        Some(std::mem::take(&mut *state))
    }
}

/// The screenshot requests the control handle has queued.
#[derive(Default)]
pub struct Screenshots {
    queue: Mutex<VecDeque<Shot>>,
}

impl Screenshots {
    /// Queues a request for the composed frame and waits for the loop to answer it. The frame has
    /// `mode * supersample` pixels on each axis, which is the mode itself when the output is not supersampled.
    ///
    /// The ping wakes the loop so the answer arrives on the next iteration rather than
    /// on its timeout.
    pub fn request(&self, output: OutputId, ping: &Ping) -> Option<Image> {
        self.ask(output, false, ping)
    }

    /// [`Screenshots::request`] for the frame as the display shows it: the composed frame averaged down to the
    /// output's mode.
    pub fn request_display(&self, output: OutputId, ping: &Ping) -> Option<Image> {
        self.ask(output, true, ping)
    }

    /// [`Screenshots::request`] or [`Screenshots::request_display`] (`display`) for a capture a client asked for: the
    /// lock is asked again when the loop reads the pixels, so a lock that engaged after the caller looked still
    /// refuses it. The harness's plain requests are not guarded, because a test reads the cover the lock draws.
    pub fn request_guarded(&self, output: OutputId, display: bool, ping: &Ping) -> Result<Image, ShotRefusal> {
        let slot = self
            .enqueue(output, display, true, ping)
            .ok_or(ShotRefusal::Unavailable)?;
        let image = slot.wait_guarded(SCREENSHOT_TIMEOUT);
        if image.is_err() {
            self.withdraw(&slot);
        }
        image
    }

    fn ask(&self, output: OutputId, display: bool, ping: &Ping) -> Option<Image> {
        self.ask_within(output, display, ping, SCREENSHOT_TIMEOUT)
    }

    /// [`Screenshots::ask`] with its own deadline, so a test does not wait the real one out.
    ///
    /// A request that gives up withdraws itself: left in the queue it would be served later with a full readback
    /// nobody waits for.
    fn ask_within(&self, output: OutputId, display: bool, ping: &Ping, timeout: Duration) -> Option<Image> {
        let slot = self.enqueue(output, display, false, ping)?;
        let image = slot.wait(timeout);
        if image.is_none() {
            self.withdraw(&slot);
        }
        image
    }

    /// Takes a request back out of the queue, if the loop has not taken it yet.
    fn withdraw(&self, slot: &Arc<ShotSlot>) {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|shot| !Arc::ptr_eq(&shot.slot, slot));
    }

    /// Queues a request, or refuses it at once ([`None`]) when the queue is too full to take it.
    fn enqueue(&self, output: OutputId, display: bool, guarded: bool, ping: &Ping) -> Option<Arc<ShotSlot>> {
        let slot = Arc::new(ShotSlot::default());
        {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            if queue.len() >= MAX_PENDING_SHOTS {
                log::warn!(
                    area = "frame",
                    pending = queue.len(),
                    "too many screenshot requests wait; refused"
                );
                return None;
            }
            queue.push_back(Shot {
                output,
                display,
                guarded,
                slot: Arc::clone(&slot),
            });
        }
        ping.ping();
        Some(slot)
    }

    /// Answers every queued request from the backend's readback. `covered` is the lock's answer now: a guarded
    /// request is refused without reading anything while it holds.
    fn serve(&self, backend: &mut dyn Backend<State>, covered: bool) {
        loop {
            let Some(shot) = self.queue.lock().unwrap_or_else(PoisonError::into_inner).pop_front() else {
                return;
            };
            if shot.guarded && covered {
                shot.slot.serve_covered();
                continue;
            }
            let image = if shot.display {
                backend.capture_display(shot.output, false)
            } else {
                backend.screenshot(shot.output)
            };
            shot.slot.serve(image);
        }
    }
}

impl std::fmt::Debug for Screenshots {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queued = self.queue.lock().map_or(0, |queue| queue.len());
        formatter.debug_struct("Screenshots").field("queued", &queued).finish()
    }
}

/// Runs one iteration of the frame path.
///
/// Every output is asked for its frame, and only the ones that changed are drawn and
/// presented. The damage of the iteration is recorded in `stats` so a test can prove
/// that an idle iteration drew nothing and that a move damaged both rectangles.
pub fn render(
    backend: &mut dyn Backend<State>,
    state: &mut State,
    stats: &Arc<Mutex<FrameStats>>,
    screenshots: &Screenshots,
) {
    // Capture hears the turn before anything is drawn: a session the lock covers is
    // stopped here, so no frame of it is produced for the iteration that follows.
    state.poll_capture();
    // A View of a window that shows a GPU buffer reads its pixels back through the backend, where the buffer lives.
    state.begin_frame_reading(&mut |dmabuf| {
        backend
            .read_dmabuf(dmabuf)
            .map_err(|error| log::debug!(area = "frame", %error, "a client's GPU buffer was not read for a View"))
            .ok()
    });
    // An output with no place of its own (a mirror, or one that is off) is drawn after the ones that have: a mirror
    // shows the finished picture of its source, so the source is drawn first whatever order the backend lists them.
    let outputs: Vec<OutputId> = backend
        .outputs()
        .iter()
        .map(|output| output.id)
        .filter(|output| !state.is_shadow(*output))
        .collect();
    let mut drew = false;
    let mut damage: Vec<Rect> = Vec::new();
    let mut drawn_outputs: Vec<OutputId> = Vec::new();
    for output in outputs {
        let drawn = backend.render(state, output);
        let changed = state.frame_damage(output).is_some();
        if let Err(error) = &drawn {
            // A frame that cannot be drawn leaves the output's previous pixels in
            // place, which is the only safe state: showing a half-composed frame
            // or ending the session would both be worse.
            log::warn!(area = "frame", ?error, %output, "a frame could not be drawn");
        }
        if changed {
            drawn_outputs.push(output);
            if let Some(rects) = state.frame_damage(output) {
                drew = true;
                damage.clear();
                damage.extend_from_slice(rects);
            }
            if drawn.is_ok() {
                // The host described a changed frame and the backend drew it, so the
                // frame reached the output: a lock waiting for a covering frame hears
                // about it here and nowhere earlier.
                state.cover_presented(output);
                // A capture copies the frame that reached the screen, so the pixels are
                // read back here.
                state.capture_frame(backend, output);
                state.note_capture_damage(output, damage.as_slice());
                // A client that asked for presentation feedback is answered after the
                // frame reached the output, with its timestamp read now.
                state.present_frame(output);
            }
        }
    }
    for shadow in state.shadow_outputs() {
        let source_drew = shadow.source.is_some_and(|source| drawn_outputs.contains(&source));
        if !state.shadow_due(shadow, source_drew) {
            continue;
        }
        if let Some(source) = shadow.source {
            // The mirror shows what the source's screen shows, pointer included, as the source's own readback has it:
            // the frame at the source's mode, not the hi-res one a supersampled source composed.
            match backend.capture_display(source, true) {
                Some(picture) => state.set_mirror_picture(shadow.output, picture),
                None => continue,
            }
        }
        if let Err(error) = backend.render(state, shadow.output) {
            log::warn!(area = "frame", ?error, output = %shadow.output, "a mirrored frame could not be drawn");
        }
    }
    // A View the renderer drew into a texture has no picture on the CPU: the ones a consumer reads are read back now,
    // from the draws just made, and the frames and stills that waited for them are answered. This comes before the
    // copies into a GPU buffer, which read the same pictures.
    state.read_gpu_views(backend);
    // The captures into a client's GPU buffer are answered from the frames just drawn: a copy is made where the
    // renderer is, which a protocol handler is not.
    state.apply_dmabuf_captures(backend);
    // What the renderers and capture hold is measured after the frame, which is when the next turn's admissions read
    // it, and the frame cache the budget gave back is made again once the clients' buffers leave room for it.
    state.measure_gpu_memory(&*backend);
    state.resume_capture_cache_when_roomy();
    // What each output holds is published to its screen node here, before the turn's tree changes are taken, so a
    // shell that watches it hears a change on the iteration that made it.
    state.publish_gpu_memory(backend);
    state.end_frame();
    // A popup that hangs wholly off every output is drawn nowhere, so its commit damages nothing and no frame
    // follows it; its client is answered all the same, because there is no frame of it to wait for. A window the
    // shell throttles is answered when its interval has passed, which a still screen draws no frame for.
    let due = state.take_callbacks_due() || state.frame_callbacks_waiting();
    if drew || due {
        // A client that asked for a frame callback gets one answer per frame it was
        // shown, and an iteration that drew nothing showed nothing. Without this a
        // client that paces itself by the callback — a nested session always does —
        // would draw its first frame and wait for ever.
        state.send_frame_callbacks(frame_time());
        // A fifo barrier is released on the same tick: the commit it holds is shown on the
        // frame that answers its frame callback, so the next commit may follow.
        state.release_fifo_barriers();
    }
    stats
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .record(drew, &damage);
    // The lock is asked again here, where the pixels are read, and not only where the request was made.
    let covered = state.capture_covered();
    screenshots.serve(backend, covered);
}

#[cfg(test)]
mod tests {
    use gnoblin_backend::headless::Headless;
    use gnoblin_backend::{BackendError, InputSource, Present};
    use gnoblin_render_frame::error::RenderError;
    use gnoblin_render_frame::frame::OutputFrame;
    use smithay::reexports::calloop::LoopHandle;
    use smithay::reexports::wayland_server::Display;

    use crate::policy::WmOptions;
    use gnoblin_lock::{Event as LockEvent, State as LockState};
    use gnoblin_types::ClientId;

    use super::*;

    /// A session's state and the headless backend it draws on, for a loop test.
    fn session() -> (Headless, State) {
        let backend = Headless::new();
        let display: Display<State> = Display::new().expect("a display");
        let outputs = Backend::<State>::outputs(&backend).to_vec();
        let state = State::new(
            display,
            &outputs,
            "seat0",
            WmOptions::default(),
            None,
            None,
            crate::tree_frame::TreeHooks::default(),
            &gnoblin_config::Protocols::default(),
            crate::protocols::extension::TitlebarActions::default(),
            crate::server::CursorSettings::default(),
            crate::server::VisualBellSettings::default(),
            crate::server::VisualAccessibilitySettings::default(),
            &[],
            crate::lock_auth::LockAuth::new(std::sync::Arc::new(gnoblin_lock_auth::FakeAuthenticator::new()), None).0,
        )
        .expect("a state for the default output");
        (backend, state)
    }

    /// A backend whose frames always fail, to prove a bad frame does not stop the
    /// loop.
    struct Failing {
        outputs: Vec<gnoblin_backend::Output>,
    }

    impl Backend<State> for Failing {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn outputs(&self) -> &[gnoblin_backend::Output] {
            &self.outputs
        }

        fn output(&self, id: OutputId) -> Option<&gnoblin_backend::Output> {
            self.outputs.iter().find(|output| output.id == id)
        }

        fn target(&mut self, _id: OutputId) -> Option<&mut dyn gnoblin_backend::Target> {
            None
        }

        fn input(&mut self) -> &mut dyn InputSource {
            unreachable!("this backend has no input")
        }

        fn register(&mut self, _loop_handle: &LoopHandle<'static, State>) -> std::io::Result<()> {
            Ok(())
        }

        fn render(&mut self, _host: &mut State, output: OutputId) -> Result<Present, BackendError> {
            Err(BackendError::UnknownOutput(output))
        }

        fn capture(&mut self, _output: OutputId, _with_cursor: bool) -> Option<Image> {
            None
        }

        fn dispatch(&mut self, _host: &mut State) -> Vec<gnoblin_backend::BackendEvent> {
            Vec::new()
        }
    }

    /// A backend that describes a frame and then fails to draw it.
    ///
    /// It exists for the one case where "the host described a frame" and "the frame
    /// reached the output" disagree.
    struct Undrawable {
        outputs: Vec<gnoblin_backend::Output>,
        frame: OutputFrame,
    }

    impl Backend<State> for Undrawable {
        fn name(&self) -> &'static str {
            "undrawable"
        }

        fn outputs(&self) -> &[gnoblin_backend::Output] {
            &self.outputs
        }

        fn output(&self, id: OutputId) -> Option<&gnoblin_backend::Output> {
            self.outputs.iter().find(|output| output.id == id)
        }

        fn target(&mut self, _id: OutputId) -> Option<&mut dyn gnoblin_backend::Target> {
            None
        }

        fn input(&mut self) -> &mut dyn InputSource {
            unreachable!("this backend has no input")
        }

        fn register(&mut self, _loop_handle: &LoopHandle<'static, State>) -> std::io::Result<()> {
            Ok(())
        }

        fn render(&mut self, host: &mut State, output: OutputId) -> Result<Present, BackendError> {
            if host.frame(output, &mut self.frame) {
                return Err(BackendError::Render(RenderError::Failed));
            }
            Ok(Present::Nothing)
        }

        fn capture(&mut self, _output: OutputId, _with_cursor: bool) -> Option<Image> {
            None
        }

        fn dispatch(&mut self, _host: &mut State) -> Vec<gnoblin_backend::BackendEvent> {
            Vec::new()
        }
    }

    #[test]
    fn a_failed_cover_draw_does_not_prove_the_cover() {
        let (mut backend, mut state) = session();
        // A lock request arms the barrier for the one output, and the cover is what
        // the host will describe from now on.
        let effects = state.lock.apply(LockEvent::LockRequested {
            locker: ClientId::new(1),
        });
        state.apply_lock(effects);
        assert_eq!(state.lock_state(), LockState::Locking);

        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        let mut undrawable = Undrawable {
            outputs: Backend::<State>::outputs(&backend).to_vec(),
            frame: OutputFrame::new(),
        };
        render(&mut undrawable, &mut state, &stats, &screenshots);
        assert_eq!(
            state.lock_state(),
            LockState::Locking,
            "a frame that was described but not drawn proves nothing"
        );

        // The cover is still owed, so the next output that draws paints it and the
        // lock is proven.
        render(&mut backend, &mut state, &stats, &screenshots);
        assert_eq!(state.lock_state(), LockState::Locked);
    }

    /// Records whether the host handed the backend a cursor for the frame it drew.
    ///
    /// The frame draws its cursor above every layer, so a cursor here is visible over
    /// whatever the output shows. The cover test below reads it back.
    struct CursorRecorder {
        outputs: Vec<gnoblin_backend::Output>,
        cursor_drawn: Vec<bool>,
        /// How many Views each frame names alive or carries a scene of.
        views: Vec<usize>,
        /// How many times the host asked the backend to give its View textures back.
        released: usize,
    }

    impl Backend<State> for CursorRecorder {
        fn name(&self) -> &'static str {
            "cursor-recorder"
        }

        fn outputs(&self) -> &[gnoblin_backend::Output] {
            &self.outputs
        }

        fn output(&self, id: OutputId) -> Option<&gnoblin_backend::Output> {
            self.outputs.iter().find(|output| output.id == id)
        }

        fn target(&mut self, _id: OutputId) -> Option<&mut dyn gnoblin_backend::Target> {
            None
        }

        fn input(&mut self) -> &mut dyn InputSource {
            unreachable!("this backend has no input")
        }

        fn register(&mut self, _loop_handle: &LoopHandle<'static, State>) -> std::io::Result<()> {
            Ok(())
        }

        fn render(&mut self, host: &mut State, output: OutputId) -> Result<Present, BackendError> {
            let mut frame = OutputFrame::new();
            host.frame(output, &mut frame);
            self.cursor_drawn.push(frame.cursor().is_some());
            self.views.push(frame.live_views().len() + frame.view_scenes().len());
            Ok(Present::Nothing)
        }

        fn capture(&mut self, _output: OutputId, _with_cursor: bool) -> Option<Image> {
            None
        }

        fn dispatch(&mut self, _host: &mut State) -> Vec<gnoblin_backend::BackendEvent> {
            Vec::new()
        }

        fn release_views(&mut self) {
            self.released += 1;
        }
    }

    #[test]
    fn a_locked_output_draws_no_cursor_over_the_cover() {
        let (backend, mut state) = session();
        let mut recorder = CursorRecorder {
            outputs: Backend::<State>::outputs(&backend).to_vec(),
            cursor_drawn: Vec::new(),
            views: Vec::new(),
            released: 0,
        };
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();

        // Unlocked, the output shows the named pointer, so the locked result below is
        // not vacuously empty.
        render(&mut recorder, &mut state, &stats, &screenshots);
        assert_eq!(recorder.cursor_drawn, vec![true]);
        recorder.cursor_drawn.clear();

        let effects = state.lock.apply(LockEvent::LockRequested {
            locker: ClientId::new(1),
        });
        state.apply_lock(effects);
        render(&mut recorder, &mut state, &stats, &screenshots);
        assert_eq!(state.lock_state(), LockState::Locked);
        assert_eq!(
            recorder.cursor_drawn,
            vec![false],
            "a locked output must not draw a pointer over the cover"
        );
    }

    /// A View the GPU path draws is a texture the renderer keeps for as long as a frame names it alive. The cover a
    /// locked output shows names none, so the renderer gives every View's texture back at the lock: nothing the desktop
    /// showed is left in GPU memory behind the cover for whatever draws next.
    #[test]
    fn a_locked_output_names_no_view_alive_so_the_renderer_gives_each_texture_back() {
        use gnoblin_render::NodeSpec;
        let (backend, mut state) = session();
        let mut recorder = CursorRecorder {
            outputs: Backend::<State>::outputs(&backend).to_vec(),
            cursor_drawn: Vec::new(),
            views: Vec::new(),
            released: 0,
        };
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        state.composer.set_gpu_views(true);
        state.composer.with_scene(|scene| {
            let (source, view) = (gnoblin_types::NodeId::new(10), gnoblin_types::NodeId::new(11));
            scene
                .insert(source, NodeSpec::new(Rect::new(0, 0, 32, 32)))
                .expect("a source");
            scene
                .insert(view, NodeSpec::new(Rect::new(0, 0, 32, 32)))
                .expect("a view");
            scene.set_class(view, Some("view".to_owned())).expect("the class");
            scene.set_source(view, Some(source)).expect("the source");
        });

        // Unlocked, the frame names the View alive, so the locked result below is not vacuously empty.
        render(&mut recorder, &mut state, &stats, &screenshots);
        assert!(
            recorder.views[0] > 0,
            "an unlocked frame names the View: {:?}",
            recorder.views
        );
        recorder.views.clear();
        assert_eq!(recorder.released, 0, "an unlocked session keeps its View textures");

        let effects = state.lock.apply(LockEvent::LockRequested {
            locker: ClientId::new(1),
        });
        state.apply_lock(effects);
        render(&mut recorder, &mut state, &stats, &screenshots);
        assert_eq!(state.lock_state(), LockState::Locked);
        assert_eq!(
            recorder.views,
            vec![0],
            "the cover names no View, so the renderer drops every View texture at the lock"
        );
        assert_eq!(
            recorder.released, 1,
            "and the loop asks the backend to give back the textures it holds for readbacks"
        );
    }

    #[test]
    fn a_failed_protection_draw_does_not_prove_the_cover() {
        let (mut backend, mut state) = session();
        let client = ClientId::new(7);
        let effects = state
            .session_host
            .protection
            .apply(gnoblin_lock::ProtectionEvent::Protect { client });
        state.apply_protection(effects);
        let effects = state
            .session_host
            .protection
            .apply(gnoblin_lock::ProtectionEvent::Acknowledged {
                client,
                serial: state.session_host.protection.generation(),
            });
        state.apply_protection(effects);
        assert_eq!(
            state.session_host.protection.state(client),
            gnoblin_lock::ProtectionState::Covering,
            "the session's own acknowledgement is not the host's proof"
        );

        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        let mut undrawable = Undrawable {
            outputs: Backend::<State>::outputs(&backend).to_vec(),
            frame: OutputFrame::new(),
        };
        render(&mut undrawable, &mut state, &stats, &screenshots);
        assert_eq!(
            state.session_host.protection.state(client),
            gnoblin_lock::ProtectionState::Covering,
            "a frame that was described but not drawn proves nothing"
        );

        // The cover is still owed, so the loop describes it again next turn and the
        // next frame that draws proves it.
        state.poll_hosted();
        render(&mut backend, &mut state, &stats, &screenshots);
        assert_eq!(
            state.session_host.protection.state(client),
            gnoblin_lock::ProtectionState::Covered
        );
    }

    #[test]
    fn an_idle_iteration_draws_no_frame() {
        let (mut backend, mut state) = session();
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        // The first iteration draws the output's first frame, because a new output has
        // never been painted.
        render(&mut backend, &mut state, &stats, &screenshots);
        let after_first = stats.lock().expect("the stats lock").clone();
        assert_eq!(after_first.iterations, 1);
        assert_eq!(after_first.frames, 1, "the first frame paints the new output");
        for _ in 0..4 {
            render(&mut backend, &mut state, &stats, &screenshots);
        }
        let after_idle = stats.lock().expect("the stats lock").clone();
        assert_eq!(after_idle.iterations, 5, "the loop kept running");
        assert_eq!(after_idle.frames, 1, "no iteration after the first painted anything");
    }

    #[test]
    fn an_output_resize_draws_the_whole_new_output() {
        let (mut backend, mut state) = session();
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        render(&mut backend, &mut state, &stats, &screenshots);
        let output = Backend::<State>::outputs(&backend)[0].id;

        // A resize replaces the output's buffer, so the next frame has to paint all of
        // it: the old pixels are gone.
        backend
            .resize(
                output,
                gnoblin_output::DisplayMode::new(64, 48, 60_000),
                gnoblin_types::Scale::integral(1),
            )
            .expect("the mode is valid");
        // The loop does this on a backend event; here the test is the loop.
        state.sync_outputs(Backend::<State>::outputs(&backend));
        render(&mut backend, &mut state, &stats, &screenshots);

        let stats = stats.lock().expect("the stats lock").clone();
        assert_eq!(stats.frames, 2, "the resize drew a second frame");
        assert_eq!(
            stats.damage,
            vec![Rect::new(0, 0, 64, 48)],
            "the whole new output is damaged"
        );
        let image = backend
            .capture(output, false)
            .expect("a readback of the resized output");
        assert_eq!(image.size(), gnoblin_types::Size::new(64, 48));
        assert_eq!(
            image.colour_at(0, 0),
            Some([0, 0, 0, 255]),
            "the new output is painted, not left transparent"
        );
    }

    #[test]
    fn a_frame_that_cannot_be_drawn_does_not_stop_the_loop() {
        let headless = Headless::new();
        let mut backend = Failing {
            outputs: Backend::<State>::outputs(&headless).to_vec(),
        };
        let display: Display<State> = Display::new().expect("a display");
        let outputs = backend.outputs.clone();
        let mut state = State::new(
            display,
            &outputs,
            "seat0",
            WmOptions::default(),
            None,
            None,
            crate::tree_frame::TreeHooks::default(),
            &gnoblin_config::Protocols::default(),
            crate::protocols::extension::TitlebarActions::default(),
            crate::server::CursorSettings::default(),
            crate::server::VisualBellSettings::default(),
            crate::server::VisualAccessibilitySettings::default(),
            &[],
            crate::lock_auth::LockAuth::new(std::sync::Arc::new(gnoblin_lock_auth::FakeAuthenticator::new()), None).0,
        )
        .expect("a state for the default output");
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        for _ in 0..3 {
            render(&mut backend, &mut state, &stats, &screenshots);
        }
        let stats = stats.lock().expect("the stats lock").clone();
        assert_eq!(stats.iterations, 3, "the loop ran every iteration");
        assert_eq!(stats.frames, 0, "no frame was drawn");
    }

    #[test]
    fn a_screenshot_request_is_answered_with_pixels() {
        let slot = Arc::new(ShotSlot::default());
        let server = Arc::clone(&slot);
        let waiter = std::thread::spawn(move || slot.wait(SCREENSHOT_TIMEOUT));
        server.serve(Some(Image::blank(gnoblin_types::Size::new(2, 2))));
        let image = waiter.join().expect("the waiter finished");
        assert_eq!(
            image.map(|image| image.size()),
            Some(gnoblin_types::Size::new(2, 2)),
            "the pixels the loop served are the pixels the caller gets"
        );
    }

    #[test]
    fn a_screenshot_the_loop_cannot_answer_returns_none_rather_than_waiting() {
        let slot = Arc::new(ShotSlot::default());
        let server = Arc::clone(&slot);
        let waiter = std::thread::spawn(move || slot.wait(SCREENSHOT_TIMEOUT));
        server.serve(None);
        assert_eq!(waiter.join().expect("the waiter finished"), None);
    }

    /// Queues a screenshot request the way [`Screenshots::request`] does, without waiting, so a test can run the loop
    /// between the request and its answer.
    fn queue(screenshots: &Screenshots, output: OutputId, guarded: bool) -> Arc<ShotSlot> {
        let slot = Arc::new(ShotSlot::default());
        screenshots.queue.lock().expect("the queue").push_back(Shot {
            output,
            display: false,
            guarded,
            slot: Arc::clone(&slot),
        });
        slot
    }

    /// Failure mode 4: the caller checked the lock from a snapshot, the lock engaged, and the loop then read the desktop.
    /// A guarded request is asked again where the pixels are read: it is answered while the session is unlocked, refused
    /// by name once the lock is requested (with nothing read), and a plain request, which the harness uses to look at the
    /// cover itself, is still answered.
    #[test]
    fn a_guarded_screenshot_is_refused_when_the_loop_finds_the_session_covered() {
        let (mut backend, mut state) = session();
        let output = Backend::<State>::outputs(&backend)[0].id;
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        let wait = Duration::from_millis(50);

        let open = queue(&screenshots, output, true);
        render(&mut backend, &mut state, &stats, &screenshots);
        assert!(
            open.wait_guarded(wait).is_ok(),
            "an unlocked session answers a guarded request"
        );

        // The request was queued while the session was unlocked; the lock engages before the loop reads.
        let guarded = queue(&screenshots, output, true);
        let plain = queue(&screenshots, output, false);
        let effects = state.lock.apply(LockEvent::LockRequested {
            locker: ClientId::new(1),
        });
        state.apply_lock(effects);
        assert_eq!(state.lock_state(), LockState::Locking);
        render(&mut backend, &mut state, &stats, &screenshots);
        assert!(
            matches!(guarded.wait_guarded(wait), Err(ShotRefusal::Locked)),
            "the loop refused the read: the session was covered"
        );
        assert!(
            plain.wait(wait).is_some(),
            "the harness's own screenshot is not guarded"
        );
    }

    #[test]
    fn stats_count_an_iteration_without_a_frame() {
        let mut stats = FrameStats::default();
        stats.record(false, &[]);
        assert_eq!(stats.iterations, 1);
        assert_eq!(stats.frames, 0, "an idle iteration drew nothing");
        stats.record(true, &[Rect::new(0, 0, 4, 4)]);
        assert_eq!(stats.frames, 1);
        assert_eq!(stats.damage, vec![Rect::new(0, 0, 4, 4)]);
        stats.record(false, &[]);
        assert_eq!(
            stats.damage,
            vec![Rect::new(0, 0, 4, 4)],
            "an idle iteration keeps the last damage"
        );
    }

    #[test]
    fn a_screenshot_that_gives_up_leaves_no_request_in_the_queue() {
        // The loop is stalled: nothing serves the queue, so a caller waits out its deadline. The request it queued must
        // go with it, or the loop serves a full readback to nobody when it wakes, once per request that ever timed out.
        let screenshots = Screenshots::default();
        let (ping, _source) = smithay::reexports::calloop::ping::make_ping().expect("a ping");
        for _ in 0..4 {
            let image = screenshots.ask_within(OutputId::new(1), false, &ping, Duration::from_millis(1));
            assert!(image.is_none(), "nothing answered");
        }
        let queued = screenshots.queue.lock().expect("the queue lock").len();
        assert_eq!(queued, 0, "every request that gave up withdrew itself");
    }

    #[test]
    fn a_screenshot_request_past_the_cap_is_refused_at_once() {
        let screenshots = Screenshots::default();
        let (ping, _source) = smithay::reexports::calloop::ping::make_ping().expect("a ping");
        {
            let mut queue = screenshots.queue.lock().expect("the queue lock");
            for _ in 0..MAX_PENDING_SHOTS {
                queue.push_back(Shot {
                    output: OutputId::new(1),
                    display: false,
                    guarded: false,
                    slot: Arc::new(ShotSlot::default()),
                });
            }
        }
        let asked = Instant::now();
        let image = screenshots.ask_within(OutputId::new(1), false, &ping, Duration::from_secs(5));
        assert!(image.is_none(), "a full queue refuses");
        assert!(
            asked.elapsed() < Duration::from_secs(1),
            "the refusal does not wait the deadline out: {:?}",
            asked.elapsed()
        );
        let queued = screenshots.queue.lock().expect("the queue lock").len();
        assert_eq!(queued, MAX_PENDING_SHOTS, "the refused request was not queued");
    }

    #[test]
    fn an_output_that_goes_away_takes_the_frame_capture_kept_of_it() {
        let (mut backend, mut state) = session();
        let handle = backend.handle();
        let stats = Arc::new(Mutex::new(FrameStats::default()));
        let screenshots = Screenshots::default();
        handle
            .connect_output(
                "Virtual-2",
                gnoblin_output::DisplayMode::new(64, 48, 60_000),
                gnoblin_types::Scale::integral(1),
            )
            .expect("a second output is queued");
        Backend::<State>::dispatch(&mut backend, &mut state);
        state.sync_outputs(Backend::<State>::outputs(&backend));
        render(&mut backend, &mut state, &stats, &screenshots);
        assert_eq!(
            state.capture_cached_outputs(),
            2,
            "each output that presented a frame has one kept for capture"
        );
        let second = Backend::<State>::outputs(&backend)[1].id;
        assert!(
            state.capture_cache_bytes(second) > 0,
            "the second output's frame has bytes"
        );

        handle.disconnect_output(second);
        Backend::<State>::dispatch(&mut backend, &mut state);
        state.sync_outputs(Backend::<State>::outputs(&backend));
        render(&mut backend, &mut state, &stats, &screenshots);
        assert_eq!(
            state.capture_cached_outputs(),
            1,
            "the frame kept for an output the session no longer has is released with it"
        );
        assert_eq!(state.capture_cache_bytes(second), 0);
    }
}
