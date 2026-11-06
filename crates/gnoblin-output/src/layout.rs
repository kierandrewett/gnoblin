//! The declarative configuration, and the layout it produces.
//!
//! Two vocabularies meet in this module and the split is deliberate. A
//! configuration is written by a person or a script and has to survive a reboot,
//! so it names outputs by connector ([`OutputConfig::connector`]); a layout is
//! what window management, accessibility and the locker consume, so it names
//! outputs by the graph token ([`gnoblin_types::OutputId`]) and carries the
//! monitor's connector name for the public record.

use gnoblin_types::{Orientation, OutputId, Rect, Scale, Size};

use crate::mode::DisplayMode;

/// Which side of its anchor an output is placed against.
///
/// This is not `gnoblin_types::Edge` (`Left`, `Right`, `Top`, `Bottom`): a
/// placement edge is a side of a monitor, and naming the vertical pair `Above`
/// and `Below` is what makes a configuration read the way a person arranges
/// screens. The two enums share two variant names and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    /// Left of the anchor, flush with its right edge.
    Left,
    /// Right of the anchor, flush with its left edge.
    Right,
    /// Above the anchor, flush with its bottom edge.
    Above,
    /// Below the anchor, flush with its top edge.
    Below,
}

/// Where an output sits along the edge it shares with its anchor.
///
/// Alignment can only move an output within the anchor's span, so it can never
/// introduce a gap in the direction the two are joined.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    /// Flush with the anchor's left or top edge.
    #[default]
    Start,
    /// Centred on the anchor's span.
    Center,
    /// Flush with the anchor's right or bottom edge.
    End,
}

/// Where an enabled, non-mirrored output is placed.
///
/// Relative placement is the declarative form: it names the output a monitor
/// stands next to, so a scale or mode change moves the neighbour by itself
/// instead of leaving a stale pixel origin. Absolute placement pins the origin,
/// which is what a stored profile or an interactive drag produces.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Placement {
    /// At the layout origin, which is where a single monitor belongs.
    Origin,
    /// At a fixed logical position.
    Absolute {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
    },
    /// Against one side of another output.
    Relative {
        /// The connector this one is placed against. A mirrored output stands for
        /// the output it clones, so naming either has the same meaning.
        of: String,
        /// Which side.
        edge: Edge,
        /// Where along that side.
        align: Align,
    },
}

/// How one output is configured.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutputConfig {
    /// The connector name, such as `"DP-1"`.
    pub connector: String,
    /// Whether the output takes part in the layout at all.
    pub enabled: bool,
    /// The mode to drive, or `None` for the preferred one.
    pub mode: Option<DisplayMode>,
    /// The scale to compose at, or `None` for the supported scale closest to 1.
    pub scale: Option<Scale>,
    /// The rotation or reflection to present. The public output record reports it
    /// as `transform`.
    pub orientation: Orientation,
    /// Whether to ask the connector for a variable refresh rate.
    pub vrr: bool,
    /// Where the output is placed. A mirrored output has no position of its own,
    /// so this is `None` for it and the engine refuses an enabled output without
    /// one.
    pub placement: Option<Placement>,
    /// The connector this one clones, if it is a mirror.
    pub mirror_of: Option<String>,
}

impl OutputConfig {
    /// An enabled output at the layout origin, with every preference left to the
    /// descriptor.
    ///
    /// Two outputs both built this way overlap, which the engine reports; the
    /// shortcut exists so a caller only has to set the placement it cares about.
    #[must_use]
    pub fn enabled(connector: impl Into<String>) -> Self {
        Self {
            connector: connector.into(),
            enabled: true,
            mode: None,
            scale: None,
            orientation: Orientation::Normal,
            vrr: false,
            placement: Some(Placement::Origin),
            mirror_of: None,
        }
    }

    /// A configured but disabled output.
    #[must_use]
    pub fn disabled(connector: impl Into<String>) -> Self {
        Self {
            connector: connector.into(),
            enabled: false,
            mode: None,
            scale: None,
            orientation: Orientation::Normal,
            vrr: false,
            placement: None,
            mirror_of: None,
        }
    }
}

/// The whole declared layout of the session.
///
/// The configuration is written for a hardware setup, so an entry for an output
/// that is not connected is inert rather than wrong: the same configuration has
/// to survive a dock being plugged in and out.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LayoutConfig {
    /// One entry per output the configuration has an opinion about.
    pub outputs: Vec<OutputConfig>,
    /// The connector that owns new windows, or `None` for the first enabled one
    /// in connection order.
    pub primary: Option<String>,
    /// Whether the layout may contain gaps.
    ///
    /// A gap is refused by default because an absolute position left behind by a
    /// mode change puts a hole in the desktop that the pointer cannot cross.
    pub allow_gaps: bool,
}

impl LayoutConfig {
    /// A configuration with no primary and no declared gaps.
    #[must_use]
    pub fn new(outputs: Vec<OutputConfig>) -> Self {
        Self {
            outputs,
            primary: None,
            allow_gaps: false,
        }
    }

    /// Every connector in `order`, enabled, left to right in a row.
    ///
    /// This is what a session uses before anything is configured and what a
    /// hotplug falls back to when the declared layout no longer resolves: it is
    /// connected, gapless and free of overlap by construction, so an unconfigured
    /// session still has a usable desktop.
    #[must_use]
    pub fn row_over(order: &[String]) -> Self {
        let mut outputs = Vec::with_capacity(order.len());
        for (index, connector) in order.iter().enumerate() {
            let placement = match index.checked_sub(1).and_then(|previous| order.get(previous)) {
                Some(previous) => Placement::Relative {
                    of: previous.clone(),
                    edge: Edge::Right,
                    align: Align::Start,
                },
                None => Placement::Origin,
            };
            outputs.push(OutputConfig {
                placement: Some(placement),
                ..OutputConfig::enabled(connector.clone())
            });
        }
        Self {
            outputs,
            primary: order.first().cloned(),
            allow_gaps: false,
        }
    }
}

/// One logical monitor: a rectangle, and the outputs that show it.
///
/// A mirrored group is one logical monitor with several members, because a clone
/// is not a second position on the desktop; it is the same content shown twice.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LogicalMonitor {
    /// The monitor's public identity: the lowest connector name among its
    /// members, which is what window `monitor_id` and `window:move_to_monitor()`
    /// accept.
    pub monitor: String,
    /// Position and size in logical pixels.
    pub rect: Rect,
    /// The scale every member composes at.
    pub scale: Scale,
    /// The rotation every member presents.
    pub orientation: Orientation,
    /// Whether the group was granted a variable refresh rate.
    pub vrr: bool,
    /// The mode every member drives.
    pub mode: DisplayMode,
    /// Every output that shows this monitor, as graph tokens, in connector order:
    /// the first entry is the output the monitor is named after.
    pub members: Vec<OutputId>,
}

impl LogicalMonitor {
    /// Whether `output` shows this monitor.
    #[must_use]
    pub fn contains(&self, output: OutputId) -> bool {
        self.members.contains(&output)
    }

    /// The logical extent the mode, scale and orientation imply.
    ///
    /// The model's invariant check recomputes this rather than trusting the
    /// rectangle, so a layout that passed a mode through unchanged while its
    /// scale changed does not go unnoticed.
    #[must_use]
    pub fn derived_size(&self) -> Size {
        self.mode.logical_size(self.orientation, self.scale)
    }
}

/// The layout the session is presenting.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Layout {
    /// The logical monitors, in a fixed order: left to right, then top to bottom,
    /// then by monitor name.
    pub monitors: Vec<LogicalMonitor>,
    /// The output that owns new windows, as a graph token. It is always a member
    /// of some monitor when there is one.
    pub primary: Option<OutputId>,
}

impl Layout {
    /// Whether no output is being presented.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    /// The monitor with this public identity.
    #[must_use]
    pub fn monitor(&self, monitor: &str) -> Option<&LogicalMonitor> {
        self.monitors.iter().find(|entry| entry.monitor == monitor)
    }

    /// The monitor that `output` shows.
    #[must_use]
    pub fn monitor_of(&self, output: OutputId) -> Option<&LogicalMonitor> {
        self.monitors.iter().find(|monitor| monitor.contains(output))
    }
}

/// Clamps a widened coordinate back into the rectangle type.
///
/// Placement maths runs in `i64` so a relative chain across many monitors cannot
/// overflow, and only the final coordinate needs narrowing.
pub(crate) fn clamp_to_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}
