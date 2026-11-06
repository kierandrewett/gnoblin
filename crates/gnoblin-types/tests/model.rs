//! A model test over deterministic pseudo-random rectangles.
//!
//! The generator is a hand-written linear congruential generator, not a
//! dependency, so the same seed produces the same rectangles on every machine
//! and a failure prints a reproducible pair. The properties are the ones every
//! consumer relies on: an intersection lies inside both operands, a union
//! contains both, and no saturating operation panics on any input.

use gnoblin_types::{Point, Rect, Size};

/// A 64-bit LCG with the Numerical Recipes constants.
struct Rects {
    state: u64,
}

impl Rects {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 33) as u32
    }

    /// A value in `low..=high`.
    fn range(&mut self, low: i64, high: i64) -> i64 {
        let span = (high - low + 1) as u64;
        low + i64::from(self.next_u32()) % span as i64
    }

    fn rect(&mut self) -> Rect {
        Rect::new(
            self.range(-1000, 1000) as i32,
            self.range(-1000, 1000) as i32,
            self.range(0, 500) as u32,
            self.range(0, 500) as u32,
        )
    }

    fn extreme(&mut self) -> Rect {
        Rect::new(
            self.range(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            self.range(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            self.range(0, i64::from(u32::MAX)) as u32,
            self.range(0, i64::from(u32::MAX)) as u32,
        )
    }

    fn point(&mut self) -> Point {
        Point::new(self.range(-1200, 1200) as i32, self.range(-1200, 1200) as i32)
    }
}

/// Whether `outer` covers `inner`, treating an empty inner rectangle as covered
/// by anything.
fn covers(outer: Rect, inner: Rect) -> bool {
    inner.is_empty() || inner.is_inside(outer)
}

/// Whether a clamped point is inside, or exactly at the origin of the empty
/// rectangle it was clamped to.
fn inside_or_on(point: Point, rect: Rect) -> bool {
    if rect.is_empty() {
        point == Point::new(rect.x, rect.y)
    } else {
        rect.contains(point)
    }
}

/// The four interior corners of a rectangle with whole-pixel extents.
fn corners(rect: Rect) -> [Point; 4] {
    let right = rect.x + rect.width as i32 - 1;
    let bottom = rect.y + rect.height as i32 - 1;
    [
        Point::new(rect.x, rect.y),
        Point::new(right, rect.y),
        Point::new(rect.x, bottom),
        Point::new(right, bottom),
    ]
}

#[test]
fn an_intersection_is_inside_both_operands() {
    let mut rects = Rects::new(0x5eed);
    let mut overlaps = 0;
    for _ in 0..4000 {
        let left = rects.rect();
        let right = rects.rect();
        match left.intersection(right) {
            Some(both) => {
                overlaps += 1;
                assert!(
                    !both.is_empty(),
                    "{left:?} and {right:?} produced an empty intersection"
                );
                assert!(both.is_inside(left), "{both:?} not inside {left:?}");
                assert!(both.is_inside(right), "{both:?} not inside {right:?}");
                assert!(
                    both.area() <= left.area().min(right.area()),
                    "{both:?} is larger than an operand"
                );
                assert_eq!(Some(both), right.intersection(left), "intersection is not symmetric");
                for point in corners(both) {
                    assert!(left.contains(point), "{point:?} is outside {left:?}");
                    assert!(right.contains(point), "{point:?} is outside {right:?}");
                }
            }
            None => assert!(
                !left.intersects(right),
                "{left:?} intersects {right:?} without an intersection"
            ),
        }
    }
    assert!(overlaps > 100, "the generator produced too few overlaps: {overlaps}");
}

#[test]
fn a_union_contains_both_operands() {
    let mut rects = Rects::new(0xc0ffee);
    for _ in 0..4000 {
        let left = rects.rect();
        let right = rects.rect();
        let united = left.union(right);
        assert!(covers(united, left), "{united:?} does not contain {left:?}");
        assert!(covers(united, right), "{united:?} does not contain {right:?}");
        assert_eq!(united, right.union(left), "union is not symmetric");
        assert_eq!(united.union(united), united, "union is not idempotent");
        assert_eq!(left.intersection(left), if left.is_empty() { None } else { Some(left) });
        if !left.is_empty() && !right.is_empty() {
            // Coordinates are exact in the generated range, so the union starts
            // at the smaller origin of the two.
            assert_eq!(united.x, left.x.min(right.x));
            assert_eq!(united.y, left.y.min(right.y));
        }
        assert!(united.area() >= left.area() || left.is_empty());
    }
}

#[test]
fn translated_rectangles_track_their_points() {
    let mut rects = Rects::new(0xbeef);
    for _ in 0..2000 {
        let rect = rects.rect().at_least_one();
        let point = rects.point();
        let moved = rect.translate(3, -7);
        assert_eq!(moved.size(), rect.size());
        assert_eq!(moved.contains(point.translate(3, -7)), rect.contains(point));
        assert!(inside_or_on(rect.clamp_point(point), rect));
        assert_eq!(
            i64::from(rect.centre().x),
            i64::from(rect.x) + i64::from(rect.width) / 2
        );
        assert_eq!(
            i64::from(rect.centre().y),
            i64::from(rect.y) + i64::from(rect.height) / 2
        );
    }
}

#[test]
fn saturating_operations_never_panic() {
    let mut rects = Rects::new(0xdead_beef);
    for _ in 0..4000 {
        let left = rects.extreme();
        let right = rects.extreme();
        let point = Point::new(rects.range(i64::from(i32::MIN), i64::from(i32::MAX)) as i32, 0);

        let united = left.union(right);
        if !left.is_empty() && !right.is_empty() {
            // Origins are i32 and saturating conversion of an i32 is exact, so
            // the union's origin is always the smaller of the two.
            assert_eq!(united.x, left.x.min(right.x));
            assert_eq!(united.y, left.y.min(right.y));
        }
        let _ = left.intersection(right);
        let _ = left.intersects(right);
        let _ = left.touches(right);
        let _ = left.clamp_inside(right);
        let _ = left.clamp_point(point);
        let _ = left.centre();
        let _ = left.at_least_one();
        let _ = left.size().swapped();
        let _ = left.to_f64().to_pixel();
        assert_eq!(left.right(), i64::from(left.x) + i64::from(left.width));
        assert_eq!(left.bottom(), i64::from(left.y) + i64::from(left.height));
    }
}

#[test]
fn the_generator_is_deterministic() {
    let mut first = Rects::new(42);
    let mut second = Rects::new(42);
    for _ in 0..16 {
        assert_eq!(first.rect(), second.rect());
        assert_eq!(first.extreme(), second.extreme());
        assert_eq!(first.point(), second.point());
    }
    assert_eq!(Rect::from_size(Size::new(3, 4)), Rect::new(0, 0, 3, 4));
}
