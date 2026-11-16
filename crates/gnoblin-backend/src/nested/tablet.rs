//! The parent's tablets, converted into [`TabletEvent`]s.
//!
//! `zwp_tablet_v2` sends a tool's changes as separate events closed by a `frame`: a proximity, some motion, an axis or
//! two, the tip. This module keeps what has arrived since the last frame and turns the frame into the contract's events
//! in an order a device could have produced. It is plain data keyed by protocol object numbers, so every awkward case
//! is a unit test with no socket: a motion in the frame that carries the proximity, a tool that leaves with its tip
//! down, a tablet that is unplugged while a pen hovers.
//!
//! # Failure modes first
//!
//! 1. **An axis is delivered before its frame ends.** Everything a tool sends waits for `frame`, so a pen that moved and
//!    pressed in one instant is one [`TabletEvent::ToolMotion`]: `a_frame_is_one_motion_with_every_axis_it_changed`.
//! 2. **A tool enters over a surface the session does not draw.** The proximity names a surface; one that is not an
//!    output of this backend cannot be attributed, so the tool is ignored until it leaves, and counted:
//!    `a_tool_over_a_foreign_surface_is_ignored`.
//! 3. **A tablet unplugged under a hovering pen leaves the pen in proximity.** The removal ends the tools that were
//!    over it, lifting the tip first: `a_removed_tablet_ends_the_tools_over_it`.
//! 4. **A pen's tip is still down when it leaves.** The up comes before the out:
//!    `a_tool_that_leaves_with_its_tip_down_lifts_first`.
//! 5. **A ring or strip reports a stale finger.** A stop is a lift, delivered as a position of `None`:
//!    `a_ring_stop_is_a_lift`.
//! 6. **A tool or pad is announced before its description.** A device joins the seat at its `done`, so the name, ids and
//!    capabilities are known: `a_tablet_joins_the_seat_when_it_is_described`.

use std::collections::HashMap;

use gnoblin_types::{DeviceId, OutputId, PointF};

use crate::{
    DeviceCaps, DeviceInfo, DeviceKind, InputEvent, PadLayout, TabletAxes, TabletEvent, TabletTool, ToolAxes, ToolKind,
};

/// The first number a tablet device gets: above the keyboard, pointer and touch the seat numbers itself.
const FIRST_DEVICE: u64 = 16;

/// The raw range of an unsigned axis on the wire.
const RAW_MAX: f64 = 65535.0;

/// A tablet the parent described.
#[derive(Debug, Default)]
struct Tablet {
    name: String,
    vendor: u32,
    product: u32,
    /// The number the session's seat knows it by, once it was announced.
    device: Option<DeviceId>,
}

/// What a tool sent since its last frame.
#[derive(Debug, Default)]
struct ToolFrame {
    /// The tool came into proximity: the tablet it is over and the output its surface is.
    entered: Option<(u32, Option<OutputId>)>,
    left: bool,
    position: Option<PointF>,
    axes: TabletAxes,
    tip: Option<bool>,
    buttons: Vec<(u32, bool)>,
}

/// A tool the parent described.
#[derive(Debug)]
struct HostTool {
    kind: ToolKind,
    serial: u64,
    hardware_id: u64,
    axes: ToolAxes,
    /// The tablet it is over while in proximity.
    tablet: Option<u32>,
    output: Option<OutputId>,
    /// Whether the session has been told the tool is in proximity.
    inside: bool,
    /// Whether the tool entered over a surface the session cannot attribute, so everything until it leaves is ignored.
    ignored: bool,
    tip_down: bool,
    position: PointF,
    pending: ToolFrame,
}

impl HostTool {
    fn identity(&self) -> TabletTool {
        TabletTool {
            kind: self.kind,
            serial: self.serial,
            hardware_id: self.hardware_id,
            axes: self.axes,
        }
    }
}

/// A pad's ring or strip: the pad it belongs to, its number and what it last said.
#[derive(Debug)]
struct PadTouch {
    pad: u32,
    index: u32,
    /// What the next frame reports: a position, or `None` for a lift.
    pending: Option<Option<f64>>,
}

/// A pad the parent described.
#[derive(Debug, Default)]
struct HostPad {
    buttons: u32,
    rings: u32,
    strips: u32,
    device: Option<DeviceId>,
}

/// The parent's tablet seat: tablets, tools and pads, keyed by the protocol object numbers the parent used.
#[derive(Debug, Default)]
pub struct Tablets {
    tablets: HashMap<u32, Tablet>,
    tools: HashMap<u32, HostTool>,
    pads: HashMap<u32, HostPad>,
    /// Which pad each group object belongs to.
    groups: HashMap<u32, u32>,
    rings: HashMap<u32, PadTouch>,
    strips: HashMap<u32, PadTouch>,
    next_device: u64,
    out: Vec<InputEvent>,
    ignored: u64,
}

impl Tablets {
    /// A seat with no tablet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_device: FIRST_DEVICE,
            ..Self::default()
        }
    }

    /// Takes the events produced so far.
    pub fn drain(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.out)
    }

    /// How many tool events could not be attributed to an output of the session.
    #[must_use]
    pub fn ignored(&self) -> u64 {
        self.ignored
    }

    fn allocate(&mut self) -> DeviceId {
        let id = DeviceId::new(self.next_device);
        self.next_device += 1;
        id
    }

    // ---- tablets --------------------------------------------------------------------------------------------------

    /// The parent announced a tablet.
    pub fn tablet_added(&mut self, id: u32) {
        self.tablets.insert(id, Tablet::default());
    }

    /// The tablet's name.
    pub fn tablet_name(&mut self, id: u32, name: &str) {
        if let Some(tablet) = self.tablets.get_mut(&id) {
            tablet.name = name.to_owned();
        }
    }

    /// The tablet's USB ids.
    pub fn tablet_ids(&mut self, id: u32, vendor: u32, product: u32) {
        if let Some(tablet) = self.tablets.get_mut(&id) {
            tablet.vendor = vendor;
            tablet.product = product;
        }
    }

    /// The tablet is fully described: it joins the session's seat.
    pub fn tablet_done(&mut self, id: u32) {
        let device = self.allocate();
        let Some(tablet) = self.tablets.get_mut(&id) else {
            return;
        };
        if tablet.device.is_some() {
            return;
        }
        tablet.device = Some(device);
        let mut info = DeviceInfo::new(device, &tablet.name, DeviceKind::Mouse).with_caps(DeviceCaps::TABLET_TOOL);
        info.vendor = tablet.vendor;
        info.product = tablet.product;
        self.out.push(InputEvent::DeviceAdded { device: info });
    }

    /// The tablet is gone: the tools over it leave, and it leaves the seat.
    pub fn tablet_removed(&mut self, id: u32) {
        let hovering: Vec<u32> = self
            .tools
            .iter()
            .filter(|(_, tool)| tool.tablet == Some(id) && tool.inside)
            .map(|(tool, _)| *tool)
            .collect();
        for tool in hovering {
            self.leave(tool);
        }
        if let Some(device) = self.tablets.remove(&id).and_then(|tablet| tablet.device) {
            self.out.push(InputEvent::DeviceRemoved { id: device });
        }
    }

    // ---- tools ----------------------------------------------------------------------------------------------------

    /// The parent announced a tool.
    pub fn tool_added(&mut self, id: u32) {
        self.tools.insert(
            id,
            HostTool {
                kind: ToolKind::Pen,
                serial: 0,
                hardware_id: 0,
                axes: ToolAxes::NONE,
                tablet: None,
                output: None,
                inside: false,
                ignored: false,
                tip_down: false,
                position: PointF::ZERO,
                pending: ToolFrame::default(),
            },
        );
    }

    /// The tool's kind.
    pub fn tool_kind(&mut self, id: u32, kind: ToolKind) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.kind = kind;
        }
    }

    /// The tool's hardware serial, as the wire splits it.
    pub fn tool_serial(&mut self, id: u32, high: u32, low: u32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.serial = u64::from(high) << 32 | u64::from(low);
        }
    }

    /// The tool's hardware id in Wacom's format, as the wire splits it.
    pub fn tool_wacom(&mut self, id: u32, high: u32, low: u32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.hardware_id = u64::from(high) << 32 | u64::from(low);
        }
    }

    /// An axis the tool can report.
    pub fn tool_capability(&mut self, id: u32, axis: ToolAxes) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.axes = tool.axes.union(axis);
        }
    }

    /// The tool is fully described. Its kind, serial and axes arrive before this, so nothing is left to record: the
    /// tool is named by what it said, and its first proximity is its first event.
    pub fn tool_done(&mut self, _id: u32) {}

    /// The tool is gone: it leaves proximity first when it was in it.
    pub fn tool_removed(&mut self, id: u32) {
        self.leave(id);
        self.tools.remove(&id);
    }

    /// The tool came into proximity of `tablet`, over the surface that is `output` (or none of the session's).
    pub fn proximity_in(&mut self, id: u32, tablet: u32, output: Option<OutputId>) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.entered = Some((tablet, output));
            tool.pending.left = false;
        }
    }

    /// The tool left proximity.
    pub fn proximity_out(&mut self, id: u32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.left = true;
        }
    }

    /// The tip touched down (`true`) or lifted.
    pub fn tip(&mut self, id: u32, down: bool) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.tip = Some(down);
        }
    }

    /// The tool moved to a surface-local position.
    pub fn motion(&mut self, id: u32, x: f64, y: f64) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.position = Some(PointF::new(x, y));
        }
    }

    /// Pressure, as the wire's unsigned 16-bit value.
    pub fn pressure(&mut self, id: u32, raw: u32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.axes.pressure = Some(f64::from(raw) / RAW_MAX);
        }
    }

    /// Distance, as the wire's unsigned 16-bit value.
    pub fn distance(&mut self, id: u32, raw: u32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.axes.distance = Some(f64::from(raw) / RAW_MAX);
        }
    }

    /// Tilt, in degrees.
    pub fn tilt(&mut self, id: u32, x: f64, y: f64) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.axes.tilt = Some((x, y));
        }
    }

    /// Rotation, in degrees.
    pub fn rotation(&mut self, id: u32, degrees: f64) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.axes.rotation = Some(degrees);
        }
    }

    /// The slider, as the wire's signed value.
    pub fn slider(&mut self, id: u32, raw: i32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.axes.slider = Some(f64::from(raw) / RAW_MAX);
        }
    }

    /// The wheel: degrees and whole clicks.
    pub fn wheel(&mut self, id: u32, degrees: f64, clicks: i32) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.axes.wheel = Some((degrees, clicks));
        }
    }

    /// A button of the tool.
    pub fn tool_button(&mut self, id: u32, button: u32, pressed: bool) {
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.pending.buttons.push((button, pressed));
        }
    }

    /// The frame closes the tool's changes: they become events.
    pub fn frame(&mut self, id: u32) {
        let Some(mut tool) = self.tools.remove(&id) else {
            return;
        };
        let pending = std::mem::take(&mut tool.pending);
        self.apply(&mut tool, pending);
        self.tools.insert(id, tool);
    }

    /// Turns one frame's changes into events, in the order a device produces them.
    fn apply(&mut self, tool: &mut HostTool, pending: ToolFrame) {
        if let Some((tablet, output)) = pending.entered {
            tool.tablet = Some(tablet);
            tool.output = output;
            tool.ignored = output.is_none();
            tool.inside = false;
            tool.tip_down = false;
            if tool.ignored {
                self.ignored = self.ignored.saturating_add(1);
            }
        }
        if tool.ignored {
            if pending.left {
                tool.ignored = false;
                tool.tablet = None;
            }
            return;
        }
        let Some(device) = tool
            .tablet
            .and_then(|tablet| self.tablets.get(&tablet))
            .and_then(|tablet| tablet.device)
        else {
            return;
        };
        let Some(output) = tool.output else {
            return;
        };
        if let Some(position) = pending.position {
            tool.position = position;
        }
        let axes = pending.axes.limited_to(tool.axes);
        if pending.entered.is_some() {
            tool.inside = true;
            self.out.push(
                TabletEvent::ToolIn {
                    device,
                    output,
                    tool: tool.identity(),
                    position: tool.position,
                    axes,
                }
                .into(),
            );
        } else if tool.inside && (pending.position.is_some() || !axes.is_empty()) {
            self.out.push(
                TabletEvent::ToolMotion {
                    device,
                    output,
                    tool: tool.identity(),
                    position: tool.position,
                    axes,
                }
                .into(),
            );
        }
        if !tool.inside {
            return;
        }
        if let Some(down) = pending.tip.filter(|down| *down != tool.tip_down) {
            tool.tip_down = down;
            self.out.push(
                TabletEvent::ToolTip {
                    device,
                    tool: tool.identity(),
                    down,
                }
                .into(),
            );
        }
        for (button, pressed) in pending.buttons {
            self.out.push(
                TabletEvent::ToolButton {
                    device,
                    tool: tool.identity(),
                    button,
                    pressed,
                }
                .into(),
            );
        }
        if pending.left {
            self.out_of_proximity(tool, device);
        }
    }

    /// Ends a tool's stay: the tip lifts, and the tool leaves.
    fn out_of_proximity(&mut self, tool: &mut HostTool, device: DeviceId) {
        if tool.tip_down {
            tool.tip_down = false;
            self.out.push(
                TabletEvent::ToolTip {
                    device,
                    tool: tool.identity(),
                    down: false,
                }
                .into(),
            );
        }
        tool.inside = false;
        tool.tablet = None;
        self.out.push(
            TabletEvent::ToolOut {
                device,
                tool: tool.identity(),
            }
            .into(),
        );
    }

    /// Ends the tool's stay at once, for a tablet or tool that went away.
    fn leave(&mut self, id: u32) {
        let Some(mut tool) = self.tools.remove(&id) else {
            return;
        };
        if tool.inside {
            let device = tool
                .tablet
                .and_then(|tablet| self.tablets.get(&tablet))
                .and_then(|tablet| tablet.device);
            if let Some(device) = device {
                self.out_of_proximity(&mut tool, device);
            }
        }
        tool.ignored = false;
        self.tools.insert(id, tool);
    }

    // ---- pads -----------------------------------------------------------------------------------------------------

    /// The parent announced a pad.
    pub fn pad_added(&mut self, id: u32) {
        self.pads.insert(id, HostPad::default());
    }

    /// How many buttons the pad has.
    pub fn pad_buttons(&mut self, id: u32, buttons: u32) {
        if let Some(pad) = self.pads.get_mut(&id) {
            pad.buttons = buttons;
        }
    }

    /// The pad has a group: its rings and strips are announced through it.
    pub fn pad_group(&mut self, pad: u32, group: u32) {
        self.groups.insert(group, pad);
    }

    /// A group announced a ring.
    pub fn group_ring(&mut self, group: u32, ring: u32) {
        let Some(&pad) = self.groups.get(&group) else {
            return;
        };
        if let Some(record) = self.pads.get_mut(&pad) {
            self.rings.insert(
                ring,
                PadTouch {
                    pad,
                    index: record.rings,
                    pending: None,
                },
            );
            record.rings += 1;
        }
    }

    /// A group announced a strip.
    pub fn group_strip(&mut self, group: u32, strip: u32) {
        let Some(&pad) = self.groups.get(&group) else {
            return;
        };
        if let Some(record) = self.pads.get_mut(&pad) {
            self.strips.insert(
                strip,
                PadTouch {
                    pad,
                    index: record.strips,
                    pending: None,
                },
            );
            record.strips += 1;
        }
    }

    /// The pad is fully described: it joins the seat.
    pub fn pad_done(&mut self, id: u32) {
        let device = self.allocate();
        let Some(pad) = self.pads.get_mut(&id) else {
            return;
        };
        if pad.device.is_some() {
            return;
        }
        pad.device = Some(device);
        let info = DeviceInfo::new(device, "Parent compositor tablet pad", DeviceKind::Mouse)
            .with_caps(DeviceCaps::NONE)
            .with_pad(PadLayout {
                buttons: pad.buttons,
                rings: pad.rings,
                strips: pad.strips,
            });
        self.out.push(InputEvent::DeviceAdded { device: info });
    }

    /// The pad is gone.
    pub fn pad_removed(&mut self, id: u32) {
        self.rings.retain(|_, ring| ring.pad != id);
        self.strips.retain(|_, strip| strip.pad != id);
        self.groups.retain(|_, pad| *pad != id);
        if let Some(device) = self.pads.remove(&id).and_then(|pad| pad.device) {
            self.out.push(InputEvent::DeviceRemoved { id: device });
        }
    }

    /// A pad button changed.
    pub fn pad_button(&mut self, id: u32, button: u32, pressed: bool) {
        if let Some(device) = self.pads.get(&id).and_then(|pad| pad.device) {
            self.out.push(
                TabletEvent::PadButton {
                    device,
                    button,
                    pressed,
                }
                .into(),
            );
        }
    }

    /// A ring was touched at an angle in degrees.
    pub fn ring_angle(&mut self, ring: u32, degrees: f64) {
        if let Some(ring) = self.rings.get_mut(&ring) {
            ring.pending = Some(Some(degrees));
        }
    }

    /// The finger left the ring.
    pub fn ring_stop(&mut self, ring: u32) {
        if let Some(ring) = self.rings.get_mut(&ring) {
            ring.pending = Some(None);
        }
    }

    /// The ring's frame closes what it said.
    pub fn ring_frame(&mut self, ring: u32) {
        let Some(touch) = self.rings.get_mut(&ring) else {
            return;
        };
        let (pad, index, pending) = (touch.pad, touch.index, touch.pending.take());
        let (Some(angle), Some(device)) = (pending, self.pads.get(&pad).and_then(|pad| pad.device)) else {
            return;
        };
        self.out.push(
            TabletEvent::PadRing {
                device,
                ring: index,
                angle,
            }
            .into(),
        );
    }

    /// A strip was touched at a position, as the wire's unsigned 16-bit value.
    pub fn strip_position(&mut self, strip: u32, raw: u32) {
        if let Some(strip) = self.strips.get_mut(&strip) {
            strip.pending = Some(Some(f64::from(raw) / RAW_MAX));
        }
    }

    /// The finger left the strip.
    pub fn strip_stop(&mut self, strip: u32) {
        if let Some(strip) = self.strips.get_mut(&strip) {
            strip.pending = Some(None);
        }
    }

    /// The strip's frame closes what it said.
    pub fn strip_frame(&mut self, strip: u32) {
        let Some(touch) = self.strips.get_mut(&strip) else {
            return;
        };
        let (pad, index, pending) = (touch.pad, touch.index, touch.pending.take());
        let (Some(position), Some(device)) = (pending, self.pads.get(&pad).and_then(|pad| pad.device)) else {
            return;
        };
        self.out.push(
            TabletEvent::PadStrip {
                device,
                strip: index,
                position,
            }
            .into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLET: u32 = 10;
    const PEN: u32 = 20;
    const SCREEN: OutputId = OutputId::new(0);

    /// A tablet and a pen the parent described, with the announcements drained.
    fn seat() -> Tablets {
        let mut seat = Tablets::new();
        seat.tablet_added(TABLET);
        seat.tablet_name(TABLET, "Test tablet");
        seat.tablet_ids(TABLET, 0x56a, 0x37);
        seat.tablet_done(TABLET);
        seat.tool_added(PEN);
        seat.tool_kind(PEN, ToolKind::Pen);
        seat.tool_serial(PEN, 0, 77);
        seat.tool_capability(PEN, ToolAxes::PRESSURE);
        seat.tool_capability(PEN, ToolAxes::TILT);
        seat.tool_done(PEN);
        let announced = seat.drain();
        assert_eq!(announced.len(), 1, "the tablet joined the seat");
        seat
    }

    fn kinds(events: &[InputEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                InputEvent::Tablet(TabletEvent::ToolIn { .. }) => "in",
                InputEvent::Tablet(TabletEvent::ToolOut { .. }) => "out",
                InputEvent::Tablet(TabletEvent::ToolMotion { .. }) => "motion",
                InputEvent::Tablet(TabletEvent::ToolTip { down: true, .. }) => "down",
                InputEvent::Tablet(TabletEvent::ToolTip { down: false, .. }) => "up",
                InputEvent::Tablet(TabletEvent::ToolButton { .. }) => "button",
                InputEvent::DeviceRemoved { .. } => "removed",
                InputEvent::DeviceAdded { .. } => "added",
                _ => "other",
            })
            .collect()
    }

    fn enter(seat: &mut Tablets) {
        seat.proximity_in(PEN, TABLET, Some(SCREEN));
        seat.motion(PEN, 10.0, 20.0);
        seat.frame(PEN);
    }

    #[test]
    fn a_tablet_joins_the_seat_when_it_is_described() {
        let mut seat = Tablets::new();
        seat.tablet_added(TABLET);
        seat.tablet_name(TABLET, "Wacom Intuos");
        seat.tablet_ids(TABLET, 0x56a, 0x37);
        assert!(seat.drain().is_empty(), "nothing is announced before done");
        seat.tablet_done(TABLET);
        let events = seat.drain();
        let Some(InputEvent::DeviceAdded { device }) = events.first() else {
            panic!("a device joined: {events:?}");
        };
        assert_eq!(device.name, "Wacom Intuos");
        assert_eq!((device.vendor, device.product), (0x56a, 0x37));
        assert!(device.caps.contains(DeviceCaps::TABLET_TOOL));
        seat.tablet_done(TABLET);
        assert!(seat.drain().is_empty(), "a second done announces nothing");
    }

    #[test]
    fn a_frame_is_one_motion_with_every_axis_it_changed() {
        let mut seat = seat();
        enter(&mut seat);
        seat.drain();
        seat.motion(PEN, 11.5, 21.0);
        seat.pressure(PEN, 32768);
        seat.tilt(PEN, 12.0, -3.0);
        seat.distance(PEN, 100);
        seat.frame(PEN);
        let events = seat.drain();
        assert_eq!(kinds(&events), ["motion"]);
        let InputEvent::Tablet(TabletEvent::ToolMotion { position, axes, .. }) = &events[0] else {
            panic!("a motion");
        };
        assert_eq!(*position, PointF::new(11.5, 21.0));
        assert!((axes.pressure.expect("pressure") - 0.5).abs() < 0.001);
        assert_eq!(axes.tilt, Some((12.0, -3.0)));
        assert_eq!(
            axes.distance, None,
            "the tool cannot report distance, so it is not delivered"
        );
    }

    #[test]
    fn a_proximity_frame_carries_the_position_it_came_with() {
        let mut seat = seat();
        enter(&mut seat);
        let events = seat.drain();
        assert_eq!(kinds(&events), ["in"]);
        let InputEvent::Tablet(TabletEvent::ToolIn {
            position, tool, output, ..
        }) = &events[0]
        else {
            panic!("a proximity");
        };
        assert_eq!(*position, PointF::new(10.0, 20.0));
        assert_eq!((tool.kind, tool.serial), (ToolKind::Pen, 77));
        assert_eq!(*output, SCREEN);
    }

    #[test]
    fn a_tool_that_leaves_with_its_tip_down_lifts_first() {
        let mut seat = seat();
        enter(&mut seat);
        seat.tip(PEN, true);
        seat.frame(PEN);
        seat.drain();
        seat.proximity_out(PEN);
        seat.frame(PEN);
        assert_eq!(kinds(&seat.drain()), ["up", "out"]);
    }

    #[test]
    fn a_tool_over_a_foreign_surface_is_ignored() {
        let mut seat = seat();
        seat.proximity_in(PEN, TABLET, None);
        seat.motion(PEN, 1.0, 1.0);
        seat.frame(PEN);
        seat.motion(PEN, 2.0, 2.0);
        seat.tip(PEN, true);
        seat.frame(PEN);
        assert!(seat.drain().is_empty());
        assert_eq!(seat.ignored(), 1);
        seat.proximity_out(PEN);
        seat.frame(PEN);
        enter(&mut seat);
        assert_eq!(
            kinds(&seat.drain()),
            ["in"],
            "the next visit over a real window is delivered"
        );
    }

    #[test]
    fn a_removed_tablet_ends_the_tools_over_it() {
        let mut seat = seat();
        enter(&mut seat);
        seat.tip(PEN, true);
        seat.frame(PEN);
        seat.drain();
        seat.tablet_removed(TABLET);
        assert_eq!(kinds(&seat.drain()), ["up", "out", "removed"]);
    }

    #[test]
    fn a_button_is_delivered_after_the_motion_of_its_frame() {
        let mut seat = seat();
        enter(&mut seat);
        seat.drain();
        seat.motion(PEN, 12.0, 22.0);
        seat.tool_button(PEN, 0x14b, true);
        seat.frame(PEN);
        assert_eq!(kinds(&seat.drain()), ["motion", "button"]);
    }

    #[test]
    fn a_tip_that_is_already_down_is_not_pressed_again() {
        let mut seat = seat();
        enter(&mut seat);
        seat.tip(PEN, true);
        seat.frame(PEN);
        seat.drain();
        seat.tip(PEN, true);
        seat.frame(PEN);
        assert!(seat.drain().is_empty());
    }

    fn pad(seat: &mut Tablets) {
        seat.pad_added(30);
        seat.pad_buttons(30, 4);
        seat.pad_group(30, 31);
        seat.group_ring(31, 32);
        seat.group_strip(31, 33);
        seat.pad_done(30);
    }

    #[test]
    fn a_pad_joins_the_seat_with_its_layout() {
        let mut seat = Tablets::new();
        pad(&mut seat);
        let events = seat.drain();
        let Some(InputEvent::DeviceAdded { device }) = events.first() else {
            panic!("a pad joined: {events:?}");
        };
        assert_eq!(
            device.pad,
            PadLayout {
                buttons: 4,
                rings: 1,
                strips: 1
            }
        );
        assert!(device.caps.contains(DeviceCaps::TABLET_PAD));
    }

    #[test]
    fn a_ring_stop_is_a_lift() {
        let mut seat = Tablets::new();
        pad(&mut seat);
        seat.drain();
        seat.ring_angle(32, 90.0);
        seat.ring_frame(32);
        seat.ring_stop(32);
        seat.ring_frame(32);
        let angles: Vec<Option<f64>> = seat
            .drain()
            .into_iter()
            .filter_map(|event| match event {
                InputEvent::Tablet(TabletEvent::PadRing { angle, .. }) => Some(angle),
                _ => None,
            })
            .collect();
        assert_eq!(angles, [Some(90.0), None]);
    }

    #[test]
    fn a_strip_position_is_a_fraction_of_the_range() {
        let mut seat = Tablets::new();
        pad(&mut seat);
        seat.drain();
        seat.strip_position(33, 65535);
        seat.strip_frame(33);
        let events = seat.drain();
        assert!(matches!(
            events.first(),
            Some(InputEvent::Tablet(TabletEvent::PadStrip {
                strip: 0,
                position: Some(position),
                ..
            })) if (*position - 1.0).abs() < 1e-9
        ));
    }

    #[test]
    fn a_pad_button_names_the_pad_it_came_from() {
        let mut seat = Tablets::new();
        pad(&mut seat);
        seat.drain();
        seat.pad_button(30, 2, true);
        assert!(matches!(
            seat.drain().first(),
            Some(InputEvent::Tablet(TabletEvent::PadButton {
                button: 2,
                pressed: true,
                ..
            }))
        ));
    }

    #[test]
    fn a_removed_pad_leaves_the_seat() {
        let mut seat = Tablets::new();
        pad(&mut seat);
        seat.drain();
        seat.pad_removed(30);
        assert_eq!(kinds(&seat.drain()), ["removed"]);
        seat.ring_angle(32, 1.0);
        seat.ring_frame(32);
        assert!(seat.drain().is_empty(), "a ring of a removed pad says nothing");
    }
}
