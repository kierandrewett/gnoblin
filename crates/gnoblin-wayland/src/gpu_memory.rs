//! The memory an output holds, published on its screen node (`gpu_memory_*`, design/gpu-memory.md).
//!
//! The numbers come from two places that only this crate sees together: the renderer's own footprint (the backend
//! states it) and the frames capture keeps. They are published at most once a period (a second by default,
//! [`crate::Config::gpu_memory_period`]), and the tree announces a value only when it differs from the node's, so a
//! steady session publishes nothing. The first publication comes a period after the session starts. A publication is a
//! change batch and advances the tree's serial, so a test that reads the serial sets the period to zero, which never
//! publishes.
//!
//! The same module holds the budget: [`Ledger`] counts what each client holds and decides a request
//! (`gpu_memory/ledger.rs`), and the methods of [`State`] below measure what the renderers and capture hold, give
//! caches back when a client's buffer needs the room, and refuse with `wl_display.error.no_memory` when giving back
//! is not enough (design/gpu-memory.md, section 6).
//!
//! # Failure modes
//!
//! 1. **Eviction that makes the loop spin.** Releasing View pictures damages what drew them, so a budget that is
//!    always exceeded and evicts on every frame would repaint on every frame. Eviction is demand-driven instead: it
//!    runs only for the bytes a request that was just admitted needs, so a quiet session at its budget does nothing.
//! 2. **A frame cache that is taken back and made again at once.** The cache is evicted only while nothing reads it,
//!    and it is made again by a session or a screencopy frame that needs it, or by the budget having room for all of it
//!    (checked once a frame, after the frame, so the room is the one the new buffer left).
//! 3. **Evicting what is on screen.** The output target, the staging a frame in flight reads and the frame being
//!    scanned out are never counted as reclaimable, and while any lock is up no View picture is.

use std::any::Any;
use std::ffi::CString;
use std::time::{Duration, Instant};

use gnoblin_backend::Backend;
use gnoblin_backend::render::compose::SessionMemory;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::__interfaces::WL_DISPLAY_INTERFACE;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};

use crate::State;
use crate::log;

mod ledger;

use ledger::HeldMemory;
pub use ledger::{
    ClientMemory, DEFAULT_BUDGET_BYTES, DEFAULT_CLIENT_BYTES, DEFAULT_CLIENT_SURFACES, Ledger, LedgerSnapshot,
    MemoryAdmission, MemoryLimits, MemoryRefusal,
};

/// Gives back what a destroyed object cost the ledger: a `wl_buffer`'s bytes or a `wl_surface`'s count. Any other
/// object costs nothing.
///
/// Called for every object of every client as it is destroyed, by the request that destroys it or by its client going
/// away, so a client that leaves releases everything it held.
pub(crate) fn release<I: Resource + 'static>(state: &State, client: &ClientId, resource: &I) {
    let any: &dyn Any = resource;
    if let Some(buffer) = any.downcast_ref::<WlBuffer>() {
        state.gpu_ledger.release_buffer(client, buffer.id().protocol_id());
    } else if let Some(surface) = any.downcast_ref::<WlSurface>() {
        state.gpu_ledger.release_surface(client, surface.id().protocol_id());
    }
}

/// `wl_display.error.no_memory`: the server cannot hold what the client asked for. It is the error the protocol
/// names for this and it ends that client's connection, never the compositor.
const NO_MEMORY: u32 = 2;

/// How often the numbers are published by default: a staging buffer or a snapshot changes with every drawn frame, and a
/// tree change is a message to every observer.
pub(crate) const DEFAULT_PERIOD: Duration = Duration::from_secs(1);

/// When the next publication is due.
#[derive(Debug)]
pub(crate) struct Publisher {
    period: Duration,
    next: Option<Instant>,
}

impl Publisher {
    /// A publisher that publishes every `period`, or never when it is zero.
    pub(crate) fn new(period: Duration) -> Self {
        Self { period, next: None }
    }

    /// Whether a publication is due at `now`. The first call starts the period and answers no.
    fn due(&mut self, now: Instant) -> bool {
        if self.period.is_zero() {
            return false;
        }
        match self.next {
            None => {
                self.next = Some(now + self.period);
                false
            }
            Some(next) if now >= next => {
                self.next = Some(now + self.period);
                true
            }
            Some(_) => false,
        }
    }
}

impl State {
    /// Publishes, for every output, the bytes its renderer holds and the bytes capture keeps of its last frame.
    pub(crate) fn publish_gpu_memory(&mut self, backend: &dyn Backend<State>) {
        if !self.gpu_memory.due(Instant::now()) {
            return;
        }
        let (clients, refused, evicted) = self.gpu_ledger.published();
        let session = SessionMemory {
            clients,
            budget: self.gpu_ledger.limits().budget_bytes,
            refused,
            evicted,
        };
        for output in self.output_ids() {
            let footprint = backend.footprint(output);
            let capture = self.capture_cache_bytes(output);
            self.composer.set_output_memory(output, &footprint, capture, &session);
        }
    }

    /// Reads what the renderers and capture hold and how much of it may be given back, for the admissions that follow.
    ///
    /// Called once a frame and whenever eviction has changed something. It reads, it does not change.
    pub(crate) fn measure_gpu_memory(&mut self, backend: &dyn Backend<State>) {
        let locked = self.lock_is_up();
        let mut held = HeldMemory::default();
        for output in self.output_ids() {
            let footprint = backend.footprint(output);
            held.renderer = held.renderer.saturating_add(footprint.total());
            // A View picture is evictable unless a lock is up: the lock card and the desktop it hides are drawn from
            // them, and a frame between the release and the redraw must not show a hole.
            let views = if locked { 0 } else { footprint.views };
            held.reclaimable = held
                .reclaimable
                .saturating_add(footprint.releasable)
                .saturating_add(views);
            held.capture = held.capture.saturating_add(self.capture_cache_bytes(output));
        }
        held.reclaimable = held.reclaimable.saturating_add(self.capture_cache_reclaimable());
        self.gpu_ledger.observe(held);
    }

    /// Gives caches back for the bytes the admissions of this turn counted on, and measures what is held now.
    ///
    /// Called by the loop where it holds the backend: before a frame is drawn, and before a client's GPU buffer is
    /// imported. The order is the cheapest to remake first: the frame capture keeps (the next presented frame fills it
    /// again, and only if something reads it), then the View pictures (drawn again from the scene), then the blur
    /// scratch and the software renderer's snapshots (made again by the next draw).
    pub(crate) fn settle_gpu_memory(&mut self, backend: &mut dyn Backend<State>) {
        self.measure_gpu_memory(&*backend);
        let Some(needed) = self.gpu_ledger.take_need() else {
            return;
        };
        let locked = self.lock_is_up();
        let before = self.gpu_ledger.snapshot().used();
        let freed_by = |state: &Self| before.saturating_sub(state.gpu_ledger.snapshot().used());
        self.evict_capture_cache();
        self.measure_gpu_memory(&*backend);
        if freed_by(self) < needed && !locked {
            backend.release_views();
            self.measure_gpu_memory(&*backend);
        }
        if freed_by(self) < needed {
            backend.release_scratch();
            self.measure_gpu_memory(&*backend);
        }
        let freed = freed_by(self);
        self.gpu_ledger.note_evicted(freed);
        log::info!(
            area = "memory",
            needed,
            freed,
            "caches were given back to make room for a client's buffer"
        );
    }

    /// Makes the frame cache again when the budget has room for all of it, which is checked after a frame has been
    /// drawn and the clients' buffers are counted.
    pub(crate) fn resume_capture_cache_when_roomy(&mut self) {
        let back = self.capture_cache_suspended();
        if back > 0 && self.gpu_ledger.has_room(back) {
            self.resume_capture_cache();
        }
    }

    /// Ends `client`'s connection with `wl_display.error.no_memory`, naming what it asked for that passed which bound.
    ///
    /// The compositor and every other client go on: a protocol error ends one connection. The client's buffers and
    /// surfaces are released as its objects are destroyed.
    pub(crate) fn refuse_client(&self, client: &Client, refusal: MemoryRefusal) {
        let pid = client
            .get_credentials(&self.display_handle)
            .ok()
            .map(|credentials| credentials.pid);
        log::warn!(area = "memory", ?pid, %refusal, "a client's request is refused: no_memory");
        let backend = self.display_handle.backend_handle();
        let Ok(display) = backend.object_for_protocol_id(client.id(), &WL_DISPLAY_INTERFACE, 1) else {
            log::warn!(area = "memory", "the client's wl_display is gone, so it cannot be told");
            return;
        };
        // A message with a NUL cannot be sent; ours has none, and an empty one is still the right error code.
        let message = CString::new(refusal.to_string()).unwrap_or_default();
        backend.post_error(display, NO_MEMORY, message);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gnoblin_backend::headless::Headless;
    use gnoblin_backend::{BackendError, InputSource, Present};
    use gnoblin_lock::Event as LockEvent;
    use gnoblin_render_frame::footprint::Footprint;
    use gnoblin_render_frame::image::Image;
    use gnoblin_types::{OutputId, Size};
    use smithay::reexports::calloop::LoopHandle;
    use smithay::reexports::wayland_server::Display;

    use gnoblin_wm_api::WmOptions;

    use super::ledger::test_support::clients;
    use super::*;

    #[test]
    fn the_first_publication_waits_a_period_and_later_ones_are_rate_limited() {
        let mut publisher = Publisher::new(DEFAULT_PERIOD);
        let start = Instant::now();
        assert!(!publisher.due(start), "the first call only starts the period");
        assert!(!publisher.due(start + Duration::from_millis(500)));
        assert!(publisher.due(start + DEFAULT_PERIOD));
        assert!(!publisher.due(start + DEFAULT_PERIOD + Duration::from_millis(1)));
        assert!(publisher.due(start + DEFAULT_PERIOD * 2));
    }

    #[test]
    fn a_zero_period_never_publishes() {
        let mut publisher = Publisher::new(Duration::ZERO);
        let start = Instant::now();
        for step in 0..5 {
            assert!(!publisher.due(start + Duration::from_secs(step * 10)));
        }
    }

    /// A backend that reports what a renderer would hold, a frame to keep for capture, and counts what it is told to
    /// give back. Giving back changes what it reports, as a renderer's would.
    struct Holding {
        outputs: Vec<gnoblin_backend::Output>,
        footprint: Footprint,
        views_released: u32,
        scratch_released: u32,
    }

    impl Backend<State> for Holding {
        fn name(&self) -> &'static str {
            "holding"
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

        fn render(&mut self, _host: &mut State, _output: OutputId) -> Result<Present, BackendError> {
            Ok(Present::Nothing)
        }

        fn capture(&mut self, _output: OutputId, with_cursor: bool) -> Option<Image> {
            // 10 x 10 pixels of 4 bytes: 400 bytes. The frame with the cursor composed in is not offered, so the
            // cache holds one picture, as it does where the pointer is not over the output.
            (!with_cursor)
                .then(|| Image::new(Size::new(10, 10), 40, vec![0; 400]))
                .flatten()
        }

        fn dispatch(&mut self, _host: &mut State) -> Vec<gnoblin_backend::BackendEvent> {
            Vec::new()
        }

        fn footprint(&self, _output: OutputId) -> Footprint {
            self.footprint
        }

        fn release_views(&mut self) {
            self.views_released += 1;
            self.footprint.views = 0;
        }

        fn release_scratch(&mut self) {
            self.scratch_released += 1;
            self.footprint.scratch = self.footprint.scratch.saturating_sub(self.footprint.releasable);
            self.footprint.releasable = 0;
        }
    }

    /// A state with one output, a backend that holds 700 bytes (a 400 byte target, 100 releasable bytes of scratch and
    /// 200 of View pictures) and a frame of 400 bytes kept for capture, under a budget of 1500 bytes.
    fn holding() -> (Holding, State) {
        let headless = Headless::new();
        let outputs = Backend::<State>::outputs(&headless).to_vec();
        let mut state = State::new(
            Display::new().expect("a display"),
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
            crate::lock_auth::LockAuth::new(Arc::new(gnoblin_lock_auth::FakeAuthenticator::new()), None).0,
            Box::<gnoblin_wm_api::NullWm>::default(),
        )
        .expect("a state for the default output");
        state.gpu_ledger = Arc::new(Ledger::new(MemoryLimits {
            budget_bytes: 1500,
            client_bytes: 0,
            client_surfaces: 0,
        }));
        let mut backend = Holding {
            outputs,
            footprint: Footprint {
                target: 400,
                scratch: 100,
                views: 200,
                releasable: 100,
                ..Footprint::default()
            },
            views_released: 0,
            scratch_released: 0,
        };
        let output = backend.outputs[0].id;
        state.capture_frame(&mut backend, output);
        state.measure_gpu_memory(&backend);
        (backend, state)
    }

    #[test]
    fn the_measurement_counts_the_renderer_the_capture_frame_and_what_can_be_given_back() {
        let (_backend, state) = holding();
        let snapshot = state.gpu_ledger.snapshot();
        assert_eq!(snapshot.renderer_bytes, 700);
        assert_eq!(snapshot.capture_bytes, 400);
        assert_eq!(
            snapshot.reclaimable_bytes,
            400 + 200 + 100,
            "the frame, the View pictures and the scratch"
        );
    }

    #[test]
    fn caches_are_given_back_cheapest_first_and_only_as_far_as_the_need() {
        let (mut backend, mut state) = holding();
        let ids = clients(1);
        // 1100 held. A buffer of 500 is 100 past the budget; the capture frame (400) covers that alone.
        assert_eq!(
            state.gpu_ledger.admit_buffer(&ids[0], 500),
            Ok(MemoryAdmission::Evict { needed: 100 })
        );
        state.gpu_ledger.record_buffer(&ids[0], 1, 500);
        state.settle_gpu_memory(&mut backend);
        assert_eq!(
            (backend.views_released, backend.scratch_released),
            (0, 0),
            "the frame was enough"
        );
        let snapshot = state.gpu_ledger.snapshot();
        assert_eq!(snapshot.capture_bytes, 0, "the capture frame was given back");
        assert_eq!(snapshot.evicted, 400);

        // 1200 held. A buffer of 600 is 300 past the budget: the frame is gone, the View pictures (200) are not
        // enough, so the scratch (100) goes too.
        assert_eq!(
            state.gpu_ledger.admit_buffer(&ids[0], 600),
            Ok(MemoryAdmission::Evict { needed: 300 })
        );
        state.gpu_ledger.record_buffer(&ids[0], 2, 600);
        state.settle_gpu_memory(&mut backend);
        assert_eq!((backend.views_released, backend.scratch_released), (1, 1));
        let snapshot = state.gpu_ledger.snapshot();
        assert_eq!(
            (snapshot.renderer_bytes, snapshot.evicted),
            (400, 700),
            "only the target is left of the renderer"
        );
        assert!(
            snapshot.used() <= 1500,
            "the budget holds after the eviction: {snapshot:?}"
        );

        // Nothing is left to give: a buffer that does not fit is refused, and nothing is evicted for it.
        assert!(matches!(
            state.gpu_ledger.admit_buffer(&ids[0], 600),
            Err(MemoryRefusal::Budget { .. })
        ));
        assert!(!state.gpu_ledger.needs_room());
    }

    #[test]
    fn a_locked_session_keeps_every_view_picture() {
        let (mut backend, mut state) = holding();
        let ids = clients(1);
        let _effects = state.lock.apply(LockEvent::LockRequested {
            locker: gnoblin_types::ClientId::new(1),
        });
        assert!(state.lock_is_up());
        state.measure_gpu_memory(&backend);
        assert_eq!(
            state.gpu_ledger.snapshot().reclaimable_bytes,
            400 + 100,
            "the View pictures are not reclaimable under a lock"
        );
        // 1100 held. A buffer of 1000 needs 600 back, which the View pictures would make up to 700: it is refused
        // rather than taking them.
        assert!(matches!(
            state.gpu_ledger.admit_buffer(&ids[0], 1000),
            Err(MemoryRefusal::Budget { .. })
        ));
        // A buffer of 850 needs 450: the capture frame (400) is not enough and the scratch (100) is, and the View
        // pictures stay where they are.
        assert_eq!(
            state.gpu_ledger.admit_buffer(&ids[0], 850),
            Ok(MemoryAdmission::Evict { needed: 450 })
        );
        state.settle_gpu_memory(&mut backend);
        assert_eq!(backend.views_released, 0, "the lock's pictures were never released");
        assert_eq!(backend.scratch_released, 1);
        assert_eq!(backend.footprint.views, 200);
    }

    #[test]
    fn a_frame_cache_that_was_given_back_is_made_again_when_the_budget_has_room() {
        let (mut backend, mut state) = holding();
        let ids = clients(1);
        state.gpu_ledger.admit_buffer(&ids[0], 500).expect("fits with eviction");
        state.gpu_ledger.record_buffer(&ids[0], 1, 500);
        state.settle_gpu_memory(&mut backend);
        assert_eq!(state.capture_cache_suspended(), 400);

        // The frame is not read back while there is no room and nothing reads it.
        let output = backend.outputs[0].id;
        state.capture_frame(&mut backend, output);
        assert_eq!(
            state.capture_cache_bytes(output),
            0,
            "a presented frame is not kept for nobody"
        );
        state.resume_capture_cache_when_roomy();
        assert_eq!(
            state.capture_cache_suspended(),
            400,
            "700 and 500 and 400 are over 1500, so no room yet"
        );

        // The client leaves: there is room, the cache is made again by the next frame.
        state.gpu_ledger.release_buffer(&ids[0], 1);
        state.measure_gpu_memory(&backend);
        state.resume_capture_cache_when_roomy();
        assert_eq!(state.capture_cache_suspended(), 0);
        state.capture_frame(&mut backend, output);
        assert_eq!(state.capture_cache_bytes(output), 400);
    }

    #[test]
    fn a_refusal_names_the_error_the_protocol_has_for_it() {
        // `wl_display.error.no_memory` is 2 in wayland.xml; the compositor and the client tests agree on the number.
        assert_eq!(NO_MEMORY, 2);
    }
}
