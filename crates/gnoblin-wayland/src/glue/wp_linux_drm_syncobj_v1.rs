//! `gnoblin-wayland-wp-linux-drm-syncobj-v1`: the global exists only when the backend can keep its promise, because it
//! asks the backend's renderer when the GPU has finished with a buffer.

use gnoblin_backend::{Backend, ExplicitSync};
use gnoblin_wayland_protocols::wp_linux_drm_syncobj_v1::{Host, SyncobjServing};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState};

use crate::State;
use crate::log;

impl Host for State {
    fn syncobj(&mut self) -> &mut SyncobjServing {
        &mut self.syncobj
    }

    fn loop_handle(&self) -> Option<LoopHandle<'static, Self>> {
        self.loop_handle.clone()
    }
}

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.syncobj.state_mut()
    }
}

impl State {
    /// Creates the global when the backend reports a device explicit sync can be served on.
    ///
    /// Called once, before any client connects, so a session that cannot honour a release point never advertises the
    /// protocol.
    pub(crate) fn start_syncobj(&mut self, sync: Option<ExplicitSync>, display: &DisplayHandle) {
        let Some(sync) = sync else {
            log::info!(
                area = "syncobj",
                "the backend cannot honour explicit sync: wp_linux_drm_syncobj_manager_v1 is not served"
            );
            return;
        };
        self.syncobj.start::<State>(display, sync.into_device());
    }

    /// Whether the session serves `wp_linux_drm_syncobj_manager_v1`.
    #[must_use]
    pub fn serves_explicit_sync(&self) -> bool {
        self.syncobj.served()
    }

    /// Signals the release points of the buffers the GPU has finished with, after the turn's frames were drawn.
    pub(crate) fn retire_syncobj(&mut self, backend: &mut dyn Backend<State>) {
        if !self.syncobj.served() {
            return;
        }
        let retirement = backend.buffers_retired();
        self.syncobj.retire(retirement.submitted, retirement.retired);
    }
}
