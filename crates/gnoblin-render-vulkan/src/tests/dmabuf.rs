//! Tests of dmabuf import, export and capture on the linear path (design decision 12).
//!
//! A dmabuf comes from `/dev/udmabuf` through Smithay's allocator (design 10.2, source 1). A machine without it, such as a
//! gVisor box, skips these loudly: they run in CI, where `matrix.yml` loads the `udmabuf` module. What a box can prove is the
//! pure part: the format table (`formats.rs`) and the fence ioctls' refusal of a file that is not a dmabuf (`external.rs`).
//!
//! The failure modes, before the tests: a channel order wrong for one of the four formats; a buffer that is not linear,
//! single-plane and offered is drawn as garbage instead of refused; an inverted buffer drawn upside down; a capture that
//! reads before the frame is drawn; a capture written in the wrong order.

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::udmabuf::UdmabufAllocator;
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::dmabuf::{read_image, write_image};

/// A dmabuf of `edge` pixels on a side in `fourcc`, or [`None`] with a printed reason when there is no source.
fn dmabuf(edge: u32, fourcc: Fourcc) -> Option<Dmabuf> {
    let mut allocator = match UdmabufAllocator::new() {
        Ok(allocator) => allocator,
        Err(error) => {
            println!(
                "skipping: /dev/udmabuf cannot be opened ({error}); this test needs a dmabuf source and runs in CI"
            );
            return None;
        }
    };
    match allocator.create_buffer(edge, edge, fourcc, &[Modifier::Linear]) {
        Ok(buffer) => Some(buffer),
        Err(error) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI is set but udmabuf refused a buffer ({error})"
            );
            println!("skipping: udmabuf refused a buffer ({error})");
            None
        }
    }
}

/// A frame drawing the whole of `dmabuf` at the origin as commit 1 of surface 1.
fn showing(dmabuf: &Dmabuf, edge: u32) -> OutputFrame {
    let mut layer = Surface {
        source: SurfaceSource::Dmabuf(dmabuf.clone()),
        src: RectF::new(0.0, 0.0, f64::from(edge), f64::from(edge)),
        dst: Rect::new(0, 0, edge, edge),
        transform: Transform::Normal,
        opacity: 1.0,
        tint: Color::WHITE,
        corner_radius: 0.0,
        silhouette: None,
        effects: Effects::new(),
        model: None,
        secure: false,
        content: None,
    };
    layer.content = Some(Arc::new(gnoblin_render_frame::frame::ContentKey {
        surface: 1,
        commit: 1,
        damage: None,
    }));
    let mut frame = frame(Color::BLACK, SIZE);
    frame.push_layer(Layer::Surface(layer));
    frame
}

/// The picture every test writes: red on the left half, a green top row.
fn picture(edge: u32) -> Image {
    bitmap(edge, edge, |x, y| {
        if y == 0 {
            [0, 255, 0, 255]
        } else if x < edge / 2 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    })
}

#[test]
fn a_linear_dmabuf_imports_and_draws() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    for fourcc in [Fourcc::Argb8888, Fourcc::Xrgb8888, Fourcc::Abgr8888, Fourcc::Xbgr8888] {
        let Some(buffer) = dmabuf(16, fourcc) else { return };
        write_image(&buffer, &picture(16)).expect("the picture is written in the buffer's byte order");
        vulkan.draw(&showing(&buffer, 16), Scale::IDENTITY).expect("the frame");
        let image = vulkan.screenshot().expect("a readback");
        assert_eq!(image.colour_at(2, 0), Some([0, 255, 0, 255]), "{fourcc:?}: the top row");
        assert_eq!(
            image.colour_at(2, 5),
            Some([255, 0, 0, 255]),
            "{fourcc:?}: the left half"
        );
        assert_eq!(
            image.colour_at(12, 5),
            Some([0, 0, 255, 255]),
            "{fourcc:?}: the right half"
        );
        assert_eq!(
            read_image(&buffer).expect("read back").colour_at(12, 5),
            Some([0, 0, 255, 255]),
            "{fourcc:?}"
        );
        // A new surface per format: the previous one's texture is not this buffer.
        vulkan.gpu().release_textures();
    }
    clean(&vulkan);
}

/// A capture of a part of the output lands in the part of the buffer the client asked for: buffer pixel `(x, y)` of the
/// region is output pixel `(from.x + x, from.y + y)`, and what does not fit either is not written.
#[test]
fn a_region_of_a_picture_lands_where_the_region_says() {
    use gnoblin_types::Point;
    let Some(buffer) = dmabuf(16, Fourcc::Argb8888) else {
        return;
    };
    write_image(&buffer, &bitmap(16, 16, |_, _| [9, 9, 9, 255])).expect("a background");
    let Some(vulkan) = renderer(SIZE) else { return };
    // The picture's pixel (x, y) is red where x < 8 and blue elsewhere; take its right half into the buffer's left half.
    let source = bitmap(16, 16, |x, _| if x < 8 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
    vulkan
        .copy_image_region_into_dmabuf(&source, &buffer, Rect::new(0, 0, 8, 16), Point::new(8, 0))
        .expect("the region is copied");
    let read = read_image(&buffer).expect("read back");
    assert_eq!(
        read.colour_at(2, 5),
        Some([0, 0, 255, 255]),
        "the left half holds the picture's right half"
    );
    assert_eq!(
        read.colour_at(12, 5),
        Some([9, 9, 9, 255]),
        "the rest of the buffer is untouched"
    );
    // A region that reaches past the picture writes only what the picture has, each pixel at the offset it was asked at:
    // buffer pixel x of this region is picture pixel 4 + x, so x = 2 and 3 are the picture's red, x = 4 to 11 its blue
    // (the right half of the buffer held the background until now), and x = 12 and up are past the picture's edge.
    vulkan
        .copy_image_region_into_dmabuf(&source, &buffer, Rect::new(2, 0, 14, 16), Point::new(4, 0))
        .expect("an overhanging region is copied in part");
    let read = read_image(&buffer).expect("read back");
    assert_eq!(
        read.colour_at(3, 5),
        Some([255, 0, 0, 255]),
        "a pixel keeps the offset it was asked at: buffer 3 is picture 7"
    );
    assert_eq!(
        read.colour_at(11, 5),
        Some([0, 0, 255, 255]),
        "the picture's last column is written: buffer 11 is picture 15"
    );
    assert_eq!(
        read.colour_at(12, 5),
        Some([9, 9, 9, 255]),
        "past the picture's edge nothing is"
    );
    clean(&vulkan);
}

#[test]
fn an_inverted_dmabuf_reverses_the_v_axis() {
    let Some(buffer) = dmabuf(16, Fourcc::Argb8888) else {
        return;
    };
    write_image(&buffer, &picture(16)).expect("the picture is written");
    let fd = buffer
        .handles()
        .next()
        .expect("a plane")
        .try_clone_to_owned()
        .expect("a copy of the descriptor");
    let mut builder = Dmabuf::builder_from_buffer(&buffer, DmabufFlags::Y_INVERT);
    assert!(builder.add_plane(fd, 0, buffer.strides().next().expect("a stride")));
    let inverted = builder.build().expect("an inverted buffer");
    assert!(inverted.y_inverted());
    let Some(mut vulkan) = renderer(SIZE) else { return };
    vulkan
        .draw(&showing(&inverted, 16), Scale::IDENTITY)
        .expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(2, 15),
        Some([0, 255, 0, 255]),
        "the buffer's top row is drawn at the bottom"
    );
    assert_eq!(
        image.colour_at(2, 0),
        Some([255, 0, 0, 255]),
        "and its bottom row at the top"
    );
    clean(&vulkan);
}

#[test]
fn a_refused_dmabuf_is_an_error_not_a_crash() {
    // A format the renderer does not offer: two bytes a pixel.
    let Some(buffer) = dmabuf(16, Fourcc::Rgb565) else {
        return;
    };
    let Some(mut vulkan) = renderer(SIZE) else { return };
    assert_eq!(vulkan.read_dmabuf(&buffer).err(), Some(RenderError::Unsupported));
    assert_eq!(
        vulkan.copy_frame_into_dmabuf(&buffer, false).err(),
        Some(RenderError::Unsupported)
    );
    // Drawn, the layer is skipped and the rest of the frame is.
    let mut frame = showing(&buffer, 16);
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(40, 30, 8, 8),
        colour(0, 255, 0, 255),
    )));
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("the frame is drawn without the refused layer");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(2, 2),
        Some([0, 0, 0, 255]),
        "the refused buffer is not drawn"
    );
    assert_eq!(image.colour_at(42, 32), Some([0, 255, 0, 255]), "the rest is");
    clean(&vulkan);
}

#[test]
fn a_frame_copied_into_a_dmabuf_reads_back_equal() {
    let Some(target) = dmabuf(64, Fourcc::Argb8888) else {
        return;
    };
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = frame(Color::BLACK, SIZE);
    solid(&mut frame, Rect::new(0, 0, 20, 10), colour(255, 0, 0, 255));
    solid(&mut frame, Rect::new(20, 10, 20, 10), colour(0, 0, 255, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    vulkan.copy_frame_into_dmabuf(&target, false).expect("the copy");
    let copied = vulkan.read_dmabuf(&target).expect("the buffer read back");
    let shown = vulkan.screenshot().expect("a readback");
    for point in [(2, 2), (25, 12), (50, 40)] {
        assert_eq!(
            copied.colour_at(point.0, point.1),
            shown.colour_at(point.0, point.1),
            "{point:?}"
        );
    }
    assert_eq!(copied.colour_at(2, 2), Some([255, 0, 0, 255]));
    clean(&vulkan);
}

#[test]
fn an_image_is_copied_into_a_dmabuf_and_read_back() {
    let Some(buffer) = dmabuf(16, Fourcc::Xbgr8888) else {
        return;
    };
    let Some(vulkan) = renderer(SIZE) else { return };
    vulkan.copy_image_into_dmabuf(&picture(16), &buffer).expect("the copy");
    assert_eq!(
        vulkan.read_dmabuf(&buffer).expect("read").colour_at(2, 5),
        Some([255, 0, 0, 255])
    );
}

#[test]
fn a_dmabufs_writer_fence_can_be_taken_and_waited_for() {
    let Some(buffer) = dmabuf(16, Fourcc::Argb8888) else {
        return;
    };
    let fd = buffer.handles().next().expect("a plane");
    crate::external::wait_for_writer(fd, Duration::from_secs(1)).expect("a buffer nobody writes is ready");
    match crate::external::export_sync_file(fd, crate::external::SYNC_READ) {
        Ok(sync_file) => {
            use std::os::fd::AsFd;
            crate::external::import_sync_file(fd, sync_file.as_fd(), crate::external::SYNC_WRITE)
                .expect("the fence goes back into the buffer");
        }
        // A kernel before 6.0 has neither ioctl, and the caller waits on the CPU: the refusal is the contract.
        Err(error) => println!("the kernel has no sync_file ioctls on a dmabuf ({error}); the CPU wait is used"),
    }
}
