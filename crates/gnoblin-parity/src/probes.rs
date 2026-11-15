//! The probes every live target answers: focus, popups and input, observed as a Wayland client sees them.
//!
//! A probe's id is a step id in a scenario. Each one stimulates the compositor through the shared client
//! ([`crate::wayland_peer`]) and reports what the client was told, so the same step runs on the Rust candidate and on
//! stock Mutter 51 and compares against one expectation.
//!
//! Focus is observed as the `activated` state of an `xdg_toplevel` configure, which is how any client learns it has
//! focus and needs no input device. Popup placement is the position and size of the `xdg_popup` configure, relative
//! to the parent's window geometry.
//!
//! Input is injected the way an input emulator does it, through the compositor's EIS server ([`crate::ei_peer`]), and
//! observed as the `wl_keyboard` and `wl_pointer` events the shared client receives. The client's window is
//! fullscreen, so a pointer position anywhere on the output is a position over it on every compositor, and the
//! observation does not depend on where a compositor places an ordinary window.
//!
//! A probe waits for the change it expects and then reports the state it finds either way, so a compositor that does
//! not make the change shows up as a difference in the observation instead of a failed run.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity};

use crate::ei_peer::{EiPeer, EisDoor};
use crate::target::TargetError;
use crate::wayland_peer::{Place, Popup, Toplevel, WaylandPeer};

/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;
/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

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
    "input.key.reaches-focused-toplevel",
    "input.pointer.relative-motion",
    "input.pointer.button",
    "input.pointer.scroll",
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
    /// How the target hands out an EIS socket, or `None` for a target that has none.
    door: Option<Box<dyn EisDoor>>,
    /// The EI client, connected by the first input step.
    ei: Option<EiPeer>,
    /// The capability names the EIS server offered, which tell the Wayland client what the seat should advertise.
    eis_capabilities: Vec<String>,
}

impl Probes {
    /// Connects the shared client to the session's Wayland socket. `door` is how the input steps reach the session's
    /// EIS server.
    pub(crate) fn connect(
        target: &'static str,
        socket: &Path,
        door: Option<Box<dyn EisDoor>>,
    ) -> Result<Self, TargetError> {
        Ok(Self {
            target,
            peer: WaylandPeer::connect(target, socket)?,
            windows: BTreeMap::new(),
            popups: BTreeMap::new(),
            door,
            ei: None,
            eis_capabilities: Vec::new(),
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

    /// The EI client, which `ensure_input` connected.
    fn ei(&mut self) -> Result<&mut EiPeer, TargetError> {
        let target = self.target;
        self.ei.as_mut().ok_or_else(|| TargetError::Live {
            target: target.to_owned(),
            detail: "the EI client is not connected".to_owned(),
        })
    }

    /// Connects the EI client, maps the fullscreen window the input steps watch and waits until the keyboard is on it,
    /// once. The EI client comes first because a Mutter seat only advertises a keyboard once an input device exists, and
    /// it is EIS that makes one: the Wayland peer must see that capability before it asks for the keyboard, and the seat
    /// is bound before the window maps, so the keyboard focus the map causes is seen.
    fn ensure_input(&mut self) -> Result<(), TargetError> {
        if self.ei.is_none() {
            let target = self.target;
            let door = self.door.as_mut().ok_or_else(|| TargetError::Live {
                target: target.to_owned(),
                detail: "the target has no EIS door, so it cannot be given input".to_owned(),
            })?;
            let stream = door.open()?;
            let mut ei = EiPeer::connect(self.target, stream)?;
            self.eis_capabilities = ei.ready()?;
            self.ei = Some(ei);
        }
        self.peer.enable_input(&self.eis_capabilities)?;
        if !self.windows.contains_key("input") {
            let window = self.peer.map_fullscreen("org.gnoblin.parity.input", "input")?;
            self.windows.insert("input", window);
        }
        self.wait_activated("input", true, SETTLE)?;
        self.peer.wait_input(SETTLE, |log| log.keyboard_entered)?;
        Ok(())
    }

    /// Like `ensure_input`, and makes sure the pointer is over the window: a pointer that has not moved yet may not
    /// have entered any surface, so one pixel to the right and back brings it in.
    fn ensure_pointer(&mut self) -> Result<(), TargetError> {
        self.ensure_input()?;
        if !self.peer.input().pointer_entered {
            let ei = self.ei()?;
            ei.motion(1.0, 0.0)?;
            ei.motion(-1.0, 0.0)?;
            self.peer.wait_input(SETTLE, |log| log.pointer_entered)?;
        }
        Ok(())
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
            "input.key.reaches-focused-toplevel" => {
                self.ensure_input()?;
                self.peer.clear_input();
                let ei = self.ei()?;
                ei.key(KEY_A, true)?;
                ei.key(KEY_A, false)?;
                self.peer.wait_input(SETTLE, |log| log.keys.len() >= 2)?;
                let log = self.peer.input();
                Ok(json!({
                    "keyboard_entered": log.keyboard_entered,
                    "keys": edges(&log.keys),
                }))
            }
            "input.pointer.relative-motion" => {
                self.ensure_pointer()?;
                self.peer.clear_input();
                let ei = self.ei()?;
                for _ in 0..3 {
                    ei.motion(10.0, 5.0)?;
                }
                self.peer.wait_input(SETTLE, |log| log.motions.len() >= 3)?;
                let log = self.peer.input();
                // The positions are surface coordinates, which differ between compositors by wherever the pointer
                // started. What the injection decides is the step between two motions, so that is what is reported.
                let steps: Vec<Value> = log
                    .motions
                    .windows(2)
                    .map(|pair| {
                        let (dx, dy) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
                        json!([dx.round() as i64, dy.round() as i64])
                    })
                    .collect();
                Ok(json!({ "pointer_entered": log.pointer_entered, "motions": log.motions.len(), "steps": steps }))
            }
            "input.pointer.button" => {
                self.ensure_pointer()?;
                self.peer.clear_input();
                let ei = self.ei()?;
                ei.button(BTN_LEFT, true)?;
                ei.button(BTN_LEFT, false)?;
                self.peer.wait_input(SETTLE, |log| log.buttons.len() >= 2)?;
                Ok(json!({ "buttons": edges(&self.peer.input().buttons) }))
            }
            "input.pointer.scroll" => {
                self.ensure_pointer()?;
                self.peer.clear_input();
                self.ei()?.scroll(0.0, 15.0)?;
                self.peer.wait_input(SETTLE, |log| !log.axes.is_empty())?;
                let log = self.peer.input();
                // A compositor scales a scroll the way it likes, so the observation is which axis moved and which
                // way, not by how much.
                Ok(json!({
                    "axis_events": !log.axes.is_empty(),
                    "vertical_only": log.axes.iter().all(|(vertical, _)| *vertical),
                    "positive": log.axes.iter().map(|(_, value)| value).sum::<f64>() > 0.0,
                }))
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

/// Key or button edges as an observation: the code and whether it went down.
fn edges(edges: &[(u32, bool)]) -> Value {
    Value::Array(
        edges
            .iter()
            .map(|(code, pressed)| json!({ "code": code, "pressed": pressed }))
            .collect(),
    )
}
