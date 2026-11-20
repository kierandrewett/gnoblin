//! Tablet input as every backend delivers it: tools in proximity and pads.
//!
//! The backend contract (`gnoblin_backend::tablet`) wraps [`TabletEvent`] in its input event; the protocol crate
//! (`gnoblin-wayland-zwp-tablet-v2`) serves it. The types are here so neither names the other.

use gnoblin_types::{DeviceId, OutputId, PointF};

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
