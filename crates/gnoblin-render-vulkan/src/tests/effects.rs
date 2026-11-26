//! Shadows, inset shadows, borders, backgrounds and filter grades on the Vulkan renderer.
//!
//! The shared golden scenes (`gnoblin_render_frame::golden`) pin whole pictures; these say what each picture means.
//! Each scene is chosen so a probe sits well inside or well outside the shape being tested (a pixel's centre is at
//! `x + 0.5`), which keeps the expected value a statement about the effect and not about one driver's antialiasing.
//!
//! What can go wrong, and the test that catches it:
//!
//! - an effect drawn outside the damage: `a_shadow_does_not_leave_the_damage`;
//! - a wrong paint order (a border under the content, a background over it): `a_background_is_under_the_content...`;
//! - a physical size taken for a logical one: `a_border_scales_with_a_fractional_output_scale`;
//! - a shape that ignores the radius or the silhouette: the rounded and silhouette tests;
//! - an opacity or tint an effect does not carry: `an_effect_fades_with_its_layer`;
//! - a grade that differs between the shader and the CPU twin, or that leaks onto the backdrop: the grade tests.

use gnoblin_render::{Border, FilterParams, Pass, Shadow, Vec3};
use gnoblin_render_frame::golden;
use gnoblin_render_frame::grade::{self, Grade};

use super::*;

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

fn drop_shadow(offset: Vec3, blur: f32, spread: f32, opacity: f32, frame: bool) -> Pass {
    Pass::Shadow(Shadow {
        offset,
        blur,
        spread,
        opacity,
        color: Color::BLACK,
        frame,
        inset: false,
    })
}

fn chain(passes: Vec<Pass>) -> Effects {
    let mut effects = Effects::new();
    for pass in passes {
        effects.push(pass).expect("a pass in range");
    }
    effects
}

/// A layer over `rect` with no colour of its own that carries only `passes`, which is how a window's border is stated.
fn ring(rect: Rect, radius: f32, passes: Vec<Pass>) -> Solid {
    let mut solid = Solid::new(rect, Color::TRANSPARENT);
    solid.corner_radius = radius;
    solid.effects = chain(passes);
    solid
}

/// `layers` over `background`, drawn on a fresh renderer at `scale` and read back. `None` where there is no Vulkan.
fn drawn(size: Size, scale: Scale, background: Color, layers: Vec<Solid>) -> Option<Image> {
    let mut vulkan = renderer(size)?;
    let mut frame = frame(background, size);
    for layer in layers {
        frame.push_layer(Layer::Solid(layer));
    }
    vulkan.draw(&frame, scale).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    clean(&vulkan);
    Some(image)
}

fn probe(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image.colour_at(x, y).expect("a pixel of the picture")
}

fn luma(pixel: [u8; 4]) -> u32 {
    (u32::from(pixel[0]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[2]) * 114) / 1000
}

#[test]
fn the_shared_scenes_match_their_goldens_on_vulkan() {
    let mut vulkan = None;
    for case in &golden::CASES {
        // The blur is not drawn yet, so a scene that has one is left to the renderer lane that draws it.
        if golden::blurs(case) {
            println!(
                "leaving out the {} golden: it blurs, and this renderer does not yet",
                case.name
            );
            continue;
        }
        let Some(vulkan) = vulkan.get_or_insert_with(|| renderer(golden::SIZE)).as_mut() else {
            return;
        };
        vulkan
            .draw(&golden::frame_of(case), Scale::IDENTITY)
            .expect("the frame");
        let image = vulkan.screenshot().expect("a readback");
        golden::check(case, "Vulkan", &image);
        clean(vulkan);
    }
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
        "the third column is the backdrop"
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
        [255; 4],
        "the offset leaves one clear pixel beside the edge"
    );
    assert_eq!(probe(&image, 4, 30), [255; 4], "past the stroke is clear");
    assert_eq!(probe(&image, 8, 30), [255; 4], "the layer's own edge is not stroked");
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
    // The corner's arc is centred on (20, 20) with radius 12. Pixel (12, 12) is 10.6 from it, 1.4 inside the arc: in the
    // stroke. Pixel (13, 13) is 9.2 from it, 2.8 inside: past the stroke, so the fill.
    assert_eq!(
        probe(&image, 12, 12),
        [255, 0, 0, 255],
        "the border bends round the corner"
    );
    assert_eq!(probe(&image, 13, 13), [0, 255, 0, 255], "and the fill is inside it");
}

#[test]
fn an_effect_fades_with_its_layer() {
    let size = Size::new(64, 64);
    // A tint with half alpha fades the border, as it fades the layer's own pixels.
    let mut tinted = ring(Rect::new(8, 8, 48, 48), 0.0, vec![stroke(2.0, 0.0, true, RED)]);
    tinted.tint = Color::new(1.0, 1.0, 1.0, 0.5);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![tinted]) else {
        return;
    };
    let pixel = probe(&image, 8, 30);
    assert!(
        pixel[0].abs_diff(128) <= 2 && pixel[2].abs_diff(128) <= 2 && pixel[1] == 0,
        "half red over blue, got {pixel:?}"
    );
    // The same tint fades a background.
    let mut filled = ring(Rect::new(8, 8, 48, 48), 0.0, vec![Pass::Background { color: RED }]);
    filled.tint = Color::new(1.0, 1.0, 1.0, 0.5);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![filled]) else {
        return;
    };
    let pixel = probe(&image, 32, 32);
    assert!(
        pixel[0].abs_diff(128) <= 2 && pixel[2].abs_diff(128) <= 2 && pixel[1] == 0,
        "half the background over blue, got {pixel:?}"
    );
}

#[test]
fn a_background_is_under_the_content_and_the_border_over_it() {
    let size = Size::new(64, 64);
    // An opaque green layer hides its red background, and its red border draws over the green.
    let mut layer = Solid::new(Rect::new(8, 8, 48, 48), GREEN);
    layer.effects = chain(vec![Pass::Background { color: RED }, stroke(2.0, 0.0, true, RED)]);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![layer]) else {
        return;
    };
    assert_eq!(
        probe(&image, 32, 32),
        [0, 255, 0, 255],
        "the content hides the background"
    );
    assert_eq!(probe(&image, 9, 32), [255, 0, 0, 255], "the border is over the content");
    // A transparent layer shows its background.
    let layer = ring(Rect::new(8, 8, 48, 48), 0.0, vec![Pass::Background { color: RED }]);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![layer]) else {
        return;
    };
    assert_eq!(
        probe(&image, 32, 32),
        [255, 0, 0, 255],
        "a clear layer shows its background"
    );
    assert_eq!(probe(&image, 4, 32), [0, 0, 255, 255], "and only inside the layer");
}

#[test]
fn a_background_is_cut_by_the_radius() {
    let size = Size::new(64, 64);
    let layer = ring(Rect::new(8, 8, 48, 48), 12.0, vec![Pass::Background { color: RED }]);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![layer]) else {
        return;
    };
    assert_eq!(probe(&image, 9, 9), [0, 0, 255, 255], "the corner is cut");
    assert_eq!(probe(&image, 32, 9), [255, 0, 0, 255], "the straight edge is whole");
}

#[test]
fn an_inset_shadow_is_darkest_at_the_edge_and_stays_inside_the_shape() {
    let size = Size::new(64, 64);
    let inset = Pass::Shadow(Shadow {
        offset: Vec3::ZERO,
        blur: 8.0,
        spread: 0.0,
        opacity: 1.0,
        color: Color::BLACK,
        frame: false,
        inset: true,
    });
    let layer = ring(Rect::new(8, 8, 48, 48), 10.0, vec![inset]);
    let Some(image) = drawn(size, Scale::IDENTITY, Color::WHITE, vec![layer]) else {
        return;
    };
    let mut previous = 0;
    for x in 8..16 {
        let brightness = luma(probe(&image, x, 32));
        assert!(
            brightness >= previous,
            "the shadow fades going inwards: x = {x} is {brightness}, the pixel nearer the edge was {previous}"
        );
        previous = brightness;
    }
    assert!(luma(probe(&image, 8, 32)) < 40, "the edge is nearly black");
    assert_eq!(probe(&image, 32, 32), [255; 4], "the middle is untouched");
    assert_eq!(probe(&image, 4, 32), [255; 4], "nothing leaks past the edge");
    assert_eq!(probe(&image, 8, 8), [255; 4], "nor past the rounded corner");
}

#[test]
fn an_inset_shadow_offset_moves_the_hole() {
    let size = Size::new(64, 64);
    let inset = Pass::Shadow(Shadow {
        offset: Vec3::new(6.0, 0.0, 0.0),
        blur: 4.0,
        spread: 0.0,
        opacity: 1.0,
        color: Color::BLACK,
        frame: false,
        inset: true,
    });
    let Some(image) = drawn(
        size,
        Scale::IDENTITY,
        Color::WHITE,
        vec![ring(Rect::new(8, 8, 48, 48), 0.0, vec![inset])],
    ) else {
        return;
    };
    assert!(
        luma(probe(&image, 10, 32)) < 5,
        "the left side is under the whole shadow: the hole moved away from it"
    );
    assert!(
        luma(probe(&image, 53, 32)) > 200,
        "the right side is inside the hole's blur and nearly clear"
    );
}

#[test]
fn a_rounded_shadow_is_darkest_at_the_edge_and_never_darker_further_out() {
    let size = Size::new(96, 96);
    let mut solid = Solid::new(Rect::new(32, 32, 32, 32), RED);
    solid.corner_radius = 8.0;
    solid.effects = chain(vec![drop_shadow(Vec3::ZERO, 10.0, 0.0, 1.0, false)]);
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
        [255; 4],
        "past the blur the backdrop is untouched"
    );
    assert!(
        luma(probe(&image, 30, 30)) > luma(probe(&image, 30, 48)),
        "a rounded corner's shadow is lighter than the edge's at the same distance"
    );
}

#[test]
fn a_shadow_darkens_next_to_the_edge_and_fades() {
    let size = Size::new(96, 96);
    let mut solid = Solid::new(Rect::new(24, 24, 48, 24), RED);
    solid.effects = chain(vec![drop_shadow(Vec3::new(0.0, 4.0, 0.0), 8.0, 0.0, 0.5, false)]);
    let Some(image) = drawn(size, Scale::IDENTITY, Color::WHITE, vec![solid]) else {
        return;
    };
    // The window's bottom edge is at y = 48 and the shadow is offset four pixels down, so the shadow's own edge is at
    // y = 52 and its falloff runs to about y = 60.
    let near = luma(probe(&image, 48, 54));
    let far = luma(probe(&image, 48, 59));
    let away = luma(probe(&image, 48, 80));
    assert!(
        near < away && far < away,
        "the shadow is darker than the background: near {near}, far {far}, away {away}"
    );
    assert!(near < far, "and it decays with distance: near {near}, far {far}");
    assert_eq!(
        probe(&image, 48, 40),
        [255, 0, 0, 255],
        "the window itself is untouched by its own shadow"
    );
}

#[test]
fn a_shadow_does_not_leave_the_damage() {
    let size = Size::new(64, 64);
    let Some(mut vulkan) = renderer(size) else { return };
    // Fill the output first, so a pixel outside the next frame's damage has a colour to keep.
    let green = colour(0, 255, 0, 255);
    let mut first = frame(green, size);
    solid(&mut first, Rect::new(0, 0, 64, 64), green);
    vulkan.draw(&first, Scale::IDENTITY).expect("the first frame");

    // A shadow that reaches well beyond the damage rectangle: only the damaged half may change.
    let mut second = OutputFrame::new();
    second.set_background(Color::WHITE);
    second.push_damage(Rect::new(0, 0, 32, 64));
    let mut layer = Solid::new(Rect::new(8, 8, 8, 8), RED);
    layer.effects = chain(vec![drop_shadow(Vec3::ZERO, 12.0, 0.0, 1.0, false)]);
    second.push_layer(Layer::Solid(layer));
    vulkan.draw(&second, Scale::IDENTITY).expect("the second frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 40, 12),
        [0, 255, 0, 255],
        "a pixel the frame did not damage kept its colour, so the shadow could not reach it"
    );
    assert_eq!(
        probe(&image, 30, 12),
        [255; 4],
        "the damaged half was repainted, background and all"
    );
    clean(&vulkan);
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
        solid.effects = chain(vec![drop_shadow(Vec3::ZERO, 8.0, 0.0, 1.0, follows_frame)]);
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
        [255; 4],
        "the content's shadow stops 12 pixels short of it"
    );
    assert!(
        luma(probe(&with_frame, 48, 12)) < luma(probe(&content_only, 48, 12)),
        "above the frame the shadow already reaches"
    );
    assert_eq!(
        probe(&with_frame, 48, 60),
        [255, 0, 0, 255],
        "the content is drawn over its shadow"
    );
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
        "the content's own top-left corner lies in the frame's straight left edge"
    );
    // A border on the content follows the frame it is a part of.
    let mut content = layer(Rect::new(8, 20, 48, 36), Color::TRANSPARENT, Some(frame));
    content.effects = chain(vec![stroke(2.0, 0.0, true, RED)]);
    let Some(image) = drawn(size, Scale::IDENTITY, BLUE, vec![content]) else {
        return;
    };
    assert_eq!(
        probe(&image, 8, 40),
        [255, 0, 0, 255],
        "the border runs along the frame's left edge"
    );
    assert_eq!(
        probe(&image, 9, 54),
        [0, 0, 255, 255],
        "and bends round the frame's corner"
    );
}

#[test]
fn a_border_scales_with_a_fractional_output_scale() {
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
    assert_eq!(
        probe(&rounded, 13, 13),
        [0, 0, 255, 255],
        "the corner is cut at the physical radius"
    );
    assert_eq!(probe(&rounded, 30, 13), [0, 255, 0, 255], "and the top edge is whole");
}

/// 256 straight colours, one pixel each, on a 16 by 16 grid.
fn colours() -> Vec<[u8; 3]> {
    (0..=255u32)
        .map(|i| [i as u8, (i * 7 % 256) as u8, (i * 13 % 256 + 40) as u8])
        .collect()
}

/// The grades the grade tests run: a vivid one, a dull one, and one with only a contrast.
fn grades() -> [Grade; 3] {
    [
        Grade {
            saturate: 1.7,
            brightness: 1.2,
            contrast: 1.4,
            noise: 0.0,
        },
        Grade {
            saturate: 0.3,
            brightness: 0.8,
            contrast: 0.6,
            noise: 0.0,
        },
        Grade {
            contrast: 2.0,
            ..Grade::IDENTITY
        },
    ]
}

fn filter_of(grade: Grade) -> Pass {
    Pass::Filter(FilterParams {
        blur: 0.0,
        saturate: grade.saturate,
        brightness: grade.brightness,
        contrast: grade.contrast,
    })
}

#[test]
fn the_shader_grade_matches_the_cpu_grade_within_one_level() {
    let Some(mut vulkan) = renderer(Size::new(16, 16)) else {
        return;
    };
    let picture = bitmap(16, 16, |x, y| {
        let [r, g, b] = colours()[(y * 16 + x) as usize];
        [r, g, b, 255]
    });
    for grade in grades() {
        let layer = Surface {
            source: SurfaceSource::Snapshot(std::sync::Arc::new(picture.clone())),
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0,
            },
            dst: Rect::new(0, 0, 16, 16),
            transform: Transform::Normal,
            opacity: 1.0,
            tint: Color::WHITE,
            corner_radius: 0.0,
            silhouette: None,
            effects: chain(vec![filter_of(grade)]),
            model: None,
            secure: false,
        };
        let mut surfaces = frame(Color::BLACK, Size::new(16, 16));
        surfaces.push_layer(Layer::Surface(layer));
        vulkan.draw(&surfaces, Scale::IDENTITY).expect("the frame");
        let image = vulkan.screenshot().expect("a readback");
        // The same colours as solid layers, graded on the CPU before they are drawn: the two routes agree.
        let mut solids = frame(Color::BLACK, Size::new(16, 16));
        for (index, [r, g, b]) in colours().into_iter().enumerate() {
            let mut cell = Solid::new(
                Rect::new((index % 16) as i32, (index / 16) as i32, 1, 1),
                Color::from_rgba8(r, g, b, 255),
            );
            cell.effects = chain(vec![filter_of(grade)]);
            solids.push_layer(Layer::Solid(cell));
        }
        vulkan.draw(&solids, Scale::IDENTITY).expect("the frame of solids");
        let graded_solids = vulkan.screenshot().expect("a readback");
        for (index, [r, g, b]) in colours().into_iter().enumerate() {
            let (x, y) = ((index % 16) as i32, (index / 16) as i32);
            let want = grade::apply(grade, Color::from_rgba8(r, g, b, 255));
            let want = [want.r, want.g, want.b].map(|channel| (channel * 255.0).round() as u8);
            for (what, image) in [("surface", &image), ("solid", &graded_solids)] {
                let got = probe(image, x, y);
                assert!(
                    got.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= 1),
                    "{what} ({r}, {g}, {b}) with {grade:?}: drew {got:?}, the CPU twin says {want:?}"
                );
            }
        }
    }
    clean(&vulkan);
}

#[test]
fn a_filter_grades_the_layer_and_not_what_is_behind_it() {
    let size = Size::new(32, 32);
    let mut layer = Solid::new(Rect::new(8, 8, 16, 16), colour(200, 100, 50, 255));
    layer.effects = chain(vec![filter_of(grades()[0])]);
    let Some(image) = drawn(size, Scale::IDENTITY, colour(200, 100, 50, 255), vec![layer]) else {
        return;
    };
    assert_eq!(
        probe(&image, 2, 2),
        [200, 100, 50, 255],
        "the backdrop keeps its colour"
    );
    assert_ne!(
        probe(&image, 16, 16),
        [200, 100, 50, 255],
        "the layer's colour is graded"
    );
}

#[test]
fn an_identity_filter_changes_nothing() {
    let size = Size::new(32, 32);
    let plain = Solid::new(Rect::new(8, 8, 16, 16), colour(200, 100, 50, 255));
    let mut filtered = plain.clone();
    filtered.effects = chain(vec![Pass::Filter(FilterParams::IDENTITY)]);
    let (Some(plain), Some(filtered)) = (
        drawn(size, Scale::IDENTITY, Color::BLACK, vec![plain]),
        drawn(size, Scale::IDENTITY, Color::BLACK, vec![filtered]),
    ) else {
        return;
    };
    assert_eq!(
        worst_difference(&plain, &filtered).0,
        0,
        "an identity filter draws like none"
    );
}

/// The `shadow_opacity` scene has a window with glass, which blurs and which this renderer does not draw yet. Its other
/// window is a plain one, and the left of the picture is that window and its shadow alone: the glass window's shadow
/// starts 14 pixels in. The plain window drawn by itself must match that part of the picture.
#[test]
fn an_outer_shadow_fades_with_its_layers_opacity() {
    let Some(mut vulkan) = renderer(golden::SIZE) else {
        return;
    };
    let plain = golden::Case {
        name: "shadow_opacity",
        layers: || {
            let case = golden::CASES
                .iter()
                .find(|case| case.name == "shadow_opacity")
                .expect("the shadow_opacity scene");
            (case.layers)().into_iter().take(1).collect()
        },
        software: false,
        tolerance: golden::SHAPE_TOLERANCE,
        probes: |_| {},
    };
    vulkan
        .draw(&golden::frame_of(&plain), Scale::IDENTITY)
        .expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    let want = golden::golden(&plain);
    for y in 0..golden::SIZE.height as i32 {
        for x in 0..14 {
            let got = probe(&image, x, y);
            let golden = want[(y as u32 * golden::SIZE.width + x as u32) as usize];
            assert!(
                got.iter()
                    .zip(golden)
                    .all(|(got, want)| got.abs_diff(want) <= golden::SHAPE_TOLERANCE),
                "({x}, {y}) is {got:?}, the golden of a window with the same shadow says {golden:?}"
            );
        }
    }
    clean(&vulkan);
}
