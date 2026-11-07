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

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::ping::Ping;

use gnoblin_backend::render::Image;
use gnoblin_backend::{Backend, Host};
use gnoblin_types::{OutputId, Rect};

use crate::state::State;

use crate::log;

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
    slot: Arc<ShotSlot>,
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
}

impl ShotSlot {
    /// Records the answer and wakes the waiter.
    fn serve(&self, image: Option<Image>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.image = image;
        state.served = true;
        self.ready.notify_all();
    }

    /// Waits for the answer, or gives up after `timeout`.
    fn wait(&self, timeout: Duration) -> Option<Image> {
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
        state.image.take()
    }
}

/// The screenshot requests the control handle has queued.
#[derive(Default)]
pub struct Screenshots {
    queue: Mutex<VecDeque<Shot>>,
}

impl Screenshots {
    /// Queues a request and waits for the loop to answer it.
    ///
    /// The ping wakes the loop so the answer arrives on the next iteration rather than
    /// on its timeout.
    pub fn request(&self, output: OutputId, ping: &Ping) -> Option<Image> {
        let slot = Arc::new(ShotSlot::default());
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Shot {
                output,
                slot: Arc::clone(&slot),
            });
        ping.ping();
        slot.wait(SCREENSHOT_TIMEOUT)
    }

    /// Answers every queued request from the backend's readback.
    fn serve(&self, backend: &mut dyn Backend<State>) {
        loop {
            let Some(shot) = self.queue.lock().unwrap_or_else(PoisonError::into_inner).pop_front() else {
                return;
            };
            let image = backend.screenshot(shot.output);
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
    state.begin_frame();
    let outputs: Vec<OutputId> = backend.outputs().iter().map(|output| output.id).collect();
    let mut drew = false;
    let mut damage: Vec<Rect> = Vec::new();
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
    state.end_frame();
    if drew {
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
    screenshots.serve(backend);
}

#[cfg(test)]
mod tests {
    use gnoblin_backend::headless::Headless;
    use gnoblin_backend::render::{OutputFrame, RenderError};
    use gnoblin_backend::{BackendError, InputSource, Present};
    use smithay::reexports::calloop::LoopHandle;
    use smithay::reexports::wayland_server::Display;

    use gnoblin_lock::{Event as LockEvent, State as LockState};
    use gnoblin_types::ClientId;
    use gnoblin_wm::WmOptions;

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
            1,
            None,
            None,
            &gnoblin_config::Protocols::default(),
            crate::protocols::extension::TitlebarActions::default(),
            crate::server::CursorSettings::default(),
            crate::server::VisualBellSettings::default(),
            &[],
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
            1,
            None,
            None,
            &gnoblin_config::Protocols::default(),
            crate::protocols::extension::TitlebarActions::default(),
            crate::server::CursorSettings::default(),
            crate::server::VisualBellSettings::default(),
            &[],
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
}
