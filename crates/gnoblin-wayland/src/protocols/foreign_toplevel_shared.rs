//! The window record both toplevel lists are fed from.
//!
//! [`zwlr_foreign_toplevel_management_v1`](super::zwlr_foreign_toplevel_management_v1)
//! and [`ext_foreign_toplevel_list_v1`](super::ext_foreign_toplevel_list_v1) each announce
//! the same windows, so a shell that reads one cannot see a different set of windows than
//! a shell that reads the other. This module keeps the one record and works out what
//! changed; each protocol module sends its own events for those changes.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId as ClientKey;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use tracing::debug;
use wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;

use gnoblin_types::{OutputId, WindowId};

use super::{ext_foreign_toplevel_list_v1 as ext, zwlr_foreign_toplevel_management_v1 as wlr};
use crate::State;

/// The state flags of a window, as the window manager states them: one set, not a list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct States(pub(super) u32);

impl States {
    pub(super) const MAXIMISED: u32 = 1 << 0;
    pub(super) const MINIMISED: u32 = 1 << 1;
    pub(super) const ACTIVATED: u32 = 1 << 2;
    pub(super) const FULLSCREEN: u32 = 1 << 3;
}

/// What one window's handles have been told, so an event only goes out when a value
/// actually changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Sent {
    /// The title last sent.
    pub(super) title: String,
    /// The application id last sent.
    pub(super) app_id: String,
    /// The state flags last sent.
    pub(super) states: States,
    /// The outputs the shell has been told about.
    pub(super) outputs: Vec<OutputId>,
    /// Whether the initial batch has been sent, so a shell gets a `done` even when
    /// every value is the default one.
    pub(super) introduced: bool,
}

/// The published record of one window.
pub(super) struct ToplevelEntry {
    /// The window manager's identity for the window.
    pub(super) window: WindowId,
    /// The standardised list's handle for the window, shared by every list instance.
    pub(super) ext: ForeignToplevelHandle,
    /// One handle object per binding of the wlr manager.
    pub(super) handles: Vec<wlr::ZwlrForeignToplevelManagementV1HandleObject>,
    /// What those handles have been told.
    pub(super) sent: Sent,
}

/// The published window list.
#[derive(Default)]
pub(crate) struct ForeignToplevels {
    pub(super) entries: Vec<ToplevelEntry>,
    /// The manager bindings that asked for events.
    ///
    /// A manager leaves the list when its client sends `stop`, because the protocol
    /// says no further toplevel is announced after `finished`.
    pub(super) managers: Vec<ZwlrForeignToplevelManagerV1>,
}

/// One window, as the window manager states it for publication.
pub(super) struct ToplevelState {
    pub(super) window: WindowId,
    pub(super) title: String,
    pub(super) app_id: String,
    pub(super) states: States,
    pub(super) outputs: Vec<OutputId>,
}

/// Which values of one window changed since they were last sent.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ToplevelChanges {
    pub(super) title: bool,
    pub(super) app_id: bool,
    pub(super) states: bool,
    pub(super) outputs: bool,
    /// The window has not been introduced to the lists yet.
    pub(super) introduce: bool,
}

impl ToplevelChanges {
    /// Whether anything changed, which is when a `done` goes out.
    pub(super) fn any(self) -> bool {
        self.title || self.app_id || self.states || self.outputs || self.introduce
    }
}

impl State {
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
                wlr::send_closed(&entry);
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
            let changes = ToplevelChanges {
                title: entry.sent.title != published.title,
                app_id: entry.sent.app_id != published.app_id,
                states: entry.sent.states != published.states,
                outputs: entry.sent.outputs != published.outputs,
                introduce: !entry.sent.introduced,
            };

            ext::send_changes(&entry.ext, &published, changes);
            wlr::send_changes(entry, &published, changes, &bound);
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
        let ext = ext::publish(&mut self.foreign_toplevel_list_state, published);
        let handles = wlr::create_handles(self, published.window);
        self.foreign_toplevels.entries.push(ToplevelEntry {
            window: published.window,
            ext,
            handles,
            sent: Sent::default(),
        });
    }
}
