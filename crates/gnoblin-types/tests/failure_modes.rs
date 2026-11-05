//! The failure modes the primitives exist to prevent.
//!
//! Each test names one way a value type can go wrong at a crate boundary:
//! arithmetic that overflows, an empty rectangle read as a pixel, a fractional
//! scale that produces a non-integral logical size, and a depth value silently
//! dropped.

use gnoblin_types::{Insets, Point, PointF, Rect, Scale, Size, Transform, Unsupported3d, Unsupported3dField, Vec3};

#[test]
fn edges_do_not_overflow_and_unions_saturate() {
    // An origin near the top of the signed range plus an extent near the top of
    // the unsigned range is exactly the pair that overflows an `i32` edge.
    let far = Rect::new(i32::MAX - 1, i32::MAX - 1, u32::MAX, u32::MAX);
    assert_eq!(far.right(), i64::from(i32::MAX - 1) + i64::from(u32::MAX));
    assert_eq!(far.bottom(), i64::from(i32::MAX - 1) + i64::from(u32::MAX));
    assert!(far.contains(Point::new(i32::MAX - 1, i32::MAX - 1)));
    assert!(far.contains(Point::new(i32::MAX, i32::MAX)));

    let near = Rect::new(i32::MIN, i32::MIN, 10, 10);
    let united = near.union(far);
    // The span from i32::MIN to `far.right()` does not fit in a `u32` extent, so
    // the union saturates instead of wrapping into a tiny rectangle.
    assert_eq!(united.width, u32::MAX);
    assert_eq!(united.x, i32::MIN);

    // Intersection of two extreme rectangles is exact or absent, never a panic.
    let overlap = near.intersection(far);
    assert!(overlap.is_none() || overlap.is_some_and(|rect| rect.is_inside(near) && rect.is_inside(far)));

    // Arbitrary extreme values, including a zero extent, must not panic.
    for width in [0, 1, u32::MAX] {
        for height in [0, 1, u32::MAX] {
            let rect = Rect::new(i32::MIN, i32::MAX, width, height);
            let _ = rect.union(Rect::new(i32::MAX, i32::MIN, u32::MAX, 0));
            let _ = rect.intersection(Rect::new(i32::MAX, i32::MIN, 1, 1));
            let _ = rect.clamp_inside(Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX));
            let _ = rect.clamp_point(Point::new(i32::MAX, i32::MIN));
            let _ = rect.centre();
        }
    }
}

#[test]
fn an_empty_rectangle_is_not_a_zero_sized_pixel() {
    let empty = Rect::new(40, 50, 0, 10);
    assert!(empty.is_empty());
    assert!(!empty.contains(Point::new(40, 50)));
    assert!(!empty.intersects(empty));
    assert_eq!(empty.intersection(empty), None);
    assert_eq!(empty.union(Rect::new(1, 2, 3, 4)), Rect::new(1, 2, 3, 4));
    assert_eq!(Rect::new(1, 2, 3, 4).union(empty), Rect::new(1, 2, 3, 4));
    // Two empty rectangles cover nothing, so their union is the canonical empty
    // rectangle in either order.
    assert_eq!(Rect::EMPTY.union(empty), Rect::EMPTY);
    assert_eq!(empty.union(Rect::EMPTY), Rect::EMPTY);
    assert_eq!(empty.clamp_point(Point::new(0, 0)), Point::new(40, 50));

    // Rounding up is explicit and keeps the origin.
    assert_eq!(empty.at_least_one(), Rect::new(40, 50, 1, 10));
    assert_eq!(Size::ZERO.at_least_one(), Size::new(1, 1));
    assert_eq!(Size::default(), Size::ZERO);

    // A zero-size rectangle never counts as an overlap, so placement scans do
    // not trip over a degenerate client request.
    let pixel = Rect::new(40, 50, 1, 1);
    assert!(!empty.intersects(pixel));
    assert!(pixel.intersects(pixel));
}

#[test]
fn a_fractional_scale_keeps_logical_sizes_integral() {
    let one_and_a_half = Scale::new(3, 2);
    assert_eq!(one_and_a_half, Scale::from_percent(150));
    assert_eq!(one_and_a_half.numerator(), 3);
    assert_eq!(one_and_a_half.denominator(), 2);

    // Rounding half up: 2560 / 1.5 is 1706.66, which becomes 1707, not 1706.
    assert_eq!(one_and_a_half.apply(2560), 1707);
    assert_eq!(one_and_a_half.apply(1440), 960);
    assert_eq!(one_and_a_half.apply(3), 2);
    assert_eq!(one_and_a_half.apply(0), 1);
    assert_eq!(one_and_a_half.logical_size(Size::new(2560, 1440)), Size::new(1707, 960));

    // Every logical size is a whole, non-zero pixel count for any scale and any
    // physical extent: the result type cannot carry a fraction.
    for numerator in 1..=8u32 {
        for denominator in 1..=8u32 {
            let scale = Scale::new(numerator, denominator);
            for extent in [1u32, 2, 3, 7, 13, 1920, 2560, u32::MAX] {
                let logical = scale.apply(extent);
                assert!(logical >= 1, "{scale:?} applied to {extent} produced {logical}");
            }
        }
    }

    // A zero scale means identity instead of a division by zero.
    assert_eq!(Scale::new(0, 5), Scale::IDENTITY);
    assert_eq!(Scale::new(5, 0), Scale::IDENTITY);
    assert!(Scale::IDENTITY.is_identity());
    assert_eq!(Scale::IDENTITY.as_f64(), 1.0);

    // The order is by value: 3/2 is larger than 4/3 although 3 < 4.
    assert!(Scale::new(3, 2) > Scale::new(4, 3));
}

#[test]
fn a_3d_value_is_refused_by_name_instead_of_dropped() {
    let mut transform = Transform::translate(10.0, 20.0);
    assert!(transform.is_2d());
    assert_eq!(transform, Transform::from_2d(Vec3::flat(10.0, 20.0), 0.0, Vec3::ONE));

    transform.offset.z = 5.0;
    assert_eq!(
        transform.check_2d().expect_err("depth offset"),
        Unsupported3d::new(Unsupported3dField::OffsetZ)
    );
    assert_eq!(Unsupported3d::CODE, "unsupported-3d");
    assert!(
        transform
            .check_2d()
            .expect_err("refused")
            .to_string()
            .contains("offset_z")
    );

    // A refused transform maps no point, so a caller cannot accidentally render
    // a node with its depth value ignored.
    assert!(transform.apply_point(PointF::new(1.0, 1.0)).is_err());

    // Each member is refused, in a stable order.
    let fields = [
        (
            Unsupported3dField::RotationX,
            Transform {
                rotation_degrees: Vec3::new(1.0, 0.0, 0.0),
                ..Transform::IDENTITY
            },
        ),
        (
            Unsupported3dField::RotationY,
            Transform {
                rotation_degrees: Vec3::new(0.0, 1.0, 0.0),
                ..Transform::IDENTITY
            },
        ),
        (
            Unsupported3dField::ScaleZ,
            Transform {
                scale: Vec3::new(1.0, 1.0, 2.0),
                ..Transform::IDENTITY
            },
        ),
        (
            Unsupported3dField::Depth,
            Transform {
                depth: 1.0,
                ..Transform::IDENTITY
            },
        ),
    ];
    for (expected, transform) in fields {
        assert_eq!(transform.check_2d().expect_err("refused").field(), expected);
        assert!(!transform.is_2d());
    }

    // The 2D constructors cannot produce a refused value.
    for transform in [
        Transform::IDENTITY,
        Transform::translate(-3.5, 4.5),
        Transform::scaled(2.0, 0.5),
        Transform::rotate_z(45.0),
    ] {
        assert!(transform.is_2d(), "{transform:?}");
        assert!(transform.apply_point(PointF::new(1.0, 2.0)).is_ok());
    }
}

#[test]
fn insets_empty_a_work_area_instead_of_wrapping_it() {
    let output = Rect::new(0, 0, 1920, 1080);
    let panels = Insets::new(32, 0, 40, 0);
    assert_eq!(panels.applied_to(output), Rect::new(0, 32, 1920, 1008));

    // More reserved space than the output has leaves nothing, not a huge extent
    // produced by subtracting into the unsigned range.
    let greedy = Insets::uniform(2000);
    assert!(greedy.applied_to(output).is_empty());
    assert_eq!(greedy.applied_to(output).at_least_one().size(), Size::new(1, 1));

    assert!(Insets::ZERO.is_zero());
    assert_eq!(Insets::uniform(-4).applied_to(output), Rect::new(-4, -4, 1928, 1088));
}

#[test]
fn a_point_outside_every_range_clamps_to_the_rectangle() {
    let bounds = Rect::new(-10, -10, 20, 20);
    assert_eq!(bounds.clamp_point(Point::new(100, -100)), Point::new(9, -10));
    assert_eq!(bounds.centre(), Point::new(0, 0));
    assert_eq!(bounds.clamp_inside(Rect::new(0, 0, 5, 5)), Rect::new(0, 0, 20, 20));
    assert_eq!(Rect::new(1, 1, 2, 2).clamp_inside(bounds), Rect::new(1, 1, 2, 2));
}

#[test]
fn an_orientation_names_itself_and_swaps_only_on_quarter_turns() {
    use gnoblin_types::Orientation;

    let named: Vec<&str> = Orientation::ALL.iter().map(|orientation| orientation.name()).collect();
    assert_eq!(
        named,
        [
            "normal",
            "90",
            "180",
            "270",
            "flipped",
            "flipped-90",
            "flipped-180",
            "flipped-270"
        ]
    );
    for orientation in Orientation::ALL {
        assert_eq!(Orientation::parse(orientation.name()), Some(orientation));
        assert_eq!(orientation.to_string(), orientation.name());
    }
    assert_eq!(Orientation::parse("Rotate90"), None);
    assert_eq!(Orientation::parse(""), None);
    assert_eq!(Orientation::default(), Orientation::Normal);

    // A quarter turn presents the mode's height horizontally; a half turn does
    // not, and the flipped variants follow the rotation they carry.
    for orientation in [
        Orientation::Rotate90,
        Orientation::Rotate270,
        Orientation::Flipped90,
        Orientation::Flipped270,
    ] {
        assert!(orientation.swaps_axes(), "{orientation:?}");
    }
    for orientation in [
        Orientation::Normal,
        Orientation::Rotate180,
        Orientation::Flipped,
        Orientation::Flipped180,
    ] {
        assert!(!orientation.swaps_axes(), "{orientation:?}");
    }
    assert_eq!(Orientation::ALL.iter().filter(|o| o.is_flipped()).count(), 4);
}
