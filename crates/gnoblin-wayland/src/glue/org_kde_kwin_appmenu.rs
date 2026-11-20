//! `gnoblin-wayland-org-kde-kwin-appmenu`: a window's menu address is published on its node.

use gnoblin_wayland_protocols::org_kde_kwin_appmenu::{self, KdeAppmenus};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl org_kde_kwin_appmenu::Host for State {
    fn publish_appmenu(&mut self, surface: &WlSurface, service: &str, path: &str) {
        if let Some(window) = self.window_of(surface) {
            self.composer.set_window_appmenu(window, service, path);
        }
    }

    fn appmenus(&mut self) -> &mut KdeAppmenus {
        &mut self.kde_appmenus
    }
}
