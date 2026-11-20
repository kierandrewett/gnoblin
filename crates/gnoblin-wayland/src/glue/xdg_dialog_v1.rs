//! `gnoblin-wayland-xdg-dialog-v1`: a dialog hint is a role change of the window.

use gnoblin_wayland_protocols::xdg_dialog_v1;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::dialog::{ToplevelDialogHint, XdgDialogHandler};

use crate::State;

impl XdgDialogHandler for State {
    fn dialog_hint_changed(&mut self, toplevel: ToplevelSurface, _hint: ToplevelDialogHint) {
        xdg_dialog_v1::dialog_hint_changed(self, toplevel.wl_surface());
    }
}
