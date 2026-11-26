//! The backdrop blur, the filter blur and the images they are drawn through, on the Vulkan renderer.
//!
//! The shared goldens (`gnoblin_render_frame::golden`) pin whole pictures; these say what each picture means, and what
//! the renderer does around them. What can go wrong, and the test that catches it:
//!
//! - a blur that reads the wrong rows or columns (an axis swapped, the image upside down): the edge is blurred along
//!   both axes;
//! - a blur that reads what is past the output as black or clear, not as its edge: `a_blur_at_the_edge_repeats_the_edge`;
//! - a group that is not faded as one, or a glass that reaches outside its layer: the isolated-group tests;
//! - a tint, a grade or a grain applied to every sweep, or grain counted from the wrong corner: the grain test;
//! - a partial redraw that blurs something other than a full one does: `a_partial_redraw_matches_a_full_one`;
//! - a blurred layer nobody damaged that still costs images, and a steady frame that allocates:
//!   `a_blur_nobody_damaged_costs_nothing` and `a_steady_blur_makes_no_images`;
//! - a shader that reads a backdrop a blur is part of and gets the sharp one: `a_shader_reads_the_glass_below_it`.

use gnoblin_render::{BlurParams, FilterParams, Pass, Shadow, Vec3};
use gnoblin_render_frame::grade;

use super::*;

/// A blur of `radius` with `passes` sweeps at full resolution, no grade and no grain.
fn blur(radius: f32, passes: u32) -> BlurParams {
    BlurParams {
        radius,
        passes,
        downscale: 1.0,
        ..BlurParams::default()
    }
}

/// A layer with no colour of its own over `dst`, with `passes`.
fn glass(dst: Rect, passes: Vec<Pass>) -> Layer {
    let mut layer = Solid::new(dst, Color::TRANSPARENT);
    for pass in passes {
        layer.effects.push(pass).expect("a pass");
    }
    Layer::Solid(layer)
}

/// A black left half and a white right half of a 64 by 64 output, with `layers` over them.
fn edge(layers: Vec<Layer>) -> OutputFrame {
    let size = Size::new(64, 64);
    let mut frame = frame(Color::BLACK, size);
    solid(&mut frame, Rect::new(0, 0, 32, 64), Color::BLACK);
    solid(&mut frame, Rect::new(32, 0, 32, 64), Color::WHITE);
    for layer in layers {
        frame.push_layer(layer);
    }
    frame
}

/// Draws `frame` on a fresh renderer and reads it back. `None` where this machine has no Vulkan.
fn drawn(frame: &OutputFrame, size: Size) -> Option<Image> {
    let mut vulkan = renderer(size)?;
    vulkan.draw(frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    clean(&vulkan);
    Some(image)
}

fn pixel(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image.colour_at(x, y).expect("a pixel")
}

fn red(image: &Image, x: i32, y: i32) -> u32 {
    u32::from(pixel(image, x, y)[0])
}

#[test]
fn a_blur_blurs_a_sharp_edge_behind_the_layer() {
    let frame = edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(12.0, 1))])]);
    let Some(image) = drawn(&frame, Size::new(64, 64)) else {
        return;
    };
    let (middle, close, far, beyond) = (
        red(&image, 32, 32),
        red(&image, 28, 32),
        red(&image, 12, 32),
        red(&image, 52, 32),
    );
    assert!(
        (60..=195).contains(&middle),
        "the edge is a gradient at its middle: {middle}"
    );
    assert!(
        close > 4 && close < middle,
        "four pixels to the black side is a ramp: {close} next to {middle}"
    );
    assert!(far <= 4, "the far side is still black: {far}");
    assert!(beyond >= 251, "and the white side is still white: {beyond}");
}

#[test]
fn a_horizontal_edge_is_blurred_the_same_way() {
    // The same scene turned a quarter: black above, white below. A blur that swapped its axes or read the image upside
    // down would not leave the black on top.
    let size = Size::new(64, 64);
    let mut frame = frame(Color::BLACK, size);
    solid(&mut frame, Rect::new(0, 0, 64, 32), Color::BLACK);
    solid(&mut frame, Rect::new(0, 32, 64, 32), Color::WHITE);
    frame.push_layer(glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(12.0, 1))]));
    let Some(image) = drawn(&frame, size) else { return };
    let (middle, close, far_above, far_below) = (
        red(&image, 32, 32),
        red(&image, 32, 28),
        red(&image, 32, 12),
        red(&image, 32, 52),
    );
    assert!(
        (60..=195).contains(&middle),
        "the edge is a gradient at its middle: {middle}"
    );
    assert!(
        close > 4 && close < middle,
        "above the edge is a ramp: {close} next to {middle}"
    );
    assert!(far_above <= 4, "the top is still black: {far_above}");
    assert!(far_below >= 251, "and the bottom still white: {far_below}");
}

#[test]
fn more_blur_passes_spread_the_edge_further() {
    let one = edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(12.0, 1))])]);
    let four = edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(12.0, 4))])]);
    let (Some(one), Some(four)) = (drawn(&one, Size::new(64, 64)), drawn(&four, Size::new(64, 64))) else {
        return;
    };
    let (near_one, near_four) = (red(&one, 24, 32), red(&four, 24, 32));
    assert!(
        near_four > near_one + 4,
        "four passes reach further: {near_four} against {near_one}"
    );
}

#[test]
fn a_blur_tint_is_mixed_over_the_blurred_backdrop() {
    let tinted = BlurParams {
        tint: colour(255, 0, 0, 128),
        ..blur(12.0, 1)
    };
    let frame = edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(tinted)])]);
    let Some(image) = drawn(&frame, Size::new(64, 64)) else {
        return;
    };
    let [r, g, b, _] = pixel(&image, 52, 32);
    assert!(r >= 240, "the red tint keeps the red of the white side: {r}");
    assert!((110..=145).contains(&g), "half of the green is mixed away: {g}");
    assert!(
        g.abs_diff(b) <= 4,
        "green and blue take the same share: {g} against {b}"
    );
}

#[test]
fn a_blur_at_the_edge_repeats_the_edge() {
    // A red background under a white rectangle: the blur reads past the output's left edge, where the only colour defined
    // is the edge's own.
    let size = Size::new(64, 64);
    let mut frame = frame(colour(255, 0, 0, 255), size);
    solid(&mut frame, Rect::new(0, 0, 32, 64), Color::WHITE);
    frame.push_layer(glass(Rect::new(0, 0, 16, 64), vec![Pass::Blur(blur(10.0, 1))]));
    let Some(image) = drawn(&frame, size) else { return };
    let [r, g, b, a] = pixel(&image, 0, 32);
    assert_eq!(a, 255);
    assert!(
        g >= 90 && b >= 90 && r == 255,
        "the edge pixel repeats the white beside it: {r} {g} {b}"
    );
}

/// The blur of `backdrop` as `blur.wgsl` computes it: nine taps `step` pixels apart with a Gaussian of sigma 1.5 taps,
/// normalised. The backdrop is constant down a column, so the vertical sweep leaves it as it is.
fn reference_blur(backdrop: fn(i32) -> f32, x: i32, step: i32) -> f32 {
    let mut total = 0.0;
    let mut weights = 0.0;
    for index in -4..=4 {
        let weight = (-0.5 * (index * index) as f32 / 2.25).exp();
        total += weight * backdrop(x + index * step);
        weights += weight;
    }
    total / weights
}

fn edge_backdrop(x: i32) -> f32 {
    if x >= 32 { 1.0 } else { 0.0 }
}

/// A straight (200, 100, 50) layer at `alpha`, covering `quad`, with a blur of radius 9 behind it, drawn at group `opacity`.
fn glass_over_edge(quad: Rect, opacity: f32, alpha: u8) -> Option<Image> {
    let premultiplied = |channel: u8| (f32::from(channel) * f32::from(alpha) / 255.0).round() as u8;
    let picture = bitmap(quad.width, quad.height, |_, _| {
        [premultiplied(200), premultiplied(100), premultiplied(50), alpha]
    });
    let layer = Layer::Surface(Surface {
        source: SurfaceSource::Snapshot(std::sync::Arc::new(picture)),
        src: RectF {
            x: 0.0,
            y: 0.0,
            width: f64::from(quad.width),
            height: f64::from(quad.height),
        },
        dst: quad,
        transform: Transform::Normal,
        opacity,
        tint: Color::WHITE,
        corner_radius: 0.0,
        silhouette: None,
        effects: {
            let mut effects = Effects::new();
            effects.push(Pass::Blur(blur(9.0, 1))).expect("a blur");
            effects
        },
        model: None,
        secure: false,
        content: None,
    });
    let size = Size::new(64, 64);
    let mut frame = frame(Color::BLACK, size);
    solid(&mut frame, Rect::new(32, 0, 32, 64), Color::WHITE);
    frame.push_layer(layer);
    drawn(&frame, size)
}

/// The frame's colour at (x, 32) under isolated-group compositing: `out = (1 - g) * D + g * (P + (1 - a) * B)`, with `P`
/// premultiplied, in levels.
fn group_expected(opacity: f32, alpha: u8, x: i32) -> [f32; 3] {
    let a = f32::from(alpha) / 255.0;
    let backdrop = edge_backdrop(x);
    let blurred = reference_blur(edge_backdrop, x, 3);
    [200.0, 100.0, 50.0].map(|channel| {
        let p = channel / 255.0 * a;
        ((1.0 - opacity) * backdrop + opacity * (p + (1.0 - a) * blurred)) * 255.0
    })
}

#[test]
fn a_blurred_node_composites_as_one_isolated_group() {
    let quad = Rect::new(16, 0, 32, 64);
    // Translucent glass fully blurred behind; opaque glass at half opacity; a transparent cut-out at half opacity, which is
    // the blur alone; and a layer at zero opacity, which is the backdrop alone.
    for (opacity, alpha) in [(1.0, 77), (0.5, 255), (0.5, 0), (0.0, 77)] {
        let Some(image) = glass_over_edge(quad, opacity, alpha) else {
            return;
        };
        for x in [24, 40] {
            let want = group_expected(opacity, alpha, x);
            let got = pixel(&image, x, 32);
            for channel in 0..3 {
                assert!(
                    (f32::from(got[channel]) - want[channel]).abs() <= 2.0,
                    "g {opacity}, a {alpha}, x {x}, channel {channel}: got {}, want {:.1}",
                    got[channel],
                    want[channel]
                );
            }
        }
    }
}

#[test]
fn a_blur_reaches_nothing_outside_its_node() {
    // Just left of the edge the backdrop is black but its blur is not: the taps reach the white side. Outside the node it
    // must stay the backdrop's black.
    assert!(reference_blur(edge_backdrop, 28, 3) * 255.0 > 15.0);
    let Some(image) = glass_over_edge(Rect::new(40, 0, 8, 64), 1.0, 255) else {
        return;
    };
    assert!(red(&image, 28, 32) <= 2, "outside the node the backdrop is untouched");
}

#[test]
fn a_blurred_node_fades_monotonically_with_group_opacity() {
    let mut reds = Vec::new();
    for opacity in [1.0, 0.75, 0.5, 0.25, 0.0] {
        let Some(image) = glass_over_edge(Rect::new(16, 0, 32, 64), opacity, 77) else {
            return;
        };
        reds.push(red(&image, 40, 32));
    }
    assert!(
        reds.windows(2).all(|pair| pair[1] > pair[0]),
        "the white side rises as the group fades to the backdrop: {reds:?}"
    );
}

#[test]
fn a_downscaled_blur_agrees_with_the_full_one_on_where_things_are() {
    let blurred = |downscale: f32| {
        let params = BlurParams {
            downscale,
            ..blur(12.0, 2)
        };
        drawn(
            &edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(params)])]),
            Size::new(64, 64),
        )
    };
    let (Some(full), Some(half), Some(quarter)) = (blurred(1.0), blurred(0.5), blurred(0.25)) else {
        return;
    };
    for (what, image) in [("half", &half), ("a quarter", &quarter)] {
        let (middle, far_left, far_right) = (red(image, 32, 32), red(image, 4, 32), red(image, 60, 32));
        assert!(
            middle.abs_diff(red(&full, 32, 32)) <= 20,
            "{what}: the middle of the ramp is where it is at full resolution: {middle}"
        );
        assert!(
            far_left <= 4 && far_right >= 251,
            "{what}: the sides are what they were"
        );
        let ramp: Vec<u32> = (20..45).map(|x| red(image, x, 32)).collect();
        assert!(
            ramp.windows(2).all(|pair| pair[1] >= pair[0]),
            "{what}: the edge is a ramp: {ramp:?}"
        );
    }
}

#[test]
fn the_grain_is_counted_from_the_layers_corner() {
    // A flat mid grey, so every change is the grain. The layer's corner is at (10, 12), and grain at a pixel is the CPU
    // twin's at its offset from that corner.
    let size = Size::new(40, 40);
    let mut frame = frame(colour(128, 128, 128, 255), size);
    let noisy = BlurParams {
        noise: 1.0,
        ..blur(4.0, 1)
    };
    frame.push_layer(glass(Rect::new(10, 12, 20, 16), vec![Pass::Blur(noisy)]));
    let Some(image) = drawn(&frame, size) else { return };
    for (x, y) in [(0, 0), (3, 5), (11, 7), (19, 15)] {
        let want = (128.0 / 255.0 + grade::grain(1.0, x, y)).clamp(0.0, 1.0) * 255.0;
        let got = f32::from(pixel(&image, 10 + x as i32, 12 + y as i32)[0]);
        assert!(
            (got - want).abs() <= 2.0,
            "grain at ({x}, {y}) from the corner: got {got}, the twin says {want:.1}"
        );
    }
    assert_eq!(pixel(&image, 5, 5)[0], 128, "outside the layer there is none");
}

#[test]
fn blurs_on_one_layer_compound() {
    let one = edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(8.0, 1))])]);
    let two = edge(vec![glass(
        Rect::new(0, 0, 64, 64),
        vec![Pass::Blur(blur(8.0, 1)), Pass::Blur(blur(8.0, 1))],
    )]);
    let (Some(one), Some(two)) = (drawn(&one, Size::new(64, 64)), drawn(&two, Size::new(64, 64))) else {
        return;
    };
    assert!(
        red(&two, 24, 32) > red(&one, 24, 32),
        "the second blur spreads what the first left"
    );
}

#[test]
fn a_glass_over_a_glass_blurs_what_the_one_below_shows() {
    let below = glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(8.0, 1))]);
    let above = glass(Rect::new(8, 8, 48, 48), vec![Pass::Blur(blur(8.0, 1))]);
    let (Some(alone), Some(both)) = (
        drawn(&edge(vec![above.clone()]), Size::new(64, 64)),
        drawn(&edge(vec![below, above]), Size::new(64, 64)),
    ) else {
        return;
    };
    assert!(
        red(&both, 24, 32) > red(&alone, 24, 32),
        "the glass above blurs the blurred edge, which is wider than the sharp one"
    );
}

#[test]
fn a_filter_blur_spills_past_the_layers_edge_and_keeps_its_middle() {
    let size = Size::new(64, 40);
    let mut frame = frame(colour(240, 230, 210, 255), size);
    let mut layer = Solid::new(Rect::new(20, 10, 24, 20), colour(230, 120, 30, 255));
    layer
        .effects
        .push(Pass::Filter(FilterParams {
            blur: 8.0,
            ..FilterParams::default()
        }))
        .expect("a filter");
    frame.push_layer(Layer::Solid(layer));
    let Some(image) = drawn(&frame, size) else { return };
    assert!(
        pixel(&image, 32, 20)[1].abs_diff(120) <= 3,
        "the middle keeps its colour: {:?}",
        pixel(&image, 32, 20)
    );
    assert!(
        pixel(&image, 17, 20)[1] < 230,
        "the blur spills past the left edge: {:?}",
        pixel(&image, 17, 20)
    );
    assert!(
        pixel(&image, 46, 20)[1] < 230,
        "and past the right: {:?}",
        pixel(&image, 46, 20)
    );
    assert!(
        pixel(&image, 20, 20)[1] > 120,
        "the edge itself is softened to the backdrop"
    );
    assert_eq!(
        pixel(&image, 2, 2),
        [240, 230, 210, 255],
        "far from the layer nothing is blurred"
    );
}

#[test]
fn a_filter_blur_keeps_what_hangs_off_the_output_where_it_was() {
    let layer = |x: i32| {
        let mut solid = Solid::new(Rect::new(x, 6, 16, 10), colour(230, 120, 30, 255));
        solid
            .effects
            .push(Pass::Filter(FilterParams {
                blur: 4.0,
                ..FilterParams::default()
            }))
            .expect("a filter");
        Layer::Solid(solid)
    };
    let scene = |size: Size, x: i32| {
        let mut frame = frame(colour(240, 230, 210, 255), size);
        solid(
            &mut frame,
            Rect::new(0, 0, size.width, size.height),
            colour(240, 230, 210, 255),
        );
        frame.push_layer(layer(x));
        drawn(&frame, size)
    };
    let (Some(whole), Some(cut)) = (scene(Size::new(48, 24), 10), scene(Size::new(34, 24), -4)) else {
        return;
    };
    for y in 0..24 {
        for x in 0..34 {
            let (a, b) = (pixel(&cut, x, y), pixel(&whole, x + 14, y));
            assert!(
                a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2),
                "a layer cut by the output's edge, at {x},{y}: {a:?} against {b:?}"
            );
        }
    }
    assert!(pixel(&cut, 0, 10)[1] < 200, "the cut layer is there");
}

#[test]
fn a_filter_blur_fades_with_the_layer() {
    let size = Size::new(40, 24);
    let draw = |opacity: f32| {
        let picture = bitmap(16, 10, |_, _| [230, 120, 30, 255]);
        let mut surface = Surface {
            source: SurfaceSource::Snapshot(std::sync::Arc::new(picture)),
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 10.0,
            },
            dst: Rect::new(10, 6, 16, 10),
            transform: Transform::Normal,
            opacity,
            tint: Color::WHITE,
            corner_radius: 0.0,
            silhouette: None,
            effects: Effects::new(),
            model: None,
            secure: false,
            content: None,
        };
        surface
            .effects
            .push(Pass::Filter(FilterParams {
                blur: 4.0,
                ..FilterParams::default()
            }))
            .expect("a filter");
        let mut frame = frame(Color::BLACK, size);
        frame.push_layer(Layer::Surface(surface));
        drawn(&frame, size)
    };
    let (Some(full), Some(half)) = (draw(1.0), draw(0.5)) else {
        return;
    };
    for (x, y) in [(18, 11), (8, 11), (28, 11)] {
        let (a, b) = (pixel(&full, x, y), pixel(&half, x, y));
        for channel in 0..3 {
            assert!(
                (f32::from(b[channel]) * 2.0 - f32::from(a[channel])).abs() <= 3.0,
                "({x}, {y}) at half opacity is half of the opaque one: {b:?} against {a:?}"
            );
        }
    }
}

#[test]
fn glass_and_a_filter_blur_on_one_layer_draw_both() {
    let size = Size::new(64, 64);
    let mut frame = frame(Color::BLACK, size);
    solid(&mut frame, Rect::new(0, 0, 32, 64), Color::BLACK);
    solid(&mut frame, Rect::new(32, 0, 32, 64), Color::WHITE);
    let mut layer = Solid::new(Rect::new(8, 8, 48, 48), colour(255, 0, 0, 255));
    layer.effects.push(Pass::Blur(blur(12.0, 1))).expect("a blur");
    layer
        .effects
        .push(Pass::Filter(FilterParams {
            blur: 4.0,
            ..FilterParams::default()
        }))
        .expect("a filter");
    frame.push_layer(Layer::Solid(layer));
    let Some(image) = drawn(&frame, size) else { return };
    // The red layer is opaque, so the glass behind it is hidden; the filter's blur spills red past the layer's edge over
    // what is behind, which the glass did not reach.
    assert!(
        pixel(&image, 32, 32)[0] >= 250,
        "the layer's own colour is in the middle"
    );
    assert!(
        pixel(&image, 6, 32)[0] > 0,
        "the picture spills outside the layer: {:?}",
        pixel(&image, 6, 32)
    );
}

#[test]
fn a_partial_redraw_matches_a_full_one() {
    // Two frames: the backdrop changes under a blurred layer. A renderer that redraws the damage and blurs what the last
    // frame left in the target would seam the picture; this one blurs what the layers below draw, so the damage is only
    // where pixels are written.
    let size = Size::new(64, 64);
    let scene = |left: Color| {
        let mut frame = frame(Color::BLACK, size);
        solid(&mut frame, Rect::new(0, 0, 32, 64), left);
        solid(&mut frame, Rect::new(32, 0, 32, 64), Color::WHITE);
        frame.push_layer(glass(Rect::new(8, 8, 48, 48), vec![Pass::Blur(blur(10.0, 2))]));
        frame
    };
    let Some(mut partial) = renderer(size) else { return };
    let Some(mut whole) = renderer(size) else { return };
    partial
        .draw(&scene(Color::BLACK), Scale::IDENTITY)
        .expect("the first frame");
    let mut second = OutputFrame::new();
    second.set_background(Color::BLACK);
    // Only the changed half is damaged; the frame grows the damage to the whole blurred layer, as the composer does.
    second.push_damage(Rect::new(0, 0, 32, 64));
    solid(&mut second, Rect::new(0, 0, 32, 64), colour(255, 0, 0, 255));
    solid(&mut second, Rect::new(32, 0, 32, 64), Color::WHITE);
    second.push_layer(glass(Rect::new(8, 8, 48, 48), vec![Pass::Blur(blur(10.0, 2))]));
    second.expand_damage_for_blur();
    assert!(
        second
            .damage()
            .iter()
            .any(|rect| Rect::new(8, 8, 48, 48).is_inside(*rect)),
        "the blurred layer is damaged whole"
    );
    partial.draw(&second, Scale::IDENTITY).expect("the second frame");
    whole
        .draw(&scene(colour(255, 0, 0, 255)), Scale::IDENTITY)
        .expect("a full frame");
    let (a, b) = (
        partial.screenshot().expect("a readback"),
        whole.screenshot().expect("a readback"),
    );
    let (worst, at) = worst_difference(&a, &b);
    assert!(
        worst <= 1,
        "a partial redraw differs from a full one by {worst} at {at:?}"
    );
    clean(&partial);
    clean(&whole);
}

#[test]
fn a_blur_nobody_damaged_costs_nothing() {
    let size = Size::new(64, 64);
    let Some(mut vulkan) = renderer(size) else { return };
    let mut first = edge(vec![glass(Rect::new(40, 40, 20, 20), vec![Pass::Blur(blur(8.0, 2))])]);
    vulkan.draw(&first, Scale::IDENTITY).expect("the first frame");
    let held = vulkan.blur_images.len();
    assert!(held > 0, "the first frame made images for its blur");
    // The next frame damages a corner nowhere near the blurred layer, or what it reads.
    first = OutputFrame::new();
    first.set_background(Color::BLACK);
    first.push_damage(Rect::new(0, 0, 8, 8));
    solid(&mut first, Rect::new(0, 0, 32, 64), Color::BLACK);
    solid(&mut first, Rect::new(32, 0, 32, 64), Color::WHITE);
    first.push_layer(glass(Rect::new(40, 40, 20, 20), vec![Pass::Blur(blur(8.0, 2))]));
    let before = vulkan.blur_images.made();
    vulkan.draw(&first, Scale::IDENTITY).expect("the second frame");
    assert_eq!(
        vulkan.blur_images.made(),
        before,
        "no image was made for a blur nobody reads"
    );
    clean(&vulkan);
}

#[test]
fn a_steady_blur_makes_no_images() {
    let size = Size::new(64, 64);
    let Some(mut vulkan) = renderer(size) else { return };
    let frame = edge(vec![glass(Rect::new(8, 8, 48, 48), vec![Pass::Blur(blur(10.0, 3))])]);
    vulkan.draw(&frame, Scale::IDENTITY).expect("the first frame");
    let made = vulkan.blur_images.made();
    assert!(made > 0, "the first frame made images");
    for _ in 0..3 {
        vulkan.draw(&frame, Scale::IDENTITY).expect("a steady frame");
    }
    assert_eq!(
        vulkan.blur_images.made(),
        made,
        "a steady frame asks the pool for the images it already has"
    );
    assert!(vulkan.footprint().scratch > 0, "the pool's images are counted");
    assert_eq!(
        vulkan.footprint().releasable,
        vulkan.footprint().scratch,
        "and can be given back"
    );
    clean(&vulkan);
}

#[test]
fn a_shadow_and_a_border_around_glass_draw_outside_its_group() {
    let size = Size::new(64, 64);
    let mut frame = frame(colour(240, 230, 210, 255), size);
    let mut layer = Solid::new(Rect::new(16, 16, 24, 24), Color::TRANSPARENT);
    layer
        .effects
        .push(Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 6.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: false,
        }))
        .expect("a shadow");
    layer.effects.push(Pass::Blur(blur(6.0, 1))).expect("a blur");
    layer
        .effects
        .push(Pass::Border(gnoblin_render::Border {
            width: 2.0,
            offset: 0.0,
            inset: false,
            color: colour(255, 0, 0, 255),
        }))
        .expect("a border");
    frame.push_layer(Layer::Solid(layer));
    let Some(image) = drawn(&frame, size) else { return };
    assert!(
        pixel(&image, 14, 28)[0] > 200 && pixel(&image, 14, 28)[1] < 50,
        "the border runs along the glass"
    );
    assert!(
        luma(pixel(&image, 12, 28)) < 220,
        "the shadow is behind the glass, round it"
    );
    assert_eq!(
        pixel(&image, 28, 28),
        [240, 230, 210, 255],
        "the glass shows its flat backdrop"
    );
}

fn luma(pixel: [u8; 4]) -> u32 {
    (u32::from(pixel[0]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[2]) * 114) / 1000
}

#[test]
fn a_blur_paints_only_the_damage() {
    let size = Size::new(64, 64);
    let Some(mut vulkan) = renderer(size) else { return };
    // Fill the output first, so a pixel outside the next frame's damage has a colour to keep.
    let mut first = frame(colour(0, 255, 0, 255), size);
    solid(&mut first, Rect::new(0, 0, 64, 64), colour(0, 255, 0, 255));
    vulkan.draw(&first, Scale::IDENTITY).expect("the first frame");
    let mut second = OutputFrame::new();
    second.set_background(Color::BLACK);
    second.push_damage(Rect::new(0, 0, 32, 64));
    solid(&mut second, Rect::new(0, 0, 32, 64), Color::BLACK);
    solid(&mut second, Rect::new(32, 0, 32, 64), Color::WHITE);
    second.push_layer(glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(10.0, 1))]));
    vulkan.draw(&second, Scale::IDENTITY).expect("the second frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        pixel(&image, 50, 32),
        [0, 255, 0, 255],
        "an undamaged pixel kept its colour"
    );
    assert!(
        red(&image, 31, 32) > 20,
        "the damaged half is blurred towards the white edge"
    );
    clean(&vulkan);
}

#[test]
fn a_filter_blur_paints_only_the_damage() {
    let size = Size::new(40, 24);
    let Some(mut vulkan) = renderer(size) else { return };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 18, 24));
    let mut layer = Solid::new(Rect::new(10, 6, 16, 10), colour(230, 120, 30, 255));
    layer
        .effects
        .push(Pass::Filter(FilterParams {
            blur: 4.0,
            ..FilterParams::default()
        }))
        .expect("a filter");
    frame.push_layer(Layer::Solid(layer));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_ne!(pixel(&image, 12, 10), [0, 0, 0, 0], "the damaged part is drawn");
    assert_eq!(
        pixel(&image, 24, 10),
        [0, 0, 0, 0],
        "and the undamaged part keeps what it had"
    );
    clean(&vulkan);
}

#[test]
fn a_blur_radius_is_logical_and_scales_with_the_output() {
    // The same scene at scale 2 on a 64 pixel output is a 32 pixel scene with a radius of 6: the ramp is the radius 12 one.
    let size = Size::new(64, 64);
    let mut logical = OutputFrame::new();
    logical.set_background(Color::BLACK);
    logical.push_damage(Rect::new(0, 0, 32, 32));
    solid(&mut logical, Rect::new(0, 0, 16, 32), Color::BLACK);
    solid(&mut logical, Rect::new(16, 0, 16, 32), Color::WHITE);
    logical.push_layer(glass(Rect::new(0, 0, 32, 32), vec![Pass::Blur(blur(6.0, 1))]));
    let Some(mut vulkan) = renderer(size) else { return };
    vulkan.draw(&logical, Scale::new(2, 1)).expect("the frame");
    let scaled = vulkan.screenshot().expect("a readback");
    clean(&vulkan);
    let physical = edge(vec![glass(Rect::new(0, 0, 64, 64), vec![Pass::Blur(blur(12.0, 1))])]);
    let Some(plain) = drawn(&physical, size) else { return };
    for x in [24, 28, 32, 36, 40] {
        assert!(
            red(&scaled, x, 32).abs_diff(red(&plain, x, 32)) <= 3,
            "column {x}: {} against {}",
            red(&scaled, x, 32),
            red(&plain, x, 32)
        );
    }
}
