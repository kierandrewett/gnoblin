//! The KDE server-decoration client the tests of `org_kde_kwin_server_decoration` drive: the modes it is told land in
//! [`App::kde_default_mode`] and [`App::kde_modes`].

use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_plasma::server_decoration::client::{
    org_kde_kwin_server_decoration::{self as decoration, OrgKdeKwinServerDecoration},
    org_kde_kwin_server_decoration_manager::{self as manager, OrgKdeKwinServerDecorationManager},
};

use super::App;

impl Dispatch<OrgKdeKwinServerDecorationManager, ()> for App {
    fn event(
        state: &mut Self,
        _: &OrgKdeKwinServerDecorationManager,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::DefaultMode { mode } = event {
            state.kde_default_mode = Some(mode);
        }
    }
}

impl Dispatch<OrgKdeKwinServerDecoration, ()> for App {
    fn event(
        state: &mut Self,
        _: &OrgKdeKwinServerDecoration,
        event: decoration::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let decoration::Event::Mode { mode } = event {
            state.kde_modes.push(mode);
        }
    }
}
