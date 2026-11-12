//! Golden-pixel tests of the rasteriser, one group per op.
//!
//! Rectangle, line, clip and opacity expectations are written out by hand in the test. Text and icon pictures are
//! shown as small coverage maps checked in under `tests/golden/`; set `GNOBLIN_UPDATE_GOLDEN=1` to rewrite them,
//! then read the diff. Text is always drawn with the embedded font, so the goldens hold on every machine.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use gnoblin_paint::{CanvasPainter, FontSet, PaintList, PaintOp, Picture, Rgba8, decode, encode};

const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
const NONE: Rgba8 = Rgba8::TRANSPARENT;
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn painter() -> CanvasPainter {
    CanvasPainter::new(Arc::new(FontSet::embedded()))
}

fn paint(ops: Vec<PaintOp>, width: u32, height: u32) -> Picture {
    painter()
        .paint(&PaintList { ops }, width, height)
        .expect("a picture of a sane size")
        .picture
}

fn fill(x: f32, y: f32, width: f32, height: f32, radius: f32, colour: Rgba8) -> PaintOp {
    PaintOp::Rect {
        x,
        y,
        width,
        height,
        radius,
        fill: colour,
        stroke: NONE,
        stroke_width: 0.0,
    }
}

fn text(x: f32, y: f32, size: f32, text: &str) -> PaintOp {
    PaintOp::Text {
        x,
        y,
        size,
        colour: WHITE,
        font: "sans".to_owned(),
        text: text.to_owned(),
    }
}

fn icon(size: f32, name: &str) -> PaintOp {
    PaintOp::Icon {
        x: 0.0,
        y: 0.0,
        size,
        colour: WHITE,
        name: name.to_owned(),
    }
}

fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) -> bool {
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= tolerance)
}

#[track_caller]
fn assert_pixel(picture: &Picture, x: u32, y: u32, expected: [u8; 4]) {
    let actual = picture.pixel(x, y).expect("inside the picture");
    assert!(
        near(actual, expected, 1),
        "({x}, {y}) is {actual:?}, expected {expected:?}"
    );
}

/// A coverage map: one character per pixel by alpha, `#` solid, `+` more than half, `.` some, space none.
fn coverage(picture: &Picture) -> String {
    let mut out = String::new();
    for y in 0..picture.height {
        for x in 0..picture.width {
            let alpha = picture.pixel(x, y).expect("inside")[3];
            out.push(match alpha {
                0 => ' ',
                1..=63 => '.',
                64..=191 => '+',
                _ => '#',
            });
        }
        out.push('\n');
    }
    out
}

#[track_caller]
fn assert_golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("GNOBLIN_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the golden directory");
        std::fs::write(&path, actual).expect("the golden file");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{name} is missing; run with GNOBLIN_UPDATE_GOLDEN=1"));
    assert_eq!(
        actual, expected,
        "{name} differs; run with GNOBLIN_UPDATE_GOLDEN=1 and read the diff"
    );
}

#[test]
fn an_empty_list_is_a_transparent_picture() {
    let picture = paint(Vec::new(), 3, 2);
    assert_eq!((picture.width, picture.height), (3, 2));
    assert!(picture.data.iter().all(|byte| *byte == 0));
    assert_eq!(picture.data.len(), 3 * 2 * 4);
}

#[test]
fn a_filled_rectangle_covers_exactly_its_pixels() {
    let picture = paint(vec![fill(1.0, 1.0, 3.0, 2.0, 0.0, RED)], 6, 4);
    for y in 0..4 {
        for x in 0..6 {
            let inside = (1..4).contains(&x) && (1..3).contains(&y);
            assert_pixel(&picture, x, y, if inside { [255, 0, 0, 255] } else { CLEAR });
        }
    }
}

#[test]
fn a_translucent_fill_is_premultiplied() {
    let picture = paint(
        vec![fill(0.0, 0.0, 2.0, 2.0, 0.0, Rgba8::new(255, 255, 255, 128))],
        2,
        2,
    );
    assert_pixel(&picture, 0, 0, [128, 128, 128, 128]);
    assert_pixel(&picture, 1, 1, [128, 128, 128, 128]);
    let straight = picture.to_straight();
    assert!(near(
        [straight[0], straight[1], straight[2], straight[3]],
        [255, 255, 255, 128],
        1
    ));
}

#[test]
fn a_later_op_draws_over_an_earlier_one() {
    let picture = paint(
        vec![fill(0.0, 0.0, 4.0, 4.0, 0.0, RED), fill(2.0, 0.0, 2.0, 4.0, 0.0, BLUE)],
        4,
        4,
    );
    assert_pixel(&picture, 1, 2, [255, 0, 0, 255]);
    assert_pixel(&picture, 2, 2, [0, 0, 255, 255]);
}

#[test]
fn a_rounded_rectangle_leaves_its_corners_clear_and_covers_the_right_area() {
    let picture = paint(vec![fill(0.0, 0.0, 10.0, 10.0, 5.0, RED)], 10, 10);
    assert_pixel(&picture, 0, 0, CLEAR);
    assert_pixel(&picture, 9, 0, CLEAR);
    assert_pixel(&picture, 0, 9, CLEAR);
    assert_pixel(&picture, 9, 9, CLEAR);
    assert_pixel(&picture, 5, 5, [255, 0, 0, 255]);
    assert!(
        picture.pixel(4, 0).expect("inside")[3] > 128,
        "the top edge's middle is mostly covered"
    );
    assert!(
        picture.pixel(0, 4).expect("inside")[3] > 128,
        "the left edge's middle is mostly covered"
    );
    // The shape's area is the square less four corners: 100 - (4 - pi) * 25 = 78.54 pixels. Edge coverage comes from
    // sampling, so the summed alpha is close to that and not exact.
    let covered: u32 = picture
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| u32::from(pixel[3]))
        .sum();
    let ideal = 78.54 * 255.0;
    assert!(
        (f64::from(covered) - ideal).abs() < ideal * 0.03,
        "the covered area is {} pixels, expected about 78.5",
        f64::from(covered) / 255.0
    );
}

#[test]
fn a_radius_over_half_the_side_is_clamped_to_a_pill() {
    let big = paint(vec![fill(0.0, 0.0, 12.0, 4.0, 100.0, RED)], 12, 4);
    let exact = paint(vec![fill(0.0, 0.0, 12.0, 4.0, 2.0, RED)], 12, 4);
    assert_eq!(big, exact);
}

#[test]
fn an_outline_is_drawn_inside_the_edge() {
    let picture = paint(
        vec![PaintOp::Rect {
            x: 0.0,
            y: 0.0,
            width: 6.0,
            height: 6.0,
            radius: 0.0,
            fill: NONE,
            stroke: BLUE,
            stroke_width: 1.0,
        }],
        6,
        6,
    );
    for y in 0..6 {
        for x in 0..6 {
            let border = x == 0 || y == 0 || x == 5 || y == 5;
            assert_pixel(&picture, x, y, if border { [0, 0, 255, 255] } else { CLEAR });
        }
    }
}

#[test]
fn a_line_one_pixel_wide_on_a_pixel_centre_fills_one_row() {
    let picture = paint(
        vec![PaintOp::TextLine {
            x1: 0.0,
            y1: 2.5,
            x2: 6.0,
            y2: 2.5,
            width: 1.0,
            colour: RED,
        }],
        6,
        5,
    );
    for y in 0..5 {
        for x in 0..6 {
            assert_pixel(&picture, x, y, if y == 2 { [255, 0, 0, 255] } else { CLEAR });
        }
    }
}

#[test]
fn a_clip_limits_what_is_drawn_and_nested_clips_intersect() {
    let clip = |x: f32, y: f32, width: f32, height: f32| PaintOp::PushClip {
        x,
        y,
        width,
        height,
        radius: 0.0,
    };
    let picture = paint(
        vec![
            clip(1.0, 1.0, 4.0, 4.0),
            clip(3.0, 0.0, 4.0, 4.0),
            fill(0.0, 0.0, 8.0, 8.0, 0.0, RED),
            PaintOp::PopClip,
            fill(0.0, 4.0, 8.0, 1.0, 0.0, BLUE),
            PaintOp::PopClip,
            fill(7.0, 7.0, 1.0, 1.0, 0.0, RED),
        ],
        8,
        8,
    );
    for y in 0..8 {
        for x in 0..8 {
            let expected = if (3..5).contains(&x) && (1..4).contains(&y) {
                [255, 0, 0, 255]
            } else if y == 4 && (1..5).contains(&x) {
                // The blue row is under the outer clip alone: the inner clip has ended.
                [0, 0, 255, 255]
            } else if (x, y) == (7, 7) {
                [255, 0, 0, 255]
            } else {
                CLEAR
            };
            assert_pixel(&picture, x, y, expected);
        }
    }
}

#[test]
fn a_rounded_clip_rounds_what_it_holds() {
    let picture = paint(
        vec![
            PaintOp::PushClip {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                radius: 5.0,
            },
            fill(0.0, 0.0, 10.0, 10.0, 0.0, RED),
            PaintOp::PopClip,
        ],
        10,
        10,
    );
    assert_pixel(&picture, 0, 0, CLEAR);
    assert_pixel(&picture, 5, 5, [255, 0, 0, 255]);
}

#[test]
fn an_empty_clip_passes_nothing() {
    let picture = paint(
        vec![
            PaintOp::PushClip {
                x: 2.0,
                y: 2.0,
                width: 0.0,
                height: 4.0,
                radius: 0.0,
            },
            fill(0.0, 0.0, 8.0, 8.0, 0.0, RED),
            PaintOp::PopClip,
        ],
        8,
        8,
    );
    assert!(picture.data.iter().all(|byte| *byte == 0));
}

#[test]
fn an_opacity_group_composites_once_so_overlaps_inside_do_not_show_through() {
    let picture = paint(
        vec![
            PaintOp::PushOpacity(0.5),
            fill(0.0, 0.0, 4.0, 4.0, 0.0, WHITE),
            fill(2.0, 0.0, 4.0, 4.0, 0.0, WHITE),
            PaintOp::PopOpacity,
        ],
        6,
        4,
    );
    for x in 0..6 {
        assert_pixel(&picture, x, 1, [128, 128, 128, 128]);
    }
}

#[test]
fn opacity_zero_draws_nothing_and_one_changes_nothing() {
    let drawn = |alpha: f32| {
        paint(
            vec![
                PaintOp::PushOpacity(alpha),
                fill(0.0, 0.0, 2.0, 2.0, 0.0, RED),
                PaintOp::PopOpacity,
            ],
            2,
            2,
        )
    };
    assert!(drawn(0.0).data.iter().all(|byte| *byte == 0));
    assert_eq!(drawn(1.0), paint(vec![fill(0.0, 0.0, 2.0, 2.0, 0.0, RED)], 2, 2));
}

#[test]
fn text_is_drawn_from_the_embedded_font() {
    let picture = paint(vec![text(1.0, 0.0, 16.0, "Hi")], 22, 24);
    let map = coverage(&picture);
    assert!(map.contains('#'), "the strokes are solid somewhere");
    assert_golden("text_hi_16.txt", &map);
}

#[test]
fn text_with_a_lowercase_run_matches_its_golden() {
    let picture = paint(vec![text(0.0, 0.0, 13.0, "Window")], 52, 19);
    assert_golden("text_window_13.txt", &coverage(&picture));
}

#[test]
fn a_transparent_text_colour_draws_nothing() {
    let mut op = text(0.0, 0.0, 14.0, "Hi");
    if let PaintOp::Text { colour, .. } = &mut op {
        *colour = NONE;
    }
    assert!(paint(vec![op], 20, 21).data.iter().all(|byte| *byte == 0));
}

#[test]
fn the_ink_of_a_string_lies_inside_its_measured_extent() {
    let painter = painter();
    for (string, size) in [("Hello, World", 20.0), ("Titlebar text", 13.0), ("Éy|g.", 16.0)] {
        let extent = painter.measure(string, size, "sans");
        let margin = 10_u32;
        let picture = painter
            .paint(
                &PaintList {
                    ops: vec![text(margin as f32, margin as f32, size, string)],
                },
                extent.width + 2 * margin,
                extent.height + 2 * margin,
            )
            .expect("a small picture")
            .picture;
        let (mut left, mut right, mut top, mut bottom) = (u32::MAX, 0, u32::MAX, 0);
        for y in 0..picture.height {
            for x in 0..picture.width {
                if picture.pixel(x, y).expect("inside")[3] != 0 {
                    left = left.min(x);
                    right = right.max(x + 1);
                    top = top.min(y);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        assert!(left != u32::MAX, "{string:?} drew nothing");
        // A glyph's ink may lean a pixel past its advance; the extent is the advance.
        assert!(left + 1 >= margin, "{string:?} starts left of its box: {left}");
        assert!(
            right <= margin + extent.width + 1,
            "{string:?} ends right of its extent: {right}"
        );
        assert!(
            right + 3 >= margin + extent.width,
            "{string:?} ends well short of its extent: {right}"
        );
        assert!(
            top >= margin && bottom <= margin + extent.height,
            "{string:?} leaves its line box: {top}..{bottom}"
        );
    }
}

#[test]
fn a_second_line_is_drawn_one_line_box_lower() {
    let one = paint(vec![text(0.0, 0.0, 14.0, "Hi")], 20, 42);
    let two = paint(vec![text(0.0, 0.0, 14.0, "Hi\nHi")], 20, 42);
    let shifted = paint(vec![text(0.0, 21.0, 14.0, "Hi")], 20, 42);
    for y in 0..21 {
        for x in 0..20 {
            assert_eq!(one.pixel(x, y), two.pixel(x, y), "first line at ({x}, {y})");
            assert_eq!(
                shifted.pixel(x, y + 21),
                two.pixel(x, y + 21),
                "second line at ({x}, {y})"
            );
        }
    }
}

#[test]
fn the_close_icon_is_a_cross_and_matches_its_golden() {
    let picture = paint(vec![icon(16.0, "window-close-symbolic")], 16, 16);
    assert_pixel(&picture, 0, 0, CLEAR);
    assert_pixel(&picture, 15, 0, CLEAR);
    assert_pixel(&picture, 8, 0, CLEAR);
    assert!(
        picture.pixel(8, 8).expect("inside")[3] > 200,
        "the strokes cross at the centre"
    );
    for y in 0..16 {
        for x in 0..16 {
            let a = picture.pixel(x, y).expect("inside");
            let b = picture.pixel(15 - x, y).expect("inside");
            assert!(near(a, b, 2), "the cross is mirror symmetric at ({x}, {y})");
        }
    }
    assert_golden("icon_window_close_16.txt", &coverage(&picture));
}

#[test]
fn the_window_icons_match_their_goldens() {
    let mut art = String::new();
    for name in [
        "window-minimize",
        "window-maximize",
        "window-restore",
        "list-add",
        "open-menu",
    ] {
        let picture = paint(vec![icon(16.0, name)], 16, 16);
        assert!(picture.data.iter().any(|byte| *byte != 0), "{name} drew nothing");
        writeln!(art, "{name}").expect("a string");
        art.push_str(&coverage(&picture));
    }
    assert_golden("icons_16.txt", &art);
}

#[test]
fn an_icon_scales_with_its_size() {
    let small = paint(vec![icon(16.0, "window-maximize")], 32, 32);
    let large = paint(vec![icon(32.0, "window-maximize")], 32, 32);
    let count = |picture: &Picture| {
        picture
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 128)
            .count()
    };
    assert!(count(&large) > 2 * count(&small));
}

#[test]
fn an_icon_that_is_not_built_in_draws_nothing_and_is_reported_once() {
    let rendered = painter()
        .paint(
            &PaintList {
                ops: vec![icon(16.0, "nonesuch-symbolic"), icon(16.0, "nonesuch-symbolic")],
            },
            16,
            16,
        )
        .expect("a picture");
    assert!(rendered.picture.data.iter().all(|byte| *byte == 0));
    assert_eq!(rendered.missing_icons, ["nonesuch-symbolic"]);
}

#[test]
fn an_empty_or_oversized_picture_is_refused() {
    let painter = painter();
    let list = PaintList::default();
    assert!(painter.paint(&list, 0, 10).is_none());
    assert!(painter.paint(&list, 10, 0).is_none());
    assert!(painter.paint(&list, 5000, 5000).is_none());
    assert!(painter.paint(&list, 4096, 4096).is_some());
}

#[test]
fn a_list_left_open_is_closed_and_still_drawn() {
    let picture = paint(
        vec![PaintOp::PushOpacity(1.0), fill(0.0, 0.0, 2.0, 2.0, 0.0, RED)],
        2,
        2,
    );
    assert_pixel(&picture, 0, 0, [255, 0, 0, 255]);
}

#[test]
fn painting_twice_gives_the_same_bytes() {
    let ops = vec![
        fill(0.0, 0.0, 40.0, 24.0, 6.0, Rgba8::new(30, 30, 46, 255)),
        text(8.0, 2.0, 13.0, "Determinism"),
        icon(12.0, "window-close-symbolic"),
    ];
    assert_eq!(paint(ops.clone(), 64, 24), paint(ops, 64, 24));
}

#[test]
fn an_encoded_list_paints_as_the_list_does() {
    let list = PaintList {
        ops: vec![fill(0.0, 0.0, 4.0, 4.0, 1.0, RED), text(0.0, 0.0, 12.0, "ok")],
    };
    let decoded = decode(&encode(&list)).expect("a list we encoded");
    let painter = painter();
    assert_eq!(painter.paint(&list, 16, 18), painter.paint(&decoded, 16, 18));
}

// ---- Scale, huge text, metrics -------------------------------------------------------------------------------------

fn ink_box(picture: &Picture) -> (u32, u32, u32, u32) {
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..picture.height {
        for x in 0..picture.width {
            if picture.pixel(x, y).expect("inside")[3] != 0 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (left, top, right, bottom)
}

#[test]
fn a_scaled_paint_draws_the_same_list_at_the_scaled_size() {
    let painter = painter();
    let list = PaintList {
        ops: vec![
            fill(2.0, 2.0, 6.0, 4.0, 0.0, RED),
            PaintOp::TextLine {
                x1: 0.0,
                y1: 9.5,
                x2: 10.0,
                y2: 9.5,
                width: 1.0,
                colour: BLUE,
            },
        ],
    };
    let one = painter.paint_scaled(&list, 10, 10, 1.0).expect("one").picture;
    assert_eq!(
        Some(one.clone()),
        painter.paint(&list, 10, 10).map(|rendered| rendered.picture)
    );
    let two = painter.paint_scaled(&list, 10, 10, 2.0).expect("two").picture;
    assert_eq!((two.width, two.height), (20, 20));
    assert_pixel(&two, 4, 4, [255, 0, 0, 255]);
    assert_pixel(&two, 11, 7, [255, 0, 0, 255]);
    assert_pixel(&two, 3, 4, CLEAR);
    assert_pixel(&two, 15, 4, [255, 0, 0, 255]);
    assert_pixel(&two, 16, 4, CLEAR);
    assert_pixel(&two, 10, 18, [0, 0, 255, 255]);
    assert_pixel(&two, 10, 19, [0, 0, 255, 255]);
    assert_pixel(&two, 10, 17, CLEAR);

    let fractional = painter
        .paint_scaled(&list, 10, 10, 1.5)
        .expect("one and a half")
        .picture;
    assert_eq!((fractional.width, fractional.height), (15, 15));
    assert_pixel(&fractional, 4, 4, [255, 0, 0, 255]);
    let tiny = painter
        .paint_scaled(&list, 10, 10, 0.01)
        .expect("at least one pixel")
        .picture;
    assert_eq!((tiny.width, tiny.height), (1, 1));
}

#[test]
fn a_scaled_paint_refuses_a_scale_it_cannot_draw_and_a_picture_over_the_cap() {
    let painter = painter();
    let list = PaintList::default();
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(painter.paint_scaled(&list, 10, 10, scale).is_none(), "scale {scale}");
    }
    assert!(painter.paint_scaled(&list, 0, 10, 2.0).is_none());
    assert!(
        painter.paint_scaled(&list, 3000, 3000, 2.0).is_none(),
        "36 million pixels"
    );
    assert!(
        painter.paint_scaled(&list, 2048, 2048, 2.0).is_some(),
        "exactly the cap"
    );
}

#[test]
fn scaled_text_is_sharper_not_stretched() {
    let painter = painter();
    let list = PaintList {
        ops: vec![text(2.0, 2.0, 14.0, "Hello")],
    };
    let one = painter.paint_scaled(&list, 60, 26, 1.0).expect("one").picture;
    let two = painter.paint_scaled(&list, 60, 26, 2.0).expect("two").picture;
    let (l1, t1, r1, b1) = ink_box(&one);
    let (l2, t2, r2, b2) = ink_box(&two);
    for (small, large) in [(l1, l2), (t1, t2), (r1, r2), (b1, b2)] {
        assert!(
            large.abs_diff(small * 2) <= 2,
            "the ink of the 2x picture lies where twice the 1x ink does: {small} vs {large}"
        );
    }
    let covered = |picture: &Picture| {
        picture
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] != 0)
            .count()
    };
    assert!(
        covered(&two) > covered(&one) * 3,
        "four times the pixels carry about four times the ink: {} vs {}",
        covered(&two),
        covered(&one)
    );
}

#[test]
fn a_huge_text_draws_only_what_is_on_the_canvas() {
    let painter = painter();
    let all = "x".repeat(16 * 1024);
    let few = "x".repeat(64);
    for size in [12.0, 200.0, 1024.0] {
        let long = painter
            .paint(
                &PaintList {
                    ops: vec![text(0.0, 0.0, size, &all)],
                },
                80,
                40,
            )
            .expect("a small picture")
            .picture;
        let short = painter
            .paint(
                &PaintList {
                    ops: vec![text(0.0, 0.0, size, &few)],
                },
                80,
                40,
            )
            .expect("a small picture")
            .picture;
        assert_eq!(long, short, "size {size}: the characters past the edge change nothing");
    }
    // Lines far below the canvas are not drawn either, and a line far above it neither.
    let lines = format!("{}\nbottom", "\n".repeat(8000));
    let below = painter
        .paint(
            &PaintList {
                ops: vec![text(0.0, 0.0, 14.0, &lines)],
            },
            40,
            40,
        )
        .expect("a small picture")
        .picture;
    assert!(below.data.iter().all(|byte| *byte == 0));
    let above = painter
        .paint(
            &PaintList {
                ops: vec![text(0.0, -300_000.0, 14.0, &lines)],
            },
            40,
            40,
        )
        .expect("a small picture")
        .picture;
    assert!(above.data.iter().all(|byte| *byte == 0));
}

#[test]
fn text_metrics_give_the_extent_and_a_baseline_the_glyphs_rest_on() {
    let painter = painter();
    for size in [12.0_f32, 16.0, 24.0] {
        let metrics = painter.measure_checked("HEH", size, "sans").expect("in range");
        let extent = painter.measure("HEH", size, "sans");
        assert_eq!((metrics.width, metrics.height), (extent.width, extent.height));
        assert!(
            metrics.ascent > 0 && metrics.ascent < metrics.height,
            "{size}: {metrics:?}"
        );
        // Capitals sit on the baseline, so the lowest ink row ends where the ascent says.
        let picture = painter
            .paint(
                &PaintList {
                    ops: vec![text(0.0, 0.0, size, "HEH")],
                },
                metrics.width + 4,
                metrics.height,
            )
            .expect("a small picture")
            .picture;
        let (_, _, _, bottom) = ink_box(&picture);
        assert!(
            bottom.abs_diff(metrics.ascent) <= 1,
            "{size}: ink ends at {bottom}, the ascent is {}",
            metrics.ascent
        );
    }
    let two = painter.measure_checked("a\nb", 16.0, "sans").expect("in range");
    let one = painter.measure_checked("a", 16.0, "sans").expect("in range");
    assert_eq!(two.height, one.height * 2);
    assert_eq!(two.ascent, one.ascent, "the baseline is the first line's");
}
