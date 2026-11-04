//! Snapping zones and the rectangles they occupy.
//!
//! Snapping is tiling-free: the window keeps its floating state and its previous
//! rectangle, takes the zone's rectangle while snapped, and returns to the previous
//! rectangle when it unsnaps. Zones are derived from the work area so that a panel
//! changing its reservation moves a snapped window with it.

use crate::geometry::Rect;

/// One of the eight edges and corners a window can snap to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum SnapZone {
    /// The left half.
    Left,
    /// The right half.
    Right,
    /// The top half.
    Top,
    /// The bottom half.
    Bottom,
    /// The top-left quarter.
    TopLeft,
    /// The top-right quarter.
    TopRight,
    /// The bottom-left quarter.
    BottomLeft,
    /// The bottom-right quarter.
    BottomRight,
}

impl SnapZone {
    /// Every zone, in a fixed order so callers and tests can iterate them.
    pub const ALL: [Self; 8] = [
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    /// The rectangle this zone covers inside a work area.
    ///
    /// Halves that do not divide evenly give the extra pixel to the right or lower
    /// half, and the four halves of a work area tile it exactly with no gap and no
    /// overlap.
    #[must_use]
    pub fn rect(self, work_area: Rect) -> Rect {
        let half_width = work_area.width / 2;
        let half_height = work_area.height / 2;
        let right_width = work_area.width - half_width;
        let lower_height = work_area.height - half_height;
        let mid_x = work_area.x.saturating_add(half_width);
        let mid_y = work_area.y.saturating_add(half_height);
        match self {
            Self::Left => Rect::new(work_area.x, work_area.y, half_width, work_area.height),
            Self::Right => Rect::new(mid_x, work_area.y, right_width, work_area.height),
            Self::Top => Rect::new(work_area.x, work_area.y, work_area.width, half_height),
            Self::Bottom => Rect::new(work_area.x, mid_y, work_area.width, lower_height),
            Self::TopLeft => Rect::new(work_area.x, work_area.y, half_width, half_height),
            Self::TopRight => Rect::new(mid_x, work_area.y, right_width, half_height),
            Self::BottomLeft => Rect::new(work_area.x, mid_y, half_width, lower_height),
            Self::BottomRight => Rect::new(mid_x, mid_y, right_width, lower_height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_cover_the_work_area_exactly() {
        let work_area = Rect::new(0, 0, 1921, 1081);
        assert_eq!(
            SnapZone::Left.rect(work_area).right(),
            SnapZone::Right.rect(work_area).x
        );
        assert_eq!(SnapZone::Right.rect(work_area).right(), work_area.right());
        assert_eq!(
            SnapZone::Top.rect(work_area).bottom(),
            SnapZone::Bottom.rect(work_area).y
        );
        assert_eq!(SnapZone::Bottom.rect(work_area).bottom(), work_area.bottom());
    }

    #[test]
    fn quarters_tile_without_overlap() {
        let work_area = Rect::new(100, 50, 1000, 700);
        let zones = [
            SnapZone::TopLeft,
            SnapZone::TopRight,
            SnapZone::BottomLeft,
            SnapZone::BottomRight,
        ];
        let rects: Vec<Rect> = zones.iter().map(|zone| zone.rect(work_area)).collect();
        for (index, left) in rects.iter().enumerate() {
            assert!(left.is_inside(&work_area));
            for right in rects.iter().skip(index + 1) {
                assert!(!left.intersects(right));
            }
        }
        let covered: i64 = rects
            .iter()
            .map(|rect| i64::from(rect.width) * i64::from(rect.height))
            .sum();
        assert_eq!(covered, i64::from(work_area.width) * i64::from(work_area.height));
    }

    #[test]
    fn a_zone_follows_an_offset_and_reserved_work_area() {
        let work_area = Rect::new(64, 32, 1856, 1000);
        let left = SnapZone::Left.rect(work_area);
        assert_eq!(left, Rect::new(64, 32, 928, 1000));
    }
}
