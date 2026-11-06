//! XML protocol `xdg_dialog_v1` (`<protocol name="xdg_dialog_v1">`); interfaces served: `xdg_wm_dialog_v1`.
//!
//! `xdg_wm_dialog_v1`: a client says a toplevel is a dialog, and whether it is modal.
//!
//! Smithay keeps the hint on the toplevel's role. Each change is forwarded to the window
//! manager through [`State::sync_window_role`], which records the window's kind as
//! [`gnoblin_wm::WindowKind::Modal`] or [`gnoblin_wm::WindowKind::Dialog`] when the window
//! has a parent, and as `Normal` otherwise. The kind is exposed on the session snapshot as
//! `modal`, so a shell can read whether a window asked to be modal.
//!
//! The hint is only honoured for a toplevel with a parent. The protocol makes the hint
//! advisory for a parentless toplevel, so a parentless dialog is a normal window.
//!
//! Modality is enforced by the window manager, which is why this global is advertised: an
//! open modal child takes focus from its parent, and a parent's focus request is redirected
//! to that child until the child is no longer modal. A dialog stays a floating transient
//! above its parent, so it is never tiled over the window it belongs to.
//!
//! # Failure modes
//!
//! 1. **A second dialog object for one toplevel.** Smithay posts
//!    `xdg_wm_dialog_v1.already_used`; the first dialog keeps the toplevel.
//! 2. **A hint outlives its toplevel.** The hint lives on the toplevel's role, so it is
//!    dropped with the toplevel and the window manager forgets the window's kind with it.
//! 3. **A hint with no parent.** The window manager records `Normal`, because a dialog
//!    without a parent has nothing to be modal to.
//! 4. **The parent is blocked only while the modal is open.** Unsetting modality, or
//!    closing the child, releases the parent to take focus again.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::dialog::{ToplevelDialogHint, XdgDialogHandler, XdgDialogState};

use crate::State;

/// Creates the `xdg_wm_dialog_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> XdgDialogState {
    XdgDialogState::new::<State>(display)
}

impl XdgDialogHandler for State {
    fn dialog_hint_changed(&mut self, toplevel: ToplevelSurface, _hint: ToplevelDialogHint) {
        if let Some(window) = self.window_of(toplevel.wl_surface()) {
            self.sync_window_role(window);
        }
    }
}
