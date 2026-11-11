//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_compositor`, `wl_subcompositor`.
//!
//! `wl_compositor` and `wl_subcompositor` (XML `wayland.xml`), with the sync-subsurface rules
//! applied in its commit handler. `wl_shm` lives in `wl_shm.rs`.
//!
//! Surfaces, subsurfaces and shared-memory buffers are Smithay's job once the
//! handlers below exist: the compositor keeps surface state, tracks the buffer tree
//! and completes frame callbacks. What this module adds is the one step a compositor
//! owns — a buffer commit means the client has content, so the window it belongs to
//! is now mapped and the policy hears about a title or application id change.
//!
//! ## What a commit does
//!
//! 1. `on_commit_buffer_handler` applies the pending surface state.
//! 2. The buffer the surface now shows is checked: a `wl_shm` pool the client shrank under it
//!    is `wl_shm.invalid_fd` on the buffer, and a viewport source outside it is
//!    `wp_viewport.out_of_buffer` (a mistake like these is a protocol error for that client,
//!    not for the session).
//! 3. The popup manager is told, so a popup that was waiting for a parent is
//!    tracked once it has one.
//! 4. A lock surface's commit is reported to the session lock, which is what makes
//!    it a destination for input once it has a buffer.
//! 5. Only for a non-synchronous subsurface, the commit is attributed to its root
//!    surface, which is the toplevel the window manager knows.
//! 6. The window the surface belongs to, through its subsurface parents or its popup root, is marked so its surface
//!    tree (`surface_tree.rs`) is built again before the next frame, and the surface is recorded as having
//!    committed so its pixels are damaged.

use std::sync::{Arc, OnceLock};

use gnoblin_render_frame::shm::{ShmError, probe};
use smithay::backend::renderer::utils::{RendererSurfaceStateUserData, on_commit_buffer_handler};
use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface, with_states,
};
use smithay::xwayland::XWaylandClientData;

use crate::ProcessIdentity;
use crate::State;
use crate::access::{Access, Registration};

/// Data every connected client carries.
///
/// Smithay needs a `CompositorClientState` per client to hold its own view of that
/// client's surface state. A client inserted without this data is a bug in the
/// server, which is why [`CompositorHandler::client_compositor_state`] can rely on
/// it existing: a client's own requests cannot reach that path.
///
/// It also carries the connection's [`Registration`], which is how the access roster
/// learns that a connection is gone: dropping this data is the one thing a disconnect
/// reliably does.
#[derive(Debug)]
pub struct WlCompositorClientState {
    /// Smithay's per-client compositor state.
    pub compositor_state: CompositorClientState,
    /// The connection's claim on its access-roster entry.
    pub registration: Registration,
    /// The connection's process, read when it connected; unset where its credentials
    /// could not be read.
    pub process: OnceLock<ProcessIdentity>,
}

impl WlCompositorClientState {
    /// The data a connection is inserted with.
    #[must_use]
    pub fn new(access: &Arc<Access>) -> Self {
        Self {
            compositor_state: CompositorClientState::default(),
            registration: Registration::new(Arc::clone(access)),
            process: OnceLock::new(),
        }
    }
}

impl ClientData for WlCompositorClientState {}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Xwayland is inserted with Smithay's own client data, because its compositor
        // state is where the client scale and the X11 surface association live; every
        // other client carries this server's [`WlCompositorClientState`].
        if let Some(state) = client.get_data::<WlCompositorClientState>() {
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
        on_commit_buffer_handler::<Self>(surface);
        // A buffer whose pool the client shrank is the client's error; so is a viewport source that reaches
        // past the buffer the commit left attached.
        verify_buffer(surface);
        crate::protocols::wp_viewporter::validate_commit(surface);
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

/// Raises `wl_shm.invalid_fd` on the buffer a surface shows when the client shrank the pool's file under
/// it, which is the only way its pixels can be gone.
///
/// The check reads the last byte of the buffer (`gnoblin_render_frame::shm::probe`), so it costs the same
/// for any size. Without it the first read of the missing pages is a bus error the renderer survives by
/// drawing nothing, and the client is never told.
fn verify_buffer(surface: &WlSurface) {
    let buffer = with_states(surface, |states| {
        let renderer = states.data_map.get::<RendererSurfaceStateUserData>()?;
        let renderer = renderer.lock().ok()?;
        renderer.buffer().cloned()
    });
    let Some(buffer) = buffer else {
        return;
    };
    if probe(&buffer) == Err(ShmError::Unmapped) {
        buffer.post_error(
            wl_shm::Error::InvalidFd,
            "the buffer's pool no longer holds all of the buffer",
        );
    }
}
