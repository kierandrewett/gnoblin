//! What the effects of a layer do beyond the shared golden pictures: the backdrop blur's downscale, grade and grain, the
//! order the effects are drawn in, and the filter's picture.
//!
//! Each test is one way the effect could be wrong and still draw something plausible:
//!
//! 1. A downscaled blur runs at full resolution all the same: the reduced targets it leases are the proof.
//! 2. A downscaled blur comes out flipped, shifted or at the wrong width: four quadrants of different colours, blurred
//!    at full and at reduced resolution, must agree to a few levels.
//! 3. Grain is counted from the screen and not the layer, so it swims under a window that moves.
//! 4. The saturation of a backdrop blur is ignored.
//! 5. The order effects are listed in changes the picture.
//! 6. An inset shadow ignores the layer's tint, and a translucent background does not tint the blur under it.
//! 7. A filter's blur loses what hangs off the output, or shifts it.
//! 8. A filter's blur paints outside the frame's damage.
//! 9. A filter's blur does not fade with the layer.
//! 10. The pictures a filter leases are not given back, so the pool grows with every frame.
//! 11. A grade over a shell's shader pass is dropped, because the pass has no grade of its own.
//! 12. A layer with glass and a filter blur draws one of them only.

use super::tests::{gles, probe};
use super::*;
use gnoblin_render::{BlurParams, Border, Effects, FilterParams, Shadow, Vec3};
use gnoblin_render_frame::frame::Solid;
use gnoblin_render_frame::grade::{self, Grade};
use std::sync::Arc;

fn colour(rgb: [u8; 3]) -> Color {
    Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255)
}

fn chain(passes: Vec<Pass>) -> Effects {
    let mut effects = Effects::new();
    for pass in passes {
        effects.push(pass).expect("a pass in range");
    }
    effects
}

/// A solid layer over `dst`.
fn fill(dst: Rect, paint: Color) -> Layer {
    Layer::Solid(Solid::new(dst, paint))
}

/// A layer with no colour of its own that carries only `passes`.
fn clear(dst: Rect, radius: f32, passes: Vec<Pass>) -> Layer {
    let mut solid = Solid::new(dst, Color::TRANSPARENT);
    solid.corner_radius = radius;
    solid.effects = chain(passes);
    Layer::Solid(solid)
}

fn frame_of(size: Size, layers: Vec<Layer>) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    for layer in layers {
        frame.push_layer(layer);
    }
    frame
}

fn render(renderer: &mut Gles, frame: &OutputFrame) -> Image {
    renderer.draw(frame, Scale::IDENTITY).expect("the frame draws");
    renderer.screenshot().expect("a readback")
}

fn rgb(image: &Image, x: i32, y: i32) -> [u8; 3] {
    let [r, g, b, _] = probe(image, x, y);
    [r, g, b]
}

#[track_caller]
fn assert_close(got: [u8; 3], want: [u8; 3], tolerance: u8, what: &str) {
    assert!(
        got.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= tolerance),
        "{what}: drew {got:?}, expected {want:?}"
    );
}

fn glass(passes: u32, downscale: f32) -> BlurParams {
    BlurParams {
        radius: 6.0,
        passes,
        downscale,
        ..BlurParams::default()
    }
}

/// Four quadrants, red, green, blue and white, split at the middle of an output of `size`.
fn quadrants(size: Size) -> Vec<Layer> {
    let (half_w, half_h) = (size.width / 2, size.height / 2);
    let quad = |x: u32, y: u32, rgb: [u8; 3]| fill(Rect::new(x as i32, y as i32, half_w, half_h), colour(rgb));
    vec![
        quad(0, 0, [255, 0, 0]),
        quad(half_w, 0, [0, 255, 0]),
        quad(0, half_h, [0, 0, 255]),
        quad(half_w, half_h, [255, 255, 255]),
    ]
}

#[test]
fn a_downscaled_blur_leases_targets_of_a_reduced_size_and_a_full_one_leases_none() {
    let size = Size::new(48, 48);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let layers = |downscale: f32| {
        let mut layers = quadrants(size);
        layers.push(clear(
            Rect::new(8, 8, 32, 32),
            0.0,
            vec![Pass::Blur(glass(2, downscale))],
        ));
        layers
    };
    render(&mut renderer, &frame_of(size, layers(1.0)));
    assert_eq!(
        renderer.device.spare_bytes(),
        0,
        "a blur at full resolution needs no reduced target"
    );
    render(&mut renderer, &frame_of(size, layers(0.5)));
    let reduced = renderer.device.spare_bytes();
    assert!(reduced > 0, "a blur at half resolution works in targets of its own");
    let whole = renderer.device.target(size).expect("a target");
    assert!(
        reduced < whole.bytes(),
        "and the two of them together are smaller than one the size of the output: {reduced} bytes"
    );
    renderer.device.release(whole);
}

#[test]
fn a_downscaled_blur_agrees_with_the_full_one_on_where_things_are() {
    let size = Size::new(48, 48);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let draw = |renderer: &mut Gles, downscale: f32| {
        let mut layers = quadrants(size);
        layers.push(clear(
            Rect::new(8, 8, 32, 32),
            0.0,
            vec![Pass::Blur(glass(2, downscale))],
        ));
        render(renderer, &frame_of(size, layers))
    };
    let full = draw(&mut renderer, 1.0);
    // Where the four colours meet, the middle of the blur is an equal mix of all four.
    let middle = rgb(&full, 24, 24);
    assert!(
        middle.iter().all(|channel| (100..160).contains(channel)),
        "the full blur mixes the four quadrants at their corner: {middle:?}"
    );
    for downscale in [0.5, 0.25] {
        let reduced = draw(&mut renderer, downscale);
        for y in 8..40 {
            for x in 8..40 {
                assert_close(
                    rgb(&reduced, x, y),
                    rgb(&full, x, y),
                    32,
                    &format!("downscale {downscale} at {x},{y}"),
                );
            }
        }
        assert_close(
            rgb(&reduced, 12, 12),
            rgb(&full, 12, 12),
            12,
            "the red quadrant stays at the top left",
        );
        assert!(
            rgb(&reduced, 12, 12)[0] > rgb(&reduced, 12, 36)[0] + 100,
            "red is above blue: a flipped reduction would swap them"
        );
    }
}

#[test]
fn grain_is_counted_from_the_layer_and_moves_with_it() {
    let size = Size::new(40, 20);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let grey = [128, 128, 128];
    let noise = 1.0;
    let draw = |renderer: &mut Gles, x: i32, downscale: f32| {
        let params = BlurParams {
            radius: 2.0,
            passes: 1,
            downscale,
            noise,
            ..BlurParams::default()
        };
        render(
            renderer,
            &frame_of(
                size,
                vec![
                    fill(Rect::from_size(size), colour(grey)),
                    clear(Rect::new(x, 6, 16, 8), 0.0, vec![Pass::Blur(params)]),
                ],
            ),
        )
    };
    for downscale in [1.0, 0.5] {
        let (left, right) = (draw(&mut renderer, 4, downscale), draw(&mut renderer, 19, downscale));
        let mut grainy = 0;
        for j in 0..8u32 {
            for i in 0..16u32 {
                let want = (f32::from(grey[0]) / 255.0 + grade::grain(noise, i, j)).clamp(0.0, 1.0);
                let want = (want * 255.0).round() as u8;
                for (image, origin) in [(&left, 4), (&right, 19)] {
                    let got = rgb(image, origin + i as i32, 6 + j as i32);
                    assert!(
                        got.iter().all(|channel| channel.abs_diff(want) <= 2),
                        "downscale {downscale}, layer at {origin}, pixel {i},{j}: drew {got:?}, the grain says {want}"
                    );
                }
                grainy += usize::from(want != grey[0]);
            }
        }
        assert!(grainy > 32, "the grain is not flat: {grainy} of 128 pixels moved");
        assert_eq!(rgb(&left, 30, 10), grey, "outside the layer there is none");
    }
}

#[test]
fn the_saturation_of_a_blur_grades_the_backdrop() {
    let size = Size::new(24, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let backdrop = [200, 100, 100];
    for saturation in [0.0, 2.0] {
        let params = BlurParams {
            radius: 3.0,
            passes: 1,
            downscale: 1.0,
            saturation,
            ..BlurParams::default()
        };
        let image = render(
            &mut renderer,
            &frame_of(
                size,
                vec![
                    fill(Rect::from_size(size), colour(backdrop)),
                    clear(Rect::new(4, 4, 16, 16), 0.0, vec![Pass::Blur(params)]),
                ],
            ),
        );
        let graded = grade::apply(
            Grade {
                saturate: saturation,
                ..Grade::IDENTITY
            },
            colour(backdrop),
        );
        let want = [graded.r, graded.g, graded.b].map(|channel| (channel * 255.0).round() as u8);
        assert_close(rgb(&image, 12, 12), want, 2, &format!("saturation {saturation}"));
        assert_eq!(rgb(&image, 1, 1), backdrop, "outside the glass nothing is graded");
    }
}

fn one_of_each() -> Vec<Pass> {
    vec![
        Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 2.0, 0.0),
            blur: 4.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::from_rgba8(0, 0, 0, 160),
            frame: false,
            inset: false,
        }),
        Pass::Blur(glass(1, 0.5)),
        Pass::Background {
            color: Color::from_rgba8(200, 40, 40, 128),
        },
        Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 2.0, 0.0),
            blur: 4.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::from_rgba8(0, 0, 0, 200),
            frame: false,
            inset: true,
        }),
        Pass::Border(Border {
            width: 1.0,
            offset: 0.0,
            inset: false,
            color: Color::from_rgba8(0, 200, 0, 255),
        }),
    ]
}

#[test]
fn the_order_effects_are_listed_in_does_not_change_the_picture() {
    let size = Size::new(40, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let draw = |renderer: &mut Gles, passes: Vec<Pass>| {
        let mut layers = vec![
            fill(Rect::new(0, 0, 20, 32), colour([30, 60, 160])),
            fill(Rect::new(20, 0, 20, 32), colour([240, 230, 210])),
        ];
        layers.push(clear(Rect::new(8, 7, 24, 16), 3.0, passes));
        render(renderer, &frame_of(size, layers))
    };
    let listed = draw(&mut renderer, one_of_each());
    let mut reversed = one_of_each();
    reversed.reverse();
    let backwards = draw(&mut renderer, reversed);
    for y in 0..32 {
        for x in 0..40 {
            assert_eq!(
                probe(&listed, x, y),
                probe(&backwards, x, y),
                "the pixel at {x},{y} depends on the order the effects were listed in"
            );
        }
    }
    assert_ne!(
        rgb(&listed, 20, 15),
        rgb(&draw(&mut renderer, Vec::new()), 20, 15),
        "and the effects drew something"
    );
}

#[test]
fn a_translucent_background_tints_the_blur_under_it() {
    let size = Size::new(24, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let backdrop = [240, 230, 210];
    let image = render(
        &mut renderer,
        &frame_of(
            size,
            vec![
                fill(Rect::from_size(size), colour(backdrop)),
                clear(
                    Rect::new(4, 4, 16, 16),
                    0.0,
                    vec![
                        Pass::Background {
                            color: Color::from_rgba8(200, 40, 40, 128),
                        },
                        Pass::Blur(glass(1, 0.5)),
                    ],
                ),
            ],
        ),
    );
    // Half of the fill over the backdrop, whichever order the two were listed in.
    let want = [0, 1, 2].map(|i| ((f32::from([200u8, 40, 40][i]) + f32::from(backdrop[i])) / 2.0).round() as u8);
    assert_close(rgb(&image, 12, 12), want, 3, "the background is over the blur");
}

#[test]
fn an_inset_shadow_takes_the_layers_tint() {
    let size = Size::new(24, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let inset = Pass::Shadow(Shadow {
        offset: Vec3::ZERO,
        blur: 4.0,
        spread: 0.0,
        opacity: 1.0,
        color: Color::from_rgba8(0, 0, 0, 200),
        frame: false,
        inset: true,
    });
    let edge = |renderer: &mut Gles, tint: Color| {
        let mut solid = Solid::new(Rect::new(4, 4, 16, 16), Color::TRANSPARENT);
        solid.tint = tint;
        solid.effects = chain(vec![inset.clone()]);
        let image = render(
            renderer,
            &frame_of(
                size,
                vec![
                    fill(Rect::from_size(size), colour([255, 255, 255])),
                    Layer::Solid(solid),
                ],
            ),
        );
        rgb(&image, 4, 12)
    };
    let plain = edge(&mut renderer, Color::WHITE);
    let faded = edge(&mut renderer, Color::new(1.0, 1.0, 1.0, 0.5));
    // The edge pixel's centre is half a pixel inside, where the falloff has reached about 0.96 of the colour's alpha.
    assert_close(plain, [64, 64, 64], 4, "the shadow at the edge, untinted");
    assert_close(faded, [159, 159, 159], 4, "a tint of half alpha halves the shadow");
}

fn filtered(blur: f32) -> Vec<Pass> {
    vec![Pass::Filter(FilterParams {
        blur,
        ..FilterParams::default()
    })]
}

fn orange_with(passes: Vec<Pass>) -> Layer {
    let mut solid = Solid::new(Rect::new(10, 6, 16, 10), colour([230, 120, 30]));
    solid.effects = chain(passes);
    Layer::Solid(solid)
}

#[test]
fn a_filter_blur_keeps_what_hangs_off_the_output_where_it_was() {
    let backdrop = colour([240, 230, 210]);
    let Some(mut wide) = gles(Size::new(48, 24)) else {
        return;
    };
    let Some(mut narrow) = gles(Size::new(34, 24)) else {
        return;
    };
    let layer = |x: i32| {
        let mut solid = Solid::new(Rect::new(x, 6, 16, 10), colour([230, 120, 30]));
        solid.effects = chain(filtered(4.0));
        Layer::Solid(solid)
    };
    let whole = render(
        &mut wide,
        &frame_of(
            Size::new(48, 24),
            vec![fill(Rect::new(0, 0, 48, 24), backdrop), layer(10)],
        ),
    );
    // The same scene moved fourteen pixels left, so four columns of the layer are off the output.
    let cut = render(
        &mut narrow,
        &frame_of(
            Size::new(34, 24),
            vec![fill(Rect::new(0, 0, 34, 24), backdrop), layer(-4)],
        ),
    );
    for y in 0..24 {
        for x in 0..34 {
            assert_close(
                rgb(&cut, x, y),
                rgb(&whole, x + 14, y),
                2,
                &format!("a layer cut by the output's edge, at {x},{y}"),
            );
        }
    }
    assert!(rgb(&cut, 0, 10)[1] < 200, "the cut layer is there");
}

#[test]
fn a_filter_blur_paints_only_the_damage() {
    let size = Size::new(40, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 18, 24));
    frame.push_layer(orange_with(filtered(4.0)));
    let image = render(&mut renderer, &frame);
    assert_ne!(probe(&image, 12, 10), [0, 0, 0, 0], "the damaged part is drawn");
    assert_eq!(
        probe(&image, 24, 10),
        [0, 0, 0, 0],
        "and the undamaged part keeps what it had"
    );
}

#[test]
fn a_filter_blur_fades_with_the_layer() {
    let size = Size::new(40, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let pale = [240, 230, 210];
    let mut pixels = Vec::new();
    for _ in 0..16 * 10 {
        pixels.extend_from_slice(&[230, 120, 30, 255]);
    }
    let picture = Arc::new(Image::new(Size::new(16, 10), 64, pixels).expect("an image that holds its rows"));
    let layer = |opacity: f32| {
        Layer::Surface(Surface {
            source: SurfaceSource::Snapshot(picture.clone()),
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 10.0,
            },
            dst: Rect::new(10, 6, 16, 10),
            transform: smithay::utils::Transform::Normal,
            opacity,
            tint: Color::WHITE,
            corner_radius: 0.0,
            silhouette: None,
            effects: chain(filtered(2.0)),
            model: None,
            secure: false,
            content: None,
        })
    };
    let image = render(
        &mut renderer,
        &frame_of(size, vec![fill(Rect::from_size(size), colour(pale)), layer(0.5)]),
    );
    let want = [0, 1, 2].map(|i| ((f32::from([230u8, 120, 30][i]) + f32::from(pale[i])) / 2.0).round() as u8);
    assert_close(
        rgb(&image, 18, 11),
        want,
        4,
        "half the blurred window over the backdrop",
    );
}

#[test]
fn the_pictures_a_filter_leases_are_given_back_every_frame() {
    let size = Size::new(40, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let frame = frame_of(size, vec![orange_with(filtered(3.0))]);
    let mut held = Vec::new();
    for _ in 0..6 {
        render(&mut renderer, &frame);
        held.push(renderer.device.spare_bytes());
    }
    assert!(held[1] > 0, "the picture and its sweep target wait in the pool");
    assert_eq!(held[1], held[5], "and a steady frame asks for no more: {held:?}");
}

/// A shader that returns the node's colour as it is.
const KEEP: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    return c;
}";

#[test]
fn a_grade_is_put_on_what_a_shader_pass_drew() {
    let size = Size::new(24, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let shader = || Pass::Shader(ShaderPass::new(KEEP, &[], false));
    let grey = Pass::Filter(FilterParams {
        saturate: 0.0,
        ..FilterParams::default()
    });
    let mut layer = |passes: Vec<Pass>| {
        let mut solid = Solid::new(Rect::new(4, 4, 16, 16), colour([255, 0, 0]));
        solid.effects = chain(passes);
        render(&mut renderer, &frame_of(size, vec![Layer::Solid(solid)]))
    };
    assert_close(
        rgb(&layer(vec![shader()]), 12, 12),
        [255, 0, 0],
        3,
        "the shader keeps the red",
    );
    assert_close(
        rgb(&layer(vec![shader(), grey]), 12, 12),
        [54, 54, 54],
        4,
        "and the filter takes it to its luma",
    );
}

#[test]
fn glass_and_a_filter_blur_on_one_layer_draw_both() {
    let size = Size::new(40, 24);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut solid = Solid::new(Rect::new(10, 6, 16, 10), colour([230, 120, 30]));
    solid.effects = chain(vec![Pass::Blur(glass(1, 0.5)), filtered(3.0)[0].clone()]);
    let image = render(
        &mut renderer,
        &frame_of(
            size,
            vec![
                fill(Rect::new(0, 0, 20, 24), colour([30, 60, 160])),
                fill(Rect::new(20, 0, 20, 24), colour([240, 230, 210])),
                Layer::Solid(solid),
            ],
        ),
    );
    assert_close(
        rgb(&image, 18, 11),
        [230, 120, 30],
        10,
        "the layer's own colour is over the glass",
    );
    assert!(rgb(&image, 8, 11)[0] > 36, "its blur spills past the edge");
    assert_eq!(
        rgb(&image, 2, 2),
        [30, 60, 160],
        "and the backdrop far away is untouched"
    );
}
