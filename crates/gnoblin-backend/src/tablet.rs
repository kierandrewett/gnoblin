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

pub use gnoblin_wayland_types::tablet::{PadLayout, TabletAxes, TabletEvent, TabletTool, ToolAxes, ToolKind};

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
