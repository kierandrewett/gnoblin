//! Tests of the Vulkan renderer.
//!
//! Every test that draws needs a Vulkan device, and the machines these run on may have none:
//! the instances the lane is gated on have no GPU and use the software driver `lavapipe`, and a
//! developer's laptop may have nothing at all. A test that cannot create a renderer because
//! there is no loader or no device prints why and returns, so the workspace's tests pass on a
//! machine without Vulkan. Any other failure to create one is a test failure.
//!
//! # Parity with the software renderer
//!
//! A frame is drawn by [`Software`] and by [`Vulkan`] and the two readbacks are compared pixel
//! by pixel. The tolerance is stated per test:
//!
//! - **0** where both renderers only copy or fill: opaque solids, an unscaled opaque surface,
//!   a bitmap drawn one to one.
//! - **[`BLEND`]** (1 per channel) where a blend or a multiplication rounds: Pixman works in
//!   8-bit integers and the GPU in floating point, so the two can land one step apart.
//!
//! The validation layer is on whenever it is installed, and a drawing test fails on anything it
//! reports.

use ash::vk;
use gnoblin_render::Effects;
use gnoblin_types::{Color, Rect, RectF, Scale, Size};
use smithay::utils::Transform;

use super::harness::{BufferSpec, ShmClient};
use super::pipeline::ShaderSources;
use super::*;
use gnoblin_render_frame::frame::{Layer, Model, Overlay, Solid, Surface};
use gnoblin_render_frame::image::Image;
use gnoblin_render_software::Software;

/// The largest per-channel difference accepted where a blend rounds.
const BLEND: u8 = 1;

/// A renderer, or [`None`] with a printed reason when this machine has no Vulkan.
fn renderer(size: Size) -> Option<Vulkan> {
    match Vulkan::new(size) {
        Ok(renderer) => {
            println!(
                "Vulkan device: {} ({:?}), validation layer {}",
                renderer.device().name,
                renderer.device().kind,
                if renderer.validation_enabled() { "on" } else { "off" }
            );
            Some(renderer)
        }
        Err(error @ (VulkanError::NoLoader { .. } | VulkanError::NoDevice { .. })) => {
            println!("skipping: {error}; there is no Vulkan device on this machine");
            None
        }
        Err(error) => panic!("the Vulkan renderer could not be created: {error}"),
    }
}

/// Fails if the validation layer reported anything.
fn clean(vulkan: &Vulkan) {
    let messages = vulkan.take_validation_messages();
    assert!(messages.is_empty(), "the validation layer reported: {messages:#?}");
}

fn software(size: Size) -> Software {
    Software::new(size).expect("a software renderer")
}

fn colour(red: u8, green: u8, blue: u8, alpha: u8) -> Color {
    Color::from_rgba8(red, green, blue, alpha)
}

/// A frame with a background and damage over the whole `size`.
fn frame(background: Color, size: Size) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(background);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    frame
}

fn solid(frame: &mut OutputFrame, rect: Rect, colour: Color) {
    frame.push_layer(Layer::Solid(Solid::new(rect, colour)));
}

/// A surface layer drawing `src` of `buffer` at `dst`.
fn surface(buffer: &ShmClient, index: usize, src: RectF, dst: Rect, opacity: f32, tint: Color) -> Layer {
    Layer::Surface(Surface {
        buffer: buffer.buffer(index),
        src,
        dst,
        transform: Transform::Normal,
        opacity,
        tint,
        corner_radius: 0.0,
        effects: Effects::new(),
        model: None,
    })
}

/// A bitmap of straight `[r, g, b, a]` pixels.
fn bitmap(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Image {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    Image::new(Size::new(width, height), width as usize * 4, data).expect("a bitmap")
}

const SIZE: Size = Size::new(64, 48);

/// The largest per-channel difference between two readbacks, and where it is.
fn worst_difference(left: &Image, right: &Image) -> (u8, (i32, i32)) {
    assert_eq!(left.size(), right.size(), "the readbacks are the same size");
    let size = left.size();
    let mut worst = (0, (0, 0));
    for y in 0..size.height as i32 {
        for x in 0..size.width as i32 {
            let a = left.colour_at(x, y).expect("a pixel");
            let b = right.colour_at(x, y).expect("a pixel");
            let difference = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            if difference > worst.0 {
                worst = (difference, (x, y));
            }
        }
    }
    worst
}

/// Asserts two readbacks agree to within `tolerance` per channel, and prints how far they do.
fn assert_close(what: &str, software: &Image, vulkan: &Image, tolerance: u8) {
    let (worst, at) = worst_difference(software, vulkan);
    println!("{what}: largest channel difference {worst} at {at:?} (tolerance {tolerance})");
    assert!(
        worst <= tolerance,
        "{what}: the renderers differ by {worst} at {at:?}: software {:?}, vulkan {:?}",
        software.colour_at(at.0, at.1),
        vulkan.colour_at(at.0, at.1)
    );
}

/// Draws `frames` in order on both renderers at `scale` and compares what each captures.
fn compare(what: &str, size: Size, scale: Scale, frames: &[OutputFrame], tolerance: u8) -> Option<(Image, Image)> {
    let mut vulkan = renderer(size)?;
    let mut software = software(size);
    for frame in frames {
        software.draw(frame, scale).expect("the software frame");
        vulkan.draw(frame, scale).expect("the Vulkan frame");
    }
    let expected = software.capture(true).expect("a software readback");
    let actual = vulkan.capture(true).expect("a Vulkan readback");
    assert_close(what, &expected, &actual, tolerance);
    clean(&vulkan);
    Some((expected, actual))
}

#[test]
fn solid_colours_match_the_software_renderer() {
    let mut frame = frame(colour(10, 20, 30, 255), SIZE);
    solid(&mut frame, Rect::new(2, 2, 20, 20), colour(255, 0, 0, 255));
    solid(&mut frame, Rect::new(12, 12, 20, 20), colour(0, 255, 0, 255));
    solid(&mut frame, Rect::new(40, 4, 10, 30), colour(0, 0, 255, 255));
    compare("solid colours", SIZE, Scale::IDENTITY, &[frame], 0);
}

#[test]
fn overlapping_translucent_layers_match_the_software_renderer() {
    let mut frame = frame(colour(200, 100, 50, 255), SIZE);
    solid(&mut frame, Rect::new(4, 4, 30, 30), colour(255, 0, 0, 255));
    solid(&mut frame, Rect::new(16, 10, 30, 30), colour(0, 255, 0, 128));
    solid(&mut frame, Rect::new(24, 20, 30, 24), colour(0, 0, 255, 64));
    compare(
        "two overlapping translucent layers",
        SIZE,
        Scale::IDENTITY,
        &[frame],
        BLEND,
    );
}

/// A pattern whose every pixel differs, so a wrong crop, offset or channel order shows.
fn pattern(x: i32, y: i32) -> [u8; 4] {
    [
        (x * 15 % 256) as u8,
        (y * 15 % 256) as u8,
        ((x + y) * 7 % 256) as u8,
        255,
    ]
}

#[test]
fn a_textured_surface_matches_the_software_renderer() {
    let client = ShmClient::new(vec![BufferSpec::argb(16, 16, pattern)]);
    let mut frame = frame(colour(10, 20, 30, 255), SIZE);
    // Whole, and a crop of the middle placed elsewhere.
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(0.0, 0.0, 16.0, 16.0),
        Rect::new(8, 4, 16, 16),
        1.0,
        Color::WHITE,
    ));
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(4.0, 4.0, 8.0, 8.0),
        Rect::new(40, 20, 8, 8),
        1.0,
        Color::WHITE,
    ));
    compare(
        "a textured surface and a crop of it",
        SIZE,
        Scale::IDENTITY,
        &[frame],
        0,
    );
}

#[test]
fn an_xrgb_buffer_is_drawn_opaque() {
    // The fourth byte of an xrgb pixel is padding. Here it is zero, which would make every
    // pixel transparent if it were read as alpha.
    let client = ShmClient::new(vec![BufferSpec::xrgb(16, 16, |x, y| {
        let [red, green, blue, _] = pattern(x, y);
        [red, green, blue, 0]
    })]);
    let mut frame = frame(colour(90, 90, 90, 255), SIZE);
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(0.0, 0.0, 16.0, 16.0),
        Rect::new(8, 8, 16, 16),
        1.0,
        Color::WHITE,
    ));
    let (_, vulkan) = compare("an xrgb surface", SIZE, Scale::IDENTITY, &[frame], 0).unzip();
    if let Some(vulkan) = vulkan {
        assert_eq!(
            vulkan.colour_at(8, 8),
            Some([0, 0, 0, 255]),
            "the first pixel is opaque"
        );
    }
}

#[test]
fn a_translucent_buffer_blends_with_what_is_behind_it() {
    // Premultiplied, as a shm buffer's pixels are: no channel exceeds the alpha.
    let client = ShmClient::new(vec![BufferSpec::argb(16, 16, |x, y| {
        let alpha = ((x + y) * 8).min(255) as u8;
        [alpha / 2, alpha / 3, alpha / 4, alpha]
    })]);
    let mut frame = frame(colour(40, 80, 120, 255), SIZE);
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(0.0, 0.0, 16.0, 16.0),
        Rect::new(8, 8, 16, 16),
        1.0,
        Color::WHITE,
    ));
    compare("a translucent buffer", SIZE, Scale::IDENTITY, &[frame], BLEND);
}

#[test]
fn a_layer_opacity_of_a_half_matches_the_software_renderer() {
    let client = ShmClient::new(vec![BufferSpec::argb(16, 16, pattern)]);
    let mut frame = frame(colour(200, 50, 50, 255), SIZE);
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(0.0, 0.0, 16.0, 16.0),
        Rect::new(8, 8, 16, 16),
        0.5,
        Color::WHITE,
    ));
    compare("a surface at opacity 0.5", SIZE, Scale::IDENTITY, &[frame], BLEND);
}

#[test]
fn a_tint_matches_the_software_renderer() {
    let client = ShmClient::new(vec![BufferSpec::argb(16, 16, pattern)]);
    let mut frame = frame(colour(20, 20, 20, 255), SIZE);
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(0.0, 0.0, 16.0, 16.0),
        Rect::new(8, 8, 16, 16),
        1.0,
        Color::new(0.5, 1.0, 0.25, 1.0),
    ));
    compare("a tinted surface", SIZE, Scale::IDENTITY, &[frame], BLEND);
}

#[test]
fn the_overlay_and_the_cursor_match_the_software_renderer() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut software = software(SIZE);
    let frame = decorated(SIZE, 16);
    software.draw(&frame, Scale::IDENTITY).expect("the software frame");
    vulkan.draw(&frame, Scale::IDENTITY).expect("the Vulkan frame");
    // Both with the overlay drawn and with it left out, with and without the cursor.
    let composed = vulkan.read_target(false, true).expect("a composed readback");
    assert_close(
        "the overlay and the cursor, composed",
        &software.composed().expect("a composed readback"),
        &composed,
        BLEND,
    );
    for with_cursor in [false, true] {
        let expected = software.capture(with_cursor).expect("a software capture");
        let actual = vulkan.capture(with_cursor).expect("a Vulkan capture");
        assert_close(
            if with_cursor {
                "a capture with the cursor"
            } else {
                "a capture without the cursor"
            },
            &expected,
            &actual,
            BLEND,
        );
    }
    clean(&vulkan);
}

#[test]
fn overlapping_damage_blends_a_translucent_layer_once() {
    let mut frame = OutputFrame::new();
    frame.set_background(colour(255, 255, 255, 255));
    frame.push_damage(Rect::new(0, 0, 40, 40));
    frame.push_damage(Rect::new(20, 20, 40, 40));
    solid(&mut frame, Rect::new(0, 0, 64, 48), colour(0, 0, 0, 128));
    compare(
        "overlapping damage under a translucent layer",
        SIZE,
        Scale::IDENTITY,
        &[frame],
        BLEND,
    );
}

#[test]
fn a_scale_converts_the_frame_to_physical_pixels() {
    let size = Size::new(32, 32);
    let mut frame = OutputFrame::new();
    frame.push_damage(Rect::new(0, 0, 16, 16));
    solid(&mut frame, Rect::new(0, 0, 4, 4), colour(255, 0, 0, 255));
    let (_, vulkan) = compare("a layer at scale 2", size, Scale::integral(2), &[frame], 0).unzip();
    if let Some(vulkan) = vulkan {
        assert_eq!(
            vulkan.colour_at(7, 7),
            Some([255, 0, 0, 255]),
            "the layer covers 8 pixels"
        );
        assert_eq!(
            vulkan.colour_at(8, 8),
            Some([0, 0, 0, 255]),
            "and stops at the scaled edge"
        );
    }
}

/// A frame with the overlay and the cursor over a layer, the cursor's left edge at `cursor_x`.
///
/// The overlay covers x 10 to 22. A cursor at 16 sits on it, which is the case where a capture
/// without the overlay also restores the pixels under the cursor, as the other renderers do.
fn decorated(size: Size, cursor_x: i32) -> OutputFrame {
    let mut frame = frame(colour(30, 60, 90, 255), size);
    solid(&mut frame, Rect::new(4, 4, 40, 30), colour(255, 128, 0, 255));
    let overlay = bitmap(12, 12, |x, y| [200, 40, 40, (x * 20 + y * 5).min(255) as u8]);
    frame.push_layer(Layer::Overlay(
        Overlay::new(Rect::new(10, 10, 12, 12), &overlay).expect("an overlay"),
    ));
    let cursor = bitmap(6, 6, |x, y| if x == y { [255, 255, 255, 255] } else { [0, 0, 0, 160] });
    frame.set_cursor_image(Overlay::new(Rect::new(cursor_x, 14, 6, 6), &cursor));
    frame
}

#[test]
fn a_capture_leaves_the_overlay_out_and_the_cursor_when_asked() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    vulkan.draw(&decorated(SIZE, 30), Scale::IDENTITY).expect("the frame");
    let orange = Some([255, 128, 0, 255]);
    let without = vulkan.capture(false).expect("a capture");
    assert_eq!(without.colour_at(12, 12), orange, "the overlay is not in a capture");
    assert_eq!(
        without.colour_at(30, 14),
        orange,
        "the cursor is out unless it is asked for"
    );
    let with = vulkan.capture(true).expect("a capture");
    assert_eq!(with.colour_at(12, 12), orange, "the overlay is still out");
    assert_eq!(with.colour_at(30, 14), Some([255, 255, 255, 255]), "the cursor is in");
    let composed = vulkan.read_target(false, true).expect("a composed readback");
    assert_ne!(
        composed.colour_at(12, 12),
        orange,
        "the composed frame does show the overlay"
    );
    clean(&vulkan);
}

#[test]
fn damage_limits_what_is_repainted() {
    let Some(mut vulkan) = renderer(Size::new(16, 16)) else {
        return;
    };
    let whole = Rect::new(0, 0, 16, 16);
    let mut first = OutputFrame::new();
    first.set_background(colour(255, 0, 0, 255));
    first.push_damage(whole);
    solid(&mut first, whole, colour(0, 0, 255, 255));
    vulkan.draw(&first, Scale::IDENTITY).expect("the first frame");
    // The second frame repaints only the left half with a green background.
    let mut second = OutputFrame::new();
    second.set_background(colour(0, 255, 0, 255));
    second.push_damage(Rect::new(0, 0, 8, 16));
    vulkan.draw(&second, Scale::IDENTITY).expect("the second frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(2, 8),
        Some([0, 255, 0, 255]),
        "the damaged half is repainted"
    );
    assert_eq!(
        image.colour_at(12, 8),
        Some([0, 0, 255, 255]),
        "the undamaged half keeps the first frame"
    );
    clean(&vulkan);
}

#[test]
fn a_surface_scaled_up_is_filtered_and_covers_its_destination() {
    let client = ShmClient::new(vec![BufferSpec::argb(4, 4, |_, _| [20, 120, 220, 255])]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(surface(
        &client,
        0,
        RectF::new(0.0, 0.0, 4.0, 4.0),
        Rect::new(8, 8, 32, 32),
        1.0,
        Color::WHITE,
    ));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    for (x, y) in [(8, 8), (23, 23), (39, 39)] {
        assert_eq!(
            image.colour_at(x, y),
            Some([20, 120, 220, 255]),
            "inside the scaled surface at ({x}, {y})"
        );
    }
    assert_eq!(image.colour_at(40, 40), Some([0, 0, 0, 255]), "and nothing beyond it");
    clean(&vulkan);
}

#[test]
fn a_rounded_corner_shows_what_is_behind() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(colour(0, 0, 255, 255), SIZE);
    let mut rounded = Solid::new(Rect::new(8, 8, 32, 32), colour(255, 0, 0, 255));
    rounded.corner_radius = 12.0;
    frame.push_layer(Layer::Solid(rounded));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(8, 8),
        Some([0, 0, 255, 255]),
        "the corner is the background"
    );
    assert_eq!(
        image.colour_at(24, 24),
        Some([255, 0, 0, 255]),
        "the middle is the layer"
    );
    assert_eq!(
        image.colour_at(24, 8),
        Some([255, 0, 0, 255]),
        "the straight edge is the layer"
    );
    clean(&vulkan);
}

#[test]
fn a_readback_is_top_left_first() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    solid(&mut frame, Rect::new(0, 0, 8, 8), colour(255, 0, 0, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(1, 1),
        Some([255, 0, 0, 255]),
        "the drawn rectangle is at the top left, so the image is not upside down"
    );
    assert_eq!(
        image.colour_at(1, 46),
        Some([0, 0, 0, 255]),
        "and the bottom left is the background"
    );
    assert_eq!(
        image.colour_at(62, 1),
        Some([0, 0, 0, 255]),
        "and the top right is the background"
    );
    clean(&vulkan);
}

#[test]
fn an_empty_frame_draws_nothing() {
    let Some(mut vulkan) = renderer(Size::new(8, 8)) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(colour(1, 2, 3, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("nothing drawn");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(0, 0),
        Some([0, 0, 0, 0]),
        "an undamaged frame paints no pixel, not even the background"
    );
    clean(&vulkan);
}

#[test]
fn a_resize_replaces_the_target() {
    let Some(mut vulkan) = renderer(Size::new(8, 8)) else {
        return;
    };
    let mut frame = frame(colour(255, 0, 0, 255), Size::new(8, 8));
    solid(&mut frame, Rect::new(0, 0, 8, 8), colour(255, 0, 0, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the first frame");
    vulkan.resize(Size::new(32, 16)).expect("a bigger target");
    assert_eq!(vulkan.size(), Size::new(32, 16));
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(image.size(), Size::new(32, 16), "the readback follows the target");
    assert_eq!(
        image.colour_at(0, 0),
        Some([0, 0, 0, 0]),
        "a replaced target holds no pixels, so the caller damages everything"
    );
    assert_eq!(
        vulkan.resize(Size::new(0, 4)),
        Err(RenderError::Degenerate { width: 0, height: 4 })
    );
    clean(&vulkan);
}

#[test]
fn a_layer_with_a_model_is_not_drawn() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    let mut tilted = Solid::new(Rect::new(0, 0, 16, 16), colour(255, 0, 0, 255));
    tilted.model = Some(Model {
        matrix: gnoblin_render::Mat4::IDENTITY,
        opaque: true,
    });
    frame.push_layer(Layer::Solid(tilted));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(4, 4),
        Some([0, 0, 0, 255]),
        "the depth pass is not here, so the layer is left out"
    );
    clean(&vulkan);
}

#[test]
fn a_frame_with_more_textures_than_one_descriptor_pool_holds_is_drawn() {
    // 300 surfaces is more than the 256 sets of a pool, so the arena has to open a second one.
    let client = ShmClient::new(vec![BufferSpec::argb(2, 2, |_, _| [200, 100, 50, 255])]);
    let Some(mut vulkan) = renderer(Size::new(64, 64)) else {
        return;
    };
    let mut frame = frame(Color::BLACK, Size::new(64, 64));
    for index in 0..300 {
        frame.push_layer(surface(
            &client,
            0,
            RectF::new(0.0, 0.0, 2.0, 2.0),
            Rect::new((index % 30) * 2, (index / 30) * 2, 2, 2),
            1.0,
            Color::WHITE,
        ));
    }
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(image.colour_at(1, 1), Some([200, 100, 50, 255]), "the first surface");
    assert_eq!(image.colour_at(59, 19), Some([200, 100, 50, 255]), "the last surface");
    clean(&vulkan);
}

// Failure modes.

#[test]
fn a_missing_loader_is_a_named_error() {
    let options = VulkanOptions {
        loader: Loader::Library("/nonexistent/libvulkan-gnoblin-test.so".into()),
        ..VulkanOptions::default()
    };
    let error = Vulkan::with_options(SIZE, &options).expect_err("there is no such loader");
    assert!(matches!(error, VulkanError::NoLoader { .. }), "got {error}");
    // The caller falls back on the conversion, which says only that the renderer is not there.
    assert_eq!(error.into_render_error(), RenderError::Unavailable);
}

#[test]
fn an_unmatched_device_is_a_named_error() {
    let options = VulkanOptions {
        device: DeviceChoice::Named("no device has this name".to_owned()),
        ..VulkanOptions::default()
    };
    match Vulkan::with_options(SIZE, &options) {
        Err(VulkanError::NoDevice { reason }) => {
            println!("refused as expected: {reason}");
            assert!(!reason.is_empty(), "the reason names each refusal");
        }
        Err(VulkanError::NoLoader { .. }) => println!("skipping: there is no Vulkan loader on this machine"),
        Err(other) => panic!("expected no suitable device, got {other}"),
        Ok(_) => panic!("a device name nothing matches must not produce a renderer"),
    }
}

#[test]
fn a_broken_shader_is_a_startup_error() {
    let shared = ShaderSources::shared();
    let broken = ShaderSources {
        solid: "this is not WGSL",
        ..shared
    };
    // No device is asked: the passes are compiled first, so this runs on any machine.
    let error = Vulkan::with_sources(SIZE, &VulkanOptions::default(), &broken).expect_err("a broken pass");
    match error {
        VulkanError::Shader { pass, .. } => assert_eq!(pass, "solid"),
        other => panic!("expected a shader error, got {other}"),
    }
}

#[test]
fn a_lost_device_drops_the_frame_and_is_reported() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    solid(&mut frame, Rect::new(0, 0, 8, 8), colour(255, 0, 0, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("a frame before the loss");
    vulkan.context.faults.submit.set(Some(vk::Result::ERROR_DEVICE_LOST));
    assert_eq!(
        vulkan.draw(&frame, Scale::IDENTITY),
        Err(RenderError::Lost),
        "the frame is dropped and reported as lost"
    );
    assert!(vulkan.is_lost());
    // Nothing touches the device again: every call answers at once.
    assert_eq!(vulkan.draw(&frame, Scale::IDENTITY), Err(RenderError::Lost));
    assert_eq!(vulkan.capture(false).err(), Some(RenderError::Lost));
    assert_eq!(vulkan.resize(Size::new(16, 16)), Err(RenderError::Lost));
    drop(vulkan);
    // The caller recreates the renderer and carries on.
    let mut again = renderer(SIZE).expect("a new renderer");
    again
        .draw(&frame, Scale::IDENTITY)
        .expect("a frame on the new renderer");
    assert_eq!(
        again.screenshot().expect("a readback").colour_at(1, 1),
        Some([255, 0, 0, 255])
    );
    clean(&again);
}

#[test]
fn a_texture_that_cannot_be_allocated_skips_its_layer() {
    let client = ShmClient::new(vec![
        BufferSpec::argb(8, 8, |_, _| [255, 0, 0, 255]),
        BufferSpec::argb(8, 8, |_, _| [0, 255, 0, 255]),
    ]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    for (index, x) in [(0, 0), (1, 16)] {
        frame.push_layer(surface(
            &client,
            index,
            RectF::new(0.0, 0.0, 8.0, 8.0),
            Rect::new(x, 0, 8, 8),
            1.0,
            Color::WHITE,
        ));
    }
    // The first texture runs out of memory; the second is made.
    vulkan
        .context
        .faults
        .texture_allocation
        .set(Some((0, vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)));
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("the frame is drawn without the first layer");
    assert!(
        !vulkan.is_lost(),
        "running out of memory for a texture is not a lost device"
    );
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(2, 2),
        Some([0, 0, 0, 255]),
        "the layer without a texture is skipped"
    );
    assert_eq!(
        image.colour_at(18, 2),
        Some([0, 255, 0, 255]),
        "the other layer is drawn"
    );
    clean(&vulkan);
}

#[test]
fn a_destroyed_buffer_skips_its_layer() {
    let mut client = ShmClient::new(vec![
        BufferSpec::argb(8, 8, |_, _| [255, 0, 0, 255]),
        BufferSpec::argb(8, 8, |_, _| [0, 255, 0, 255]),
    ]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    for (index, x) in [(0, 0), (1, 16)] {
        frame.push_layer(surface(
            &client,
            index,
            RectF::new(0.0, 0.0, 8.0, 8.0),
            Rect::new(x, 0, 8, 8),
            1.0,
            Color::WHITE,
        ));
    }
    client.destroy(0);
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("the frame is drawn without the destroyed buffer");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(2, 2),
        Some([0, 0, 0, 255]),
        "the destroyed buffer's layer is skipped"
    );
    assert_eq!(
        image.colour_at(18, 2),
        Some([0, 255, 0, 255]),
        "the live buffer is drawn"
    );
    clean(&vulkan);
}

#[test]
#[allow(unsafe_code)]
fn validation_reports_a_driver_misuse() {
    let Some(vulkan) = renderer(SIZE) else { return };
    if !vulkan.validation_enabled() {
        println!("skipping: the validation layer is not installed or this is not a debug build");
        return;
    }
    // A buffer of no bytes is invalid usage, which the layer reports as an error.
    let info = vk::BufferCreateInfo::default()
        .size(0)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    // SAFETY: the call is the deliberate misuse; whatever the driver returns is destroyed.
    unsafe {
        if let Ok(buffer) = vulkan.context.device.create_buffer(&info, None) {
            vulkan.context.device.destroy_buffer(buffer, None);
        }
    }
    let messages = vulkan.take_validation_messages();
    assert!(
        !messages.is_empty(),
        "the layer must report a zero-sized buffer, or no test here can fail on validation"
    );
    println!("the layer said: {}", messages[0]);
}
