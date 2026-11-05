//! The declarative configuration, and the layout it produces.

use crate::geometry::{Rect, Size};
use crate::ids::OutputId;
use crate::mode::Mode;
use crate::scale::Scale;
use crate::transform::Transform;

/// Which side of its anchor an output is placed against.
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
        /// The output this one is placed against. A mirrored output stands for
        /// the output it clones, so naming either has the same meaning.
        of: OutputId,
        /// Which side.
        edge: Edge,
        /// Where along that side.
        align: Align,
    },
}

/// How one output is configured.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutputConfig {
    /// The output this entry configures.
    pub id: OutputId,
    /// Whether the output takes part in the layout at all.
    pub enabled: bool,
    /// The mode to drive, or `None` for the preferred one.
    pub mode: Option<Mode>,
    /// The scale to compose at, or `None` for the supported scale closest to 1.
    pub scale: Option<Scale>,
    /// The rotation or reflection to present.
    pub transform: Transform,
    /// Whether to ask the connector for a variable refresh rate.
    pub vrr: bool,
    /// Where the output is placed. A mirrored output has no position of its own,
    /// so this is `None` for it and the engine refuses an enabled output without
    /// one.
    pub placement: Option<Placement>,
    /// The output this one clones, if it is a mirror.
    pub mirror_of: Option<OutputId>,
}

impl OutputConfig {
    /// An enabled output at the layout origin, with every preference left to the
    /// descriptor.
    ///
    /// Two outputs both built this way overlap, which the engine reports; the
    /// shortcut exists so a caller only has to set the placement it cares about.
    #[must_use]
    pub fn enabled(id: OutputId) -> Self {
        Self {
            id,
            enabled: true,
            mode: None,
            scale: None,
            transform: Transform::Normal,
            vrr: false,
            placement: Some(Placement::Origin),
            mirror_of: None,
        }
    }

    /// A configured but disabled output.
    #[must_use]
    pub fn disabled(id: OutputId) -> Self {
        Self {
            id,
            enabled: false,
            mode: None,
            scale: None,
            transform: Transform::Normal,
            vrr: false,
            placement: None,
            mirror_of: None,
        }
    }
}

/// The whole declared layout of the session.
///
/// The configuration is written for a hardware setup, so an entry for an output
/// that is not connected is inert rather than wrong: the same Lua configuration
/// has to survive a dock being plugged in and out.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LayoutConfig {
    /// One entry per output the configuration has an opinion about.
    pub outputs: Vec<OutputConfig>,
    /// The output that owns new windows, or `None` for the first enabled one in
    /// connection order.
    pub primary: Option<OutputId>,
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

    /// Every output in `order`, enabled, left to right in a row.
    ///
    /// This is what a session uses before anything is configured and what a
    /// hotplug falls back to when the declared layout no longer resolves: it is
    /// connected, gapless and free of overlap by construction, so an unconfigured
    /// session still has a usable desktop.
    #[must_use]
    pub fn row_over(order: &[OutputId]) -> Self {
        let mut outputs = Vec::with_capacity(order.len());
        for (index, id) in order.iter().enumerate() {
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
                ..OutputConfig::enabled(id.clone())
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
    /// The monitor's identity: the lowest connector name among its members, which
    /// is what window `monitor_id` and `window:move_to_monitor()` accept.
    pub id: OutputId,
    /// Position and size in logical pixels.
    pub rect: Rect,
    /// The scale every member composes at.
    pub scale: Scale,
    /// The rotation every member presents.
    pub transform: Transform,
    /// Whether the group was granted a variable refresh rate.
    pub vrr: bool,
    /// The mode every member drives.
    pub mode: Mode,
    /// Every output that shows this monitor, in connector order.
    pub members: Vec<OutputId>,
}

impl LogicalMonitor {
    /// Whether `output` shows this monitor.
    #[must_use]
    pub fn contains(&self, output: &OutputId) -> bool {
        self.members.contains(output)
    }

    /// The logical extent the mode, scale and transform imply.
    ///
    /// The model's invariant check recomputes this rather than trusting the
    /// rectangle, so a layout that passed a mode through unchanged while its scale
    /// changed does not go unnoticed.
    #[must_use]
    pub fn derived_size(&self) -> Size {
        self.mode.logical_size(self.transform, self.scale)
    }
}

/// The layout the session is presenting.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Layout {
    /// The logical monitors, in a fixed order: left to right, then top to bottom,
    /// then by identity.
    pub monitors: Vec<LogicalMonitor>,
    /// The output that owns new windows. It is always a member of some monitor
    /// when there is one.
    pub primary: Option<OutputId>,
}

impl Layout {
    /// Whether no output is being presented.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    /// The monitor with this identity.
    #[must_use]
    pub fn monitor(&self, id: &OutputId) -> Option<&LogicalMonitor> {
        self.monitors.iter().find(|monitor| &monitor.id == id)
    }

    /// The monitor that `output` shows.
    #[must_use]
    pub fn monitor_of(&self, output: &OutputId) -> Option<&LogicalMonitor> {
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
