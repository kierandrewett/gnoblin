//! `gnoblin-wayland-org-kde-kwin-server-decoration`: the older decoration protocol, answered by the policy
//! `xdg-decoration` uses.

use gnoblin_wayland_protocols::org_kde_kwin_server_decoration::{self, KdeDecorations};
use gnoblin_wayland_protocols::zxdg_decoration_v1::negotiated;
use gnoblin_wayland_types::decoration::{DecorationWanted, WindowDecoration};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl org_kde_kwin_server_decoration::Host for State {
    fn decorations_available(&self) -> bool {
        self.composer.decorations_available()
    }

    fn answer_is_server(&self, wanted: DecorationWanted) -> bool {
        negotiated(wanted, self.composer.decorations_available()) == WindowDecoration::Server
    }

    fn ask_decoration(&mut self, surface: &WlSurface, wanted: DecorationWanted) {
        let Some(window) = self.window_of(surface) else {
            return;
        };
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.decoration_asked = Some(wanted);
        }
        self.apply_decoration(window);
    }

    fn kde_decorations(&mut self) -> &mut KdeDecorations {
        &mut self.kde_decorations
    }
}
