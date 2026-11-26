//! The GLES half of the shared golden scenes, and the shared shader's colour grade, grain, inset shadow and downsample
//! flag on a real GLES context.
//!
//! The golden scenes live in `gnoblin_render_frame::golden` so the Vulkan renderer is held to the same pictures. A
//! machine without a GLES context prints the reason and skips, as the rest of this crate's tests do.

use super::tests::{gles, probe};
use super::*;
use gnoblin_render_frame::frame::Solid;
use gnoblin_render_frame::golden;
use gnoblin_render_frame::grade::{self, Grade};
use gnoblin_render_frame::shader::uniforms::IDENTITY_GRADE;
use gnoblin_render_software::Software;

#[test]
fn the_shared_scenes_match_their_goldens_on_gles() {
    golden::check_all("GLES", |frame| {
        let mut renderer = gles(golden::SIZE)?;
        renderer.draw(frame, Scale::IDENTITY).expect("drawn");
        Some(renderer.screenshot().expect("a readback"))
    });
}

#[test]
fn the_scenes_the_software_renderer_draws_match_the_same_goldens() {
    for case in golden::CASES.iter().filter(|case| case.software) {
        let mut renderer = Software::new(golden::SIZE).expect("a software renderer");
        renderer.draw(&golden::frame_of(case), Scale::IDENTITY).expect("drawn");
        golden::check(case, "software", &renderer.screenshot().expect("a readback"));
    }
}

/// 256 straight colours, one pixel each, on a 16 by 16 grid.
fn colours() -> Vec<[u8; 3]> {
    (0..=255u32)
        .map(|i| [i as u8, (i * 7 % 256) as u8, (i * 13 % 256 + 40) as u8])
        .collect()
}

/// Draws `pass` over each of the `colours` on a 16 by 16 target and reads the picture back.
fn solid_grid(renderer: &mut Gles, grade: Grade) -> Image {
    let Gles {
        device, target, passes, ..
    } = renderer;
    let mut bytes = Vec::new();
    device.clear(target, Color::BLACK);
    device.begin(target).expect("bound");
    for (index, [r, g, b]) in colours().into_iter().enumerate() {
        let cell = Rect::new((index % 16) as i32, (index / 16) as i32, 1, 1);
        let mut uniforms = Uniforms::solid(cell, Color::from_rgba8(r, g, b, 255), Color::WHITE, 1.0);
        uniforms.grade = grade.to_array();
        passes.solid.pack(&uniforms, &mut bytes);
        device
            .draw(&passes.solid, &bytes, cell, None, QuadPlacement::FLAT)
            .expect("drawn");
    }
    device.end().expect("flushed");
    renderer.screenshot().expect("a readback")
}

#[test]
fn the_shader_grade_matches_the_cpu_grade_within_one_level() {
    let Some(mut renderer) = gles(Size::new(16, 16)) else {
        return;
    };
    for grade in [
        Grade::IDENTITY,
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
            saturate: 0.0,
            brightness: 2.0,
            contrast: 1.5,
            noise: 0.0,
        },
    ] {
        let picture = solid_grid(&mut renderer, grade);
        for (index, [r, g, b]) in colours().into_iter().enumerate() {
            let want = grade::apply(grade, Color::from_rgba8(r, g, b, 255));
            let want = [want.r, want.g, want.b].map(|channel| (channel * 255.0).round() as u8);
            let got = probe(&picture, (index % 16) as i32, (index / 16) as i32);
            assert!(
                got.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= 1),
                "{grade:?} on {:?}: the shader drew {got:?}, the CPU says {want:?}",
                [r, g, b]
            );
        }
    }
}

#[test]
fn an_identity_grade_leaves_every_pixel_as_it_was() {
    let Some(mut renderer) = gles(Size::new(16, 16)) else {
        return;
    };
    let picture = solid_grid(&mut renderer, Grade::IDENTITY);
    assert_eq!(Grade::IDENTITY.to_array(), IDENTITY_GRADE);
    for (index, [r, g, b]) in colours().into_iter().enumerate() {
        assert_eq!(
            probe(&picture, (index % 16) as i32, (index / 16) as i32),
            [r, g, b, 255]
        );
    }
}

#[test]
fn the_shader_grain_matches_the_cpu_grain_within_one_level() {
    let size = Size::new(16, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let grey = 128u8;
    let pixels = vec![grey; 16 * 16 * 4];
    let texture = renderer.device.texture((16, 16), &pixels).expect("a texture");
    let quad = Rect::new(0, 0, 16, 16);
    let Gles {
        device, target, passes, ..
    } = &mut renderer;
    let mut bytes = Vec::new();
    device.clear(target, Color::BLACK);
    device.begin(target).expect("bound");
    // A zero tap step reads the flat grey at every tap, so the blur is the grey itself and the grain is all that is added.
    let mut uniforms = Uniforms::surface(quad, [0.0, 0.0, 1.0, 1.0], Color::TRANSPARENT, 1.0, 0.0);
    uniforms.grade = [1.0, 1.0, 1.0, 1.0];
    passes.blur.pack(&uniforms, &mut bytes);
    device
        .draw(&passes.blur, &bytes, quad, Some(texture), QuadPlacement::FLAT)
        .expect("drawn");
    device.end().expect("flushed");
    let picture = renderer.screenshot().expect("a readback");
    for y in 0..16u32 {
        for x in 0..16u32 {
            let want = (f32::from(grey) / 255.0 + grade::grain(1.0, x, y)).clamp(0.0, 1.0);
            let want = (want * 255.0).round() as u8;
            let got = probe(&picture, x as i32, y as i32);
            assert!(
                got[..3].iter().all(|channel| channel.abs_diff(want) <= 2),
                "({x}, {y}): the shader drew {got:?}, the CPU grain says {want}"
            );
        }
    }
}

#[test]
fn an_inset_shadow_is_full_at_the_edge_and_gone_blur_pixels_inside() {
    let size = Size::new(24, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let program = program::link(&mut renderer.device, shaders::VERTEX, shaders::INSET_SHADOW).expect("a linked pass");
    let quad = Rect::new(0, 0, 24, 16);
    let Gles { device, target, .. } = &mut renderer;
    let mut bytes = Vec::new();
    device.clear(target, Color::from_rgba8(10, 20, 30, 255));
    device.begin(target).expect("bound");
    let uniforms = Uniforms::inset_shadow(quad, [0.0, 0.0], 6.0, 0.0, Color::from_rgba8(255, 0, 0, 255), 1.0, 6.0);
    program.pack(&uniforms, &mut bytes);
    device
        .draw(&program, &bytes, quad, None, QuadPlacement::FLAT)
        .expect("drawn");
    device.end().expect("flushed");
    let picture = renderer.screenshot().expect("a readback");
    assert!(
        probe(&picture, 12, 0)[0] > 235,
        "red at the top edge: {:?}",
        probe(&picture, 12, 0)
    );
    assert!(probe(&picture, 0, 8)[0] > 235, "red at the left edge");
    assert_eq!(
        probe(&picture, 12, 8),
        [10, 20, 30, 255],
        "nothing at the centre, 8 pixels from the nearest edge"
    );
    assert_eq!(
        probe(&picture, 0, 0),
        [10, 20, 30, 255],
        "nothing leaks past the rounded corner"
    );
    let falloff: Vec<u8> = (0..6).map(|y| probe(&picture, 12, y)[0]).collect();
    assert!(
        falloff.windows(2).all(|pair| pair[0] >= pair[1]),
        "it fades inwards: {falloff:?}"
    );
}

/// A frame whose top rows are bright and whose bottom rows are dark, so a flipped read shows.
fn top_light(size: Size) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, size.width, size.height / 4),
        Color::from_rgba8(240, 200, 120, 255),
    )));
    frame
}

#[test]
fn the_downsample_flip_flag_reads_the_rows_the_other_way_up() {
    for (virtual_size, shown) in [
        (Size::new(16, 16), Size::new(8, 8)),
        (Size::new(12, 12), Size::new(8, 8)),
    ] {
        let Some(mut renderer) = gles(virtual_size) else {
            return;
        };
        renderer.draw(&top_light(virtual_size), Scale::IDENTITY).expect("drawn");
        let mut run = |top_down: bool| {
            let mut block = downsample::DownsampleUniforms::new(virtual_size, shown).bytes();
            if top_down {
                block[24..28].copy_from_slice(&1.0_f32.to_ne_bytes());
            }
            let destination = renderer.shown_target(shown).expect("a target");
            let Gles {
                device, target, passes, ..
            } = &mut renderer;
            device.begin(&destination).expect("bound");
            device.scissor(None);
            device.blend_replace().expect("replace");
            device
                .draw(
                    &passes.downsample,
                    &block,
                    Rect::new(0, 0, shown.width, shown.height),
                    Some(target.texture()),
                    QuadPlacement::FLAT,
                )
                .expect("drawn");
            device.blend_premultiplied().expect("premultiplied");
            device.end().expect("flushed");
            let rows = device.read(&destination).expect("a readback");
            renderer.device.release(destination);
            rows
        };
        let as_gles_has_it = run(false);
        let flagged = run(true);
        let stride = shown.width as usize * 4;
        let reversed: Vec<u8> = as_gles_has_it.chunks(stride).rev().flatten().copied().collect();
        assert_eq!(flagged, reversed, "{virtual_size:?}: the flag only turns the rows over");
        assert_ne!(
            as_gles_has_it, flagged,
            "the scene is not symmetric, so the flip is visible"
        );
    }
}
