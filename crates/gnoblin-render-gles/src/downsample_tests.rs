//! The downsample pass on a real GLES context: the pixels a supersampled output shows.
//!
//! The expectation in every test is the software renderer's: the same frame drawn by [`Software`] at the virtual size
//! and averaged by [`Image::downsample`], which is what a headless output shows. A machine without a GLES context
//! prints the reason and skips, as the rest of this crate's tests do. These run on llvmpipe in CI and on whatever
//! rasteriser a developer has; a GPU rounds a float mean on its own terms, so a comparison over a whole picture allows
//! one level per channel and says so, while the two probes the pass is for (the checker and the one-pixel line) are
//! exact.

use super::tests::{gles, probe};
use super::*;
use gnoblin_render_frame::frame::Solid;
use gnoblin_render_software::Software;

/// A frame of `size` pixels, black, with a white 1-pixel cell wherever `x + y` is even: the worst case for an
/// average, which has to give the mean of black and white and not a copy of either.
fn checker(size: Size) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    for y in 0..size.height {
        for x in 0..size.width {
            if (x + y) % 2 == 0 {
                frame.push_layer(Layer::Solid(Solid::new(
                    Rect::new(x as i32, y as i32, 1, 1),
                    Color::WHITE,
                )));
            }
        }
    }
    frame
}

/// A frame of `size` pixels, black, with a red line one pixel wide at column `x`.
fn line(size: Size, x: i32) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(x, 0, 1, size.height),
        Color::from_rgba8(200, 0, 0, 255),
    )));
    frame
}

/// What the software renderer shows of `frame` composed at `virtual_size` and displayed at `shown`.
fn software_shows(frame: &OutputFrame, scale: Scale, virtual_size: Size, shown: Size) -> Image {
    let mut software = Software::new(virtual_size).expect("a software renderer");
    software.draw(frame, scale).expect("drawn");
    software.screenshot().expect("a readback").downsample(shown)
}

/// The most any channel of any pixel differs between two pictures of the same size.
fn largest_difference(a: &Image, b: &Image) -> u8 {
    assert_eq!(a.size(), b.size(), "pictures of different sizes cannot be compared");
    a.data()
        .iter()
        .zip(b.data())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

#[test]
fn a_checker_at_twice_the_size_is_the_mean_of_black_and_white() {
    let virtual_size = Size::new(16, 16);
    let shown = Size::new(8, 8);
    let Some(mut renderer) = gles(virtual_size) else {
        return;
    };
    let frame = checker(virtual_size);
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let picture = renderer.downsampled(shown).expect("the averaged picture");
    assert_eq!(picture.size(), shown);
    for (x, y) in [(0, 0), (3, 4), (7, 7)] {
        // Each 2x2 block holds two white and two black pixels: (255 + 255 + 0 + 0) / 4 = 127.5, rounded half up.
        assert_eq!(probe(&picture, x, y), [128, 128, 128, 255], "({x}, {y})");
    }
    let reference = software_shows(&frame, Scale::IDENTITY, virtual_size, shown);
    assert_eq!(
        picture.data(),
        reference.data(),
        "the GLES average is the software one on {}",
        renderer.driver()
    );
}

#[test]
fn a_one_pixel_line_is_its_coverage_at_twice_and_four_times_the_size() {
    for factor in [2u32, 4] {
        let shown = Size::new(8, 4);
        let virtual_size = Size::new(shown.width * factor, shown.height * factor);
        let Some(mut renderer) = gles(virtual_size) else {
            return;
        };
        // A line one virtual pixel wide covers 1 / factor of the display pixel it crosses.
        let frame = line(virtual_size, 5);
        renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
        let picture = renderer.downsampled(shown).expect("the averaged picture");
        let column = (5 / factor) as i32;
        let expected = (200 + factor / 2) / factor;
        assert_eq!(
            probe(&picture, column, 1),
            [expected as u8, 0, 0, 255],
            "a line of one virtual pixel is 1/{factor} of its display pixel"
        );
        assert_eq!(
            probe(&picture, column + 1, 1),
            [0, 0, 0, 255],
            "and the next pixel is untouched"
        );
        let reference = software_shows(&frame, Scale::IDENTITY, virtual_size, shown);
        assert_eq!(picture.data(), reference.data(), "factor {factor}");
    }
}

#[test]
fn a_factor_that_is_not_whole_is_sampled_bilinearly_like_the_cpu() {
    // 1.5: a 12x9 surface shown at 8x6.
    let virtual_size = Size::new(12, 9);
    let shown = Size::new(8, 6);
    let Some(mut renderer) = gles(virtual_size) else {
        return;
    };
    let frame = line(virtual_size, 6);
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let picture = renderer.downsampled(shown).expect("the averaged picture");
    let reference = software_shows(&frame, Scale::IDENTITY, virtual_size, shown);
    // The driver's bilinear weights have a few bits of fraction, so two levels of 255 is the allowance.
    let difference = largest_difference(&picture, &reference);
    assert!(
        difference <= 2,
        "the GLES bilinear sample is within 2 levels of the CPU's, not {difference}, on {}",
        renderer.driver()
    );
    assert_eq!(probe(&picture, 0, 2), [0, 0, 0, 255], "far from the line stays black");
}

#[test]
fn the_same_size_is_a_copy() {
    let size = Size::new(8, 8);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let frame = checker(size);
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let picture = renderer.downsampled(size).expect("the picture");
    assert_eq!(picture.data(), renderer.screenshot().expect("a readback").data());
}

/// A scan-out buffer of `size` over a texture the context made, standing in for the GBM buffer of a real output,
/// filled with opaque red so a pixel the pass does not write is seen.
fn scan_buffer(renderer: &mut Gles, size: Size) -> Option<ScanBuffer> {
    let red: Vec<u8> = (0..size.width * size.height).flat_map(|_| [255, 0, 0, 255]).collect();
    let texture = renderer
        .device
        .renderer()
        .import_memory(
            &red,
            READBACK_FORMAT,
            (size.width as i32, size.height as i32).into(),
            false,
        )
        .ok()?;
    let framebuffer = renderer.device.framebuffer_over(&texture).ok()?;
    Some(ScanBuffer {
        framebuffer,
        _texture: texture,
    })
}

/// The pixels of a scan-out buffer.
fn buffer_picture(renderer: &mut Gles, buffer: &ScanBuffer, size: Size) -> Image {
    let mut bytes = renderer.device.read(&buffer.framebuffer).expect("a readback");
    let stride = size.width as usize * 4;
    flip_rows(&mut bytes, stride);
    Image::new(size, stride, bytes).expect("a packed image")
}

#[test]
fn a_supersampled_frame_is_averaged_into_the_scan_out_buffer() {
    // A 16x8 mode at supersample 2 and scale 1: composed at 32x16, compositing scale 2, shown at 16x8.
    let (shown, virtual_size) = (Size::new(16, 8), Size::new(32, 16));
    let Some(mut renderer) = gles(virtual_size) else {
        return;
    };
    let Some(mut buffer) = scan_buffer(&mut renderer, shown) else {
        return;
    };
    let rectangle = |damage: Rect| {
        let mut frame = OutputFrame::new();
        frame.set_background(Color::BLACK);
        frame.push_damage(damage);
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(3, 1, 4, 3),
            Color::from_rgba8(10, 200, 90, 255),
        )));
        frame
    };
    // The damage names only a corner: the target nobody has painted is painted whole the first time all the same.
    let frame = rectangle(Rect::new(0, 0, 2, 2));
    let scale = Scale::integral(2);
    renderer
        .draw_scanout_downsampled(&mut buffer, &frame, scale)
        .expect("drawn");
    let picture = buffer_picture(&mut renderer, &buffer, shown);

    assert_eq!(
        probe(&picture, 4, 2),
        [10, 200, 90, 255],
        "a rectangle on whole display pixels keeps its colour"
    );
    assert_eq!(
        probe(&picture, 12, 6),
        [0, 0, 0, 255],
        "and the rest of the first frame is the background everywhere, not the red the buffer held"
    );
    let reference = software_shows(&rectangle(Rect::new(0, 0, 16, 8)), scale, virtual_size, shown);
    assert!(
        largest_difference(&picture, &reference) <= 1,
        "the scan-out picture is the software average on {}",
        renderer.driver()
    );

    // A second frame that damages nothing still averages into the buffer, because a flip buffer holds an old frame.
    let Some(mut fresh) = scan_buffer(&mut renderer, shown) else {
        return;
    };
    let mut idle = OutputFrame::new();
    idle.set_background(Color::BLACK);
    renderer
        .draw_scanout_downsampled(&mut fresh, &idle, scale)
        .expect("drawn");
    let again = buffer_picture(&mut renderer, &fresh, shown);
    assert_eq!(
        again.data(),
        picture.data(),
        "the persistent target keeps the frame, so an idle flip shows it in a buffer that never saw it"
    );
}

#[test]
fn a_damaged_frame_repaints_only_its_damage_in_the_virtual_target() {
    let (shown, virtual_size) = (Size::new(8, 8), Size::new(16, 16));
    let Some(mut renderer) = gles(virtual_size) else {
        return;
    };
    let Some(mut buffer) = scan_buffer(&mut renderer, shown) else {
        return;
    };
    let scale = Scale::integral(2);
    let mut first = OutputFrame::new();
    first.set_background(Color::from_rgba8(0, 0, 255, 255));
    first.push_damage(Rect::new(0, 0, 8, 8));
    renderer
        .draw_scanout_downsampled(&mut buffer, &first, scale)
        .expect("the first frame");

    let mut second = OutputFrame::new();
    second.set_background(Color::from_rgba8(0, 255, 0, 255));
    // Logical x 0..4 is physical x 0..8 at scale 2.
    second.push_damage(Rect::new(0, 0, 4, 8));
    renderer
        .draw_scanout_downsampled(&mut buffer, &second, scale)
        .expect("the second frame");
    let picture = buffer_picture(&mut renderer, &buffer, shown);
    assert_eq!(probe(&picture, 1, 3), [0, 255, 0, 255], "the damaged half is repainted");
    assert_eq!(
        probe(&picture, 6, 3),
        [0, 0, 255, 255],
        "the other half keeps the first frame"
    );
}

#[test]
fn a_target_past_the_drivers_limit_is_refused_and_the_old_one_stays() {
    let Some(mut renderer) = gles(Size::new(8, 8)) else {
        return;
    };
    let limit = renderer.max_texture_size();
    assert!(limit >= 2048, "GLES 3 guarantees 2048, got {limit}");
    let frame = checker(Size::new(8, 8));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let before = renderer.screenshot().expect("a readback");
    assert_eq!(
        renderer.resize(Size::new(limit + 1, 8)),
        Err(RenderError::Unavailable),
        "one pixel past {limit} is refused"
    );
    assert_eq!(renderer.size(), Size::new(8, 8), "the renderer keeps its size");
    assert_eq!(
        renderer.screenshot().expect("a readback").data(),
        before.data(),
        "and the frame that was on screen"
    );
    assert!(
        Gles::new(Size::new(8, limit + 1)).is_err(),
        "a renderer is not made over the limit either"
    );
}
