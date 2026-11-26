//! Tests of the shell's own shader pass on the Vulkan renderer.
//!
//! The cases are the GLES renderer's: a pass recolours its layer, reads its params and time, keeps the layer's tint and
//! opacity, chains in order, reads the backdrop with the origin at the top-left, and reads a secure layer as black. They
//! need a Vulkan device like every drawing test here, and print why and return on a machine that has none. Results are
//! compared to what the pass computes, with a tolerance of [`BLEND`] where a float becomes an 8-bit channel: no two
//! renderers are bit-identical.

use std::time::Duration;

use gnoblin_render::{Pass, ShaderKey, ShaderPass};
use gnoblin_render_frame::shader::user;

use super::*;

/// A shell's source with `body` as the body of `effect`.
fn source(body: &str) -> String {
    format!("fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {{\n{body}\n}}\n")
}

/// A shader pass of `body` with `params`, flagged as the compositor flags it after validating.
fn pass(body: &str, params: &[f32]) -> Pass {
    let source = source(body);
    let validated = user::validate(&source).expect("a valid shader");
    Pass::Shader(ShaderPass::new(&source, params, validated.backdrop()))
}

/// A solid layer carrying `passes`.
fn shaded(dst: Rect, colour: Color, passes: &[Pass]) -> Layer {
    let mut solid = Solid::new(dst, colour);
    for pass in passes {
        solid.effects.push(pass.clone()).expect("a pass");
    }
    Layer::Solid(solid)
}

/// `layer` with `passes` added to its effects.
fn with_passes(mut layer: Layer, passes: &[Pass]) -> Layer {
    if let Layer::Surface(surface) = &mut layer {
        for pass in passes {
            surface.effects.push(pass.clone()).expect("a pass");
        }
    }
    layer
}

/// Draws `frame` until the worker has answered for every source in it, then reads the target back. The draws before the last
/// are the ones that find a pass still compiling, and their reports are dropped.
fn settle(vulkan: &mut Vulkan, frame: &OutputFrame, scale: Scale) {
    for _ in 0..4000 {
        vulkan.draw(frame, scale).expect("the frame");
        if vulkan.device.shaders.borrow().pending() == 0 {
            return;
        }
        frame.shader_timings().take();
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the worker did not answer in 20 s");
}

/// Draws `frame`, after its shaders are built, and reads the target back.
fn draw(vulkan: &mut Vulkan, frame: &OutputFrame) -> Image {
    settle(vulkan, frame, Scale::IDENTITY);
    vulkan.screenshot().expect("a readback")
}

/// Asserts a pixel is `expected` to within `tolerance` per channel.
fn assert_pixel(image: &Image, at: (i32, i32), expected: [u8; 4], tolerance: u8) {
    let actual = image.colour_at(at.0, at.1).expect("a pixel");
    let worst = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= tolerance,
        "pixel {at:?} is {actual:?}, expected {expected:?} within {tolerance}"
    );
}

const SWAP: &str = "return vec4<f32>(c.g, c.r, c.b, c.a);";
const BACKDROP: &str = "return backdrop(uv);";

#[test]
fn a_shader_pass_recolours_its_layer() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(colour(10, 20, 30, 255), SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[pass(SWAP, &[])],
    ));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (12, 12), [0, 255, 0, 255], BLEND);
    assert_pixel(&image, (2, 2), [10, 20, 30, 255], 0);
    clean(&vulkan);
}

#[test]
fn a_shader_pass_reads_its_params() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        Color::WHITE,
        &[pass(
            "return vec4<f32>(p[0], p[1], p[7], 1.0);",
            &[1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.25],
        )],
    ));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (12, 12), [255, 128, 64, 255], BLEND);
    clean(&vulkan);
}

#[test]
fn a_shader_pass_reads_a_time_that_advances() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        Color::WHITE,
        &[pass("return vec4<f32>(clamp(t, 0.0, 1.0), 0.0, 0.0, 1.0);", &[])],
    ));
    let first = draw(&mut vulkan, &frame).colour_at(12, 12).expect("a pixel")[0];
    std::thread::sleep(Duration::from_millis(200));
    let second = draw(&mut vulkan, &frame).colour_at(12, 12).expect("a pixel")[0];
    println!("t as a red channel: {first}, then {second} after 200 ms");
    assert!(
        second >= first.saturating_add(40),
        "the time went from {first} to {second}"
    );
    clean(&vulkan);
}

#[test]
fn an_identity_shader_keeps_the_layers_tint_opacity_and_corners() {
    let client = ShmClient::new(vec![BufferSpec::argb(16, 16, pattern)]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let build = |passes: &[Pass]| {
        let mut frame = frame(colour(20, 40, 60, 255), SIZE);
        let mut layer = surface(
            &client,
            0,
            RectF::new(0.0, 0.0, 16.0, 16.0),
            Rect::new(8, 8, 16, 16),
            0.75,
            Color::new(0.5, 1.0, 0.25, 1.0),
        );
        if let Layer::Surface(surface) = &mut layer {
            surface.corner_radius = 5.0;
        }
        frame.push_layer(with_passes(layer, passes));
        frame
    };
    let plain = draw(&mut vulkan, &build(&[]));
    let shaded = draw(&mut vulkan, &build(&[pass("return c;", &[])]));
    assert_close("an identity shader against none", &plain, &shaded, BLEND);
    clean(&vulkan);
}

#[test]
fn a_shader_on_a_translucent_solid_reads_its_premultiplied_colour() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let build = |passes: &[Pass]| {
        let mut frame = frame(Color::BLACK, SIZE);
        frame.push_layer(shaded(Rect::new(8, 8, 16, 16), colour(255, 0, 0, 128), passes));
        frame
    };
    let plain = draw(&mut vulkan, &build(&[]));
    let shaded = draw(&mut vulkan, &build(&[pass("return c;", &[])]));
    assert_close("a translucent solid through an identity shader", &plain, &shaded, BLEND);
    assert_pixel(&shaded, (12, 12), [128, 0, 0, 255], BLEND);
    clean(&vulkan);
}

#[test]
fn passes_apply_in_list_order() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let red_only = pass("return vec4<f32>(c.r, 0.0, 0.0, c.a);", &[]);
    let swap = pass(SWAP, &[]);
    let draw_with = |vulkan: &mut Vulkan, passes: &[Pass]| {
        let mut frame = frame(Color::BLACK, SIZE);
        frame.push_layer(shaded(Rect::new(8, 8, 16, 16), colour(255, 0, 0, 255), passes));
        draw(vulkan, &frame)
    };
    let swapped_then_red = draw_with(&mut vulkan, &[swap.clone(), red_only.clone()]);
    assert_pixel(&swapped_then_red, (12, 12), [0, 0, 0, 255], BLEND);
    let red_then_swapped = draw_with(&mut vulkan, &[red_only, swap.clone()]);
    assert_pixel(&red_then_swapped, (12, 12), [0, 255, 0, 255], BLEND);
    // Three and four passes need both scratch images and the first one again.
    let three = draw_with(&mut vulkan, &[swap.clone(), swap.clone(), swap.clone()]);
    assert_pixel(&three, (12, 12), [0, 255, 0, 255], BLEND);
    let four = draw_with(&mut vulkan, &[swap.clone(), swap.clone(), swap.clone(), swap]);
    assert_pixel(&four, (12, 12), [255, 0, 0, 255], BLEND);
    clean(&vulkan);
}

#[test]
fn only_the_last_pass_applies_the_layers_tint() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    let mut solid = Solid::new(Rect::new(8, 8, 16, 16), colour(255, 255, 255, 255));
    solid.tint = Color::new(0.5, 0.5, 0.5, 1.0);
    for pass in [pass("return c;", &[]), pass("return c;", &[])] {
        solid.effects.push(pass).expect("a pass");
    }
    frame.push_layer(Layer::Solid(solid));
    let image = draw(&mut vulkan, &frame);
    // A tint applied by each of two passes would give a quarter.
    assert_pixel(&image, (12, 12), [128, 128, 128, 255], BLEND);
    clean(&vulkan);
}

/// Red above the middle of the output, blue below it.
fn red_over_blue(frame: &mut OutputFrame) {
    solid(frame, Rect::new(0, 0, 64, 24), colour(255, 0, 0, 255));
    solid(frame, Rect::new(0, 24, 64, 24), colour(0, 0, 255, 255));
}

#[test]
fn a_shader_pass_reads_the_backdrop_with_the_origin_at_the_top_left() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let region = Rect::new(16, 8, 32, 32);
    let mut direct = frame(Color::BLACK, SIZE);
    red_over_blue(&mut direct);
    direct.push_layer(shaded(region, Color::WHITE, &[pass(BACKDROP, &[])]));
    let image = draw(&mut vulkan, &direct);
    assert_pixel(&image, (20, 12), [255, 0, 0, 255], BLEND);
    assert_pixel(&image, (20, 36), [0, 0, 255, 255], BLEND);
    assert_pixel(&image, (4, 4), [255, 0, 0, 255], 0);

    let mut flipped = frame(Color::BLACK, SIZE);
    red_over_blue(&mut flipped);
    flipped.push_layer(shaded(
        region,
        Color::WHITE,
        &[pass("return backdrop(vec2<f32>(uv.x, 1.0 - uv.y));", &[])],
    ));
    let image = draw(&mut vulkan, &flipped);
    assert_pixel(&image, (20, 12), [0, 0, 255, 255], BLEND);
    assert_pixel(&image, (20, 36), [255, 0, 0, 255], BLEND);
    clean(&vulkan);
}

#[test]
fn a_backdrop_reads_the_layers_below_as_they_are_drawn() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    solid(&mut frame, Rect::new(0, 0, 64, 48), colour(255, 0, 0, 255));
    // A layer that swaps red to green, and above it a layer that shows what is behind it.
    frame.push_layer(shaded(
        Rect::new(8, 8, 32, 32),
        colour(255, 0, 0, 255),
        &[pass(SWAP, &[])],
    ));
    frame.push_layer(shaded(Rect::new(16, 16, 16, 16), Color::WHITE, &[pass(BACKDROP, &[])]));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (20, 20), [0, 255, 0, 255], BLEND);
    assert_pixel(&image, (10, 10), [0, 255, 0, 255], BLEND);
    assert_pixel(&image, (2, 2), [255, 0, 0, 255], 0);
    clean(&vulkan);
}

#[test]
fn a_secure_layer_reads_as_black_in_a_backdrop_and_is_drawn_in_the_frame() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    let mut green = Solid::new(Rect::new(0, 0, 64, 48), colour(0, 255, 0, 255));
    green.secure = true;
    frame.push_layer(Layer::Solid(green));
    frame.push_layer(shaded(Rect::new(16, 8, 32, 32), Color::WHITE, &[pass(BACKDROP, &[])]));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (4, 4), [0, 255, 0, 255], 0);
    assert_pixel(&image, (30, 20), [0, 0, 0, 255], 0);
    clean(&vulkan);
}

#[test]
fn a_backdrop_layer_partly_off_the_output_is_drawn_where_it_is_seen() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    red_over_blue(&mut frame);
    frame.push_layer(shaded(Rect::new(-8, -8, 24, 24), Color::WHITE, &[pass(BACKDROP, &[])]));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (2, 2), [255, 0, 0, 255], BLEND);
    clean(&vulkan);
}

#[test]
fn a_backdrop_is_built_for_the_first_three_layers_that_read_one() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    solid(&mut frame, Rect::new(0, 0, 64, 48), colour(255, 255, 255, 255));
    for x in [4, 12, 20, 28] {
        frame.push_layer(shaded(
            Rect::new(x, 8, 6, 6),
            colour(255, 0, 255, 255),
            &[pass(BACKDROP, &[])],
        ));
    }
    let image = draw(&mut vulkan, &frame);
    for x in [6, 14, 22] {
        assert_pixel(&image, (x, 10), [255, 255, 255, 255], BLEND);
    }
    assert_pixel(&image, (30, 10), [255, 0, 255, 255], BLEND);
    let refused = frame.shader_timings().take().refused;
    assert_eq!(
        refused.iter().map(|(_, code)| code.as_str()).collect::<Vec<_>>(),
        ["backdrop-unavailable"],
        "the layer past the third is reported"
    );
    clean(&vulkan);
}

#[test]
fn a_source_that_does_not_compile_leaves_its_layer_drawn_and_is_reported() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let broken = ShaderPass::new("this is not WGSL", &[], false);
    let key = broken.key();
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[Pass::Shader(broken)],
    ));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (12, 12), [255, 0, 0, 255], 0);
    let report = frame.shader_timings().take();
    assert_eq!(report.refused.len(), 1, "the source is refused once for the frame");
    assert_eq!(report.refused[0].0, key);
    assert_eq!(
        report.refused[0].1, "compile-failed",
        "a short code, the detail is in the log"
    );
    assert!(report.costs.is_empty(), "a pass that was not drawn cost nothing");

    // The refusal is kept, so the next frame names it again without compiling.
    draw(&mut vulkan, &frame);
    assert_eq!(frame.shader_timings().take().refused.len(), 1);
    assert_eq!(vulkan.device.shaders.borrow().len(), 1);
    clean(&vulkan);
}

#[test]
fn a_pass_that_reads_a_backdrop_it_was_not_flagged_for_is_refused() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let honest = source(BACKDROP);
    let liar = ShaderPass::new(&honest, &[], false);
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[Pass::Shader(liar)],
    ));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (12, 12), [255, 0, 0, 255], 0);
    let report = frame.shader_timings().take();
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.refused[0].1, "backdrop-unavailable");
    clean(&vulkan);
}

#[test]
fn a_pipeline_the_driver_refuses_is_reported_and_the_layer_is_drawn_without_it() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[pass(SWAP, &[])],
    ));
    vulkan
        .device
        .context
        .faults
        .pipeline
        .set(Some(vk::Result::ERROR_OUT_OF_HOST_MEMORY));
    let image = draw(&mut vulkan, &frame);
    assert_pixel(&image, (12, 12), [255, 0, 0, 255], 0);
    let report = frame.shader_timings().take();
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.refused[0].1, "pipeline-failed");
    clean(&vulkan);
}

/// A frame of `count` layers of two pixels, each with a source of its own that draws `green`.
fn many_sources(count: usize) -> (OutputFrame, Vec<ShaderKey>) {
    let mut frame = frame(Color::BLACK, SIZE);
    let mut keys = Vec::new();
    for index in 0..count {
        let source = format!("{}// {index}\n", source("return vec4<f32>(0.0, 1.0, 0.0, 1.0);"));
        let validated = user::validate(&source).expect("a valid shader");
        let pass = ShaderPass::new(&source, &[], validated.backdrop());
        keys.push(pass.key());
        let (x, y) = ((index % 32) as i32 * 2, (index / 32) as i32 * 2);
        frame.push_layer(shaded(
            Rect::new(x, y, 2, 2),
            colour(255, 0, 0, 255),
            &[Pass::Shader(pass)],
        ));
    }
    (frame, keys)
}

#[test]
fn the_pipelines_are_kept_by_source_and_never_more_than_64() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let (frame, keys) = many_sources(70);
    draw(&mut vulkan, &frame);
    assert_eq!(vulkan.device.shaders.borrow().len(), 64, "the cache is bounded");
    assert!(
        vulkan.device.shaders.borrow().contains(keys[0]),
        "a source the frame used stays"
    );
    assert!(
        !vulkan.device.shaders.borrow().contains(keys[69]),
        "the extra was never built"
    );
    clean(&vulkan);
}

#[test]
fn a_lost_device_drains_the_pipelines() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[pass(SWAP, &[])],
    ));
    draw(&mut vulkan, &frame);
    assert_eq!(vulkan.device.shaders.borrow().len(), 1);
    vulkan
        .device
        .context
        .faults
        .submit
        .set(Some(vk::Result::ERROR_DEVICE_LOST));
    assert_eq!(vulkan.draw(&frame, Scale::IDENTITY), Err(RenderError::Lost));
    assert_eq!(
        vulkan.device.shaders.borrow().len(),
        0,
        "a lost device drops every pipeline"
    );
    assert!(vulkan.is_lost());
}

#[test]
fn the_cost_of_a_shader_is_reported_to_the_frame_by_source() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let first = pass(SWAP, &[]);
    let second = pass("return c;", &[]);
    let keys: Vec<ShaderKey> = [&first, &second]
        .iter()
        .map(|pass| match pass {
            Pass::Shader(shader) => shader.key(),
            _ => unreachable!("a shader pass"),
        })
        .collect();
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(0, 0, 16, 16),
        colour(255, 0, 0, 255),
        std::slice::from_ref(&first),
    ));
    frame.push_layer(shaded(Rect::new(16, 0, 16, 16), colour(255, 0, 0, 255), &[first]));
    frame.push_layer(shaded(Rect::new(32, 0, 16, 16), colour(255, 0, 0, 255), &[second]));
    draw(&mut vulkan, &frame);
    let report = frame.shader_timings().take();
    let reported: Vec<ShaderKey> = report.costs.iter().map(|cost| cost.key).collect();
    assert_eq!(
        reported, keys,
        "one entry for each source drawn, a source drawn twice summed"
    );
    for cost in &report.costs {
        println!("{}: {:?}", cost.key.get(), cost.elapsed);
        assert!(cost.elapsed < Duration::from_secs(10), "{:?}", cost.elapsed);
    }
    assert!(
        frame.shader_timings().take().costs.is_empty(),
        "a take empties the handle"
    );
    clean(&vulkan);
}

#[test]
fn a_frame_without_shader_passes_reports_nothing() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    solid(&mut frame, Rect::new(0, 0, 16, 16), colour(255, 0, 0, 255));
    draw(&mut vulkan, &frame);
    let report = frame.shader_timings().take();
    assert!(report.costs.is_empty() && report.refused.is_empty());
}

#[test]
fn a_shader_layer_and_its_backdrop_are_in_physical_pixels_at_a_scale() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    // The same picture as the flipped backdrop above, in logical pixels at twice the scale.
    let mut frame = frame(Color::BLACK, Size::new(32, 24));
    solid(&mut frame, Rect::new(0, 0, 32, 12), colour(255, 0, 0, 255));
    solid(&mut frame, Rect::new(0, 12, 32, 12), colour(0, 0, 255, 255));
    frame.push_layer(shaded(
        Rect::new(8, 4, 16, 16),
        Color::WHITE,
        &[pass("return backdrop(vec2<f32>(uv.x, 1.0 - uv.y));", &[])],
    ));
    settle(&mut vulkan, &frame, Scale::integral(2));
    let image = vulkan.screenshot().expect("a readback");
    assert_pixel(&image, (20, 12), [0, 0, 255, 255], BLEND);
    assert_pixel(&image, (20, 36), [255, 0, 0, 255], BLEND);
    assert_pixel(&image, (4, 4), [255, 0, 0, 255], 0);
    clean(&vulkan);
}

#[test]
fn a_shader_layer_is_repainted_only_in_the_damage() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let build = |param: f32, damage: Rect| {
        let mut frame = OutputFrame::new();
        frame.set_background(Color::BLACK);
        frame.push_damage(damage);
        frame.push_layer(shaded(
            Rect::new(8, 8, 16, 16),
            Color::WHITE,
            &[pass("return vec4<f32>(p[0], 0.0, 0.0, 1.0);", &[param])],
        ));
        frame
    };
    let image = draw(&mut vulkan, &build(1.0, Rect::new(0, 0, 64, 48)));
    assert_pixel(&image, (9, 9), [255, 0, 0, 255], BLEND);
    assert_pixel(&image, (20, 20), [255, 0, 0, 255], BLEND);
    // A second frame changes the pass and damages the corner of the layer only.
    let image = draw(&mut vulkan, &build(0.0, Rect::new(0, 0, 12, 12)));
    assert_pixel(&image, (9, 9), [0, 0, 0, 255], BLEND);
    assert_pixel(&image, (20, 20), [255, 0, 0, 255], BLEND);
    clean(&vulkan);
}

#[test]
fn a_chain_of_identity_passes_equals_none_for_a_cropped_surface() {
    let client = ShmClient::new(vec![BufferSpec::argb(16, 16, pattern)]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let build = |passes: &[Pass]| {
        let mut frame = frame(colour(20, 40, 60, 255), SIZE);
        let layer = surface(
            &client,
            0,
            RectF::new(4.0, 4.0, 8.0, 8.0),
            Rect::new(8, 8, 8, 8),
            1.0,
            Color::WHITE,
        );
        frame.push_layer(with_passes(layer, passes));
        frame
    };
    let plain = draw(&mut vulkan, &build(&[]));
    let identity = pass("return c;", &[]);
    let shaded = draw(&mut vulkan, &build(&[identity.clone(), identity.clone(), identity]));
    assert_close("three identity passes against none", &plain, &shaded, BLEND);
    clean(&vulkan);
}

#[test]
fn the_first_draw_does_not_wait_for_the_build_and_a_later_one_has_the_pass() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let signal = vulkan.redraw_signal();
    let broken_first = pass(SWAP, &[]);
    let key = match &broken_first {
        Pass::Shader(shader) => shader.key(),
        _ => unreachable!("a shader pass"),
    };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(Rect::new(8, 8, 16, 16), colour(255, 0, 0, 255), &[broken_first]));
    // A build that takes 400 ms, as a slow driver's would.
    vulkan
        .device
        .context
        .faults
        .build_delay
        .set(Some(Duration::from_millis(400)));
    let started = std::time::Instant::now();
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let took = started.elapsed();
    println!("the first draw took {took:?} with a 400 ms build");
    assert!(
        took < Duration::from_millis(300),
        "the draw waited for the build: {took:?}"
    );
    let image = vulkan.screenshot().expect("a readback");
    assert_pixel(&image, (12, 12), [255, 0, 0, 255], 0);
    assert_eq!(
        frame.shader_timings().take().refused,
        [(key, "compiling".to_owned())],
        "the node says it is compiling"
    );
    assert!(!signal.take(), "the build is not done, so no frame is asked for");
    let mut asked = false;
    for _ in 0..1000 {
        if signal.take() {
            asked = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(asked, "the worker asked for another frame when the build was done");
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_pixel(&image, (12, 12), [0, 255, 0, 255], BLEND);
    assert!(frame.shader_timings().take().refused.is_empty());
    clean(&vulkan);
}

#[test]
fn dropping_the_renderer_during_a_build_is_safe() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[pass(SWAP, &[])],
    ));
    vulkan
        .device
        .context
        .faults
        .build_delay
        .set(Some(Duration::from_millis(300)));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    assert_eq!(vulkan.device.shaders.borrow().pending(), 1);
    clean(&vulkan);
    // The worker is still in its build: the drop joins it before the device goes.
    drop(vulkan);
}

#[test]
fn a_device_lost_during_a_build_is_safe_and_drains_the_worker() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(shaded(
        Rect::new(8, 8, 16, 16),
        colour(255, 0, 0, 255),
        &[pass(SWAP, &[])],
    ));
    vulkan
        .device
        .context
        .faults
        .build_delay
        .set(Some(Duration::from_millis(300)));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    vulkan
        .device
        .context
        .faults
        .submit
        .set(Some(vk::Result::ERROR_DEVICE_LOST));
    assert_eq!(vulkan.draw(&frame, Scale::IDENTITY), Err(RenderError::Lost));
    assert_eq!(vulkan.device.shaders.borrow().pending(), 0);
    assert_eq!(vulkan.device.shaders.borrow().len(), 0);
}

#[test]
fn a_frame_with_more_sources_than_the_cache_holds_refuses_the_extras_and_builds_nothing_again() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let (frame, keys) = many_sources(70);
    let image = draw(&mut vulkan, &frame);
    for index in [0, 31, 32, 63] {
        let (x, y) = ((index % 32) * 2, (index / 32) * 2);
        assert_pixel(&image, (x, y), [0, 255, 0, 255], BLEND);
    }
    for index in [64, 69] {
        let (x, y) = ((index % 32) * 2, (index / 32) * 2);
        assert_pixel(&image, (x, y), [255, 0, 0, 255], 0);
    }
    let report = frame.shader_timings().take();
    let full: Vec<ShaderKey> = report
        .refused
        .iter()
        .filter(|(_, code)| code == "cache-full")
        .map(|(key, _)| *key)
        .collect();
    assert_eq!(full, keys[64..], "the extras, and only they, are cache-full");
    assert_eq!(vulkan.device.shaders.borrow().len(), 64);
    assert!(vulkan.device.shaders.borrow().contains(keys[0]) && !vulkan.device.shaders.borrow().contains(keys[69]));
    // Steady frames build nothing: no build is asked of the worker, the same 64 stay, and the 6 refused sources are refused
    // again each frame (they are not built and not remembered as built). Nothing here depends on how long a frame takes.
    let asked = vulkan.device.shaders.borrow().requested();
    assert_eq!(
        asked, 64,
        "the first frame asked for the 64 sources the cache holds and no more"
    );
    for _ in 0..3 {
        vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
        assert_eq!(
            vulkan.device.shaders.borrow().requested(),
            asked,
            "no build is asked for again"
        );
        assert_eq!(vulkan.device.shaders.borrow().pending(), 0, "nothing is built again");
        assert_eq!(vulkan.device.shaders.borrow().len(), 64);
        assert!(vulkan.device.shaders.borrow().contains(keys[0]) && !vulkan.device.shaders.borrow().contains(keys[69]));
        let refused: Vec<ShaderKey> = frame
            .shader_timings()
            .take()
            .refused
            .iter()
            .filter(|(_, code)| code == "cache-full")
            .map(|(key, _)| *key)
            .collect();
        assert_eq!(refused, keys[64..], "the same extras are refused every frame");
    }
    clean(&vulkan);
}

#[test]
fn a_filter_grade_is_applied_after_a_shader_pass() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    let mut layer = Solid::new(Rect::new(8, 8, 16, 16), colour(255, 0, 0, 255));
    layer.effects.push(pass(SWAP, &[])).expect("a pass");
    layer
        .effects
        .push(Pass::Filter(gnoblin_render::FilterParams {
            blur: 0.0,
            saturate: 0.0,
            brightness: 0.5,
            contrast: 1.0,
        }))
        .expect("a filter");
    frame.push_layer(Layer::Solid(layer));
    let image = draw(&mut vulkan, &frame);
    // The swap makes the layer green; the grade then takes the colour to the grey of its luma, halved.
    let grey = (0.715_f32 * 0.5 * 255.0).round() as u8;
    assert_pixel(&image, (12, 12), [grey, grey, grey, 255], 1);
    assert_pixel(&image, (2, 2), [0, 0, 0, 255], 0);
    clean(&vulkan);
}

#[test]
fn the_worker_writes_the_pipeline_cache_after_sixteen_builds_and_counts_them() {
    let directory = tempfile::tempdir().expect("a directory");
    let options = VulkanOptions {
        pipeline_cache: PipelineCacheLocation::Directory(directory.path().to_owned()),
        ..options()
    };
    let mut vulkan = match Vulkan::with_options(SIZE, &options) {
        Ok(vulkan) => vulkan,
        Err(error @ (VulkanError::NoLoader { .. } | VulkanError::NoDevice { .. })) => {
            println!("skipping: {error}; there is no Vulkan device on this machine");
            return;
        }
        Err(error) => panic!("the Vulkan renderer could not be created: {error}"),
    };
    let file = options
        .pipeline_cache
        .file(&vulkan.device.context.cache_identity)
        .expect("a file for the device");
    // The shipped pipelines: the layer passes and the effect passes, made when the device starts.
    let shipped = vulkan.stats().pipelines_built;
    assert!(shipped >= 2, "at least the solid and surface pipelines");
    assert!(!file.exists(), "nothing is written before there is something to keep");
    // Fifteen builds are not enough for a write.
    let (few, _) = many_sources(15);
    settle(&mut vulkan, &few, Scale::IDENTITY);
    assert_eq!(vulkan.stats().pipelines_built, shipped + 15);
    assert!(!file.exists(), "fifteen builds do not make the worker write");
    // Twenty more distinct sources cross sixteen builds, and the file is written while the device is still running.
    let (many, _) = many_sources(40);
    settle(&mut vulkan, &many, Scale::IDENTITY);
    assert!(vulkan.stats().pipelines_built > shipped + 15);
    let written = std::fs::read(&file).expect("the worker wrote the cache after sixteen builds");
    assert_eq!(&written[..4], b"GNVC");
    clean(&vulkan);
}
