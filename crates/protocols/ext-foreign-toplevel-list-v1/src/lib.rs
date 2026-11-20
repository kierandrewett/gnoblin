//! XML protocol `ext_foreign_toplevel_list_v1` (`<protocol name="ext_foreign_toplevel_list_v1">`); interfaces served: `ext_foreign_toplevel_list_v1`.
//!
//! `ext_foreign_toplevel_list_v1`: the standardised, read-only toplevel list.
//!
//! The standardised list is the read-only half of the wlroots protocol with an
//! identifier a shell can use as a stable key. Its windows come from the same record
//! as [`zwlr_foreign_toplevel_management_v1`](super::zwlr_foreign_toplevel_management_v1),
//! see [`foreign_toplevel_shared`](super::foreign_toplevel_shared).

use gnoblin_types::WindowId;
use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use gnoblin_wayland_foreign_toplevel_shared::{ToplevelChanges, ToplevelState};
use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::{
    ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
};
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListGlobalData, ForeignToplevelListHandler, ForeignToplevelListState,
};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "ext-foreign-toplevel-list-v1",
    interfaces: &[ServedInterface::new("ext_foreign_toplevel_list_v1", 1)],
    privilege: Privilege::Roster(Global::ForeignToplevelList),
};

/// Creates the `ext_foreign_toplevel_list_v1` global, visible to the clients the roster calls privileged.
pub fn register<D>(registrar: &Registrar<'_>) -> ForeignToplevelListState
where
    D: ForeignToplevelListHandler + GlobalDispatch<ExtForeignToplevelListV1, ForeignToplevelListGlobalData>,
{
    ForeignToplevelListState::new_with_filter::<D>(registrar.display(), registrar.filter(Global::ForeignToplevelList))
}

/// The identifier a window is published under.
///
/// Derived from the window identity rather than random: a shell that reconnects sees
/// the same key for the same window, which is what makes it useful as a map key, and
/// a test can predict it. Sixteen hex digits keep it inside the protocol's 32-character
/// printable-ascii limit.
fn identifier(window: WindowId) -> String {
    format!("gnoblin-{:016x}", window.get())
}

/// Publishes a new window on the standardised list and returns its handle.
pub fn publish<D>(list: &mut ForeignToplevelListState, published: &ToplevelState) -> ForeignToplevelHandle
where
    D: ForeignToplevelListHandler + Dispatch<ExtForeignToplevelHandleV1, ForeignToplevelHandle>,
{
    list.new_toplevel_with_identifier::<D>(
        published.title.clone(),
        published.app_id.clone(),
        identifier(published.window),
    )
}

/// Sends the changed title and application id of a window, then one `done`.
///
/// Nothing is sent when nothing changed, so an idle session sends no events here.
pub fn send_changes(handle: &ForeignToplevelHandle, published: &ToplevelState, changes: ToplevelChanges) {
    if changes.title {
        handle.send_title(&published.title);
    }
    if changes.app_id {
        handle.send_app_id(&published.app_id);
    }
    if changes.any() {
        handle.send_done();
    }
}
