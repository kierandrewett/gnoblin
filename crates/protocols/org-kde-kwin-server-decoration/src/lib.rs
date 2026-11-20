//! XML protocol `server_decoration` (KDE): the interfaces `org_kde_kwin_server_decoration_manager` and
//! `org_kde_kwin_server_decoration`.
//!
//! The older of the two ways a toolkit asks who draws a window's frame: GTK 3 and Qt 5 speak it where they do not
//! speak `xdg-decoration`. It is answered by the same negotiation, not a second one. A client's request becomes
//! what `zxdg_decoration_v1` records as the window's `decoration_wanted`, the compositor's decoration policy answers it,
//! and the answer goes back out here as a `mode` event: server-side only while a shell that holds the tree has
//! declared that it draws a frame, client-side otherwise (`design/node-api-refactor.md`, Decision 2).
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
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use wayland_protocols_plasma::server_decoration::server::org_kde_kwin_server_decoration::{
    self as decoration, OrgKdeKwinServerDecoration,
};
use wayland_protocols_plasma::server_decoration::server::org_kde_kwin_server_decoration_manager::{
    self as manager, OrgKdeKwinServerDecorationManager,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use gnoblin_wayland_types::decoration::DecorationWanted;

gnoblin_log::logger!(log = "wayland");

/// The version of `org_kde_kwin_server_decoration_manager` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "org-kde-kwin-server-decoration",
    interfaces: &[ServedInterface::new(
        "org_kde_kwin_server_decoration_manager",
        MANAGER_VERSION,
    )],
    privilege: Privilege::Core,
};

/// The protocol's `mode` values.
const MODE_CLIENT: u32 = 1;
const MODE_SERVER: u32 = 2;

/// What this protocol asks of the compositor.
pub trait Host: Sized + 'static {
    /// Whether a shell that holds the tree has declared that it draws a frame.
    fn decorations_available(&self) -> bool;
    /// Whether a client that wants `wanted` is told server-side now. This is the one negotiation `xdg-decoration` uses.
    fn answer_is_server(&self, wanted: DecorationWanted) -> bool;
    /// Records what the client of `surface`'s window asked for and answers it; nothing happens for a surface with no
    /// toplevel yet.
    fn ask_decoration(&mut self, surface: &WlSurface, wanted: DecorationWanted);
    /// The protocol's bookkeeping.
    fn kde_decorations(&mut self) -> &mut KdeDecorations;
}

/// One decoration object a client made.
struct KdeDecorationRecord {
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
pub struct KdeDecorations {
    /// Kept alive so `org_kde_kwin_server_decoration_manager` stays registered.
    #[allow(dead_code)]
    global: GlobalId,
    /// The bound managers, which hear the default mode again when it changes.
    managers: Vec<OrgKdeKwinServerDecorationManager>,
    /// Every live decoration object.
    records: Vec<KdeDecorationRecord>,
}

/// The global data of the manager: whether the configuration offers it.
pub struct OrgKdeKwinServerDecorationManagerGlobalData {
    offered: bool,
}

/// The user data of a bound manager.
pub struct OrgKdeKwinServerDecorationManagerUserData;

/// The user data of one decoration object: the surface it decorates.
pub struct DecorationUserData {
    surface: WlSurface,
}

/// Creates the `org_kde_kwin_server_decoration_manager` global. `offered` is `protocols.kde-server-decoration`: when
/// false the client sees a compositor that has no such protocol.
pub fn register<D>(registrar: &Registrar<'_>, offered: bool) -> KdeDecorations
where
    D: GlobalDispatch<OrgKdeKwinServerDecorationManager, OrgKdeKwinServerDecorationManagerGlobalData> + 'static,
{
    KdeDecorations {
        global: registrar
            .display()
            .create_global::<D, OrgKdeKwinServerDecorationManager, _>(
                MANAGER_VERSION,
                OrgKdeKwinServerDecorationManagerGlobalData { offered },
            ),
        managers: Vec::new(),
        records: Vec::new(),
    }
}

/// The protocol mode of a window drawn by the server (`true`) or by the client.
const fn mode_of(server: bool) -> u32 {
    if server { MODE_SERVER } else { MODE_CLIENT }
}

/// What a client that requested `requested` (or nothing) wants. Asking for nothing is wanting the server's default,
/// which is server-side drawing whenever someone draws it; asking for `none` is wanting no server frame.
const fn wanted_of(requested: Option<u32>) -> DecorationWanted {
    match requested {
        None | Some(MODE_SERVER) => DecorationWanted::Server,
        Some(_) => DecorationWanted::Client,
    }
}

/// A window's decoration was decided: every decoration object of its surface is told, if it was not already.
///
/// Called by the compositor wherever a change in who draws a frame lands.
pub fn announce<H: Host>(host: &mut H, surface: &WlSurface, server: bool) {
    let mode = mode_of(server);
    for record in &mut host.kde_decorations().records {
        if record.surface == *surface && record.sent != Some(mode) && record.object.is_alive() {
            record.object.mode(mode);
            record.sent = Some(mode);
        }
    }
}

/// A toplevel was created: a decoration object that was made for its surface first asks for it now.
pub fn toplevel_created<H: Host>(host: &mut H, surface: &WlSurface) {
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
pub fn default_changed<H: Host>(host: &mut H, available: bool) {
    let managers = &mut host.kde_decorations().managers;
    managers.retain(Resource::is_alive);
    for manager in managers.iter() {
        manager.default_mode(mode_of(available));
    }
}

impl<D> GlobalDispatch2<OrgKdeKwinServerDecorationManager, D> for OrgKdeKwinServerDecorationManagerGlobalData
where
    D: Host + Dispatch<OrgKdeKwinServerDecorationManager, OrgKdeKwinServerDecorationManagerUserData> + 'static,
{
    fn bind(
        &self,
        state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinServerDecorationManager>,
        data_init: &mut DataInit<'_, D>,
    ) {
        let bound = data_init.init(resource, OrgKdeKwinServerDecorationManagerUserData);
        bound.default_mode(mode_of(state.decorations_available()));
        state.kde_decorations().managers.push(bound);
    }

    fn can_view(&self, _client: &Client) -> bool {
        self.offered
    }
}

impl<D> Dispatch2<OrgKdeKwinServerDecorationManager, D> for OrgKdeKwinServerDecorationManagerUserData
where
    D: Host + Dispatch<OrgKdeKwinServerDecoration, DecorationUserData> + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _manager: &OrgKdeKwinServerDecorationManager,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
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
                let first = mode_of(state.answer_is_server(DecorationWanted::Server));
                object.mode(first);
                state.kde_decorations().records.push(KdeDecorationRecord {
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
                state.ask_decoration(&surface, DecorationWanted::Server);
            }
            _ => log::warn!(
                area = "decoration",
                "a KDE server decoration manager request this compositor does not serve was ignored"
            ),
        }
    }
}

impl<D: Host> Dispatch2<OrgKdeKwinServerDecoration, D> for DecorationUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        object: &OrgKdeKwinServerDecoration,
        request: decoration::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let decoration::Request::RequestMode { mode } = request {
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
            let wanted = wanted_of(Some(mode));
            state.ask_decoration(&self.surface, wanted);
            // A surface with no toplevel was not answered by the negotiation, so the answer is the one a
            // window with this wish would get now.
            let answer = mode_of(state.answer_is_server(wanted));
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
        // `release` is the destructor: the object is forgotten in `destroyed`, which also runs for a client that
        // went away.
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, object: &OrgKdeKwinServerDecoration) {
        let records = &mut state.kde_decorations().records;
        records.retain(|record| record.object != *object);
        // The last object of a surface going away is the client withdrawing its preference.
        let still_decorated = records.iter().any(|record| record.surface == self.surface);
        if !still_decorated {
            state.ask_decoration(&self.surface, DecorationWanted::Unset);
        }
    }
}
