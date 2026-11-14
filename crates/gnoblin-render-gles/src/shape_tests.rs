//! Rounded corners, borders and a shadow that follows the frame, probed pixel by pixel.
//!
//! Each case renders one small scene and reads named pixels back. The scenes are chosen so a probe sits well inside
//! or well outside the shape being tested (a pixel's centre is at `x + 0.5`), which is what keeps the expected value
//! a statement about the effect and not about one driver's antialiasing. The goldens in [`super::golden`] pin the
//! full pictures; these say what each picture means.

use super::tests::{gles, layered_frame, luma, probe, spatial_solid};
use super::*;
use gnoblin_render::{Border, Effects, Mat4, Shadow, Vec3};
use gnoblin_render_frame::frame::Solid;

const BLUE: Color = Color::new(0.0, 0.0, 1.0, 1.0);
const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);
const GREEN: Color = Color::new(0.0, 1.0, 0.0, 1.0);

fn stroke(width: f32, offset: f32, inset: bool, color: Color) -> Pass {
    Pass::Border(Border {
        width,
        offset,
        inset,
        color,
    })
}

fn chain(passes: Vec<Pass>) -> Effects {
    let mut effects = Effects::new();
    for pass in passes {
        effects.push(pass).expect("a pass in range");
    }
    effects
}

/// A transparent layer over `rect` that carries only `passes`, which is how a window's border is stated.
fn ring(rect: Rect, radius: f32, passes: Vec<Pass>) -> Solid {
    let mut solid = Solid::new(rect, Color::TRANSPARENT);
    solid.corner_radius = radius;
    solid.effects = chain(passes);
    solid
}

fn drawn(size: Size, scale: Scale, background: Color, layers: Vec<Solid>) -> Option<Image> {
    let mut renderer = gles(size)?;
    renderer
        .draw(&layered_frame(background, size, layers), scale)
        .expect("drawn");
    Some(renderer.screenshot().expect("a readback"))
}

#[test]
fn a_silhouette_cuts_every_layer_of_a_frame_from_the_frames_corners() {
    let size = Size::new(64, 64);
    let frame = Rect::new(8, 8, 48, 48);
    let layer = |rect: Rect, colour: Color, silhouette: Option<Rect>| {
        let mut solid = Solid::new(rect, colour);
        solid.corner_radius = 10.0;
        solid.silhouette = silhouette;
        solid
    };
    let Some(image) = drawn(
        size,
        Scale::IDENTITY,
        BLUE,
        vec![
            layer(Rect::new(8, 8, 48, 12), RED, Some(frame)),
            layer(Rect::new(8, 20, 48, 36), GREEN, Some(frame)),
        ],
    ) else {
        return;
    };
    assert_eq!(
        probe(&image, 9, 9),
        [0, 0, 255, 255],
        "the frame's top-left corner is cut"
    );
    assert_eq!(
        probe(&image, 54, 9),
        [0, 0, 255, 255],
        "and its top-right corner, on the bar"
    );
    assert_eq!(
        probe(&image, 9, 54),
        [0, 0, 255, 255],
        "and the content's bottom-left corner"
    );
    assert_eq!(
        probe(&image, 9, 21),
        [0, 255, 0, 255],
        "the content's own top-left corner is not cut: it lies in the frame's straight left edge"
    );
    assert_eq!(probe(&image, 32, 14), [255, 0, 0, 255], "the bar draws");
    assert_eq!(probe(&image, 32, 40), [0, 255, 0, 255], "the content draws");

    // Without the silhouette each layer rounds its own corners, which is the broken look the silhouette removes.
    let Some(alone) = drawn(
        size,
        Scale::IDENTITY,
        BLUE,
        vec![
            layer(Rect::new(8, 8, 48, 12), RED, None),
            layer(Rect::new(8, 20, 48, 36), GREEN, None),
        ],
    ) else {
        return;
    };
    assert_eq!(
        probe(&alone, 9, 21),
        [0, 0, 255, 255],
        "a layer rounded to itself cuts the corner where the bar meets the content"
    );
}

#[test]
fn an_inside_border_is_the_stroke_width_wide_along_the_edge() {
    let size = Size::new(64, 64);
    let layer = ring(Rect::new(8, 8, 48, 48), 0.0, vec![stroke(2.0, 0.0, true, RED)]);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![layer]) else {
        return;
    };
    assert_eq!(
        probe(&image, 8, 30),
        [255, 0, 0, 255],
        "the outermost column is the border"
    );
    assert_eq!(probe(&image, 9, 30), [255, 0, 0, 255], "and so is the next");
    assert_eq!(
        probe(&image, 10, 30),
        [0, 0, 255, 255],
        "the third column is inside the stroke's width, so it is the backdrop"
    );
    assert_eq!(probe(&image, 7, 30), [0, 0, 255, 255], "an inside border stays inside");
    assert_eq!(probe(&image, 30, 9), [255, 0, 0, 255], "the top edge too");
    assert_eq!(probe(&image, 55, 30), [255, 0, 0, 255], "and the right");
    assert_eq!(probe(&image, 30, 55), [255, 0, 0, 255], "and the bottom");
    assert_eq!(probe(&image, 32, 32), [0, 0, 255, 255], "the middle is not painted");
}

#[test]
fn an_outside_border_sits_beyond_the_edge_by_its_offset() {
    let size = Size::new(64, 64);
    let layer = ring(Rect::new(8, 8, 48, 48), 0.0, vec![stroke(2.0, 1.0, false, RED)]);
    let Some(image) = drawn(size, Scale::IDENTITY, Color::WHITE, vec![layer]) else {
        return;
    };
    assert_eq!(probe(&image, 5, 30), [255, 0, 0, 255], "the far column of the stroke");
    assert_eq!(probe(&image, 6, 30), [255, 0, 0, 255], "the near column of the stroke");
    assert_eq!(
        probe(&image, 7, 30),
        [255, 255, 255, 255],
        "the offset leaves one clear pixel beside the edge"
    );
    assert_eq!(probe(&image, 4, 30), [255, 255, 255, 255], "past the stroke is clear");
    assert_eq!(
        probe(&image, 8, 30),
        [255, 255, 255, 255],
        "the layer's own edge is not stroked"
    );
}

#[test]
fn borders_stack_in_list_order_as_concentric_rings() {
    let size = Size::new(64, 64);
    let layer = ring(
        Rect::new(8, 8, 48, 48),
        0.0,
        vec![stroke(2.0, 0.0, true, RED), stroke(2.0, 2.0, true, GREEN)],
    );
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![layer]) else {
        return;
    };
    assert_eq!(probe(&image, 9, 30), [255, 0, 0, 255], "the first border is outermost");
    assert_eq!(
        probe(&image, 10, 30),
        [0, 255, 0, 255],
        "the second sits its offset inside it"
    );
    assert_eq!(probe(&image, 11, 30), [0, 255, 0, 255]);
    assert_eq!(probe(&image, 12, 30), [0, 0, 255, 255], "and then the backdrop");
}

#[test]
fn a_border_follows_the_rounded_corner_and_the_fill_is_cut_with_it() {
    let size = Size::new(64, 64);
    let mut fill = Solid::new(Rect::new(8, 8, 48, 48), GREEN);
    fill.corner_radius = 12.0;
    fill.effects = chain(vec![stroke(2.0, 0.0, true, RED)]);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![fill]) else {
        return;
    };
    assert_eq!(
        probe(&image, 9, 9),
        [0, 0, 255, 255],
        "the corner is cut, border and fill alike"
    );
    assert_eq!(
        probe(&image, 8, 32),
        [255, 0, 0, 255],
        "the straight edge is the border"
    );
    assert_eq!(probe(&image, 32, 32), [0, 255, 0, 255], "the middle is the fill");
    // The corner's arc is centred on (20, 20) with radius 12. Pixel (12, 12) is 10.6 from it, 1.4 inside the arc:
    // in the stroke. Pixel (13, 13) is 9.2 from it, 2.8 inside: past the stroke, so the fill.
    assert_eq!(
        probe(&image, 12, 12),
        [255, 0, 0, 255],
        "the border bends round the corner"
    );
    assert_eq!(probe(&image, 13, 13), [0, 255, 0, 255], "and the fill is inside it");
}

#[test]
fn a_border_fades_with_the_layers_tint_alpha() {
    let size = Size::new(64, 64);
    let mut layer = ring(Rect::new(8, 8, 48, 48), 0.0, vec![stroke(2.0, 0.0, true, RED)]);
    layer.tint = Color::new(1.0, 1.0, 1.0, 0.5);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![layer]) else {
        return;
    };
    let pixel = probe(&image, 8, 30);
    assert!(
        pixel[0].abs_diff(128) <= 2 && pixel[2].abs_diff(128) <= 2 && pixel[1] == 0,
        "half red over blue, got {pixel:?}"
    );
}

#[test]
fn a_rounded_shadow_is_darkest_at_the_edge_and_never_darker_further_out() {
    let size = Size::new(96, 96);
    let mut solid = Solid::new(Rect::new(32, 32, 32, 32), RED);
    solid.corner_radius = 8.0;
    solid.effects = chain(vec![Pass::Shadow(Shadow {
        offset: Vec3::ZERO,
        blur: 10.0,
        spread: 0.0,
        opacity: 1.0,
        color: Color::BLACK,
        frame: false,
    })]);
    let Some(image) = drawn(size, Scale::IDENTITY, Color::WHITE, vec![solid]) else {
        return;
    };
    let mut previous = 0;
    for x in (12..32).rev() {
        let brightness = luma(probe(&image, x, 48));
        assert!(
            brightness >= previous,
            "the shadow gets no darker going away from the edge: x = {x} is {brightness}, the pixel nearer was {previous}"
        );
        previous = brightness;
    }
    assert!(
        luma(probe(&image, 31, 48)) < luma(probe(&image, 22, 48)),
        "the shadow is darker at the edge than ten pixels out"
    );
    assert_eq!(
        probe(&image, 8, 48),
        [255, 255, 255, 255],
        "past the blur the backdrop is untouched"
    );
    // The shadow takes the layer's shape: at a corner it is further from the layer than along an edge, so it is
    // lighter there at the same distance in x and y.
    assert!(
        luma(probe(&image, 30, 30)) > luma(probe(&image, 30, 48)),
        "a rounded corner's shadow is lighter than the edge's at the same distance"
    );
}

/// A shadow that follows the frame covers the room above the content that a titlebar fills, and one that does not
/// leaves it clear.
#[test]
fn a_frame_shadow_covers_the_decoration_the_content_shadow_leaves_out() {
    let size = Size::new(96, 96);
    let frame = Rect::new(16, 16, 64, 64);
    let body = |follows_frame: bool| {
        let mut solid = Solid::new(Rect::new(16, 40, 64, 40), RED);
        solid.silhouette = Some(frame);
        solid.effects = chain(vec![Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 8.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: follows_frame,
        })]);
        solid
    };
    let Some(with_frame) = drawn(size, Scale::IDENTITY, Color::WHITE, vec![body(true)]) else {
        return;
    };
    let Some(content_only) = drawn(size, Scale::IDENTITY, Color::WHITE, vec![body(false)]) else {
        return;
    };
    assert!(
        luma(probe(&with_frame, 48, 28)) < 100,
        "the frame's room above the content is shadowed"
    );
    assert_eq!(
        probe(&content_only, 48, 28),
        [255, 255, 255, 255],
        "the content's shadow stops 12 pixels short of it"
    );
    assert!(
        luma(probe(&with_frame, 48, 12)) < luma(probe(&content_only, 48, 12)),
        "above the frame the shadow already reaches"
    );
    assert_eq!(
        probe(&with_frame, 48, 60),
        [255, 0, 0, 255],
        "the content itself is drawn over its shadow"
    );
}

#[test]
fn radius_and_border_scale_with_a_fractional_output_scale() {
    // 96 physical pixels at scale 3/2 are 64 logical ones. The layer is (8, 8, 40, 40) logical, so (12, 12, 60, 60)
    // physical, and its 2 logical pixel border is 3 physical pixels wide.
    let size = Size::new(96, 96);
    let layer = ring(Rect::new(8, 8, 40, 40), 0.0, vec![stroke(2.0, 0.0, true, RED)]);
    let Some(image) = drawn(size, Scale::new(3, 2), BLUE, vec![layer]) else {
        return;
    };
    for x in 12..15 {
        assert_eq!(
            probe(&image, x, 40),
            [255, 0, 0, 255],
            "column {x} is in the 3 pixel stroke"
        );
    }
    assert_eq!(
        probe(&image, 15, 40),
        [0, 0, 255, 255],
        "the stroke ends after 3 physical pixels"
    );
    assert_eq!(
        probe(&image, 11, 40),
        [0, 0, 255, 255],
        "and starts at the physical edge"
    );

    let mut fill = Solid::new(Rect::new(8, 8, 40, 40), GREEN);
    fill.corner_radius = 8.0;
    let Some(rounded) = drawn(size, Scale::new(3, 2), BLUE, vec![fill]) else {
        return;
    };
    // The radius is 12 physical pixels, so the corner pixel (13, 13) lies 12.0 + 1.5*sqrt(2) - ... outside the arc.
    assert_eq!(
        probe(&rounded, 13, 13),
        [0, 0, 255, 255],
        "the corner is cut at the physical radius"
    );
    assert_eq!(probe(&rounded, 30, 13), [0, 255, 0, 255], "and the top edge is whole");
}

/// A layer drawn through a model matrix is not on the flat plane its silhouette is stated in, so a border or a shadow
/// on it is refused as a blur is: the frame paints what it paints without them.
#[test]
fn a_border_and_a_frame_shadow_on_a_projected_layer_draw_nothing_extra() {
    let size = Size::new(32, 32);
    let plain = {
        let solid = spatial_solid(Rect::new(8, 8, 16, 16), RED, Mat4::IDENTITY, true);
        layered_frame(Color::WHITE, size, vec![solid])
    };
    let effectful = {
        let mut solid = spatial_solid(Rect::new(8, 8, 16, 16), RED, Mat4::IDENTITY, true);
        solid.silhouette = Some(Rect::new(4, 4, 24, 24));
        solid.effects = chain(vec![
            stroke(2.0, 1.0, false, GREEN),
            Pass::Shadow(Shadow {
                offset: Vec3::ZERO,
                blur: 6.0,
                spread: 0.0,
                opacity: 1.0,
                color: Color::BLACK,
                frame: true,
            }),
        ]);
        layered_frame(Color::WHITE, size, vec![solid])
    };
    let Some(mut renderer) = gles(size) else {
        return;
    };
    renderer.draw(&plain, Scale::IDENTITY).expect("drawn");
    let expected = renderer.screenshot().expect("a readback");
    renderer.draw(&effectful, Scale::IDENTITY).expect("drawn");
    let actual = renderer.screenshot().expect("a readback");
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(probe(&actual, x, y), probe(&expected, x, y), "at {x},{y}");
        }
    }
}
