//! XML protocol `fifo_v1` (`<protocol name="fifo_v1">`); interfaces served: `wp_fifo_manager_v1`.
//!
//! `wp_fifo_v1`: a client orders its commits, so each one is shown before the next.
//!
//! A client that sets a barrier and then waits on it does not commit the next frame
//! until the compositor has shown the one before. Smithay holds a waiting commit behind
//! a blocker and records the barrier each commit set; the compositor owes two things:
//! release a barrier once its frame is on the way, and release the blocker that barrier
//! held. This module does both from the frame loop, at the same point frame callbacks go
//! out, so a barrier is released exactly when the client would have had a frame answer.
//!
//! The release does not depend on the window being drawn. A window that is covered or
//! off-screen is still mapped, still has a barrier, and still has to be released, or its
//! client waits for ever. So the pass walks every mapped window, not the drawn ones.
//!
//! # Failure modes
//!
//! 1. **A barrier nobody releases.** The client waits on its next commit for ever. The
//!    pass visits every mapped window on each frame tick, so a barrier set on any mapped
//!    surface is released on the next tick.
//! 2. **A barrier on an unmapped window.** Not released here: an unmapped surface has no
//!    frame tick to answer, and it is released when it maps and the next tick runs.
//! 3. **A surface destroyed with a barrier set.** The barrier is dropped with the surface
//!    state; the client's fifo object is gone with it, so nothing waits on it.
//! 4. **A barrier signalled twice.** Smithay ignores the second signal, so a barrier
//!    that was already released on an earlier tick is harmless to release again.
//! 5. **A surface whose client has gone.** Its commits are gone with it, so there is
//!    nothing to release; the barrier is signalled, which is harmless, and the client
//!    release is skipped because there is no client to release.

use gnoblin_wayland_api::host::CoreHost;
use smithay::reexports::wayland_protocols::wp::fifo::v1::server::wp_fifo_manager_v1::WpFifoManagerV1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch, Resource};
use smithay::wayland::compositor::{self, CompositorClientState, CompositorHandler};
use smithay::wayland::fifo::{FifoBarrierCachedState, FifoManagerData, FifoManagerState};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-fifo-v1",
    interfaces: &[ServedInterface::new("wp_fifo_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// What the barrier release needs of the compositor.
pub trait Host: CoreHost + CompositorHandler + Sized + 'static {
    /// The surfaces of every mapped window, drawn or not.
    fn mapped_surfaces(&self) -> Vec<WlSurface>;
}

/// Creates the `wp_fifo_manager_v1` global. Smithay's managed state places a blocker on a
/// commit that waits on a barrier, so the compositor only releases those.
pub fn register<D>(registrar: &Registrar<'_>) -> FifoManagerState
where
    D: GlobalDispatch<WpFifoManagerV1, FifoManagerData> + Dispatch<WpFifoManagerV1, FifoManagerData> + 'static,
{
    FifoManagerState::new::<D>(registrar.display())
}

/// Releases the fifo barriers of the host's windows.
pub trait FifoExt: Host {
    /// Releases the fifo barrier of every mapped window, and the commits it blocked.
    ///
    /// Called from the frame loop with the frame callbacks. A barrier is taken out of the
    /// surface's current state before it is signalled, so a second pass does not see it.
    fn release_fifo_barriers(&mut self) {
        for surface in self.mapped_surfaces() {
            let barrier = compositor::with_states(&surface, |states| {
                states
                    .cached_state
                    .get::<FifoBarrierCachedState>()
                    .current()
                    .barrier
                    .take()
            });
            let Some(barrier) = barrier else {
                continue;
            };
            barrier.signal();
            let Some(client) = surface.client() else {
                continue;
            };
            let dh = self.display_handle().clone();
            let client_state: &CompositorClientState = self.client_compositor_state(&client);
            client_state.blocker_cleared(self, &dh);
        }
    }
}

impl<T: Host> FifoExt for T {}
