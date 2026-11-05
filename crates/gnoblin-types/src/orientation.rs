//! How an output presents the mode it is driven at.

use std::fmt;

/// A rotation or reflection applied to an output's content.
///
/// An orientation is a property of the panel, not of the scene: a monitor can be
/// mounted sideways and every surface on it is presented rotated. It is the same
/// value the display configuration stores, the EDID and mode resolution use to
/// swap axes, and the IPC carries as a string, so it lives here instead of in
/// any one of the three.
///
/// The variants and their order match `MtkMonitorTransform`, which is what the
/// public output record's `transform` strings are derived from, so a migration
/// keeps its spelling. [`Orientation::name`] is the published string; the
/// variant names are the Rust spelling and are not on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Orientation {
    /// No rotation or reflection.
    #[default]
    Normal,
    /// Rotated a quarter turn clockwise.
    Rotate90,
    /// Rotated half a turn clockwise.
    Rotate180,
    /// Rotated three quarters of a turn clockwise.
    Rotate270,
    /// Mirrored.
    Flipped,
    /// Mirrored, then rotated a quarter turn clockwise.
    Flipped90,
    /// Mirrored, then rotated half a turn clockwise.
    Flipped180,
    /// Mirrored, then rotated three quarters of a turn clockwise.
    Flipped270,
}

impl Orientation {
    /// Every orientation, in the order the published strings list them.
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::Rotate90,
        Self::Rotate180,
        Self::Rotate270,
        Self::Flipped,
        Self::Flipped90,
        Self::Flipped180,
        Self::Flipped270,
    ];

    /// The published name, as the configuration and the IPC spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::Flipped => "flipped",
            Self::Flipped90 => "flipped-90",
            Self::Flipped180 => "flipped-180",
            Self::Flipped270 => "flipped-270",
        }
    }

    /// The orientation a published name stands for.
    ///
    /// [`None`] for anything else, rather than a repair: a name that arrives
    /// from a configuration file or a peer must be refused where it is read, so
    /// the caller can report the line that was wrong.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|orientation| orientation.name() == text)
    }

    /// Whether this orientation presents the mode's height horizontally.
    ///
    /// A quarter turn makes the logical monitor taller than it is wide, and
    /// getting this wrong is how a rotated output ends up with a stale
    /// neighbour overlapping it.
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }

    /// Whether this orientation mirrors the output.
    #[must_use]
    pub const fn is_flipped(self) -> bool {
        matches!(
            self,
            Self::Flipped | Self::Flipped90 | Self::Flipped180 | Self::Flipped270
        )
    }
}

impl fmt::Display for Orientation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
