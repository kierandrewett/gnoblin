//! Output orientation.

/// A rotation or reflection of the output's content.
///
/// The variants and their order match `MtkMonitorTransform`, which is what the
/// public output record's `transform` strings are derived from, so a mode size
/// swaps axes exactly where the reference does: for the odd rotations and their
/// flipped counterparts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Transform {
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

impl Transform {
    /// Whether this transform presents the mode's height horizontally.
    ///
    /// A quarter turn makes the logical monitor taller than it is wide, and
    /// getting this wrong is how a rotated output ends up with a stale neighbour
    /// overlapping it.
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }

    /// Whether this transform mirrors the output.
    #[must_use]
    pub const fn is_flipped(self) -> bool {
        matches!(
            self,
            Self::Flipped | Self::Flipped90 | Self::Flipped180 | Self::Flipped270
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_quarter_turns_swap_the_axes() {
        for transform in [
            Transform::Rotate90,
            Transform::Rotate270,
            Transform::Flipped90,
            Transform::Flipped270,
        ] {
            assert!(transform.swaps_axes(), "{transform:?} presents the mode rotated");
        }
        for transform in [
            Transform::Normal,
            Transform::Rotate180,
            Transform::Flipped,
            Transform::Flipped180,
        ] {
            assert!(!transform.swaps_axes(), "{transform:?} keeps the mode's axes");
        }
    }

    #[test]
    fn half_the_transforms_mirror() {
        let mirrored = [
            Transform::Normal,
            Transform::Rotate90,
            Transform::Rotate180,
            Transform::Rotate270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ]
        .into_iter()
        .filter(|transform| transform.is_flipped())
        .count();
        assert_eq!(mirrored, 4);
    }
}
