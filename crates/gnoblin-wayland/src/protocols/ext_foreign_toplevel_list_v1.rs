//! XML protocol `ext_foreign_toplevel_list_v1` (`<protocol name="ext_foreign_toplevel_list_v1">`); interfaces served: `ext_foreign_toplevel_list_v1`.
//!
//! `ext_foreign_toplevel_list_v1`: the standardised, read-only toplevel list.
//!
//! The standardised list is the read-only half of the wlroots protocol with an
//! identifier a shell can use as a stable key. Its windows come from the same record
//! as [`zwlr_foreign_toplevel_management_v1`](super::zwlr_foreign_toplevel_management_v1),
//! see [`foreign_toplevel_shared`](super::foreign_toplevel_shared).

use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};

use gnoblin_types::WindowId;

use super::foreign_toplevel_shared::{ToplevelChanges, ToplevelState};
use crate::State;

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
pub(super) fn publish(list: &mut ForeignToplevelListState, published: &ToplevelState) -> ForeignToplevelHandle {
    list.new_toplevel_with_identifier::<State>(
        published.title.clone(),
        published.app_id.clone(),
        identifier(published.window),
    )
}

/// Sends the changed title and application id of a window, then one `done`.
///
/// Nothing is sent when nothing changed, so an idle session sends no events here.
pub(super) fn send_changes(handle: &ForeignToplevelHandle, published: &ToplevelState, changes: ToplevelChanges) {
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

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
    }
}
