//! XML protocol `server_decoration` (KDE): the interfaces `org_kde_kwin_server_decoration_manager` and
//! `org_kde_kwin_server_decoration`.
//!
//! The older of the two ways a toolkit asks who draws a window's frame: GTK 3 and Qt 5 speak it where they do not
//! speak `xdg-decoration`. It is answered by the same negotiation, not a second one. A client's request becomes
//! what `zxdg_decoration_v1` records as the window's `decoration_wanted`, [`State::apply_decoration`] answers it,
//! and the answer goes back out here as a `mode` event: server-side only while a shell that holds the tree has
//! declared that it draws a frame, client-side otherwise (`design/archive/node-api-refactor.md`, Decision 2).
//! The same switch that turns `xdg-decoration` off (`protocols.xdg-decoration`) is not this one: the protocol has
//! its own, `protocols.kde-server-decoration`, and a session that turns it off advertises nothing.
//!
//! ## The protocol's modes
//!
//! `none` (undecorated, a popup), `client` and `server`. A window nobody draws a frame for is `client` to the
//! protocol: the compositor has no undecorated state to promise, so a request for `none` is answered `client`.
//!
//! - A manager announces the session's default mode when it is bound: `server` while a shell declares that it draws
//!   decorations and `client` otherwise. The announcement is sent again when the shell starts or stops drawing one.
//! - A decoration object starts in that default mode and is told it directly after it is created. A client that
//!   never asks for another mode therefore wants what the server prefers, which is the protocol's own text.
//! - A request is answered with a `mode` event even when the mode is the one the object already had: the protocol
//!   makes the answer the acknowledgement.
//! - A decoration object made for a surface that has no toplevel yet is answered at once from the session's state, and
//!   its request is applied when the toplevel appears ([`toplevel_created`]). A decoration released before then is
//!   forgotten, and one released after returns the window to "no preference".
//!
//! # Failure modes
//!
//! Each is a test in `tests/kde_server_decoration.rs`:
//!
//! 1. **The manager is advertised when the configuration turned it off**: `a_disabled_protocol_is_not_advertised`.
//! 2. **A window is frameless under a shell that draws none**: a client that asks for `server` with no shell is told
//!    `client`: `a_client_is_told_client_side_while_no_shell_draws_decorations`.
//! 3. **A client is told server-side without a frame**: with a shell that declares decorations the same client is told
//!    `server`, and told `client` again when the shell stops: `a_shell_that_draws_decorations_makes_it_server_side`.
//! 4. **A request is lost**: a request is acknowledged, and a request for `client` is honoured under a shell that
//!    draws decorations: `a_request_for_client_side_is_honoured_and_acknowledged`.
//! 5. **A request made before the toplevel exists is lost**: `a_request_made_before_the_toplevel_is_applied_to_it`.

use smithay::reexports::wayland_server::backend::{ClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use wayland_protocols_plasma::server_decoration::server::org_kde_kwin_server_decoration::{
    self as decoration, OrgKdeKwinServerDecoration,
};
use wayland_protocols_plasma::server_decoration::server::org_kde_kwin_server_decoration_manager::{
    self as manager, OrgKdeKwinServerDecorationManager,
};

use gnoblin_backend::render::compose::{DecorationWanted, WindowDecoration};

use crate::State;
use crate::log;

/// The version of `org_kde_kwin_server_decoration_manager` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// The protocol's `mode` values.
const MODE_CLIENT: u32 = 1;
const MODE_SERVER: u32 = 2;

/// What this module asks of the compositor. The whole of its reach into [`State`] is these three methods, so the
/// protocol can be served from outside this crate's state.
pub(crate) trait DecorationHost {
    /// Whether a shell that holds the tree has declared that it draws a frame.
    fn decorations_available(&self) -> bool;
    /// Records what the client of `surface`'s window asked for and answers it; nothing happens for a surface with
    /// no toplevel yet.
    fn ask_decoration(&mut self, surface: &WlSurface, wanted: DecorationWanted);
    /// The protocol's bookkeeping.
    fn kde_decorations(&mut self) -> &mut KdeDecorations;
}

impl DecorationHost for State {
    fn decorations_available(&self) -> bool {
        self.composer.decorations_available()
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

/// One decoration object a client made.
struct Record {
    /// The surface it decorates.
    surface: WlSurface,
    /// The object, which is told each mode.
    object: OrgKdeKwinServerDecoration,
    /// What the client last requested, `None` while it has asked for nothing and is in the server's default mode.
    requested: Option<u32>,
    /// The mode the object was last told.
    sent: Option<u32>,
}

/// The global and what the session keeps for the objects clients made.
pub(crate) struct KdeDecorations {
    /// Kept alive so `org_kde_kwin_server_decoration_manager` stays registered.
    #[allow(dead_code)]
    global: GlobalId,
    /// The bound managers, which hear the default mode again when it changes.
    managers: Vec<OrgKdeKwinServerDecorationManager>,
    /// Every live decoration object.
    records: Vec<Record>,
}

/// The global data of the manager: whether the configuration offers it.
pub(crate) struct ManagerGlobalData {
    offered: bool,
}

/// The user data of a bound manager.
pub(crate) struct ManagerUserData;

/// The user data of one decoration object: the surface it decorates.
pub(crate) struct DecorationUserData {
    surface: WlSurface,
}

/// Creates the `org_kde_kwin_server_decoration_manager` global. `offered` is `protocols.kde-server-decoration`: when
/// false the client sees a compositor that has no such protocol.
pub(crate) fn global(display: &DisplayHandle, offered: bool) -> KdeDecorations {
    KdeDecorations {
        global: display.create_global::<State, OrgKdeKwinServerDecorationManager, _>(
            MANAGER_VERSION,
            ManagerGlobalData { offered },
        ),
        managers: Vec::new(),
        records: Vec::new(),
    }
}

/// The protocol mode a window drawn as `decoration` has.
const fn mode_of(decoration: WindowDecoration) -> u32 {
    match decoration {
        WindowDecoration::Server => MODE_SERVER,
        WindowDecoration::Client | WindowDecoration::None => MODE_CLIENT,
    }
}

/// What a client that requested `requested` (or nothing) wants. Asking for nothing is wanting the server's default,
/// which is server-side drawing whenever someone draws it; asking for `none` is wanting no server frame.
const fn wanted_of(requested: Option<u32>) -> DecorationWanted {
    match requested {
        None | Some(MODE_SERVER) => DecorationWanted::Server,
        Some(_) => DecorationWanted::Client,
    }
}

/// The mode a client that wants `wanted` is told, given whether a shell draws decorations.
fn told(wanted: DecorationWanted, available: bool) -> u32 {
    mode_of(crate::protocols::zxdg_decoration_v1::negotiated(wanted, available))
}

/// The session's default mode.
const fn default_mode(available: bool) -> u32 {
    if available { MODE_SERVER } else { MODE_CLIENT }
}

/// A window's decoration was decided: every decoration object of its surface is told, if it was not already.
///
/// Called from [`State::apply_decoration`], which is where every change in who draws a frame lands.
pub(crate) fn announce<H: DecorationHost>(host: &mut H, surface: &WlSurface, decision: WindowDecoration) {
    let mode = mode_of(decision);
    for record in &mut host.kde_decorations().records {
        if record.surface == *surface && record.sent != Some(mode) && record.object.is_alive() {
            record.object.mode(mode);
            record.sent = Some(mode);
        }
    }
}

/// A toplevel was created: a decoration object that was made for its surface first asks for it now.
pub(crate) fn toplevel_created<H: DecorationHost>(host: &mut H, surface: &WlSurface) {
    let Some(requested) = host
        .kde_decorations()
        .records
        .iter()
        .find(|record| record.surface == *surface)
        .map(|record| record.requested)
    else {
        return;
    };
    host.ask_decoration(surface, wanted_of(requested));
}

/// The shell started or stopped drawing decorations: the managers hear the new default.
pub(crate) fn default_changed<H: DecorationHost>(host: &mut H, available: bool) {
    let managers = &mut host.kde_decorations().managers;
    managers.retain(Resource::is_alive);
    for manager in managers.iter() {
        manager.default_mode(default_mode(available));
    }
}

impl GlobalDispatch2<OrgKdeKwinServerDecorationManager, State> for ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinServerDecorationManager>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let bound = data_init.init(resource, ManagerUserData);
        bound.default_mode(default_mode(state.decorations_available()));
        state.kde_decorations().managers.push(bound);
    }

    fn can_view(&self, _client: &Client) -> bool {
        self.offered
    }
}

impl Dispatch2<OrgKdeKwinServerDecorationManager, State> for ManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _manager: &OrgKdeKwinServerDecorationManager,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::Create { id, surface } => {
                let object = data_init.init(
                    id,
                    DecorationUserData {
                        surface: surface.clone(),
                    },
                );
                // The object is told its mode directly after it is created, from what the session would tell a
                // client that wants the server's default.
                let first = told(wanted_of(None), state.decorations_available());
                object.mode(first);
                state.kde_decorations().records.push(Record {
                    surface: surface.clone(),
                    object,
                    requested: None,
                    sent: Some(first),
                });
                log::debug!(
                    area = "decoration",
                    mode = first,
                    "a client made a KDE server decoration"
                );
                // A surface that already has its toplevel starts negotiating now; one that has none is picked up
                // by `toplevel_created`.
                state.ask_decoration(&surface, wanted_of(None));
            }
            _ => log::warn!(
                area = "decoration",
                "a KDE server decoration manager request this compositor does not serve was ignored"
            ),
        }
    }
}

impl Dispatch2<OrgKdeKwinServerDecoration, State> for DecorationUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        object: &OrgKdeKwinServerDecoration,
        request: decoration::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            decoration::Request::RequestMode { mode } => {
                log::debug!(area = "decoration", mode, "a client asked for a KDE decoration mode");
                if let Some(record) = state
                    .kde_decorations()
                    .records
                    .iter_mut()
                    .find(|record| record.object == *object)
                {
                    record.requested = Some(mode);
                    // The answer is always sent: it is how the protocol acknowledges a request.
                    record.sent = None;
                }
                state.ask_decoration(&self.surface, wanted_of(Some(mode)));
                // A surface with no toplevel was not answered by the negotiation, so the answer is the one a
                // window with this wish would get now.
                let answer = told(wanted_of(Some(mode)), state.decorations_available());
                if let Some(record) = state
                    .kde_decorations()
                    .records
                    .iter_mut()
                    .find(|record| record.object == *object && record.sent.is_none())
                {
                    record.object.mode(answer);
                    record.sent = Some(answer);
                }
            }
            // The object is forgotten in `destroyed`, which runs for a `release` and for a client that went away.
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, object: &OrgKdeKwinServerDecoration) {
        let records = &mut state.kde_decorations().records;
        records.retain(|record| record.object != *object);
        // The last object of a surface going away is the client withdrawing its preference.
        let still_decorated = records.iter().any(|record| record.surface == self.surface);
        if !still_decorated {
            state.ask_decoration(&self.surface, DecorationWanted::Unset);
        }
    }
}
