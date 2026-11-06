//! The compositing scale of a logical monitor.

use crate::geometry::Size;

/// A compositing scale, stored as an exact fraction.
///
/// A scale is the ratio between physical and logical pixels, so `1.5` and `3/2`
/// must be the same value and neither may be stored as a float: a float would
/// make "the same configuration" depend on binary rounding, and a logical size
/// derived from it would stop being integral.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scale {
    /// Physical pixels per logical pixel, multiplied by `denominator`.
    numerator: u32,
    /// Reduced denominator, never zero.
    denominator: u32,
}

impl Scale {
    /// The identity scale.
    pub const IDENTITY: Self = Self::integral(1);

    /// `numerator / denominator`, reduced.
    ///
    /// A zero numerator or denominator has no meaning as a scale, so it becomes
    /// the identity scale rather than a division by zero later.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Self {
        if numerator == 0 || denominator == 0 {
            return Self::IDENTITY;
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }

    /// A whole-number scale such as `2`.
    #[must_use]
    pub const fn integral(value: u32) -> Self {
        Self {
            numerator: if value == 0 { 1 } else { value },
            denominator: 1,
        }
    }

    /// A scale written as a percentage, so `150` is `1.5`.
    #[must_use]
    pub fn from_percent(percent: u32) -> Self {
        Self::new(percent, 100)
    }

    /// Physical pixels per logical pixel, multiplied by [`Scale::denominator`].
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// The divisor of [`Scale::numerator`].
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    /// The scale as a continuous ratio, for a renderer or a shader uniform.
    ///
    /// This is the only lossy view of a scale. It exists because a GPU wants a
    /// float; every decision about a logical size is made through
    /// [`Scale::apply`], which stays exact.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// The logical extent of `axis` physical pixels.
    ///
    /// Logical sizes are whole pixels at every scale, including fractional ones,
    /// because a logical monitor whose width is not an integer cannot be
    /// positioned flush against its neighbour: the rounding would have to happen
    /// again somewhere else and the two sides would disagree by a pixel. The rule
    /// is rounding half up, which is why `2560 / 1.5` is `1707` rather than
    /// `1706`, and the result is at least one pixel.
    #[must_use]
    pub fn apply(self, axis: u32) -> u32 {
        // A scale is physical pixels per logical pixel, so the logical extent is
        // the physical extent multiplied by the reciprocal.
        let numerator = u64::from(axis) * u64::from(self.denominator);
        let divisor = u64::from(self.numerator);
        let rounded = (numerator + divisor / 2) / divisor;
        u32::try_from(rounded).unwrap_or(u32::MAX).max(1)
    }

    /// The logical size of a physical size, one axis at a time.
    ///
    /// The result is integral because [`Scale::apply`] is, and it is never
    /// empty: a monitor scaled down to nothing is a configuration error, and a
    /// zero-sized logical monitor breaks every placement rule downstream.
    #[must_use]
    pub fn logical_size(self, physical: Size) -> Size {
        Size::new(self.apply(physical.width), self.apply(physical.height))
    }

    /// Whether this scale is `1`, so callers can skip a scaling pass.
    #[must_use]
    pub const fn is_identity(self) -> bool {
        self.numerator == self.denominator
    }
}

impl Default for Scale {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Scales are ordered by value, not by their stored fraction.
///
/// A derived ordering would compare `(numerator, denominator)` pairs, where
/// `3/2 < 4/3` even though 1.5 is larger than 1.33. Comparing by value is what
/// lets the default scale be the closest supported one without floating point.
impl Ord for Scale {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (u64::from(self.numerator) * u64::from(other.denominator))
            .cmp(&(u64::from(other.numerator) * u64::from(self.denominator)))
    }
}

impl PartialOrd for Scale {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}
