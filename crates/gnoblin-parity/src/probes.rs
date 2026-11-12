//! The probes every live target answers: focus and popups, observed as a Wayland client sees them.
//!
//! A probe's id is a step id in a scenario. Each one stimulates the compositor through the shared client
//! ([`crate::wayland_peer`]) and reports what the client was told, so the same step runs on the Rust candidate and on
//! stock Mutter 51 and compares against one expectation.
//!
//! Focus is observed as the `activated` state of an `xdg_toplevel` configure, which is how any client learns it has
//! focus and needs no input device. Popup placement is the position and size of the `xdg_popup` configure, relative
//! to the parent's window geometry.
//!
//! A probe waits for the change it expects and then reports the state it finds either way, so a compositor that does
//! not make the change shows up as a difference in the observation instead of a failed run.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity};

use crate::target::TargetError;
use crate::wayland_peer::{Place, Popup, Toplevel, WaylandPeer};

/// How long a probe waits for a change it expects before it reports the state it found.
const SETTLE: Duration = Duration::from_secs(3);
/// How long a probe watches for a change it expects not to happen.
const QUIET: Duration = Duration::from_millis(500);

/// Every step id [`Probes::run`] answers.
const STEPS: &[&str] = &[
    "focus.first-toplevel",
    "focus.second-toplevel",
    "focus.close-focused",
    "popup.place.anchor-bottom-right",
    "popup.place.gravity-top-left",
    "popup.place.offset",
    "popup.place.nested",
    "popup.reposition",
    "popup.map.parent-keeps-focus",
    "popup.destroy.parent-keeps-focus",
    "popup.grab.without-input",
];

/// Whether `step` is one of the shared probes.
pub(crate) fn handles(step: &str) -> bool {
    STEPS.contains(&step)
}

/// The shared client and the windows and popups the steps keep between them.
pub(crate) struct Probes {
    target: &'static str,
    peer: WaylandPeer,
    windows: BTreeMap<&'static str, Toplevel>,
    popups: BTreeMap<&'static str, Popup>,
}

impl Probes {
    /// Connects the shared client to the session's Wayland socket.
    pub(crate) fn connect(target: &'static str, socket: &Path) -> Result<Self, TargetError> {
        Ok(Self {
            target,
            peer: WaylandPeer::connect(target, socket)?,
            windows: BTreeMap::new(),
            popups: BTreeMap::new(),
        })
    }

    fn live(&self, detail: impl Into<String>) -> TargetError {
        TargetError::Live {
            target: self.target.to_owned(),
            detail: detail.into(),
        }
    }

    fn window(&self, name: &str) -> Result<&Toplevel, TargetError> {
        self.windows
            .get(name)
            .ok_or_else(|| self.live(format!("the `{name}` window is not mapped")))
    }

    /// Whether the compositor last told the window `name` that it has focus.
    fn activated(&self, name: &str) -> Result<bool, TargetError> {
        Ok(self.peer.toplevel_state(self.window(name)?).activated)
    }

    /// Maps the toplevel `name` if no earlier step did, and waits for it to be told it has focus. The wait is
    /// bounded: a compositor that never focuses a new window is reported by the observation, not by this call.
    fn ensure_window(&mut self, name: &'static str) -> Result<(), TargetError> {
        if !self.windows.contains_key(name) {
            let window = self.peer.map_toplevel(&format!("org.gnoblin.parity.{name}"), name)?;
            self.windows.insert(name, window);
        }
        self.wait_activated(name, true, SETTLE)
    }

    /// Waits for the window `name` to be told it has (`active`) or has not got focus, bounded by `limit`.
    fn wait_activated(&mut self, name: &'static str, active: bool, limit: Duration) -> Result<(), TargetError> {
        let window = self
            .windows
            .remove(name)
            .ok_or_else(|| self.live(format!("the `{name}` window is not mapped")))?;
        let waited = self.peer.wait_activated(&window, active, limit);
        self.windows.insert(name, window);
        waited.map(|_| ())
    }

    /// Destroys the popup `name` if it is mapped. A popup is destroyed topmost first, so closing `menu` closes the
    /// `submenu` it carries before it.
    fn close_popup(&mut self, name: &str) -> Result<(), TargetError> {
        if name == "menu"
            && let Some(popup) = self.popups.remove("submenu")
        {
            self.peer.destroy_popup(popup)?;
        }
        if let Some(popup) = self.popups.remove(name) {
            self.peer.destroy_popup(popup)?;
        }
        Ok(())
    }

    /// Maps the popup `name` of `parent`, replacing an earlier popup of that name, and returns its objects' state.
    fn map_popup(&mut self, name: &'static str, parent: Parent, place: &Place, grab: bool) -> Result<(), TargetError> {
        self.close_popup(name)?;
        let parent = match parent {
            Parent::Window(parent) => self.window(parent)?.xdg_surface().clone(),
            Parent::Popup(parent) => self
                .popups
                .get(parent)
                .ok_or_else(|| self.live(format!("the `{parent}` popup is not mapped")))?
                .xdg_surface()
                .clone(),
        };
        let popup = self.peer.map_popup(&parent, place, grab)?;
        self.popups.insert(name, popup);
        Ok(())
    }

    /// Where the compositor last placed the popup `name`.
    fn placed(&self, name: &str) -> Result<Value, TargetError> {
        let popup = self
            .popups
            .get(name)
            .ok_or_else(|| self.live(format!("the `{name}` popup is not mapped")))?;
        Ok(placement(self.peer.popup_state(popup).placed))
    }

    /// Maps a window and one popup of it, and reports the placement.
    fn place(&mut self, place: &Place) -> Result<Value, TargetError> {
        self.ensure_window("a")?;
        self.map_popup("menu", Parent::Window("a"), place, false)?;
        self.placed("menu")
    }

    /// Produces the observation for one step id.
    pub(crate) fn run(&mut self, step: &str) -> Result<Value, TargetError> {
        match step {
            "focus.first-toplevel" => {
                self.ensure_window("a")?;
                Ok(json!({ "a_activated": self.activated("a")? }))
            }
            "focus.second-toplevel" => {
                self.ensure_window("a")?;
                self.ensure_window("b")?;
                self.wait_activated("a", false, SETTLE)?;
                Ok(json!({ "a_activated": self.activated("a")?, "b_activated": self.activated("b")? }))
            }
            "focus.close-focused" => {
                let window = self
                    .windows
                    .remove("b")
                    .ok_or_else(|| self.live("`focus.close-focused` needs `focus.second-toplevel` first"))?;
                self.peer.destroy_toplevel(window)?;
                self.wait_activated("a", true, SETTLE)?;
                Ok(json!({ "a_activated": self.activated("a")? }))
            }
            "popup.place.anchor-bottom-right" => self.place(&Place {
                anchor_rect: (20, 20, 10, 10),
                size: (60, 40),
                anchor: Anchor::BottomRight,
                gravity: Gravity::BottomRight,
                offset: (0, 0),
            }),
            "popup.place.gravity-top-left" => self.place(&Place {
                anchor_rect: (80, 70, 10, 10),
                size: (60, 40),
                anchor: Anchor::TopLeft,
                gravity: Gravity::TopLeft,
                offset: (0, 0),
            }),
            "popup.place.offset" => self.place(&Place {
                anchor_rect: (50, 20, 20, 10),
                size: (60, 40),
                anchor: Anchor::Bottom,
                gravity: Gravity::Bottom,
                offset: (5, 7),
            }),
            "popup.place.nested" => {
                self.place(&Place {
                    anchor_rect: (20, 20, 10, 10),
                    size: (60, 40),
                    anchor: Anchor::BottomRight,
                    gravity: Gravity::BottomRight,
                    offset: (0, 0),
                })?;
                self.map_popup(
                    "submenu",
                    Parent::Popup("menu"),
                    &Place {
                        anchor_rect: (0, 0, 60, 40),
                        size: (30, 20),
                        anchor: Anchor::TopRight,
                        gravity: Gravity::BottomRight,
                        offset: (0, 0),
                    },
                    false,
                )?;
                self.placed("submenu")
            }
            "popup.reposition" => {
                self.place(&Place {
                    anchor_rect: (20, 20, 10, 10),
                    size: (60, 40),
                    anchor: Anchor::BottomRight,
                    gravity: Gravity::BottomRight,
                    offset: (0, 0),
                })?;
                let popup = self
                    .popups
                    .remove("menu")
                    .ok_or_else(|| self.live("the `menu` popup is not mapped"))?;
                let again = Place {
                    anchor_rect: (50, 50, 10, 10),
                    size: (40, 30),
                    anchor: Anchor::BottomRight,
                    gravity: Gravity::BottomRight,
                    offset: (0, 0),
                };
                let sent = self.peer.reposition(&popup, &again, 7);
                let waited = sent.and_then(|()| {
                    self.peer
                        .wait_popup(&popup, SETTLE, |state| state.repositioned == Some(7))
                });
                let state = self.peer.popup_state(&popup);
                self.popups.insert("menu", popup);
                waited?;
                Ok(json!({ "token": state.repositioned, "placement": placement(state.placed) }))
            }
            "popup.map.parent-keeps-focus" => {
                self.ensure_window("a")?;
                self.map_popup(
                    "menu",
                    Parent::Window("a"),
                    &Place {
                        anchor_rect: (20, 20, 10, 10),
                        size: (60, 40),
                        anchor: Anchor::BottomRight,
                        gravity: Gravity::BottomRight,
                        offset: (0, 0),
                    },
                    false,
                )?;
                self.wait_activated("a", false, QUIET)?;
                let popup = self
                    .popups
                    .get("menu")
                    .ok_or_else(|| self.live("the `menu` popup is not mapped"))?;
                let state = self.peer.popup_state(popup);
                Ok(json!({
                    "parent_activated": self.activated("a")?,
                    "popup_configured": state.placed.is_some(),
                    "popup_done": state.done,
                }))
            }
            "popup.destroy.parent-keeps-focus" => {
                self.close_popup("menu")?;
                self.wait_activated("a", false, QUIET)?;
                Ok(json!({ "parent_activated": self.activated("a")? }))
            }
            "popup.grab.without-input" => {
                self.ensure_window("a")?;
                self.map_popup(
                    "menu",
                    Parent::Window("a"),
                    &Place {
                        anchor_rect: (20, 20, 10, 10),
                        size: (60, 40),
                        anchor: Anchor::BottomRight,
                        gravity: Gravity::BottomRight,
                        offset: (0, 0),
                    },
                    true,
                )?;
                let popup = self
                    .popups
                    .remove("menu")
                    .ok_or_else(|| self.live("the `menu` popup is not mapped"))?;
                let waited = self.peer.wait_popup(&popup, QUIET, |state| state.done);
                let state = self.peer.popup_state(&popup);
                self.popups.insert("menu", popup);
                waited?;
                Ok(json!({ "popup_done": state.done, "parent_activated": self.activated("a")? }))
            }
            other => Err(self.live(format!("no shared probe for step `{other}`"))),
        }
    }
}

/// How a popup's parent is named.
#[derive(Clone, Copy)]
enum Parent {
    Window(&'static str),
    Popup(&'static str),
}

/// A popup placement as an observation.
fn placement(placed: Option<(i32, i32, i32, i32)>) -> Value {
    match placed {
        Some((x, y, width, height)) => json!({ "x": x, "y": y, "width": width, "height": height }),
        None => Value::Null,
    }
}
