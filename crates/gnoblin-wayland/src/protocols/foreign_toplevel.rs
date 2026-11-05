//! The toplevel list a shell's taskbar reads: `zwlr_foreign_toplevel_manager_v1`
//! (version 3) and `ext_foreign_toplevel_list_v1`.
//!
//! Shells use the wlroots protocol today (a taskbar lists windows, activates one,
//! closes one, minimises it, maximises it and fullscreens it), and the standardised
//! `ext_foreign_toplevel_list_v1` is the read-only half of the same idea with an
//! identifier a shell can use as a stable key. Both are fed from the same record here,
//! so a shell that reads one cannot see a different set of windows than a shell that
//! reads the other.
//!
//! ## Where the requests go
//!
//! A request from a shell is a request from a *privileged* client, so it is not
//! attributed: `activate` is the shell telling the session which window the user
//! picked in its taskbar, and it takes the window manager's activation path with
//! [`gnoblin_wm::ActivationDecision::Allow`]. Everything else is the same event the
//! window's own client could have sent with `xdg_toplevel`: minimise, maximise,
//! fullscreen. `close` is the protocol's own `xdg_toplevel.close` and goes to the
//! client rather than to the window manager, because only the client decides whether
//! it exits.
//!
//! Access is decided per client ([`crate::access`]): a taskbar inside an application
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
//!    the table holds now: the `toplevel_maximise` guards and
//!    `a_request_for_a_closed_window_does_nothing`.
//! 5. **A taskbar in a sandbox sees the list.** The access policy hides both globals
//!    from a sandboxed connection: `a_sandboxed_client_sees_no_privileged_global` in
//!    `tests/access.rs`.

use std::sync::Arc;

use smithay::reexports::wayland_server::backend::{ClientId as ClientKey, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::Dispatch2;
use smithay::wayland::GlobalDispatch2;
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};
use tracing::{debug, trace};
use wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};

use gnoblin_types::{OutputId, WindowId};
use gnoblin_wm::{ActivationDecision, Event};

use crate::State;
use crate::access::{Access, Global};

/// The version of `zwlr_foreign_toplevel_manager_v1` this session serves.
///
/// Three: the version shells build against, with `set_fullscreen`/`unset_fullscreen`
/// from two and the `parent` event from three.
pub const MANAGER_VERSION: u32 = 3;

/// The state flags of `zwlr_foreign_toplevel_handle_v1.state`, in one set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct States(u32);

impl States {
    const MAXIMISED: u32 = 1 << 0;
    const MINIMISED: u32 = 1 << 1;
    const ACTIVATED: u32 = 1 << 2;
    const FULLSCREEN: u32 = 1 << 3;

    /// The wire form: the enum values the protocol defines, as an array.
    fn to_wire(self) -> Vec<u8> {
        let mut wire = Vec::new();
        for (bit, value) in [
            (Self::MAXIMISED, handle::State::Maximized as u32),
            (Self::MINIMISED, handle::State::Minimized as u32),
            (Self::ACTIVATED, handle::State::Activated as u32),
            (Self::FULLSCREEN, handle::State::Fullscreen as u32),
        ] {
            if self.0 & bit != 0 {
                wire.push(u8::try_from(value).unwrap_or(0));
            }
        }
        wire
    }
}

/// What one window's handles have been told, so an event only goes out when a value
/// actually changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sent {
    /// The title last sent.
    title: String,
    /// The application id last sent.
    app_id: String,
    /// The state flags last sent.
    states: States,
    /// The outputs the shell has been told about.
    outputs: Vec<OutputId>,
    /// Whether the initial batch has been sent, so a shell gets a `done` even when
    /// every value is the default one.
    introduced: bool,
}

/// One handle object and the manager it belongs to.
struct HandleObject {
    /// The manager that created it, so `stop` can retire exactly its handles.
    manager: ZwlrForeignToplevelManagerV1,
    /// The handle object itself.
    object: ZwlrForeignToplevelHandleV1,
}

/// The published record of one window.
struct ToplevelEntry {
    /// The window manager's identity for the window.
    window: WindowId,
    /// The standardised list's handle for the window, shared by every list instance.
    ext: ForeignToplevelHandle,
    /// One handle object per binding of the wlr manager.
    handles: Vec<HandleObject>,
    /// What those handles have been told.
    sent: Sent,
}

/// The published window list.
#[derive(Default)]
pub(crate) struct ForeignToplevels {
    entries: Vec<ToplevelEntry>,
    /// The manager bindings that asked for events.
    ///
    /// A manager leaves the list when its client sends `stop`, because the protocol
    /// says no further toplevel is announced after `finished`.
    managers: Vec<ZwlrForeignToplevelManagerV1>,
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

/// Creates the `zwlr_foreign_toplevel_manager_v1` global.
///
/// The access roster is captured in the global's data, so the answer to "may this
/// connection bind it" is the same answer the other privileged globals give.
pub(crate) fn global(display_handle: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display_handle.create_global::<State, ZwlrForeignToplevelManagerV1, ManagerGlobalData>(
        MANAGER_VERSION,
        ManagerGlobalData {
            access: Arc::clone(access),
        },
    )
}

/// The global data of the wlr manager: who may bind it.
struct ManagerGlobalData {
    /// The roster that answers which connection may.
    access: Arc<Access>,
}

/// The user data of a bound manager object.
struct ManagerUserData;

/// The user data of one handle object.
struct HandleUserData {
    /// The window it names.
    window: WindowId,
}

impl GlobalDispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let registered = data_init.init(resource, ManagerUserData);
        state.foreign_toplevels.managers.push(registered);
        debug!(client = ?client.id(), "[toplevel] a taskbar bound the manager");
        state.introduce_foreign_toplevels(client);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::access::AccessPolicy::allows(&*self.access, &client.id(), Global::ForeignToplevelManagement)
    }
}

impl Dispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::Stop => {
                // The protocol's answer is `finished`; after it the object is inert and
                // no further toplevel is announced through it.
                manager.finished();
                state.foreign_toplevels.managers.retain(|known| known != manager);
                for entry in &mut state.foreign_toplevels.entries {
                    entry.handles.retain(|known| known.manager != *manager);
                }
                debug!("[toplevel] a taskbar stopped the manager");
            }
            _ => trace!("[toplevel] an unknown manager request"),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientKey, manager: &ZwlrForeignToplevelManagerV1) {
        state.foreign_toplevels.managers.retain(|known| known != manager);
        for entry in &mut state.foreign_toplevels.entries {
            entry.handles.retain(|known| known.manager != *manager);
        }
    }
}

impl Dispatch2<ZwlrForeignToplevelHandleV1, State> for HandleUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: handle::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let window = self.window;
        match request {
            handle::Request::Activate { seat } => state.toplevel_activated(window, &seat),
            handle::Request::Close => state.toplevel_close(window),
            handle::Request::SetMaximized => state.toplevel_maximise(window, true),
            handle::Request::UnsetMaximized => state.toplevel_maximise(window, false),
            handle::Request::SetMinimized => state.toplevel_minimise(window, true),
            handle::Request::UnsetMinimized => state.toplevel_minimise(window, false),
            handle::Request::SetFullscreen { output } => state.toplevel_fullscreen(window, true, output.as_ref()),
            handle::Request::UnsetFullscreen => state.toplevel_fullscreen(window, false, None),
            handle::Request::SetRectangle {
                surface,
                x,
                y,
                width,
                height,
            } => state.toplevel_rectangle(window, &surface, x, y, width, height),
            handle::Request::Destroy => {}
            _ => trace!("[toplevel] an unknown handle request"),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientKey, handle: &ZwlrForeignToplevelHandleV1) {
        for entry in &mut state.foreign_toplevels.entries {
            entry.handles.retain(|known| known.object != *handle);
        }
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
    }
}

impl State {
    /// Sends a newly bound manager every window that already exists.
    ///
    /// A taskbar that starts after a client has to see the windows that were already
    /// there, which is why a bind walks the table rather than waiting for the next map.
    pub(crate) fn introduce_foreign_toplevels(&mut self, client: &Client) {
        let Some(management) = self
            .foreign_toplevels
            .managers
            .iter()
            .find(|manager| manager.client().as_ref() == Some(client))
            .cloned()
        else {
            return;
        };
        let windows: Vec<WindowId> = self
            .foreign_toplevels
            .entries
            .iter()
            .map(|entry| entry.window)
            .collect();
        for window in windows {
            let Some(handle) = self.create_wlr_handle(&management, client, window) else {
                continue;
            };
            if let Some(entry) = self
                .foreign_toplevels
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
    ) -> Option<HandleObject> {
        match client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
            &self.display_handle,
            MANAGER_VERSION.min(manager.version()),
            HandleUserData { window },
        ) {
            Ok(object) => {
                manager.toplevel(&object);
                Some(HandleObject {
                    manager: manager.clone(),
                    object,
                })
            }
            Err(error) => {
                debug!(?error, "[toplevel] a handle could not be created");
                None
            }
        }
    }

    /// Brings both published lists in step with the window manager.
    ///
    /// Called from [`State::publish`] after every change, so a title, a state flag or a
    /// visibility change is announced once and an idle iteration announces nothing.
    pub(crate) fn sync_foreign_toplevels(&mut self) {
        let active = self.wm.active_workspace();
        let focused = self.wm.focused();
        let windows: Vec<ToplevelState> = self
            .wm
            .windows()
            .map(|window| {
                let mut states = States::default();
                if window.state.maximised {
                    states.0 |= States::MAXIMISED;
                }
                if window.state.minimised {
                    states.0 |= States::MINIMISED;
                }
                if window.state.fullscreen {
                    states.0 |= States::FULLSCREEN;
                }
                if focused == Some(window.id) {
                    states.0 |= States::ACTIVATED;
                }
                ToplevelState {
                    window: window.id,
                    title: window.title.clone(),
                    app_id: window.app_id.clone(),
                    states,
                    outputs: if window.is_visible(active) {
                        self.output_ids()
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();

        // Windows that left the manager: tell every handle and drop the entry.
        let gone: Vec<WindowId> = self
            .foreign_toplevels
            .entries
            .iter()
            .map(|entry| entry.window)
            .filter(|window| !windows.iter().any(|published| published.window == *window))
            .collect();
        for window in gone {
            if let Some(index) = self
                .foreign_toplevels
                .entries
                .iter()
                .position(|entry| entry.window == window)
            {
                let entry = self.foreign_toplevels.entries.remove(index);
                for handle in &entry.handles {
                    handle.object.closed();
                }
                self.foreign_toplevel_list_state.remove_toplevel(&entry.ext);
                debug!(%window, "[toplevel] closed");
            }
        }

        // The output objects each taskbar bound, taken before the entries are borrowed
        // mutably: a taskbar hears only about outputs of the connection it is on.
        let mut bound: Vec<(ClientKey, OutputId, WlOutput)> = Vec::new();
        for entry in self.output_entries() {
            for management in &self.foreign_toplevels.managers {
                let Some(client) = management.client() else {
                    continue;
                };
                for wl_output in entry.output.client_outputs(&client) {
                    bound.push((client.id(), entry.backend, wl_output));
                }
            }
        }

        for published in windows {
            let index = self
                .foreign_toplevels
                .entries
                .iter()
                .position(|entry| entry.window == published.window);
            let Some(index) = index else {
                self.publish_foreign_toplevel(&published);
                continue;
            };
            let Some(entry) = self.foreign_toplevels.entries.get_mut(index) else {
                continue;
            };
            let title_changed = entry.sent.title != published.title;
            let app_id_changed = entry.sent.app_id != published.app_id;
            let states_changed = entry.sent.states != published.states;
            let outputs_changed = entry.sent.outputs != published.outputs;
            let introduce = !entry.sent.introduced;

            if title_changed {
                entry.ext.send_title(&published.title);
            }
            if app_id_changed {
                entry.ext.send_app_id(&published.app_id);
            }
            if title_changed || app_id_changed || states_changed || outputs_changed || introduce {
                entry.ext.send_done();
            }
            for handle in &entry.handles {
                if title_changed || introduce {
                    handle.object.title(published.title.clone());
                }
                if app_id_changed || introduce {
                    handle.object.app_id(published.app_id.clone());
                }
                if states_changed || introduce {
                    handle.object.state(published.states.to_wire());
                }
                if outputs_changed || introduce {
                    let client = handle.object.client().map(|client| client.id());
                    for (owner, output, wl_output) in &bound {
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
                if title_changed || app_id_changed || states_changed || outputs_changed || introduce {
                    handle.object.done();
                }
            }
            entry.sent = Sent {
                title: published.title,
                app_id: published.app_id,
                states: published.states,
                outputs: published.outputs,
                introduced: true,
            };
        }
    }

    /// Publishes a window that was not in the list yet.
    fn publish_foreign_toplevel(&mut self, published: &ToplevelState) {
        let identifier = identifier(published.window);
        let ext = self.foreign_toplevel_list_state.new_toplevel_with_identifier::<State>(
            published.title.clone(),
            published.app_id.clone(),
            identifier.clone(),
        );
        let mut handles = Vec::new();
        for management in self.foreign_toplevels.managers.clone() {
            let Some(client) = management.client() else {
                continue;
            };
            if let Some(handle) = self.create_wlr_handle(&management, &client, published.window) {
                handles.push(handle);
            }
        }
        self.foreign_toplevels.entries.push(ToplevelEntry {
            window: published.window,
            ext,
            handles,
            sent: Sent::default(),
        });
    }

    /// A taskbar asked for a window to be activated.
    fn toplevel_activated(&mut self, window: WindowId, seat: &WlSeat) {
        let _ = seat;
        // A taskbar is one of the session's own privileged clients and the click on its
        // row is the trusted input context: the plan's rule is about a *client*
        // demanding focus, not about a shell reporting the user's action.
        let effects = self.wm.apply(Event::ActivationRequested {
            window,
            decision: ActivationDecision::Allow,
        });
        debug!(%window, "[toplevel] activated by a taskbar");
        self.apply(effects);
        self.publish();
    }

    /// A taskbar asked for a window to be closed.
    fn toplevel_close(&mut self, window: WindowId) {
        let Some(entry) = self.toplevel_of(window) else {
            trace!(%window, "[toplevel] close for a window that is gone");
            return;
        };
        entry.send_close();
        debug!(%window, "[toplevel] asked to close");
    }

    /// A taskbar asked for a window to be maximised or restored.
    fn toplevel_maximise(&mut self, window: WindowId, maximised: bool) {
        let Some(state) = self.wm.window(window).map(|managed| managed.state) else {
            trace!(%window, "[toplevel] maximise for a window that is gone");
            return;
        };
        if state.maximised != maximised {
            let effects = self.wm.apply(Event::ToggleMaximise { window });
            self.apply(effects);
            self.publish();
        }
    }

    /// A taskbar asked for a window to be minimised or restored.
    fn toplevel_minimise(&mut self, window: WindowId, minimised: bool) {
        let Some(managed) = self.wm.window(window) else {
            trace!(%window, "[toplevel] minimise for a window that is gone");
            return;
        };
        if managed.state.minimised != minimised {
            let event = if minimised {
                Event::Minimise { window }
            } else {
                Event::Unminimise { window }
            };
            let effects = self.wm.apply(event);
            self.apply(effects);
            self.publish();
        }
    }

    /// A taskbar asked for a window to be fullscreened or restored.
    fn toplevel_fullscreen(&mut self, window: WindowId, fullscreen: bool, output: Option<&WlOutput>) {
        let _ = output;
        let Some(state) = self.wm.window(window).map(|managed| managed.state) else {
            trace!(%window, "[toplevel] fullscreen for a window that is gone");
            return;
        };
        if state.fullscreen != fullscreen {
            let effects = self.wm.apply(Event::ToggleFullscreen { window });
            self.apply(effects);
            self.publish();
        }
    }

    /// A taskbar said where it draws a window's thumbnail.
    ///
    /// Nothing arranges minimised windows from it in this slice, so the rectangle is
    /// validated and then dropped: a rectangle that names a surface another connection
    /// owns, or a negative one, is the protocol's `invalid_rectangle` case and is
    /// ignored rather than applied to the wrong window.
    fn toplevel_rectangle(&mut self, window: WindowId, surface: &WlSurface, x: i32, y: i32, width: i32, height: i32) {
        let owned = self
            .surface_of(window)
            .is_some_and(|owned| owned.client() == surface.client());
        if !owned || width < 0 || height < 0 {
            debug!(%window, "[toplevel] an invalid rectangle was ignored");
            return;
        }
        trace!(%window, x, y, width, height, "[toplevel] rectangle");
    }
}

/// One window, as the window manager states it for publication.
struct ToplevelState {
    window: WindowId,
    title: String,
    app_id: String,
    states: States,
    outputs: Vec<OutputId>,
}
