//! Tablet input: tools in proximity and pads, as every backend delivers them.
//!
//! A drawing tablet is two devices to the protocol (`zwp_tablet_v2`): the **tool** a hand holds over it (a pen, an
//! eraser, a brush) and the **pad** beside it (buttons, rings, strips). The contract states both as one event family,
//! [`TabletEvent`], inside [`InputEvent::Tablet`], so a nested session, a headless one and the hardware backend hand the
//! compositor the same thing and the compositor serves `zwp_tablet_manager_v2` from one place.
//!
//! # What an event carries
//!
//! - **A tool's position** is an output-local logical position, continuous ([`PointF`]), because a pen resolves far
//!   below a pixel. The hardware reports a fraction of the tablet's own range, which the backend maps onto an output;
//!   a nested session has the host's surface-local position already.
//! - **A tool's axes** ([`TabletAxes`]) are the ones the tool changed: pressure and distance from 0 to 1, tilt and
//!   rotation in degrees, a slider from -1 to 1 and a wheel in degrees with its whole clicks. A frame that changes
//!   nothing else carries none, so a consumer applies exactly what is present.
//! - **A tool is named by what it is** ([`TabletTool`]): its type, its hardware serial and the axes it can report, not
//!   by a pointer into a backend, because one pen stays one pen when it moves from one tablet to another.
//! - **A pad's rings and strips** state a position, or `None` when the finger lifted.
//!
//! # Failure modes first
//!
//! 1. **A tool that left proximity keeps moving.** [`TabletEvent::ToolOut`] ends the tool; the consumer drops its
//!    state for it, so a stray later motion is refused instead of reviving it:
//!    `tests::a_stroke_enters_moves_touches_and_leaves`.
//! 2. **A pad's finger lifts and nothing says so.** A ring or strip with no position is the lift:
//!    `tests::a_ring_that_lifts_has_no_position`.
//! 3. **An axis a tool cannot report is delivered.** [`ToolAxes`] states what a tool has, and the injector only fills
//!    the axes a tool can report: `tests::an_injected_stroke_only_reports_the_axes_the_tool_has`.

use gnoblin_types::{DeviceId, OutputId, PointF};

use crate::InputEvent;

/// The physical kind of a tablet tool, which decides what the interaction is expected to be.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ToolKind {
    /// A generic pen.
    Pen,
    /// An eraser.
    Eraser,
    /// A paintbrush.
    Brush,
    /// A pencil.
    Pencil,
    /// An airbrush.
    Airbrush,
    /// A finger on the tablet.
    Finger,
    /// A mouse bound to the tablet.
    Mouse,
    /// A lens cursor.
    Lens,
}

/// The axes beyond x and y a tool can report.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct ToolAxes(u8);

impl ToolAxes {
    /// Nothing beyond the position.
    pub const NONE: Self = Self(0);
    /// Tilt on both axes.
    pub const TILT: Self = Self(1);
    /// Pressure.
    pub const PRESSURE: Self = Self(1 << 1);
    /// Distance from the tablet.
    pub const DISTANCE: Self = Self(1 << 2);
    /// Rotation around the tool's own axis.
    pub const ROTATION: Self = Self(1 << 3);
    /// A slider.
    pub const SLIDER: Self = Self(1 << 4);
    /// A wheel.
    pub const WHEEL: Self = Self(1 << 5);

    /// What a pen with tilt, pressure and distance reports, which is the common case.
    pub const PEN: Self = Self(Self::TILT.0 | Self::PRESSURE.0 | Self::DISTANCE.0);

    /// Both sets together.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every axis in `other` is in this set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A tablet tool, named by what it is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TabletTool {
    /// The physical kind.
    pub kind: ToolKind,
    /// The hardware serial, or zero for a tool that states none.
    pub serial: u64,
    /// The hardware id in Wacom's format, or zero.
    pub hardware_id: u64,
    /// The axes the tool can report.
    pub axes: ToolAxes,
}

impl TabletTool {
    /// A pen with a serial and the usual axes.
    #[must_use]
    pub const fn pen(serial: u64) -> Self {
        Self {
            kind: ToolKind::Pen,
            serial,
            hardware_id: 0,
            axes: ToolAxes::PEN,
        }
    }
}

/// The axis values one tool event states. A value that is `None` did not change and is not sent.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct TabletAxes {
    /// Pressure from 0 to 1.
    pub pressure: Option<f64>,
    /// Distance from the tablet from 0 to 1.
    pub distance: Option<f64>,
    /// Tilt on x and y, in degrees from the vertical.
    pub tilt: Option<(f64, f64)>,
    /// Rotation around the tool's axis, in degrees.
    pub rotation: Option<f64>,
    /// A slider position from -1 to 1.
    pub slider: Option<f64>,
    /// A wheel: degrees, and whole clicks.
    pub wheel: Option<(f64, i32)>,
}

impl TabletAxes {
    /// Whether no axis is stated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The same values with every axis `tool` cannot report removed.
    #[must_use]
    pub fn limited_to(self, axes: ToolAxes) -> Self {
        Self {
            pressure: self.pressure.filter(|_| axes.contains(ToolAxes::PRESSURE)),
            distance: self.distance.filter(|_| axes.contains(ToolAxes::DISTANCE)),
            tilt: self.tilt.filter(|_| axes.contains(ToolAxes::TILT)),
            rotation: self.rotation.filter(|_| axes.contains(ToolAxes::ROTATION)),
            slider: self.slider.filter(|_| axes.contains(ToolAxes::SLIDER)),
            wheel: self.wheel.filter(|_| axes.contains(ToolAxes::WHEEL)),
        }
    }
}

/// The layout of a tablet's pad: how many buttons, rings and strips it has.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct PadLayout {
    /// Buttons, numbered from zero.
    pub buttons: u32,
    /// Rings, numbered from zero.
    pub rings: u32,
    /// Strips, numbered from zero.
    pub strips: u32,
}

/// One tablet event: a tool's, or a pad's.
#[derive(Clone, PartialEq, Debug)]
pub enum TabletEvent {
    /// A tool came into proximity of the tablet, over `output` at `position`.
    ToolIn {
        /// The tablet the tool is over.
        device: DeviceId,
        /// The output the position is relative to.
        output: OutputId,
        /// The tool.
        tool: TabletTool,
        /// Where it is.
        position: PointF,
        /// Every axis the tool reports on arrival.
        axes: TabletAxes,
    },
    /// The tool left proximity. Whatever it held (its tip, its buttons) ended before this.
    ToolOut {
        /// The tablet it was over.
        device: DeviceId,
        /// The tool.
        tool: TabletTool,
    },
    /// The tool moved, with the axes that changed in the same instant.
    ToolMotion {
        /// The tablet.
        device: DeviceId,
        /// The output the position is relative to.
        output: OutputId,
        /// The tool.
        tool: TabletTool,
        /// Where it is now.
        position: PointF,
        /// The axes that changed.
        axes: TabletAxes,
    },
    /// The tool's tip touched the tablet, or lifted from it.
    ToolTip {
        /// The tablet.
        device: DeviceId,
        /// The tool.
        tool: TabletTool,
        /// Whether the tip is down now.
        down: bool,
    },
    /// One of the tool's buttons changed; `button` is the Linux input event code.
    ToolButton {
        /// The tablet.
        device: DeviceId,
        /// The tool.
        tool: TabletTool,
        /// The button code.
        button: u32,
        /// Whether the button is held now.
        pressed: bool,
    },
    /// A button of the pad changed, numbered from zero.
    PadButton {
        /// The pad's device.
        device: DeviceId,
        /// The button's number.
        button: u32,
        /// Whether it is held now.
        pressed: bool,
    },
    /// A ring of the pad was touched: an angle in degrees, or `None` when the finger lifted.
    PadRing {
        /// The pad's device.
        device: DeviceId,
        /// The ring's number.
        ring: u32,
        /// Where the finger is.
        angle: Option<f64>,
    },
    /// A strip of the pad was touched: a position from 0 to 1, or `None` when the finger lifted.
    PadStrip {
        /// The pad's device.
        device: DeviceId,
        /// The strip's number.
        strip: u32,
        /// Where the finger is.
        position: Option<f64>,
    },
}

impl TabletEvent {
    /// The device that produced the event.
    #[must_use]
    pub const fn device(&self) -> DeviceId {
        match self {
            Self::ToolIn { device, .. }
            | Self::ToolOut { device, .. }
            | Self::ToolMotion { device, .. }
            | Self::ToolTip { device, .. }
            | Self::ToolButton { device, .. }
            | Self::PadButton { device, .. }
            | Self::PadRing { device, .. }
            | Self::PadStrip { device, .. } => *device,
        }
    }
}

impl From<TabletEvent> for InputEvent {
    fn from(event: TabletEvent) -> Self {
        Self::Tablet(event)
    }
}

/// A synthetic hand: builds the events of one tool over one tablet, so a backend with no device (headless) and a test
/// that drives a real one produce the same stream.
///
/// The injector remembers whether the tool is in proximity and whether its tip is down, and refuses to produce an
/// event the state cannot have (a motion while out of proximity): the stream it makes is one a device could.
#[derive(Clone, Debug)]
pub struct Stylus {
    device: DeviceId,
    output: OutputId,
    tool: TabletTool,
    inside: bool,
    down: bool,
}

impl Stylus {
    /// A tool over `device`, outside proximity, drawing on `output`.
    #[must_use]
    pub const fn new(device: DeviceId, output: OutputId, tool: TabletTool) -> Self {
        Self {
            device,
            output,
            tool,
            inside: false,
            down: false,
        }
    }

    /// The tool this hand holds.
    #[must_use]
    pub const fn tool(&self) -> TabletTool {
        self.tool
    }

    /// Whether the tool is in proximity.
    #[must_use]
    pub const fn is_inside(&self) -> bool {
        self.inside
    }

    /// The tool enters proximity at `(x, y)` with `axes`.
    pub fn enter(&mut self, x: f64, y: f64, axes: TabletAxes) -> Vec<InputEvent> {
        if self.inside {
            return Vec::new();
        }
        self.inside = true;
        vec![
            TabletEvent::ToolIn {
                device: self.device,
                output: self.output,
                tool: self.tool,
                position: PointF::new(x, y),
                axes: axes.limited_to(self.tool.axes),
            }
            .into(),
        ]
    }

    /// The tool moves to `(x, y)`; empty while it is out of proximity.
    pub fn move_to(&mut self, x: f64, y: f64, axes: TabletAxes) -> Vec<InputEvent> {
        if !self.inside {
            return Vec::new();
        }
        vec![
            TabletEvent::ToolMotion {
                device: self.device,
                output: self.output,
                tool: self.tool,
                position: PointF::new(x, y),
                axes: axes.limited_to(self.tool.axes),
            }
            .into(),
        ]
    }

    /// The tip touches down, or lifts; empty when it already is.
    pub fn tip(&mut self, down: bool) -> Vec<InputEvent> {
        if !self.inside || self.down == down {
            return Vec::new();
        }
        self.down = down;
        vec![
            TabletEvent::ToolTip {
                device: self.device,
                tool: self.tool,
                down,
            }
            .into(),
        ]
    }

    /// A button of the tool changes.
    pub fn button(&mut self, button: u32, pressed: bool) -> Vec<InputEvent> {
        if !self.inside {
            return Vec::new();
        }
        vec![
            TabletEvent::ToolButton {
                device: self.device,
                tool: self.tool,
                button,
                pressed,
            }
            .into(),
        ]
    }

    /// The tool leaves proximity, lifting its tip first when it was down.
    pub fn leave(&mut self) -> Vec<InputEvent> {
        if !self.inside {
            return Vec::new();
        }
        let mut events = self.tip(false);
        self.inside = false;
        events.push(
            TabletEvent::ToolOut {
                device: self.device,
                tool: self.tool,
            }
            .into(),
        );
        events
    }

    /// A whole stroke: enter at the first point, touch down, move through the rest with the pressure rising, lift and
    /// leave. Every point is `(x, y)` on the output.
    pub fn stroke(&mut self, points: &[(f64, f64)]) -> Vec<InputEvent> {
        let mut events = Vec::new();
        let Some(&(x, y)) = points.first() else {
            return events;
        };
        events.extend(self.enter(
            x,
            y,
            TabletAxes {
                distance: Some(0.5),
                ..TabletAxes::default()
            },
        ));
        events.extend(self.tip(true));
        for (index, &(x, y)) in points.iter().enumerate().skip(1) {
            let pressure = (index as f64 / points.len() as f64).clamp(0.0, 1.0);
            events.extend(self.move_to(
                x,
                y,
                TabletAxes {
                    pressure: Some(pressure),
                    tilt: Some((10.0, -5.0)),
                    ..TabletAxes::default()
                },
            ));
        }
        events.extend(self.leave());
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLET: DeviceId = DeviceId::new(9);
    const SCREEN: OutputId = OutputId::new(0);

    fn names(events: &[InputEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                InputEvent::Tablet(TabletEvent::ToolIn { .. }) => "in",
                InputEvent::Tablet(TabletEvent::ToolOut { .. }) => "out",
                InputEvent::Tablet(TabletEvent::ToolMotion { .. }) => "motion",
                InputEvent::Tablet(TabletEvent::ToolTip { down: true, .. }) => "down",
                InputEvent::Tablet(TabletEvent::ToolTip { down: false, .. }) => "up",
                InputEvent::Tablet(TabletEvent::ToolButton { .. }) => "button",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn a_stroke_enters_moves_touches_and_leaves() {
        let mut pen = Stylus::new(TABLET, SCREEN, TabletTool::pen(7));
        let events = pen.stroke(&[(10.0, 10.0), (20.0, 12.5), (30.0, 15.0)]);
        assert_eq!(names(&events), ["in", "down", "motion", "motion", "up", "out"]);
        assert!(!pen.is_inside(), "the stroke ends outside proximity");
        assert!(
            pen.move_to(1.0, 1.0, TabletAxes::default()).is_empty(),
            "a tool that left proximity does not move"
        );
    }

    #[test]
    fn an_injected_stroke_only_reports_the_axes_the_tool_has() {
        let tool = TabletTool {
            axes: ToolAxes::PRESSURE,
            ..TabletTool::pen(1)
        };
        let mut pen = Stylus::new(TABLET, SCREEN, tool);
        let events = pen.stroke(&[(0.0, 0.0), (5.0, 5.0)]);
        let motion = events
            .iter()
            .find_map(|event| match event {
                InputEvent::Tablet(TabletEvent::ToolMotion { axes, .. }) => Some(*axes),
                _ => None,
            })
            .expect("one motion");
        assert!(motion.pressure.is_some(), "the pressure it has is reported");
        assert_eq!(motion.tilt, None, "the tilt it has not is not");
    }

    #[test]
    fn a_tip_that_is_down_lifts_before_the_tool_leaves() {
        let mut pen = Stylus::new(TABLET, SCREEN, TabletTool::pen(2));
        pen.enter(0.0, 0.0, TabletAxes::default());
        pen.tip(true);
        assert_eq!(names(&pen.leave()), ["up", "out"]);
        assert!(pen.leave().is_empty(), "leaving twice says nothing the second time");
    }

    #[test]
    fn a_tip_that_already_is_down_says_nothing() {
        let mut pen = Stylus::new(TABLET, SCREEN, TabletTool::pen(3));
        pen.enter(0.0, 0.0, TabletAxes::default());
        assert_eq!(names(&pen.tip(true)), ["down"]);
        assert!(pen.tip(true).is_empty());
    }

    #[test]
    fn a_ring_that_lifts_has_no_position() {
        let lift = TabletEvent::PadRing {
            device: TABLET,
            ring: 0,
            angle: None,
        };
        assert_eq!(lift.device(), TABLET);
        assert!(matches!(lift, TabletEvent::PadRing { angle: None, .. }));
    }
}
