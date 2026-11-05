//! `wl_compositor`, `wl_subcompositor` and `wl_shm`.
//!
//! Surfaces, subsurfaces and shared-memory buffers are Smithay's job once the
//! handlers below exist: the compositor keeps surface state, tracks the buffer tree
//! and completes frame callbacks. What this module adds is the one step a compositor
//! owns — a buffer commit means the client has content, so the window it belongs to
//! is now mapped and the policy hears about a title or application id change.
//!
//! ## What a commit does
//!
//! 1. `on_commit_buffer_handler` applies the pending surface state and, for a
//!    viewport, validates the source rectangle (a `wp_viewporter` mistake is a
//!    protocol error for that client, not for the session).
//! 2. The popup manager is told, so a popup that was waiting for a parent is
//!    tracked once it has one.
//! 3. A lock surface's commit is reported to the session lock, which is what makes
//!    it a destination for input once it has a buffer.
//! 4. Only for a non-synchronous subsurface, the commit is attributed to its root
//!    surface, which is the toplevel the window manager knows.

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::xwayland::XWaylandClientData;

use crate::State;

/// Data every connected client carries.
///
/// Smithay needs a `CompositorClientState` per client to hold its own view of that
/// client's surface state. A client inserted without this data is a bug in the
/// server, which is why [`CompositorHandler::client_compositor_state`] can rely on
/// it existing: a client's own requests cannot reach that path.
#[derive(Debug, Default)]
pub struct ClientState {
    /// Smithay's per-client compositor state.
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Xwayland is inserted with Smithay's own client data, because its compositor
        // state is where the client scale and the X11 surface association live; every
        // other client carries this server's [`ClientState`].
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        &client
            .get_data::<XWaylandClientData>()
            .expect("every client is inserted with its compositor state")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        self.popup_committed(surface);
        // A lock surface with a buffer is the only surface input may reach while the
        // session is covered, so the lock hears about it before anything else runs.
        self.lock_surface_committed(surface);
        if is_sync_subsurface(surface) {
            // A synchronous subsurface's commit is applied with its parent's, so
            // attributing it here would report a window change twice.
            return;
        }
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        self.toplevel_committed(&root);
    }
}

/// A buffer is gone: Smithay has already released whatever it tracked.
impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
