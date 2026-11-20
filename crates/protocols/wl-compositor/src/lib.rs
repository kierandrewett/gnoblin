//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_compositor`, `wl_subcompositor`.
//!
//! `wl_compositor` and `wl_subcompositor` (XML `wayland.xml`), with the sync-subsurface rules
//! applied in its commit handler. `wl_shm` has a crate of its own.
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
//!
//! The compositor's handler (`CompositorHandler`) stays with the state it answers for: it runs steps 1 to 6 in order, and
//! step 2 is [`verify_buffer`], which is this crate's.

use std::sync::{Arc, OnceLock};

use gnoblin_render_frame::shm::{ShmError, probe};
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{GlobalDispatch, Resource};
use smithay::wayland::GlobalData;
use smithay::wayland::compositor::{CompositorClientState, CompositorState, with_states};

use gnoblin_wayland_api::access::{Access, Registration};
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wl-compositor",
    interfaces: &[
        ServedInterface::new("wl_compositor", 5),
        ServedInterface::new("wl_subcompositor", 1),
    ],
    privilege: Privilege::Core,
};

/// Creates the `wl_compositor` and `wl_subcompositor` globals.
pub fn register<D>(registrar: &Registrar<'_>) -> CompositorState
where
    D: GlobalDispatch<smithay::reexports::wayland_server::protocol::wl_compositor::WlCompositor, GlobalData>
        + GlobalDispatch<smithay::reexports::wayland_server::protocol::wl_subcompositor::WlSubcompositor, GlobalData>
        + 'static,
{
    CompositorState::new::<D>(registrar.display())
}

/// Data every connected client carries.
///
/// Smithay needs a `CompositorClientState` per client to hold its own view of that
/// client's surface state. A client inserted without this data is a bug in the
/// server, which is why [`CompositorHandler::client_compositor_state`] can rely on
/// it existing: a client's own requests cannot reach that path.
///
/// `P` is what the compositor records of a connection's process; the protocol only holds the slot.
///
/// It also carries the connection's [`Registration`], which is how the access roster
/// learns that a connection is gone: dropping this data is the one thing a disconnect
/// reliably does.
#[derive(Debug)]
pub struct WlCompositorClientState<P> {
    /// Smithay's per-client compositor state.
    pub compositor_state: CompositorClientState,
    /// The connection's claim on its access-roster entry.
    pub registration: Registration,
    /// The connection's process, read when it connected; unset where its credentials
    /// could not be read.
    pub process: OnceLock<P>,
}

impl<P> WlCompositorClientState<P> {
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

impl<P: Send + Sync + 'static> ClientData for WlCompositorClientState<P> {}

/// Raises `wl_shm.invalid_fd` on the buffer a surface shows when the client shrank the pool's file under
/// it, which is the only way its pixels can be gone.
///
/// The check reads the last byte of the buffer (`gnoblin_render_frame::shm::probe`), so it costs the same
/// for any size. Without it the first read of the missing pages is a bus error the renderer survives by
/// drawing nothing, and the client is never told.
pub fn verify_buffer(surface: &WlSurface) {
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
