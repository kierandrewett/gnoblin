//! A physical mode and the logical size it produces.

use crate::geometry::Size;
use crate::scale::Scale;
use crate::transform::Transform;

/// One mode a connector can drive: physical pixels and a refresh rate.
///
/// The refresh rate is stored in millihertz because mode lists are full of
/// 59.94 Hz and 23.976 Hz entries. A whole-hertz field would either round those
/// to 60 and 24, which picks a different mode than the caller asked for, or hold
/// a float, which would make mode equality depend on binary rounding.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Mode {
    /// Active horizontal pixels, before rotation.
    pub width: u32,
    /// Active vertical pixels, before rotation.
    pub height: u32,
    /// Refresh rate in millihertz, so 60 Hz is `60_000`.
    pub refresh_mhz: u32,
}

impl Mode {
    /// A mode described in physical pixels and millihertz.
    #[must_use]
    pub const fn new(width: u32, height: u32, refresh_mhz: u32) -> Self {
        Self {
            width,
            height,
            refresh_mhz,
        }
    }

    /// The mode's size as the panel is driven, before rotation.
    #[must_use]
    pub const fn physical_size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// The mode's size as presented, after rotation.
    ///
    /// Rotation happens before scaling because a quarter turn changes which axis
    /// the physical width lies along.
    #[must_use]
    pub const fn presented_size(self, transform: Transform) -> Size {
        let size = self.physical_size();
        if transform.swaps_axes() { size.swapped() } else { size }
    }

    /// The logical extent this mode produces at `scale` under `transform`.
    ///
    /// Each axis is scaled independently and rounded to a whole pixel by
    /// [`Scale::apply`], so the result is a valid logical monitor extent at every
    /// scale rather than only at integer ones.
    #[must_use]
    pub fn logical_size(self, transform: Transform, scale: Scale) -> Size {
        let presented = self.presented_size(transform);
        Size::new(scale.apply(presented.width), scale.apply(presented.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_HD: Mode = Mode::new(1920, 1080, 60_000);

    #[test]
    fn a_quarter_turn_makes_the_logical_monitor_portrait() {
        assert_eq!(
            FULL_HD.logical_size(Transform::Rotate90, Scale::integral(1)),
            Size::new(1080, 1920)
        );
        assert_eq!(
            FULL_HD.logical_size(Transform::Rotate180, Scale::integral(1)),
            Size::new(1920, 1080)
        );
    }

    #[test]
    fn rotation_and_scale_compose_in_that_order() {
        assert_eq!(
            FULL_HD.logical_size(Transform::Rotate270, Scale::integral(2)),
            Size::new(540, 960)
        );
        assert_eq!(
            FULL_HD.logical_size(Transform::Normal, Scale::from_percent(150)),
            Size::new(1280, 720)
        );
    }
}
