//! Checks a client's request has to pass before Smithay handles it.
//!
//! [`crate::dispatch`] hands every request of every protocol object to Smithay's handler. Most of
//! what the protocols forbid is refused in there. A few rules are not: Smithay accepts the request
//! and raises the error later, or never. A rule that Smithay's handler cannot be given (it is a
//! method on a type this crate does not own) is written here and applied by the generic dispatch,
//! which asks [`admits`] first and drops a request it refuses. The request's protocol error is
//! posted here, so the client is disconnected with the error the protocol names.
//!
//! # Failure modes
//!
//! 1. **A surface that cannot take an `xdg_surface`.** `xdg_wm_base.get_xdg_surface` on a `wl_surface`
//!    with another role is `xdg_wm_base.role`, and on one with a buffer attached or committed is
//!    `xdg_wm_base.invalid_surface_state`. Smithay raises the first only at `get_toplevel` and the
//!    second at commit, so a client that creates the object and never goes on is never told.
//!    Tests: the wlcs `XdgSurfaceStableTest` cases `creating_xdg_surface_from_wl_surface_with_*`.
//! 2. **A rule that costs every request.** Which object and which request a check is for is decided
//!    from the types, so a request that has no check pays two type comparisons and nothing else.

use std::any::Any;

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_wm_base;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Resource, protocol::wl_buffer::WlBuffer};
use smithay::wayland::compositor::{BufferAssignment, SurfaceAttributes, SurfaceData, get_role, with_states};
use smithay::wayland::shell::xdg::{XDG_POPUP_ROLE, XDG_TOPLEVEL_ROLE};

/// Whether `request` on `resource` may go on to Smithay's handler. A request that is refused has
/// already had its protocol error posted.
pub(crate) fn admits<I>(resource: &I, request: &I::Request) -> bool
where
    I: Resource + 'static,
    I::Request: 'static,
{
    let request: &dyn Any = request;
    let resource: &dyn Any = resource;
    match (
        resource.downcast_ref::<xdg_wm_base::XdgWmBase>(),
        request.downcast_ref::<xdg_wm_base::Request>(),
    ) {
        (Some(wm_base), Some(xdg_wm_base::Request::GetXdgSurface { surface, .. })) => {
            match refuse_xdg_surface(surface) {
                Some((error, message)) => {
                    wm_base.post_error(error, message);
                    false
                }
                None => true,
            }
        }
        _ => true,
    }
}

/// Why `surface` cannot have an `xdg_surface`, if it cannot.
///
/// A surface that already has an xdg role keeps it, so a new `xdg_surface` on it is not a different
/// role. A buffer counts whether it is only attached or already committed.
fn refuse_xdg_surface(surface: &WlSurface) -> Option<(xdg_wm_base::Error, String)> {
    let id = surface.id();
    if let Some(role) = get_role(surface)
        && role != XDG_TOPLEVEL_ROLE
        && role != XDG_POPUP_ROLE
    {
        return Some((
            xdg_wm_base::Error::Role,
            format!("{id} already has the role {role}, so it cannot have an xdg_surface"),
        ));
    }
    has_buffer(surface).then(|| {
        (
            xdg_wm_base::Error::InvalidSurfaceState,
            format!("{id} already has a buffer attached or committed, so it cannot have an xdg_surface"),
        )
    })
}

/// Whether a surface has a buffer attached and not yet committed, or one it shows.
fn has_buffer(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        let attached = states
            .cached_state
            .get::<SurfaceAttributes>()
            .pending()
            .buffer
            .as_ref()
            .is_some_and(|buffer| matches!(buffer, BufferAssignment::NewBuffer(_)));
        attached || shown_buffer(states).is_some()
    })
}

/// The buffer the surface's last commit left it showing.
fn shown_buffer(states: &SurfaceData) -> Option<WlBuffer> {
    let renderer = states.data_map.get::<RendererSurfaceStateUserData>()?;
    let renderer = renderer.lock().ok()?;
    renderer.buffer().map(|buffer| (**buffer).clone())
}
