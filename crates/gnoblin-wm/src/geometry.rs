//! The one-pixel floor window management puts on an extent, and the narrowing
//! its edge arithmetic needs.
//!
//! Rectangles, sizes and insets are the shared primitives of `gnoblin-types`, so a
//! rectangle this crate hands back is the same value the renderer, the output
//! layout and the accessibility tree already hold. One of that type's decisions is
//! deliberately not this crate's, though: a shared logical extent is unsigned and
//! may be zero, because "this rectangle covers no pixel" is a real state for a
//! hidden surface or an output with nothing left over. Window management needs
//! every managed window to be reachable input, so a degenerate request is floored
//! at one pixel at the seams where it enters: a client's requested rectangle, an
//! output's work area and a placement size.
//!
//! # Failure modes
//!
//! 1. **A client asks for a zero or negative extent, and the window disappears
//!    from every placement scan.** [`extent`] floors it at [`MIN_EXTENT`].
//!    Covered by `a_degenerate_extent_becomes_one_pixel`.
//! 2. **An edge calculation leaves the coordinate type.** Edges are computed in
//!    [`i64`] and [`clamp_to_i32`] narrows them back, saturating instead of
//!    wrapping. Covered by `an_edge_beyond_the_coordinate_range_saturates`.

/// The smallest extent a managed window or an output's work area can have.
pub(crate) const MIN_EXTENT: u32 = 1;

/// The extent a caller's request becomes, never smaller than a pixel.
///
/// The input is [`i64`] because the callers reach it with a difference of two
/// extents and an inset, which can be negative and does not fit [`i32`] at the far
/// end of the range. A value that does not fit [`u32`] saturates to
/// [`u32::MAX`], which no real layout reaches.
pub(crate) fn extent(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0).max(MIN_EXTENT)
}

/// Clamps a widened coordinate back into [`gnoblin_types::Rect`]'s coordinate type.
///
/// The work area, placement and drag maths run in [`i64`] so an origin near the
/// edge of the range cannot overflow, and only the final coordinate needs
/// narrowing.
pub(crate) fn clamp_to_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_extent_becomes_one_pixel() {
        assert_eq!(extent(0), 1);
        assert_eq!(extent(-40), 1);
        assert_eq!(extent(i64::from(i32::MIN)), 1);
    }

    #[test]
    fn a_real_extent_is_kept_exactly() {
        assert_eq!(extent(1), 1);
        assert_eq!(extent(1920), 1920);
        assert_eq!(extent(i64::from(u32::MAX)), u32::MAX);
    }

    #[test]
    fn an_edge_beyond_the_coordinate_range_saturates() {
        assert_eq!(clamp_to_i32(i64::from(i32::MAX) + 1), i32::MAX);
        assert_eq!(clamp_to_i32(i64::from(i32::MIN) - 1), i32::MIN);
        assert_eq!(clamp_to_i32(-7), -7);
    }
}
