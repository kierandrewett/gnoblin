//! The user shader pass, probed pixel by pixel: a shell's WGSL drawn in place of a layer's own pixels, the uniforms it
//! reads, what happens to a source that does not compile, the backdrop it samples and the secure rule on that backdrop.
//!
//! Each case renders one small scene and reads named pixels back, with a tolerance of a few levels: the drivers differ in
//! how they round a premultiplied colour, and a solid's colour reaches the shader through an 8-bit texel.

use super::tests::{gles, probe};
use super::*;
use gnoblin_render::{Effects, ShaderKey};
use gnoblin_render_frame::frame::{Overlay, Solid};
use gnoblin_render_frame::shader::user::validate;
use std::time::Duration;

/// How far a channel may be from what a probe expects, in levels of 255.
const TOLERANCE: u8 = 4;

const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A shader that multiplies the node's colour: white becomes orange.
const ORANGE: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    return vec4<f32>(c.rgb * vec3<f32>(1.0, 0.5, 0.0), c.a);
}";

/// A shader that returns the backdrop where the layer is.
const GLASS: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    return backdrop(uv);
}";

/// A shader that returns the backdrop at the point `p[0], p[1]` of the layer, wherever it is drawn.
const PROBE: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    return backdrop(vec2<f32>(p[0], p[1]));
}";

/// A shader pass of `source`, which reads the backdrop when the validator says it does.
fn pass(source: &str, params: &[f32]) -> Pass {
    let backdrop = validate(source).is_ok_and(|validated| validated.backdrop());
    Pass::Shader(ShaderPass::new(source, params, backdrop))
}

fn solid(dst: Rect, colour: Color, passes: Vec<Pass>) -> Solid {
    let mut solid = Solid::new(dst, colour);
    let mut effects = Effects::new();
    for pass in passes {
        effects.push(pass).expect("a pass in range");
    }
    solid.effects = effects;
    solid
}

fn plain(dst: Rect, colour: [u8; 4]) -> Layer {
    Layer::Solid(Solid::new(
        dst,
        Color::from_rgba8(colour[0], colour[1], colour[2], colour[3]),
    ))
}

fn frame_of(size: Size, background: Color, layers: Vec<Layer>) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(background);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    for layer in layers {
        frame.push_layer(layer);
    }
    frame
}

/// Draws `frame` and reads the output back.
fn render(renderer: &mut Gles, frame: &OutputFrame) -> Image {
    renderer.draw(frame, Scale::IDENTITY).expect("the frame draws");
    renderer.screenshot().expect("a readback")
}

#[track_caller]
fn assert_pixel(image: &Image, x: i32, y: i32, want: [u8; 4], what: &str) {
    let got = probe(image, x, y);
    assert!(
        got.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= TOLERANCE),
        "{what}: the pixel at {x},{y} is {got:?}, want {want:?}"
    );
}

#[test]
fn a_user_shader_shades_its_layer() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let layer = Layer::Solid(solid(Rect::new(8, 8, 16, 16), Color::WHITE, vec![pass(ORANGE, &[])]));
    let image = render(
        &mut renderer,
        &frame_of(size, Color::new(0.0, 0.0, 1.0, 1.0), vec![layer]),
    );
    assert_pixel(
        &image,
        16,
        16,
        [255, 128, 0, 255],
        "inside the layer the shader's colour shows",
    );
    assert_pixel(&image, 2, 2, BLUE, "outside the layer the background is untouched");
    assert_pixel(&image, 28, 28, BLUE, "and on the other side too");
}

#[test]
fn a_user_shader_sees_a_solids_own_colour() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // The node is half-bright grey: the shader's `c` is that colour, so doubling it is the colour the layer shows.
    let layer = Layer::Solid(solid(
        Rect::new(8, 8, 16, 16),
        Color::new(0.25, 0.25, 0.25, 1.0),
        vec![pass(
            "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
                return vec4<f32>(c.rgb * 2.0, c.a);
            }",
            &[],
        )],
    ));
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![layer]));
    assert_pixel(&image, 16, 16, [128, 128, 128, 255], "twice a quarter grey");
}

#[test]
fn a_user_shader_reads_its_uniforms() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // The first and the last of the eight numbers: the block packs the array with a 16-byte stride, so the last one is
    // the one a wrong layout would lose.
    let layer = Layer::Solid(solid(
        Rect::new(8, 8, 16, 16),
        Color::WHITE,
        vec![pass(
            "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
                return vec4<f32>(p[0], p[7], p[3], 1.0);
            }",
            &[1.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.5],
        )],
    ));
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![layer]));
    assert_pixel(
        &image,
        16,
        16,
        [255, 128, 64, 255],
        "p[0], p[7] and p[3] are what the shell wrote",
    );
}

#[test]
fn a_shader_that_does_not_validate_leaves_the_layer_drawn_without_it() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // `effect` takes four numbers, not eight: the compositor refuses this at write time, and a renderer that is handed it
    // some other way refuses it again and draws the layer as it is.
    let mistyped = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 4>, t: f32) -> vec4<f32> {
        return vec4<f32>(0.0, 0.0, 1.0, 1.0);
    }";
    let key = ShaderKey::of(mistyped);
    let layer = Layer::Solid(solid(
        Rect::new(8, 8, 16, 16),
        Color::from_rgba8(200, 100, 50, 255),
        vec![pass(mistyped, &[])],
    ));
    let frame = frame_of(size, Color::BLACK, vec![layer]);
    let image = render(&mut renderer, &frame);
    assert_pixel(&image, 16, 16, [200, 100, 50, 255], "the layer's own colour");
    let report = frame.shader_timings().take();
    assert_eq!(report.refused.len(), 1, "the pass is named as left undrawn");
    assert_eq!(report.refused[0].0, key);
    assert_eq!(report.refused[0].1, "compile-failed");
    assert!(report.costs.is_empty(), "nothing ran, so nothing cost");
}

#[test]
fn a_broken_user_shader_leaves_the_frame_drawn() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let broken = Layer::Solid(solid(
        Rect::new(0, 0, 16, 16),
        Color::from_rgba8(10, 20, 30, 255),
        vec![pass("fn effect( {", &[])],
    ));
    let beside = plain(Rect::new(16, 16, 16, 16), GREEN);
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![broken, beside]));
    assert_pixel(
        &image,
        8,
        8,
        [10, 20, 30, 255],
        "the layer with the broken shader is drawn as it is",
    );
    assert_pixel(&image, 24, 24, GREEN, "the layers around it are drawn");

    // The renderer is not left in a state that stops the next frame: a good shader draws.
    let good = Layer::Solid(solid(Rect::new(0, 0, 16, 16), Color::WHITE, vec![pass(ORANGE, &[])]));
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![good]));
    assert_pixel(&image, 8, 8, [255, 128, 0, 255], "the next frame's shader draws");
}

#[test]
fn the_layers_tint_and_corners_still_apply_to_a_shader() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut rounded = solid(Rect::new(8, 8, 16, 16), Color::WHITE, vec![pass(ORANGE, &[])]);
    rounded.corner_radius = 8.0;
    rounded.tint = Color::new(0.5, 0.5, 0.5, 1.0);
    let image = render(
        &mut renderer,
        &frame_of(size, Color::BLACK, vec![Layer::Solid(rounded)]),
    );
    assert_pixel(
        &image,
        16,
        16,
        [128, 64, 0, 255],
        "the shader's orange under a half tint",
    );
    assert_pixel(&image, 8, 8, BLACK, "the corner is cut, so the background shows there");
}

#[test]
fn passes_chain_in_order_and_only_the_last_applies_the_layers_tint() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // The first pass paints white whatever the node was; the second keeps what it is given. The tint is the layer's, so
    // it is applied once, by the last pass: 0.5, not 0.25.
    let mut layer = solid(
        Rect::new(8, 8, 16, 16),
        Color::new(1.0, 0.0, 0.0, 1.0),
        vec![
            pass(
                "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
                    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
                }",
                &[],
            ),
            pass(
                "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
                    return vec4<f32>(c.g, c.g, c.g, 1.0);
                }",
                &[],
            ),
        ],
    );
    layer.tint = Color::new(0.5, 0.5, 0.5, 1.0);
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![Layer::Solid(layer)]));
    assert_pixel(
        &image,
        16,
        16,
        [128, 128, 128, 255],
        "the second pass read the first one's white (green was 0 in the solid), once tinted",
    );
}

#[test]
fn a_pass_reads_the_one_before_it_the_right_way_up() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let gradient = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
        return vec4<f32>(uv.y, 0.0, 0.0, 1.0);
    }";
    let identity = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
        return c;
    }";
    let rect = Rect::new(0, 0, 32, 32);
    let alone = render(
        &mut renderer,
        &frame_of(
            size,
            Color::BLACK,
            vec![Layer::Solid(solid(rect, Color::WHITE, vec![pass(gradient, &[])]))],
        ),
    );
    let chained = render(
        &mut renderer,
        &frame_of(
            size,
            Color::BLACK,
            vec![Layer::Solid(solid(
                rect,
                Color::WHITE,
                vec![pass(gradient, &[]), pass(identity, &[])],
            ))],
        ),
    );
    assert!(
        probe(&alone, 16, 2)[0] < 40 && probe(&alone, 16, 29)[0] > 215,
        "uv.y is 0 at the top of the layer and 1 at the bottom"
    );
    for y in [2, 9, 16, 23, 29] {
        let (one, two) = (probe(&alone, 16, y), probe(&chained, 16, y));
        assert!(
            one[0].abs_diff(two[0]) <= TOLERANCE,
            "row {y}: a pass that returns its input gives the same picture ({one:?} against {two:?})"
        );
    }
}

/// The base of the backdrop scenes: red on the left half, green over blue on the right, all in the lower half.
fn striped() -> Vec<Layer> {
    vec![
        plain(Rect::new(0, 0, 64, 64), WHITE),
        plain(Rect::new(0, 32, 64, 16), GREEN),
        plain(Rect::new(0, 48, 64, 16), BLUE),
        plain(Rect::new(0, 32, 32, 32), RED),
    ]
}

#[test]
fn a_shader_that_samples_the_backdrop_returns_the_pixels_beneath_it() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut layers = striped();
    layers.push(Layer::Solid(solid(
        Rect::new(16, 32, 32, 32),
        Color::new(1.0, 1.0, 0.0, 1.0),
        vec![pass(GLASS, &[])],
    )));
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, layers));
    assert_pixel(&image, 20, 36, RED, "the left of the layer is over red");
    assert_pixel(&image, 20, 60, RED, "red goes to the bottom on the left");
    assert_pixel(
        &image,
        40,
        36,
        GREEN,
        "the top right is over green: the backdrop is the right way up",
    );
    assert_pixel(&image, 40, 60, BLUE, "the bottom right is over blue");
    assert_pixel(&image, 4, 40, RED, "the layers below are drawn as they were beside it");
}

#[test]
fn a_backdrop_off_the_output_reads_what_is_on_it() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let layers = vec![
        plain(Rect::new(0, 0, 64, 64), GREEN),
        Layer::Solid(solid(Rect::new(48, 48, 32, 32), Color::WHITE, vec![pass(GLASS, &[])])),
    ];
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, layers));
    assert_pixel(
        &image,
        56,
        56,
        GREEN,
        "the part of the layer on the output sees the pixels beneath it",
    );
}

#[test]
fn a_shader_that_samples_the_backdrop_sees_the_layers_below_and_not_the_ones_above() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut layers = striped();
    layers.push(Layer::Solid(solid(
        Rect::new(16, 32, 32, 32),
        Color::WHITE,
        vec![pass(GLASS, &[])],
    )));
    // A layer above the glass covers its left edge: the glass's backdrop does not have it.
    layers.push(plain(Rect::new(16, 32, 4, 32), WHITE));
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, layers));
    assert_pixel(&image, 17, 40, WHITE, "the layer above is drawn over the glass");
    assert_pixel(
        &image,
        22,
        40,
        RED,
        "and the glass beside it still reads red, not white",
    );
}

/// The frame of the secure scenes: white, a red rectangle at the lower left, and a glass over the lower half that reads
/// the backdrop at `uv` and shows it at every pixel.
fn secure_scene(secret: bool, uv: [f32; 2]) -> OutputFrame {
    let mut window = Solid::new(Rect::new(0, 32, 16, 16), Color::new(1.0, 0.0, 0.0, 1.0));
    window.secure = secret;
    let glass = solid(
        Rect::new(0, 32, 64, 32),
        Color::new(1.0, 1.0, 0.0, 1.0),
        vec![pass(PROBE, &uv)],
    );
    frame_of(
        Size::new(64, 64),
        Color::BLACK,
        vec![
            plain(Rect::new(0, 0, 64, 64), WHITE),
            Layer::Solid(window),
            Layer::Solid(glass),
        ],
    )
}

#[test]
fn a_secure_window_reads_as_black_in_a_backdrop() {
    let Some(mut renderer) = gles(Size::new(64, 64)) else {
        return;
    };
    // Control: the same window, not marked, is what the shader reads, so the black below is the rule and not the scene.
    let open = render(&mut renderer, &secure_scene(false, [0.1, 0.1]));
    assert_pixel(&open, 40, 48, RED, "an ordinary window is read as it is");

    let secret = render(&mut renderer, &secure_scene(true, [0.1, 0.1]));
    assert_pixel(&secret, 40, 48, BLACK, "a secure window reads black, not red");
    // The secure window is still on the output where the layer is not drawn over it: this is the real frame.
    let beside = render(&mut renderer, &secure_scene(true, [0.9, 0.1]));
    assert_pixel(&beside, 40, 48, WHITE, "what is not secure beside it is read as it is");
}

#[test]
fn the_lock_card_reads_as_black_in_a_backdrop() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let red = Image::new(Size::new(1, 1), 4, vec![255, 0, 0, 255]).expect("a one-pixel image");
    let scene = |secure: bool| {
        let card = Overlay::new(Rect::new(0, 32, 16, 16), &red)
            .expect("a card")
            .with_secure(secure);
        frame_of(
            size,
            Color::BLACK,
            vec![
                plain(Rect::new(0, 0, 64, 64), WHITE),
                Layer::Overlay(card),
                Layer::Solid(solid(
                    Rect::new(0, 32, 64, 32),
                    Color::WHITE,
                    vec![pass(PROBE, &[0.1, 0.1])],
                )),
            ],
        )
    };
    let open = render(&mut renderer, &scene(false));
    assert_pixel(
        &open,
        40,
        48,
        RED,
        "an ordinary overlay below the layer is read as it is",
    );
    let locked = render(&mut renderer, &scene(true));
    assert_pixel(&locked, 40, 48, BLACK, "the lock card reads black");
}

#[test]
fn a_backdrop_that_reads_a_backdrop_is_built_in_turn() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut layers = striped();
    // The lower glass reads the stripes; the upper one reads the lower glass, which shows the stripes.
    layers.push(Layer::Solid(solid(
        Rect::new(0, 32, 64, 32),
        Color::WHITE,
        vec![pass(GLASS, &[])],
    )));
    layers.push(Layer::Solid(solid(
        Rect::new(8, 40, 16, 16),
        Color::WHITE,
        vec![pass(GLASS, &[])],
    )));
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, layers));
    assert_pixel(
        &image,
        16,
        48,
        RED,
        "the upper glass shows what the lower glass showed there",
    );
    assert_pixel(&image, 48, 40, GREEN, "and the lower glass shows the stripes");
}

/// Draws `frames` frames of `layers` (each damaging the whole output), reading the output back after every one so the GPU
/// has finished with it, and returns every cost the frames reported, in order, with one more frame drawn at the end to
/// collect the last measurement.
fn costs_over(
    renderer: &mut Gles,
    size: Size,
    frames: usize,
    layers: &dyn Fn() -> Vec<Layer>,
) -> Vec<(ShaderKey, Duration)> {
    let mut costs = Vec::new();
    for _ in 0..=frames {
        let frame = frame_of(size, Color::BLACK, layers());
        renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
        let reported = frame.shader_timings().take();
        costs.extend(reported.costs.into_iter().map(|cost| (cost.key, cost.elapsed)));
        renderer.screenshot().expect("a readback");
    }
    costs
}

fn orange_layer() -> Vec<Layer> {
    vec![Layer::Solid(solid(
        Rect::new(8, 8, 16, 16),
        Color::WHITE,
        vec![pass(ORANGE, &[])],
    ))]
}

#[test]
fn without_timer_queries_the_first_draws_of_a_source_are_measured_and_reported() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    renderer.shaders.clock.set_mode(&mut renderer.device, TimerMode::Fences);
    let costs = costs_over(&mut renderer, size, 9, &orange_layer);
    assert_eq!(
        renderer.shaders.clock.issued(),
        timing::SAMPLE_FIRST,
        "ten draws, of which the first {} are sampled and the next one in {} is not reached",
        timing::SAMPLE_FIRST,
        timing::SAMPLE_EVERY
    );
    assert!(!costs.is_empty(), "the sampled draws were reported");
    assert!(costs.iter().all(|(key, _)| *key == ShaderKey::of(ORANGE)));
    assert!(
        costs.iter().all(|(_, elapsed)| *elapsed < Duration::from_secs(2)),
        "a draw this small is not reported as a runaway: {costs:?}"
    );
}

#[test]
fn without_timer_queries_a_steady_source_is_sampled_one_frame_in_n() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    renderer.shaders.clock.set_mode(&mut renderer.device, TimerMode::Fences);
    let costs = costs_over(&mut renderer, size, 2 * timing::SAMPLE_EVERY as usize, &orange_layer);
    // Draws 1 to 3, then 30 and 60.
    assert_eq!(renderer.shaders.clock.issued(), timing::SAMPLE_FIRST + 2);
    assert!(!costs.is_empty(), "and they were reported");
    assert_eq!(
        renderer.shaders.clock.pending(),
        0,
        "none is left waiting after the readbacks"
    );
}

#[test]
fn with_timer_queries_every_draw_is_timed_by_the_gpu() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    if !renderer.device.has_timer_queries() {
        println!(
            "skipping the timer query test: {} has no GL_EXT_disjoint_timer_query, so the fake GPU in timing::tests covers the branch",
            renderer.driver()
        );
        return;
    }
    renderer
        .shaders
        .clock
        .set_mode(&mut renderer.device, TimerMode::Queries);
    let costs = costs_over(&mut renderer, size, 5, &orange_layer);
    assert_eq!(renderer.shaders.clock.issued(), 6, "every draw is timed");
    assert!(!costs.is_empty(), "the driver answered the queries");
    assert!(costs.iter().all(|(key, _)| *key == ShaderKey::of(ORANGE)));
}

#[test]
fn a_context_picks_timer_queries_only_when_it_has_them() {
    let Some(renderer) = gles(Size::new(8, 8)) else {
        return;
    };
    let want = if renderer.device.has_timer_queries() {
        TimerMode::Queries
    } else {
        TimerMode::Fences
    };
    assert_eq!(renderer.shaders.clock.mode(), want);
}

/// A frame with every kind of shader draw: a solid's colour, a chain of two passes, and a backdrop that is built and
/// copied.
fn every_kind_of_shader_frame() -> Vec<Layer> {
    let mut layers = striped();
    layers.push(Layer::Solid(solid(
        Rect::new(0, 0, 32, 32),
        Color::WHITE,
        vec![pass(ORANGE, &[]), pass(ORANGE, &[])],
    )));
    layers.push(Layer::Solid(solid(
        Rect::new(16, 32, 32, 32),
        Color::WHITE,
        vec![pass(GLASS, &[])],
    )));
    layers
}

#[test]
fn a_frame_with_shaders_never_waits_for_the_gpu() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut modes = vec![TimerMode::Fences];
    if renderer.device.has_timer_queries() {
        modes.push(TimerMode::Queries);
    }
    for mode in modes {
        renderer.shaders.clock.set_mode(&mut renderer.device, mode);
        // The first frame compiles the programs and makes the targets; the frames after it are the steady state. Both
        // are frames a shell can cause, and neither blocks.
        for frame_number in 0..4 {
            let frame = frame_of(size, Color::BLACK, every_kind_of_shader_frame());
            let before = renderer.device.blocking_waits();
            renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
            assert_eq!(
                renderer.device.blocking_waits(),
                before,
                "{mode:?}, frame {frame_number}: a frame with shaders blocked on the GPU"
            );
        }
        let image = renderer.screenshot().expect("a readback");
        assert_pixel(
            &image,
            8,
            8,
            [255, 64, 0, 255],
            "the frame is still drawn (two orange passes multiply)",
        );
        assert_pixel(&image, 20, 60, RED, "and its backdrop is still read");
    }
    let before = renderer.device.blocking_waits();
    renderer.screenshot().expect("a readback");
    assert!(
        renderer.device.blocking_waits() > before,
        "the counter counts a readback, so the zero above is a measurement and not a counter that does not move"
    );
}

/// Times `frames` draws of `layers`: the milliseconds per frame the compositor's thread spends in `draw`, and the
/// milliseconds per frame including the wait for the GPU to finish all of them. With `finish_each`, the renderer waits
/// after every frame, which is what a `glFinish` after each shader draw cost.
fn time_frames(
    renderer: &mut Gles,
    size: Size,
    frames: u32,
    finish_each: bool,
    layers: &dyn Fn() -> Vec<Layer>,
) -> (f64, f64) {
    let prepared: Vec<OutputFrame> = (0..frames + 5)
        .map(|_| frame_of(size, Color::BLACK, layers()))
        .collect();
    for frame in &prepared[..5] {
        renderer.draw(frame, Scale::IDENTITY).expect("a warm-up frame");
    }
    renderer.screenshot().expect("a readback");
    let began = Instant::now();
    for frame in &prepared[5..] {
        renderer.draw(frame, Scale::IDENTITY).expect("drawn");
        if finish_each {
            renderer.device.wait();
        }
    }
    let drawing = began.elapsed();
    renderer.device.wait();
    let finished = began.elapsed();
    let per_frame = |total: Duration| total.as_secs_f64() * 1000.0 / f64::from(frames);
    (per_frame(drawing), per_frame(finished))
}

/// Prints what a frame costs with and without a shader. It measures this machine's driver (llvmpipe, a software
/// rasteriser, where the "GPU" is the same CPU), not a real GPU, so the numbers say what the code does and not what a
/// device would. Run it with `cargo test -p gnoblin-render-gles --release measure_ -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn measure_what_a_frame_costs_with_and_without_a_shader() {
    let size = Size::new(1280, 720);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    println!("driver: {}", renderer.driver());
    let background = || {
        vec![
            plain(Rect::new(0, 0, 1280, 720), WHITE),
            plain(Rect::new(100, 100, 600, 400), RED),
        ]
    };
    let tinted = || {
        let mut layers = background();
        layers.push(Layer::Solid(solid(
            Rect::new(300, 200, 640, 360),
            Color::WHITE,
            vec![pass(ORANGE, &[])],
        )));
        layers
    };
    let glass = || {
        let mut layers = background();
        layers.push(Layer::Solid(solid(
            Rect::new(300, 200, 640, 360),
            Color::WHITE,
            vec![pass(GLASS, &[])],
        )));
        layers
    };
    let plain_layer = || {
        let mut layers = background();
        layers.push(plain(Rect::new(300, 200, 640, 360), WHITE));
        layers
    };
    let frames = 100;
    for mode in [TimerMode::Fences, TimerMode::Queries] {
        if mode == TimerMode::Queries && !renderer.device.has_timer_queries() {
            println!("{mode:?}: not offered by this driver");
            continue;
        }
        renderer.shaders.clock.set_mode(&mut renderer.device, mode);
        println!("mode {mode:?}, ms per frame as (in draw, with the GPU finished):");
        for (name, layers, finish_each) in [
            ("no shader", &plain_layer as &dyn Fn() -> Vec<Layer>, false),
            ("tint shader", &tinted, false),
            ("backdrop shader", &glass, false),
            ("tint shader, glFinish after every frame (the old way)", &tinted, true),
            (
                "backdrop shader, glFinish after every frame (the old way)",
                &glass,
                true,
            ),
        ] {
            let (drawing, finished) = time_frames(&mut renderer, size, frames, finish_each, layers);
            println!("  {name:<58} {drawing:>8.3} {finished:>8.3}");
        }
    }
}

#[test]
fn a_source_is_compiled_once_and_the_cache_is_bounded() {
    let size = Size::new(80, 80);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let source = |index: usize| {
        format!(
            "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {{
                return vec4<f32>({index}.0 / 100.0, 0.0, 0.0, 1.0);
            }}"
        )
    };
    let layers = (0..70)
        .map(|index| {
            let (x, y) = ((index % 8) as i32 * 10, (index / 8) as i32 * 8);
            Layer::Solid(solid(
                Rect::new(x, y, 10, 8),
                Color::WHITE,
                vec![pass(&source(index), &[])],
            ))
        })
        .collect();
    let frame = frame_of(size, Color::BLACK, layers);
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    assert_eq!(
        renderer.shaders.cache.len(),
        gnoblin_render_frame::shader::SHADER_CACHE_ENTRIES,
        "seventy sources leave the cache full and no fuller"
    );
    let image = renderer.screenshot().expect("a readback");
    assert_pixel(&image, 5, 4, [0, 0, 0, 255], "source 0 draws 0");
    assert_pixel(
        &image,
        75,
        4,
        [(7.0f32 / 100.0 * 255.0).round() as u8, 0, 0, 255],
        "and source 7 draws 7 hundredths",
    );

    // A second frame of the same sources compiles nothing new for the ones that stayed.
    let held = renderer.shaders.cache.len();
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn again");
    assert_eq!(
        renderer.shaders.cache.len(),
        held,
        "the cache does not grow past its bound"
    );
}

#[test]
fn a_lost_device_drops_every_compiled_shader_and_the_next_frame_compiles_again() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let layer = || Layer::Solid(solid(Rect::new(8, 8, 16, 16), Color::WHITE, vec![pass(ORANGE, &[])]));
    render(&mut renderer, &frame_of(size, Color::BLACK, vec![layer()]));
    assert_eq!(renderer.shaders.cache.len(), 1);
    renderer.drain_shaders();
    assert!(
        renderer.shaders.cache.is_empty(),
        "nothing the old context compiled is kept"
    );
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![layer()]));
    assert_pixel(
        &image,
        16,
        16,
        [255, 128, 0, 255],
        "the shader compiles again and draws",
    );
}

#[test]
fn a_shader_on_a_layer_placed_in_3d_is_not_drawn() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut tilted = solid(
        Rect::new(8, 8, 16, 16),
        Color::from_rgba8(200, 100, 50, 255),
        vec![pass(ORANGE, &[])],
    );
    tilted.model = Some(Model {
        matrix: Mat4::translation(8.0, 8.0, 0.0),
        opaque: true,
    });
    let image = render(&mut renderer, &frame_of(size, Color::BLACK, vec![Layer::Solid(tilted)]));
    assert_pixel(
        &image,
        16,
        16,
        [200, 100, 50, 255],
        "the composer lists a 3D layer's effects as skipped, so it is drawn plain",
    );
}
