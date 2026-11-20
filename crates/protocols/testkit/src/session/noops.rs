//! The objects a test binds and never reads from: their events are ignored, and `ext_session_lock_surface_v1` is acknowledged.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wayland_client::protocol::{wl_subcompositor, wl_subsurface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols_plasma::appmenu::client::{
    org_kde_kwin_appmenu::OrgKdeKwinAppmenu, org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use super::App;

wayland_client::delegate_noop!(App: ignore OrgKdeKwinAppmenuManager);
wayland_client::delegate_noop!(App: ignore OrgKdeKwinAppmenu);
wayland_client::delegate_noop!(App: ignore ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(App: ignore ZwlrVirtualPointerV1);
wayland_client::delegate_noop!(App: ignore wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(App: ignore wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(App: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for App {
    fn event(
        _: &mut Self,
        surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, .. } = event {
            surface.ack_configure(serial);
        }
    }
}

/// Whether a lock object told its client `locked`.
pub type Locked = Arc<AtomicBool>;

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, Locked> for App {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        locked: &Locked,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            locked.store(true, Ordering::SeqCst);
        }
    }
}
