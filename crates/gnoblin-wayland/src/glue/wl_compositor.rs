//! `gnoblin-wayland-wl-compositor`: the commit path runs here, against the state it changes.

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
};
use smithay::xwayland::XWaylandClientData;

use crate::ProcessIdentity;
use crate::State;

/// What every connection of this session carries: the compositor's per-client state and the process it came from.
pub(crate) type ClientState = gnoblin_wayland_protocols::wl_compositor::WlCompositorClientState<ProcessIdentity>;

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Xwayland is inserted with Smithay's own client data, because its compositor
        // state is where the client scale and the X11 surface association live; every
        // other client carries this server's [`WlCompositorClientState`].
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        &client
            .get_data::<XWaylandClientData>()
            .expect("every client is inserted with its compositor state")
            .compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        // A commit that attaches a dmabuf waits for the buffer's fences: see the protocol module.
        crate::protocols::zwp_linux_dmabuf_v1::hold_commit_until_ready(surface);
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        // A touch point on a surface cannot outlive it, whether it was a toplevel or a subsurface.
        self.end_touches_on(surface);
        // A subsurface or a popup that goes leaves its window's tree without a commit to say so.
        self.mark_all_surface_trees_dirty();
    }

    fn commit(&mut self, surface: &WlSurface) {
        // Positions the surface's parents asked for land with the state that has just been applied, ahead of
        // everything below that reads where a subsurface is.
        crate::subsurface_position::applied(surface);
        on_commit_buffer_handler::<Self>(surface);
        // A buffer whose pool the client shrank is the client's error; so is a viewport source that reaches
        // past the buffer the commit left attached.
        gnoblin_wayland_protocols::wl_compositor::verify_buffer(surface);
        gnoblin_wayland_protocols::wp_viewporter::validate_commit(surface);
        self.popups.commit(surface);
        self.popup_committed(surface);
        // The window the surface belongs to draws its surface tree again, and this surface's pixels with it.
        self.surface_committed(surface);
        // A lock surface with a buffer is the only surface input may reach while the
        // session is covered, so the lock hears about it before anything else runs.
        self.lock_surface_committed(surface);
        if is_sync_subsurface(surface) {
            // A synchronous subsurface's commit is applied with its parent's, so
            // attributing it here would report a window change twice.
            return;
        }
        // A layer surface is not a toplevel and is not a parent of one, so it is
        // recomputed before the window path: a bar's commit changes its rectangle, its
        // exclusive zone and possibly the work area, never a window's state.
        self.layer_surface_committed(surface);
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        self.toplevel_committed(&root);
    }
}
