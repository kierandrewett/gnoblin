//! The published window lists: both toplevel protocols are fed from one record of each window.
//!
//! `gnoblin-wayland-ext-foreign-toplevel-list-v1` and `gnoblin-wayland-zwlr-foreign-toplevel-management-v1` send what
//! changed; what is here reads the window manager's windows into the record they share, and decides what changed.

use gnoblin_types::{OutputId, WindowId};
use gnoblin_wayland_protocols::ext_foreign_toplevel_list_v1 as ext;
use gnoblin_wayland_protocols::foreign_toplevel_shared::{Sent, States, ToplevelChanges, ToplevelState};
use gnoblin_wayland_protocols::zwlr_foreign_toplevel_management_v1::{self as zwlr, ToplevelEntry};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId as ClientKey;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;

use crate::State;
use crate::log;

impl State {
    /// Brings both published lists in step with the window record.
    ///
    /// Called from `State::publish` after every change, so a title, a state flag or a
    /// visibility change is announced once and an idle iteration announces nothing.
    pub(crate) fn sync_foreign_toplevels(&mut self) {
        let focused = self.focus;
        let windows: Vec<ToplevelState> = self
            .wm
            .windows()
            .map(|window| {
                let entry = self.windows.get(&window.id);
                // A shell that holds the tree writes these three on the window's node.
                let flags = self.effective_state(window.id, window.state);
                let mut states = States::default();
                if flags.maximised {
                    states.0 |= States::MAXIMISED;
                }
                if flags.minimised {
                    states.0 |= States::MINIMISED;
                }
                if flags.fullscreen {
                    states.0 |= States::FULLSCREEN;
                }
                if focused == Some(window.id) {
                    states.0 |= States::ACTIVATED;
                }
                ToplevelState {
                    window: window.id,
                    title: entry.map_or_else(String::new, |entry| entry.title.clone()),
                    app_id: entry.map_or_else(String::new, |entry| entry.app_id.clone()),
                    states,
                    outputs: if self.window_shown(window.id) {
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
                zwlr::send_closed(&entry);
                self.foreign_toplevel_list_state.remove_toplevel(&entry.ext);
                log::debug!(area = "toplevel", %window, "closed");
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
            zwlr::send_changes(entry, &published, changes, &bound);
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
        let ext = ext::publish::<Self>(&mut self.foreign_toplevel_list_state, published);
        let handles = zwlr::create_handles(self, published.window);
        self.foreign_toplevels.entries.push(ToplevelEntry {
            window: published.window,
            ext,
            handles,
            sent: Sent::default(),
        });
    }
}

impl State {
    /// A taskbar asked for a window to be closed. The close goes to the client, which decides whether
    /// it exits; the window manager is not asked.
    pub(crate) fn toplevel_close(&mut self, window: WindowId) {
        let Some(entry) = self.toplevel_of(window) else {
            log::trace!(area = "toplevel", %window, "close for a window that is gone");
            return;
        };
        entry.send_close();
        log::debug!(area = "toplevel", %window, "asked to close");
    }
}
