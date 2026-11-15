//! libinput's events as the seat's, and the state a mapper has to remember.
//!
//! libinput reports what the *device* did: a relative delta, absolute device units, a wheel
//! notch in 1/120 of a step, a key code offset by the eight the X protocol reserved. The
//! contract states what the *seat* needs (design/rust-compositor.md, "Backends and outputs"):
//! a position inside an output, whole logical pixels, and the evdev codes the keymap
//! translates. [`Mapper`] is that translation, and it is pure — the device side reaches it as
//! [`DeviceEvent`] values, so every rule below is tested on a machine with no input device.
//!
//! # What the mapper remembers, and why
//!
//! Three things cannot be computed from the event in hand:
//!
//! - **The fraction of a pixel a slow pointer has not travelled yet.** A 400 dpi mouse at a
//!   slow hand speed reports less than one pixel per frame, and dropping the remainder makes
//!   the pointer stand still until the hand is fast enough: [`Mapper::event`] accumulates it.
//! - **Which buttons and touches are held, and by which device.** A device that is removed
//!   mid-drag never reports the release, and a seat that keeps the button held leaves a stuck
//!   button that eats every later click. [`Mapper::cancel_device`] is what a removal calls and
//!   [`Mapper::cancel`] is what a pause calls.
//! - **Where the pointer is**, so a pointer that is dragged off an output that is unplugged
//!   lands on the remaining one instead of vanishing.
//!
//! # Failure modes first
//!
//! 1. **A slow pointer does not move.** Relative motion keeps its fraction and reports whole
//!    pixels once there are any: `a_slow_pointer_still_moves`.
//! 2. **A device removed mid-drag leaves a button held.** [`Mapper::cancel`] releases every
//!    button it saw pressed: `a_removed_device_releases_every_held_button`.
//! 3. **A device removed mid-touch leaves a live touch point.** Every held slot is cancelled,
//!    once: `a_removed_device_cancels_every_held_touch`.
//! 4. **A cancel after a cancel reports the same release twice.** Nothing is held the second
//!    time, so nothing is reported: `a_second_cancel_reports_nothing`.
//! 5. **An absolute point outside the output is dropped.** It is clamped into the output, so a
//!    touchscreen calibrated slightly wide still reaches the pointer:
//!    `an_absolute_point_outside_the_output_is_clamped`.
//! 6. **A touchpad's fractional scroll is rounded to zero.** Scroll accumulates in 1/120 of a
//!    step, the unit `REL_WHEEL_HI_RES` uses: `fractional_scroll_adds_up_to_a_unit`.
//! 7. **The pointer is lost when its output is unplugged.** The mapper keeps it inside the
//!    layout: `unplugging_an_output_moves_the_pointer_to_one_that_is_left`.
//! 8. **A key release is reported as a press, or the reverse.** The state is carried through
//!    unchanged: `a_key_carries_its_own_edge`.
//! 9. **An event loses its device.** Every event the mapper returns names the device it was
//!    given: `every_event_names_its_device`.
//! 10. **One device's removal releases another device's button.** [`Mapper::cancel_device`]
//!     retires only that device's state: `a_removed_device_does_not_release_anothers_button`.
//! 11. **A pen is left hovering by an unplugged tablet.** The mapper remembers which tools are in proximity and
//!     whether their tips are down, so a removal or a pause ends them, tip first:
//!     `a_removed_tablet_lifts_the_tip_and_takes_the_tool_out`.
//! 12. **A tool position is not on the output.** A tablet reports a fraction of its own range and the mapper places it
//!     on the first output, continuous and clamped: `a_tool_position_is_placed_on_the_output`.

use std::collections::BTreeSet;

use gnoblin_types::{DeviceId, OutputId, Point, PointF, Size};

use gnoblin_backend::{InputEvent, TabletAxes, TabletEvent, TabletTool, TouchPhase};

/// How many millionths of a scroll step make one step.
///
/// [`InputEvent::PointerAxis`] states its values in 1/120 of a step because that is the unit
/// Linux reports high-resolution wheels in, so a scroll amount is carried as a fraction of a
/// step and converted to that unit when it is reported.
const STEPS_PER_UNIT: f64 = 120.0;

/// One output the mapper places absolute events on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Screen {
    /// The graph token the event names.
    pub id: OutputId,
    /// The output's logical extent, which is the space a position is stated in.
    pub logical: Size,
}

/// One event, in the terms the device produced it.
///
/// The variants are libinput's categories, not its types: a pointer, a wheel, absolute
/// coordinates, a key and a touch point. Absolute coordinates are normalised to 0..1 over the
/// device's own range, which is what the glue divides by the axis extents to get.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DeviceEvent {
    /// A key changed state; `code` is the evdev code, already offset from libinput's own.
    Key {
        /// The evdev key code.
        code: u32,
        /// Whether the key is now held.
        pressed: bool,
    },
    /// The pointer moved by a delta, in device units (which are pixels on a mouse).
    Motion {
        /// Horizontal movement.
        dx: f64,
        /// Vertical movement.
        dy: f64,
    },
    /// An absolute device reported a position, normalised to 0..1 on both axes.
    Absolute {
        /// Horizontal position.
        x: f64,
        /// Vertical position.
        y: f64,
    },
    /// A button changed state; `code` is the evdev button code.
    Button {
        /// The button code.
        code: u32,
        /// Whether the button is now held.
        pressed: bool,
    },
    /// A wheel or a touchpad scrolled, in fractions of a step.
    Scroll {
        /// Horizontal movement.
        horizontal: f64,
        /// Vertical movement.
        vertical: f64,
    },
    /// A touch point changed state, normalised to 0..1 on both axes.
    Touch {
        /// The device's touch identity.
        slot: u32,
        /// Horizontal position.
        x: f64,
        /// Vertical position.
        y: f64,
        /// What happened.
        phase: TouchPhase,
    },
    /// A drawing tablet's tool or pad did something.
    Tablet(TabletDeviceEvent),
}

/// What a tablet's tool or pad did, in the terms libinput reports it: positions are fractions of the tablet's own
/// range from 0 to 1, and the axes carry only what changed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TabletDeviceEvent {
    /// A tool came into (`inside`) or left proximity of the tablet.
    ToolProximity {
        /// The tool.
        tool: TabletTool,
        /// Whether it is in proximity now.
        inside: bool,
        /// Horizontal position.
        x: f64,
        /// Vertical position.
        y: f64,
        /// The axes it reports.
        axes: TabletAxes,
    },
    /// A tool moved or one of its axes changed.
    ToolAxis {
        /// The tool.
        tool: TabletTool,
        /// Horizontal position.
        x: f64,
        /// Vertical position.
        y: f64,
        /// The axes that changed.
        axes: TabletAxes,
    },
    /// A tool's tip touched down or lifted.
    ToolTip {
        /// The tool.
        tool: TabletTool,
        /// Whether the tip is down now.
        down: bool,
    },
    /// A tool's button changed; `code` is the evdev button code.
    ToolButton {
        /// The tool.
        tool: TabletTool,
        /// The button code.
        code: u32,
        /// Whether it is held now.
        pressed: bool,
    },
    /// A pad button changed.
    PadButton {
        /// The button's number.
        button: u32,
        /// Whether it is held now.
        pressed: bool,
    },
    /// A pad ring was touched at an angle in degrees, or the finger lifted.
    PadRing {
        /// The ring's number.
        ring: u32,
        /// Where the finger is.
        angle: Option<f64>,
    },
    /// A pad strip was touched at a position from 0 to 1, or the finger lifted.
    PadStrip {
        /// The strip's number.
        strip: u32,
        /// Where the finger is.
        position: Option<f64>,
    },
}

/// A tool the mapper has seen come into proximity and not leave.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Hovering {
    device: DeviceId,
    tool: TabletTool,
    tip_down: bool,
}

/// Turns device events into the events a seat applies.
///
/// It owns the pointer position, the held buttons and the held touch slots. One mapper belongs
/// to one seat and not to one device, because the seat has one pointer; what it remembers about
/// a button or a touch point is remembered *with the device that made it*, so a second mouse's
/// release finds the button its own press recorded and one device's removal releases only its
/// own.
#[derive(Clone, Debug, Default)]
pub struct Mapper {
    /// The outputs, in connection order.
    screens: Vec<Screen>,
    /// The output absolute devices report on, which is the first one until a caller says
    /// otherwise.
    absolute: Option<OutputId>,
    /// The fraction of a logical pixel the pointer has travelled but has not been moved by.
    remainder: (f64, f64),
    /// The fraction of a scroll step that has not been reported yet.
    scroll: (f64, f64),
    /// The buttons currently held and the device that holds each, so a cancel can release them.
    held: BTreeSet<(DeviceId, u32)>,
    /// The touch slots currently down and the device each belongs to, so a cancel can cancel them.
    touches: BTreeSet<(DeviceId, u32)>,
    /// Where the pointer is, when it is on an output.
    pointer: Option<(OutputId, Point)>,
    /// The tablet tools in proximity, so a removal or a pause can end them.
    hovering: Vec<Hovering>,
}

impl Mapper {
    /// A mapper that knows about no outputs.
    ///
    /// Absolute and touch events are dropped until [`Mapper::set_screens`] names one, because a
    /// position with no output to be relative to is not a position the contract can state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the output table, which is what a hotplug does.
    ///
    /// A pointer that was on an output that is gone moves to the first output that is left, so
    /// the first relative motion afterwards starts from somewhere a window can be clicked. A
    /// pointer with nowhere to go is forgotten.
    pub fn set_screens(&mut self, screens: Vec<Screen>) {
        self.screens = screens;
        self.absolute = self.screens.first().map(|screen| screen.id);
        if let Some((output, position)) = self.pointer {
            if let Some(screen) = self.screens.iter().find(|screen| screen.id == output) {
                self.pointer = Some((output, clamp(position, screen.logical)));
            } else {
                self.pointer = self.screens.first().map(|screen| (screen.id, Point::new(0, 0)));
            }
        }
    }

    /// The output absolute devices report on.
    #[must_use]
    pub fn absolute_output(&self) -> Option<OutputId> {
        self.absolute
    }

    /// Where the pointer is, when it is on an output.
    #[must_use]
    pub fn pointer(&self) -> Option<(OutputId, Point)> {
        self.pointer
    }

    /// The buttons currently held, each with the device that holds it.
    #[must_use]
    pub fn held_buttons(&self) -> &BTreeSet<(DeviceId, u32)> {
        &self.held
    }

    /// Translates one event `device` produced.
    ///
    /// A frame with several events calls this once per event and concatenates the answers, so
    /// the order the device reported is the order the seat sees. Every event it returns names
    /// `device`.
    #[must_use]
    pub fn event(&mut self, device: DeviceId, event: DeviceEvent) -> Vec<InputEvent> {
        let mut events = Vec::new();
        match event {
            DeviceEvent::Key { code, pressed } => events.push(InputEvent::Key {
                device,
                keycode: code,
                pressed,
            }),
            DeviceEvent::Button { code, pressed } => {
                if pressed {
                    self.held.insert((device, code));
                } else {
                    self.held.remove(&(device, code));
                }
                events.push(InputEvent::PointerButton {
                    device,
                    button: code,
                    pressed,
                });
            }
            DeviceEvent::Motion { dx, dy } => {
                let total = (self.remainder.0 + dx, self.remainder.1 + dy);
                let whole = (total.0.trunc(), total.1.trunc());
                self.remainder = (total.0 - whole.0, total.1 - whole.1);
                if whole.0 != 0.0 || whole.1 != 0.0 {
                    events.push(InputEvent::PointerMotionRelative {
                        device,
                        dx: saturate(whole.0),
                        dy: saturate(whole.1),
                    });
                }
            }
            DeviceEvent::Absolute { x, y } => {
                if let Some((output, position)) = self.place(x, y) {
                    self.pointer = Some((output, position));
                    events.push(InputEvent::PointerMotion {
                        device,
                        output,
                        position,
                    });
                }
            }
            DeviceEvent::Scroll { horizontal, vertical } => {
                let total = (self.scroll.0 + horizontal, self.scroll.1 + vertical);
                let reported = (total.0 * STEPS_PER_UNIT, total.1 * STEPS_PER_UNIT);
                let whole = (reported.0.trunc(), reported.1.trunc());
                self.scroll = (total.0 - whole.0 / STEPS_PER_UNIT, total.1 - whole.1 / STEPS_PER_UNIT);
                if whole.0 != 0.0 || whole.1 != 0.0 {
                    events.push(InputEvent::PointerAxis {
                        device,
                        horizontal: saturate(whole.0),
                        vertical: saturate(whole.1),
                    });
                }
            }
            DeviceEvent::Touch { slot, x, y, phase } => {
                if phase == TouchPhase::Up || phase == TouchPhase::Cancel {
                    self.touches.remove(&(device, slot));
                } else {
                    self.touches.insert((device, slot));
                }
                if let Some((output, position)) = self.place(x, y) {
                    events.push(InputEvent::Touch {
                        device,
                        output,
                        slot,
                        position,
                        phase,
                    });
                }
            }
            DeviceEvent::Tablet(event) => self.tablet(device, event, &mut events),
        }
        events
    }

    /// Translates one tablet event. A tool event with no output to place it on is dropped, as an absolute point is.
    fn tablet(&mut self, device: DeviceId, event: TabletDeviceEvent, events: &mut Vec<InputEvent>) {
        match event {
            TabletDeviceEvent::ToolProximity {
                tool,
                inside: true,
                x,
                y,
                axes,
            } => {
                let Some((output, position)) = self.place_f(x, y) else {
                    return;
                };
                self.hovering
                    .retain(|held| !(held.device == device && held.tool == tool));
                self.hovering.push(Hovering {
                    device,
                    tool,
                    tip_down: false,
                });
                events.push(
                    TabletEvent::ToolIn {
                        device,
                        output,
                        tool,
                        position,
                        axes: axes.limited_to(tool.axes),
                    }
                    .into(),
                );
            }
            TabletDeviceEvent::ToolProximity {
                tool, inside: false, ..
            } => {
                events.extend(self.leave(|held| held.device == device && held.tool == tool));
            }
            TabletDeviceEvent::ToolAxis { tool, x, y, axes } => {
                if !self.is_hovering(device, tool) {
                    return;
                }
                let Some((output, position)) = self.place_f(x, y) else {
                    return;
                };
                events.push(
                    TabletEvent::ToolMotion {
                        device,
                        output,
                        tool,
                        position,
                        axes: axes.limited_to(tool.axes),
                    }
                    .into(),
                );
            }
            TabletDeviceEvent::ToolTip { tool, down } => {
                let Some(held) = self
                    .hovering
                    .iter_mut()
                    .find(|held| held.device == device && held.tool == tool)
                else {
                    return;
                };
                if held.tip_down == down {
                    return;
                }
                held.tip_down = down;
                events.push(TabletEvent::ToolTip { device, tool, down }.into());
            }
            TabletDeviceEvent::ToolButton { tool, code, pressed } => {
                if self.is_hovering(device, tool) {
                    events.push(
                        TabletEvent::ToolButton {
                            device,
                            tool,
                            button: code,
                            pressed,
                        }
                        .into(),
                    );
                }
            }
            TabletDeviceEvent::PadButton { button, pressed } => {
                events.push(
                    TabletEvent::PadButton {
                        device,
                        button,
                        pressed,
                    }
                    .into(),
                );
            }
            TabletDeviceEvent::PadRing { ring, angle } => {
                events.push(TabletEvent::PadRing { device, ring, angle }.into());
            }
            TabletDeviceEvent::PadStrip { strip, position } => {
                events.push(
                    TabletEvent::PadStrip {
                        device,
                        strip,
                        position,
                    }
                    .into(),
                );
            }
        }
    }

    fn is_hovering(&self, device: DeviceId, tool: TabletTool) -> bool {
        self.hovering
            .iter()
            .any(|held| held.device == device && held.tool == tool)
    }

    /// Ends every hovering tool `takes` selects: the tip lifts first, then the tool leaves.
    fn leave(&mut self, takes: impl Fn(&Hovering) -> bool) -> Vec<InputEvent> {
        let mut events = Vec::new();
        let (gone, kept): (Vec<Hovering>, Vec<Hovering>) =
            std::mem::take(&mut self.hovering).into_iter().partition(takes);
        self.hovering = kept;
        for held in gone {
            if held.tip_down {
                events.push(
                    TabletEvent::ToolTip {
                        device: held.device,
                        tool: held.tool,
                        down: false,
                    }
                    .into(),
                );
            }
            events.push(
                TabletEvent::ToolOut {
                    device: held.device,
                    tool: held.tool,
                }
                .into(),
            );
        }
        events
    }

    /// Everything a seat must be told when the session stops delivering.
    ///
    /// It is the answer to "the session was paused with a button held": a release for every
    /// button and a cancellation for every touch point of every device, and nothing at all the
    /// second time.
    #[must_use]
    pub fn cancel(&mut self) -> Vec<InputEvent> {
        let mut events = self.release(|_| true);
        events.extend(self.leave(|_| true));
        self.remainder = (0.0, 0.0);
        self.scroll = (0.0, 0.0);
        events
    }

    /// Everything a seat must be told when one device is removed.
    ///
    /// It is the answer to "the device was removed with a button held": a release for every
    /// button and a cancellation for every touch point *that device* held, and nothing at all
    /// the second time. Another device's held buttons and touch points are untouched.
    #[must_use]
    pub fn cancel_device(&mut self, device: DeviceId) -> Vec<InputEvent> {
        let mut events = self.release(|held| held == device);
        events.extend(self.leave(|held| held.device == device));
        events
    }

    /// The releases and cancellations for every held button and touch point whose device `takes`.
    fn release(&mut self, takes: impl Fn(DeviceId) -> bool) -> Vec<InputEvent> {
        let mut events = Vec::new();
        let buttons: Vec<(DeviceId, u32)> = self.held.iter().copied().filter(|(device, _)| takes(*device)).collect();
        for (device, button) in buttons {
            self.held.remove(&(device, button));
            events.push(InputEvent::PointerButton {
                device,
                button,
                pressed: false,
            });
        }
        let slot_screen = self
            .absolute
            .and_then(|id| self.screens.iter().find(|screen| screen.id == id).copied());
        let touches: Vec<(DeviceId, u32)> = self
            .touches
            .iter()
            .copied()
            .filter(|(device, _)| takes(*device))
            .collect();
        for (device, slot) in touches {
            self.touches.remove(&(device, slot));
            if let Some(screen) = slot_screen {
                events.push(InputEvent::Touch {
                    device,
                    output: screen.id,
                    slot,
                    position: Point::new(0, 0),
                    phase: TouchPhase::Cancel,
                });
            }
        }
        events
    }

    /// A normalised absolute point as an output and a continuous position in it, for a tablet's tool: a pen resolves
    /// far below a pixel, so the position is not rounded to one.
    fn place_f(&self, x: f64, y: f64) -> Option<(OutputId, PointF)> {
        let screen = match self.absolute {
            Some(id) => self.screens.iter().find(|screen| screen.id == id).copied(),
            None => self.screens.first().copied(),
        }?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let position = PointF::new(
            x.clamp(0.0, 1.0) * f64::from(screen.logical.width),
            y.clamp(0.0, 1.0) * f64::from(screen.logical.height),
        );
        Some((screen.id, position))
    }

    /// A normalised absolute point as an output and a position in it.
    fn place(&self, x: f64, y: f64) -> Option<(OutputId, Point)> {
        let screen = match self.absolute {
            Some(id) => self.screens.iter().find(|screen| screen.id == id).copied(),
            None => self.screens.first().copied(),
        }?;
        let width = f64::from(screen.logical.width.max(1));
        let height = f64::from(screen.logical.height.max(1));
        let position = Point::new(
            (x.clamp(0.0, 1.0) * width).round() as i32,
            (y.clamp(0.0, 1.0) * height).round() as i32,
        );
        Some((screen.id, clamp(position, screen.logical)))
    }
}

/// A position inside an output, with the far edge pulled in by one pixel.
fn clamp(position: Point, logical: Size) -> Point {
    let max_x = i32::try_from(logical.width)
        .unwrap_or(i32::MAX)
        .saturating_sub(1)
        .max(0);
    let max_y = i32::try_from(logical.height)
        .unwrap_or(i32::MAX)
        .saturating_sub(1)
        .max(0);
    Point::new(position.x.clamp(0, max_x), position.y.clamp(0, max_y))
}

/// A whole number of pixels as the contract states it, saturating rather than wrapping.
fn saturate(value: f64) -> i32 {
    if value >= f64::from(i32::MAX) {
        i32::MAX
    } else if value <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mouse most of these tests move; the keyboard and the second mouse are for the ones that need two.
    const MOUSE: DeviceId = DeviceId::new(1);
    const KEYBOARD: DeviceId = DeviceId::new(2);
    const SECOND_MOUSE: DeviceId = DeviceId::new(3);
    const TABLET: DeviceId = DeviceId::new(4);

    fn screen(id: u64, width: u32, height: u32) -> Screen {
        Screen {
            id: OutputId::new(id),
            logical: Size::new(width, height),
        }
    }

    fn full_hd() -> Screen {
        screen(1, 1920, 1080)
    }

    fn pen() -> TabletTool {
        TabletTool::pen(5)
    }

    fn hover(mapper: &mut Mapper, device: DeviceId) -> Vec<InputEvent> {
        mapper.event(
            device,
            DeviceEvent::Tablet(TabletDeviceEvent::ToolProximity {
                tool: pen(),
                inside: true,
                x: 0.5,
                y: 0.25,
                axes: TabletAxes::default(),
            }),
        )
    }

    #[test]
    fn a_tool_position_is_placed_on_the_output() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let events = hover(&mut mapper, TABLET);
        assert_eq!(
            events,
            vec![
                TabletEvent::ToolIn {
                    device: TABLET,
                    output: OutputId::new(1),
                    tool: pen(),
                    position: PointF::new(960.0, 270.0),
                    axes: TabletAxes::default(),
                }
                .into()
            ]
        );
        let moved = mapper.event(
            TABLET,
            DeviceEvent::Tablet(TabletDeviceEvent::ToolAxis {
                tool: pen(),
                x: 2.0,
                y: -1.0,
                axes: TabletAxes {
                    pressure: Some(0.5),
                    ..TabletAxes::default()
                },
            }),
        );
        let [InputEvent::Tablet(TabletEvent::ToolMotion { position, axes, .. })] = moved.as_slice() else {
            panic!("one motion: {moved:?}");
        };
        assert_eq!(
            *position,
            PointF::new(1920.0, 0.0),
            "a position past the range is clamped to the edge"
        );
        assert_eq!(axes.pressure, Some(0.5));
    }

    #[test]
    fn a_tool_with_no_output_to_be_on_says_nothing() {
        let mut mapper = Mapper::new();
        assert!(hover(&mut mapper, TABLET).is_empty());
    }

    #[test]
    fn a_motion_of_a_tool_that_never_arrived_is_dropped() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let moved = mapper.event(
            TABLET,
            DeviceEvent::Tablet(TabletDeviceEvent::ToolAxis {
                tool: pen(),
                x: 0.1,
                y: 0.1,
                axes: TabletAxes::default(),
            }),
        );
        assert!(moved.is_empty());
    }

    #[test]
    fn a_removed_tablet_lifts_the_tip_and_takes_the_tool_out() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        hover(&mut mapper, TABLET);
        let _ = mapper.event(
            TABLET,
            DeviceEvent::Tablet(TabletDeviceEvent::ToolTip {
                tool: pen(),
                down: true,
            }),
        );
        let ended = mapper.cancel_device(TABLET);
        assert_eq!(
            ended,
            vec![
                TabletEvent::ToolTip {
                    device: TABLET,
                    tool: pen(),
                    down: false
                }
                .into(),
                TabletEvent::ToolOut {
                    device: TABLET,
                    tool: pen()
                }
                .into(),
            ]
        );
        assert!(mapper.cancel_device(TABLET).is_empty(), "a second removal ends nothing");
    }

    #[test]
    fn a_tablet_removal_leaves_another_tablets_tool() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        hover(&mut mapper, TABLET);
        assert!(mapper.cancel_device(SECOND_MOUSE).is_empty());
        assert_eq!(mapper.cancel().len(), 1, "a pause ends every tool");
    }

    #[test]
    fn a_pad_names_the_device_that_pressed_it() {
        let mut mapper = Mapper::new();
        let events = mapper.event(
            TABLET,
            DeviceEvent::Tablet(TabletDeviceEvent::PadButton {
                button: 3,
                pressed: true,
            }),
        );
        assert_eq!(
            events,
            vec![
                TabletEvent::PadButton {
                    device: TABLET,
                    button: 3,
                    pressed: true
                }
                .into()
            ]
        );
    }

    #[test]
    fn a_slow_pointer_still_moves() {
        // A mouse that reports 0.3 of a pixel per frame must still reach the pointer on the
        // fourth frame; rounding each frame to zero would freeze it for as long as the hand
        // moves slowly.
        let mut mapper = Mapper::new();
        for _ in 0..3 {
            assert!(
                mapper.event(MOUSE, DeviceEvent::Motion { dx: 0.3, dy: 0.0 }).is_empty(),
                "less than a pixel has been travelled"
            );
        }
        assert_eq!(
            mapper.event(MOUSE, DeviceEvent::Motion { dx: 0.3, dy: 0.0 }),
            vec![InputEvent::PointerMotionRelative {
                device: MOUSE,
                dx: 1,
                dy: 0
            }]
        );
    }

    #[test]
    fn a_relative_delta_keeps_its_sign() {
        let mut mapper = Mapper::new();
        assert_eq!(
            mapper.event(MOUSE, DeviceEvent::Motion { dx: -3.0, dy: 4.0 }),
            vec![InputEvent::PointerMotionRelative {
                device: MOUSE,
                dx: -3,
                dy: 4
            }]
        );
    }

    #[test]
    fn a_removed_device_releases_every_held_button() {
        let mut mapper = Mapper::new();
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Button {
                code: 272,
                pressed: true,
            },
        );
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Button {
                code: 273,
                pressed: true,
            },
        );
        let cancelled = mapper.cancel();
        assert_eq!(
            cancelled,
            vec![
                InputEvent::PointerButton {
                    device: MOUSE,
                    button: 272,
                    pressed: false
                },
                InputEvent::PointerButton {
                    device: MOUSE,
                    button: 273,
                    pressed: false
                },
            ]
        );
        assert!(mapper.held_buttons().is_empty());
    }

    #[test]
    fn a_removed_device_cancels_every_held_touch() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Touch {
                slot: 4,
                x: 0.5,
                y: 0.5,
                phase: TouchPhase::Down,
            },
        );
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Touch {
                slot: 5,
                x: 0.25,
                y: 0.25,
                phase: TouchPhase::Down,
            },
        );
        let cancelled = mapper.cancel();
        assert_eq!(cancelled.len(), 2);
        for event in &cancelled {
            match event {
                InputEvent::Touch { phase, .. } => assert_eq!(*phase, TouchPhase::Cancel),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn a_lifted_touch_is_not_cancelled_again() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Touch {
                slot: 0,
                x: 0.5,
                y: 0.5,
                phase: TouchPhase::Down,
            },
        );
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Touch {
                slot: 0,
                x: 0.5,
                y: 0.5,
                phase: TouchPhase::Up,
            },
        );
        assert!(mapper.cancel().is_empty());
    }

    #[test]
    fn a_second_cancel_reports_nothing() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Button {
                code: 272,
                pressed: true,
            },
        );
        assert_eq!(mapper.cancel().len(), 1);
        assert!(mapper.cancel().is_empty());
    }

    #[test]
    fn an_absolute_point_outside_the_output_is_clamped() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        assert_eq!(
            mapper.event(MOUSE, DeviceEvent::Absolute { x: 1.4, y: -2.0 }),
            vec![InputEvent::PointerMotion {
                device: MOUSE,
                output: OutputId::new(1),
                position: Point::new(1919, 0)
            }]
        );
    }

    #[test]
    fn an_absolute_point_lands_where_it_was_reported() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        assert_eq!(
            mapper.event(MOUSE, DeviceEvent::Absolute { x: 0.5, y: 0.25 }),
            vec![InputEvent::PointerMotion {
                device: MOUSE,
                output: OutputId::new(1),
                position: Point::new(960, 270)
            }]
        );
        assert_eq!(mapper.pointer(), Some((OutputId::new(1), Point::new(960, 270))));
    }

    #[test]
    fn fractional_scroll_adds_up_to_a_unit() {
        let mut mapper = Mapper::new();
        // A touchpad reports 1/128 of a step twice. That is 15/16 of a 1/120 unit, which is not
        // reportable on its own; the second brings the total to 1 7/8 units and reports one.
        let first = mapper.event(
            MOUSE,
            DeviceEvent::Scroll {
                horizontal: 0.0,
                vertical: 1.0 / 128.0,
            },
        );
        assert!(first.is_empty());
        let second = mapper.event(
            MOUSE,
            DeviceEvent::Scroll {
                horizontal: 0.0,
                vertical: 1.0 / 128.0,
            },
        );
        assert_eq!(
            second,
            vec![InputEvent::PointerAxis {
                device: MOUSE,
                horizontal: 0,
                vertical: 1
            }]
        );
    }

    #[test]
    fn a_whole_step_is_reported_as_one_hundred_and_twenty() {
        let mut mapper = Mapper::new();
        assert_eq!(
            mapper.event(
                MOUSE,
                DeviceEvent::Scroll {
                    horizontal: -1.0,
                    vertical: 1.0
                }
            ),
            vec![InputEvent::PointerAxis {
                device: MOUSE,
                horizontal: -120,
                vertical: 120
            }]
        );
    }

    #[test]
    fn unplugging_an_output_moves_the_pointer_to_one_that_is_left() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd(), screen(2, 1280, 720)]);
        let _ = mapper.event(MOUSE, DeviceEvent::Absolute { x: 0.9, y: 0.9 });
        assert_eq!(mapper.pointer().map(|(id, _)| id), Some(OutputId::new(1)));
        mapper.set_screens(vec![screen(2, 1280, 720)]);
        assert_eq!(
            mapper.pointer(),
            Some((OutputId::new(2), Point::new(0, 0))),
            "the pointer was on the output that went away"
        );
    }

    #[test]
    fn a_shrunk_output_pulls_the_pointer_inside_it() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let _ = mapper.event(MOUSE, DeviceEvent::Absolute { x: 1.0, y: 1.0 });
        mapper.set_screens(vec![screen(1, 800, 600)]);
        assert_eq!(mapper.pointer(), Some((OutputId::new(1), Point::new(799, 599))));
    }

    #[test]
    fn with_no_output_an_absolute_point_is_dropped() {
        let mut mapper = Mapper::new();
        assert!(mapper.event(MOUSE, DeviceEvent::Absolute { x: 0.5, y: 0.5 }).is_empty());
        assert_eq!(mapper.pointer(), None);
    }

    #[test]
    fn a_key_carries_its_own_edge() {
        let mut mapper = Mapper::new();
        assert_eq!(
            mapper.event(
                MOUSE,
                DeviceEvent::Key {
                    code: 30,
                    pressed: true
                }
            ),
            vec![InputEvent::Key {
                device: MOUSE,
                keycode: 30,
                pressed: true
            }]
        );
        assert_eq!(
            mapper.event(
                MOUSE,
                DeviceEvent::Key {
                    code: 30,
                    pressed: false
                }
            ),
            vec![InputEvent::Key {
                device: MOUSE,
                keycode: 30,
                pressed: false
            }]
        );
    }

    #[test]
    fn an_enormous_delta_saturates_instead_of_wrapping() {
        let mut mapper = Mapper::new();
        assert_eq!(
            mapper.event(
                MOUSE,
                DeviceEvent::Motion {
                    dx: f64::from(i32::MAX) * 4.0,
                    dy: 0.0
                }
            ),
            vec![InputEvent::PointerMotionRelative {
                device: MOUSE,
                dx: i32::MAX,
                dy: 0
            }]
        );
    }

    #[test]
    fn every_event_names_its_device() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        let events = [
            mapper.event(
                KEYBOARD,
                DeviceEvent::Key {
                    code: 30,
                    pressed: true,
                },
            ),
            mapper.event(
                SECOND_MOUSE,
                DeviceEvent::Button {
                    code: 272,
                    pressed: true,
                },
            ),
            mapper.event(SECOND_MOUSE, DeviceEvent::Motion { dx: 2.0, dy: 0.0 }),
            mapper.event(
                SECOND_MOUSE,
                DeviceEvent::Scroll {
                    horizontal: 0.0,
                    vertical: 1.0,
                },
            ),
            mapper.event(MOUSE, DeviceEvent::Absolute { x: 0.5, y: 0.5 }),
            mapper.event(
                MOUSE,
                DeviceEvent::Touch {
                    slot: 0,
                    x: 0.5,
                    y: 0.5,
                    phase: TouchPhase::Down,
                },
            ),
        ];
        let named: Vec<DeviceId> = events
            .iter()
            .flatten()
            .map(|event| match event {
                InputEvent::Key { device, .. }
                | InputEvent::PointerButton { device, .. }
                | InputEvent::PointerMotionRelative { device, .. }
                | InputEvent::PointerAxis { device, .. }
                | InputEvent::PointerMotion { device, .. }
                | InputEvent::Touch { device, .. } => *device,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            named,
            [KEYBOARD, SECOND_MOUSE, SECOND_MOUSE, SECOND_MOUSE, MOUSE, MOUSE]
        );
    }

    #[test]
    fn a_removed_device_does_not_release_anothers_button() {
        let mut mapper = Mapper::new();
        mapper.set_screens(vec![full_hd()]);
        for device in [MOUSE, SECOND_MOUSE] {
            let _ = mapper.event(
                device,
                DeviceEvent::Button {
                    code: 272,
                    pressed: true,
                },
            );
        }
        let _ = mapper.event(
            SECOND_MOUSE,
            DeviceEvent::Touch {
                slot: 0,
                x: 0.5,
                y: 0.5,
                phase: TouchPhase::Down,
            },
        );
        let cancelled = mapper.cancel_device(MOUSE);
        assert_eq!(
            cancelled,
            vec![InputEvent::PointerButton {
                device: MOUSE,
                button: 272,
                pressed: false
            }],
            "only the removed device's button is released, and it names that device"
        );
        assert_eq!(
            mapper.held_buttons().iter().copied().collect::<Vec<_>>(),
            vec![(SECOND_MOUSE, 272)],
            "the other mouse still holds its button"
        );
        let rest = mapper.cancel_device(SECOND_MOUSE);
        assert_eq!(
            rest.len(),
            2,
            "the second mouse's button and touch point, released once each"
        );
        assert!(mapper.cancel_device(SECOND_MOUSE).is_empty());
    }

    #[test]
    fn two_devices_holding_one_button_code_keep_their_own_press() {
        let mut mapper = Mapper::new();
        for device in [MOUSE, SECOND_MOUSE] {
            let _ = mapper.event(
                device,
                DeviceEvent::Button {
                    code: 272,
                    pressed: true,
                },
            );
        }
        let _ = mapper.event(
            MOUSE,
            DeviceEvent::Button {
                code: 272,
                pressed: false,
            },
        );
        assert_eq!(
            mapper.held_buttons().iter().copied().collect::<Vec<_>>(),
            vec![(SECOND_MOUSE, 272)],
            "one mouse's release does not clear the other mouse's press"
        );
    }
}
