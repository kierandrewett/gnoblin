//! XML protocol `wlr_foreign_toplevel_management_unstable_v1` (`<protocol name="wlr_foreign_toplevel_management_unstable_v1">`); interfaces served: `zwlr_foreign_toplevel_manager_v1`.
//!
//! `zwlr_foreign_toplevel_manager_v1` (version 3): the toplevel list a shell's taskbar reads.
//!
//! Shells use the wlroots protocol today: a taskbar lists windows, activates one, closes
//! one, minimises it, maximises it and fullscreens it. The window record both protocols
//! are fed from is [`foreign_toplevel_shared`](super::foreign_toplevel_shared); the
//! standardised read-only list is [`ext_foreign_toplevel_list_v1`](super::ext_foreign_toplevel_list_v1).
//!
//! ## Where the requests go
//!
//! A request from a shell is a request from a *privileged* client, so it is not
//! attributed: `activate` is the shell telling the session which window the user
//! picked in its taskbar. Every request, `close` included, goes to
//! `State::window_request`(crate::State): while a session shell holds the tree it is an event on the
//! window's node (`activate`, `maximize`, `unmaximize`, `fullscreen`, `unfullscreen`, `minimize`,
//! `unminimize`, `close`) and the shell decides, answering by writing the node or calling its `close`.
//! With no shell the window manager decides, and `close` is the protocol's own
//! `xdg_toplevel.close`, which goes to the client because only the client decides whether it exits.
//!
//! Access is decided per client (`gnoblin_wayland_api::access`): a taskbar inside an application
//! sandbox sees neither global.
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/foreign_toplevel.rs` unless it says otherwise.
//!
//! 1. **A window that closes keeps its taskbar entry for ever.** The entry is removed
//!    when the window leaves the window manager, and every handle object that client
//!    holds hears `closed`, so a taskbar drops the row instead of drawing a dead one:
//!    `a_closed_window_leaves_the_list`.
//! 2. **A handle outlives its client.** The handle objects are resources of the
//!    client's connection; when it disconnects they are destroyed and removed from
//!    the tracking table, so the next window creates handles only for the connections
//!    that are still there: `a_shell_that_disconnects_leaves_no_handle_behind`.
//! 3. **A state event is sent for a window that did not change.** Every field is
//!    compared against what was last sent, so an idle session sends nothing and a
//!    taskbar does not repaint on every iteration:
//!    `an_idle_session_sends_no_toplevel_events`.
//! 4. **A request for a window that is gone is applied to another one.** A handle
//!    carries its window identity, and an identity is never reused, so a request that
//!    arrives after the window closed is a no-op rather than an action on whatever
//!    the table holds now: the maximise guards and
//!    `a_request_for_a_closed_window_does_nothing`.
//! 5. **A taskbar in a sandbox sees the list.** The access policy hides both globals
//!    from a sandboxed connection: `a_sandboxed_client_sees_no_privileged_global` in
//!    `tests/access.rs`.

use std::sync::Arc;

use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::backend::{ClientId as ClientKey, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::wayland::Dispatch2;
use smithay::wayland::GlobalDispatch2;

use gnoblin_types::{OutputId, WindowId};

use gnoblin_wayland_api::access::{Access, Global};
use gnoblin_wayland_api::host::CoreHost;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use gnoblin_wayland_foreign_toplevel_shared::{Sent, States, ToplevelChanges, ToplevelState};
use gnoblin_wayland_types::window::WindowRequest;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
gnoblin_log::logger!(log = "wayland");

/// The version of `zwlr_foreign_toplevel_manager_v1` this session serves.
///
/// Three: the version shells build against, with `set_fullscreen`/`unset_fullscreen`
/// from two and the `parent` event from three.
pub const MANAGER_VERSION: u32 = 3;

/// The wire form of a window's state flags: the enum values the protocol defines, as an array.
fn states_to_wire(states: States) -> Vec<u8> {
    let mut wire = Vec::new();
    for (bit, value) in [
        (States::MAXIMISED, handle::State::Maximized as u32),
        (States::MINIMISED, handle::State::Minimized as u32),
        (States::ACTIVATED, handle::State::Activated as u32),
        (States::FULLSCREEN, handle::State::Fullscreen as u32),
    ] {
        if states.0 & bit != 0 {
            wire.push(u8::try_from(value).unwrap_or(0));
        }
    }
    wire
}

/// The published record of one window.
pub struct ToplevelEntry {
    /// The window manager's identity for the window.
    pub window: WindowId,
    /// The standardised list's handle for the window, shared by every list instance.
    pub ext: ForeignToplevelHandle,
    /// One handle object per binding of the wlr manager.
    pub handles: Vec<ZwlrForeignToplevelManagementV1HandleObject>,
    /// What those handles have been told.
    pub sent: Sent,
}

/// The published window list.
#[derive(Default)]
pub struct ForeignToplevels {
    /// Every window currently published, in the order they were introduced.
    pub entries: Vec<ToplevelEntry>,
    /// The manager bindings that asked for events.
    ///
    /// A manager leaves the list when its client sends `stop`, because the protocol
    /// says no further toplevel is announced after `finished`.
    pub managers: Vec<ZwlrForeignToplevelManagerV1>,
}

/// What the protocol needs of the compositor.
pub trait Host: CoreHost + Sized + 'static {
    /// The published window list.
    fn foreign_toplevels(&mut self) -> &mut ForeignToplevels;

    /// Asks the window manager to do `request` to `window`. The compositor decides what each request means: the shell, or
    /// the window manager when no shell holds the tree.
    fn window_request(&mut self, window: WindowId, request: WindowRequest);

    /// The surface of `window`, if it has one.
    fn surface_of(&self, window: WindowId) -> Option<WlSurface>;

    /// Brings both published lists in step with the window record.
    fn sync_foreign_toplevels(&mut self);
}

/// One handle object and the manager it belongs to.
pub struct ZwlrForeignToplevelManagementV1HandleObject {
    /// The manager that created it, so `stop` can retire exactly its handles.
    manager: ZwlrForeignToplevelManagerV1,
    /// The handle object itself.
    object: ZwlrForeignToplevelHandleV1,
}

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwlr-foreign-toplevel-management-v1",
    interfaces: &[ServedInterface::new(
        "zwlr_foreign_toplevel_manager_v1",
        MANAGER_VERSION,
    )],
    privilege: Privilege::Roster(Global::ForeignToplevelManagement),
};

/// Creates the `zwlr_foreign_toplevel_manager_v1` global.
///
/// The access roster is captured in the global's data, so the answer to "may this
/// connection bind it" is the same answer the other privileged globals give.
pub fn register<D>(registrar: &Registrar<'_>) -> GlobalId
where
    D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ZwlrForeignToplevelManagementV1ManagerGlobalData> + 'static,
{
    registrar
        .display()
        .create_global::<D, ZwlrForeignToplevelManagerV1, ZwlrForeignToplevelManagementV1ManagerGlobalData>(
            MANAGER_VERSION,
            ZwlrForeignToplevelManagementV1ManagerGlobalData {
                access: Arc::clone(registrar.access()),
            },
        )
}

/// The global data of the wlr manager: who may bind it.
pub struct ZwlrForeignToplevelManagementV1ManagerGlobalData {
    /// The roster that answers which connection may.
    access: Arc<Access>,
}

/// The user data of a bound manager object.
pub struct ZwlrForeignToplevelManagementV1ManagerUserData;

/// The user data of one handle object.
pub struct ZwlrForeignToplevelManagementV1HandleUserData {
    /// The window it names.
    window: WindowId,
}

impl<D> GlobalDispatch2<ZwlrForeignToplevelManagerV1, D> for ZwlrForeignToplevelManagementV1ManagerGlobalData
where
    D: ForeignToplevelExt + Dispatch<ZwlrForeignToplevelManagerV1, ZwlrForeignToplevelManagementV1ManagerUserData>,
{
    fn bind(
        &self,
        state: &mut D,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        let registered = data_init.init(resource, ZwlrForeignToplevelManagementV1ManagerUserData);
        state.foreign_toplevels().managers.push(registered);
        log::debug!(area = "toplevel", client = ?client.id(), "a taskbar bound the manager");
        state.introduce_foreign_toplevels(client);
    }

    fn can_view(&self, client: &Client) -> bool {
        gnoblin_wayland_api::access::AccessPolicy::allows(
            &*self.access,
            &client.id(),
            Global::ForeignToplevelManagement,
        )
    }
}

impl<D: Host> Dispatch2<ZwlrForeignToplevelManagerV1, D> for ZwlrForeignToplevelManagementV1ManagerUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            manager::Request::Stop => {
                // The protocol's answer is `finished`; after it the object is inert and
                // no further toplevel is announced through it.
                manager.finished();
                state.foreign_toplevels().managers.retain(|known| known != manager);
                for entry in &mut state.foreign_toplevels().entries {
                    entry.handles.retain(|known| known.manager != *manager);
                }
                log::debug!(area = "toplevel", "a taskbar stopped the manager");
            }
            _ => log::trace!(area = "toplevel", "an unknown manager request"),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientKey, manager: &ZwlrForeignToplevelManagerV1) {
        state.foreign_toplevels().managers.retain(|known| known != manager);
        for entry in &mut state.foreign_toplevels().entries {
            entry.handles.retain(|known| known.manager != *manager);
        }
    }
}

impl<D: Host> Dispatch2<ZwlrForeignToplevelHandleV1, D> for ZwlrForeignToplevelManagementV1HandleUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: handle::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let window = self.window;
        match request {
            handle::Request::Activate { seat } => toplevel_activated(state, window, &seat),
            handle::Request::Close => state.window_request(window, WindowRequest::Close),
            handle::Request::SetMaximized => state.window_request(window, WindowRequest::Maximise(true)),
            handle::Request::UnsetMaximized => state.window_request(window, WindowRequest::Maximise(false)),
            handle::Request::SetMinimized => state.window_request(window, WindowRequest::Minimise(true)),
            handle::Request::UnsetMinimized => state.window_request(window, WindowRequest::Minimise(false)),
            // The output a taskbar names is not honoured yet: a window goes fullscreen on its own output.
            handle::Request::SetFullscreen { output: _ } => {
                state.window_request(window, WindowRequest::Fullscreen(true))
            }
            handle::Request::UnsetFullscreen => state.window_request(window, WindowRequest::Fullscreen(false)),
            handle::Request::SetRectangle {
                surface,
                x,
                y,
                width,
                height,
            } => toplevel_rectangle(state, window, &surface, x, y, width, height),
            handle::Request::Destroy => {}
            _ => log::trace!(area = "toplevel", "an unknown handle request"),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientKey, handle: &ZwlrForeignToplevelHandleV1) {
        for entry in &mut state.foreign_toplevels().entries {
            entry.handles.retain(|known| known.object != *handle);
        }
    }
}

/// The compositor-side operations on the published list, over the host.
pub trait ForeignToplevelExt:
    Host + Dispatch<ZwlrForeignToplevelHandleV1, ZwlrForeignToplevelManagementV1HandleUserData>
{
    /// Sends a newly bound manager every window that already exists.
    ///
    /// A taskbar that starts after a client has to see the windows that were already
    /// there, which is why a bind walks the table rather than waiting for the next map.
    fn introduce_foreign_toplevels(&mut self, client: &Client) {
        let Some(management) = self
            .foreign_toplevels()
            .managers
            .iter()
            .find(|manager| manager.client().as_ref() == Some(client))
            .cloned()
        else {
            return;
        };
        let windows: Vec<WindowId> = self
            .foreign_toplevels()
            .entries
            .iter()
            .map(|entry| entry.window)
            .collect();
        for window in windows {
            let Some(handle) = self.create_wlr_handle(&management, client, window) else {
                continue;
            };
            if let Some(entry) = self
                .foreign_toplevels()
                .entries
                .iter_mut()
                .find(|entry| entry.window == window)
            {
                entry.handles.push(handle);
                // The new handle has been told nothing, so the next sync introduces it.
                entry.sent = Sent::default();
            }
        }
        self.sync_foreign_toplevels();
    }

    /// Creates one handle object on a client's manager.
    fn create_wlr_handle(
        &mut self,
        manager: &ZwlrForeignToplevelManagerV1,
        client: &Client,
        window: WindowId,
    ) -> Option<ZwlrForeignToplevelManagementV1HandleObject> {
        match client.create_resource::<ZwlrForeignToplevelHandleV1, _, Self>(
            self.display_handle(),
            MANAGER_VERSION.min(manager.version()),
            ZwlrForeignToplevelManagementV1HandleUserData { window },
        ) {
            Ok(object) => {
                manager.toplevel(&object);
                Some(ZwlrForeignToplevelManagementV1HandleObject {
                    manager: manager.clone(),
                    object,
                })
            }
            Err(error) => {
                log::debug!(area = "toplevel", ?error, "a handle could not be created");
                None
            }
        }
    }
}

impl<T: Host + Dispatch<ZwlrForeignToplevelHandleV1, ZwlrForeignToplevelManagementV1HandleUserData>> ForeignToplevelExt
    for T
{
}

/// A taskbar asked for a window to be activated.
fn toplevel_activated<D: Host>(state: &mut D, window: WindowId, seat: &WlSeat) {
    let _ = seat;
    // A taskbar is one of the session's own privileged clients and the click on its
    // row is the trusted input context: the plan's rule is about a *client*
    // demanding focus, not about a shell reporting the user's action.
    log::debug!(area = "toplevel", %window, "activated by a taskbar");
    state.window_request(window, WindowRequest::Activate);
}

/// A taskbar said where it draws a window's thumbnail.
///
/// Nothing arranges minimised windows from it in this slice, so the rectangle is
/// validated and then dropped: a rectangle that names a surface another connection
/// owns, or a negative one, is the protocol's `invalid_rectangle` case and is
/// ignored rather than applied to the wrong window.
fn toplevel_rectangle<D: Host>(
    state: &mut D,
    window: WindowId,
    surface: &WlSurface,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let owned = state
        .surface_of(window)
        .is_some_and(|owned| owned.client() == surface.client());
    if !owned || width < 0 || height < 0 {
        log::debug!(area = "toplevel", %window, "an invalid rectangle was ignored");
        return;
    }
    log::trace!(area = "toplevel", %window, x, y, width, height, "rectangle");
}

/// Creates one handle object for each manager binding, for a window that is new.
pub fn create_handles<D: ForeignToplevelExt>(
    state: &mut D,
    window: WindowId,
) -> Vec<ZwlrForeignToplevelManagementV1HandleObject> {
    let mut handles = Vec::new();
    for management in state.foreign_toplevels().managers.clone() {
        let Some(client) = management.client() else {
            continue;
        };
        if let Some(handle) = state.create_wlr_handle(&management, &client, window) {
            handles.push(handle);
        }
    }
    handles
}

/// Tells every handle of a window that it has closed.
pub fn send_closed(entry: &ToplevelEntry) {
    for handle in &entry.handles {
        handle.object.closed();
    }
}

/// Sends the changed values of a window to its handles, then one `done` each.
///
/// Output enter and leave are sent only to handles on the connection that bound the
/// output, and only for the outputs that the window moved on or off since `entry.sent`.
pub fn send_changes(
    entry: &ToplevelEntry,
    published: &ToplevelState,
    changes: ToplevelChanges,
    bound: &[(ClientKey, OutputId, WlOutput)],
) {
    for handle in &entry.handles {
        if changes.title || changes.introduce {
            handle.object.title(published.title.clone());
        }
        if changes.app_id || changes.introduce {
            handle.object.app_id(published.app_id.clone());
        }
        if changes.states || changes.introduce {
            handle.object.state(states_to_wire(published.states));
        }
        if changes.outputs || changes.introduce {
            let client = handle.object.client().map(|client| client.id());
            for (owner, output, wl_output) in bound {
                if Some(owner) != client.as_ref() {
                    continue;
                }
                if published.outputs.contains(output) && !entry.sent.outputs.contains(output) {
                    handle.object.output_enter(wl_output);
                }
                if !published.outputs.contains(output) && entry.sent.outputs.contains(output) {
                    handle.object.output_leave(wl_output);
                }
            }
        }
        if changes.any() {
            handle.object.done();
        }
    }
}
