#![allow(unsafe_code)]
//! Reading and writing the pixels of a linear dmabuf.
//!
//! A device with no `VK_EXT_image_drm_format_modifier` cannot import a buffer as an image, and a linear buffer needs none: it
//! is a plane of rows `stride` bytes apart that can be mapped (design decision 12). A client's dmabuf is read by mapping the
//! plane, telling the kernel the CPU is about to read (`DMA_BUF_IOCTL_SYNC`, which also waits for whoever is writing it, so
//! this is the implicit-sync wait of design 5.4), copying the rows a texture needs into the frame's staging buffer, and telling
//! the kernel the read is over. A capture is written the same way in reverse.
//!
//! A buffer the client marked `y_inverted` is read bottom row first, so the texture is the right way up and the layer is
//! drawn as any other.
//!
//! # Failure modes
//!
//! 1. **A buffer is read as something it is not.** Only a single linear plane in an offered format is accepted; anything else
//!    is [`DmabufError`] before a byte is read: `tests::a_refused_dmabuf_is_an_error_not_a_crash`.
//! 2. **A row reads past the mapping.** The stride, offset and height are checked against the mapping's length first.
//! 3. **A buffer is read while its writer is writing.** The read is bracketed by the kernel's sync ioctl:
//!    `tests::a_writer_fence_is_waited_before_sampling` needs a dmabuf source and runs in CI.

use std::fmt;

use gnoblin_render_frame::image::Image;
use gnoblin_types::{Point, Rect, Size};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::dmabuf::{DmabufMappingMode, DmabufSyncFlags};
use smithay::backend::allocator::{Buffer, Modifier};

use super::formats::{Layout, blue_first, layout};

/// Why a dmabuf cannot be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DmabufError {
    /// The format is not one the renderer offers.
    Format,
    /// The modifier is not linear: the buffer is tiled, or implicit.
    Modifier,
    /// The buffer has more than one plane.
    Planes,
    /// The buffer's rows or size do not fit its mapping.
    Layout,
    /// The plane could not be mapped or synchronised.
    Map(String),
}

impl fmt::Display for DmabufError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format => formatter.write_str("the dmabuf's format is not one the Vulkan renderer offers"),
            Self::Modifier => formatter.write_str("the dmabuf's modifier is not linear"),
            Self::Planes => formatter.write_str("the dmabuf has more than one plane"),
            Self::Layout => formatter.write_str("the dmabuf's rows do not fit its mapping"),
            Self::Map(reason) => write!(formatter, "the dmabuf could not be mapped: {reason}"),
        }
    }
}

impl std::error::Error for DmabufError {}

/// What a linear dmabuf is as a texture, checked: its size and layout, and whether it is read bottom row first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Linear {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub stride: usize,
    pub inverted: bool,
}

/// Checks that `dmabuf` is a single linear plane in an offered format.
///
/// # Errors
///
/// The [`DmabufError`] that names what is wrong.
pub(super) fn linear(dmabuf: &Dmabuf) -> Result<Linear, DmabufError> {
    if dmabuf.format().modifier != Modifier::Linear {
        return Err(DmabufError::Modifier);
    }
    if dmabuf.num_planes() != 1 {
        return Err(DmabufError::Planes);
    }
    let layout = layout(dmabuf.format().code).ok_or(DmabufError::Format)?;
    let size = dmabuf.size();
    let (width, height) = (
        u32::try_from(size.w)
            .ok()
            .filter(|w| *w > 0)
            .ok_or(DmabufError::Layout)?,
        u32::try_from(size.h)
            .ok()
            .filter(|h| *h > 0)
            .ok_or(DmabufError::Layout)?,
    );
    let stride = dmabuf.strides().next().ok_or(DmabufError::Layout)? as usize;
    if stride < width as usize * 4 {
        return Err(DmabufError::Layout);
    }
    Ok(Linear {
        width,
        height,
        layout,
        stride,
        inverted: dmabuf.y_inverted(),
    })
}

/// Maps the plane, brackets `access` with the kernel's sync ioctl and runs `body` on the mapped bytes.
fn with_mapping<T>(
    dmabuf: &Dmabuf,
    shape: &Linear,
    write: bool,
    body: impl FnOnce(*mut u8, usize) -> T,
) -> Result<T, DmabufError> {
    let mode = if write {
        DmabufMappingMode::READ | DmabufMappingMode::WRITE
    } else {
        DmabufMappingMode::READ
    };
    let mapping = dmabuf
        .map_plane(0, mode)
        .map_err(|error| DmabufError::Map(error.to_string()))?;
    let needed = shape.stride * (shape.height as usize - 1) + shape.width as usize * 4;
    if mapping.length() < needed {
        return Err(DmabufError::Layout);
    }
    let access = if write {
        DmabufSyncFlags::WRITE
    } else {
        DmabufSyncFlags::READ
    };
    // A kernel without the ioctl still lets the mapping be read; the bracket is what waits for a writer, so its absence is
    // logged and not fatal.
    if let Err(error) = dmabuf.sync_plane(0, DmabufSyncFlags::START | access) {
        crate::log::warn!(area = "render", %error, "a dmabuf could not be synchronised for the CPU");
    }
    let result = body(mapping.ptr().cast(), mapping.length());
    let _ = dmabuf.sync_plane(0, DmabufSyncFlags::END | access);
    Ok(result)
}

/// Copies `region` of a linear dmabuf, rows packed, into `out`, in the byte order of its layout.
///
/// # Errors
///
/// The [`DmabufError`] of a buffer that cannot be read.
pub(super) fn read_region(dmabuf: &Dmabuf, shape: &Linear, region: Rect, out: &mut [u8]) -> Result<(), DmabufError> {
    let row = region.width as usize * 4;
    let inside = region.intersection(Rect::new(0, 0, shape.width, shape.height)) == Some(region);
    if !inside || out.len() != row * region.height as usize {
        return Err(DmabufError::Layout);
    }
    with_mapping(dmabuf, shape, false, |base, _| {
        for (line, destination) in out.chunks_exact_mut(row).enumerate() {
            let y = region.y as usize + line;
            let source_row = if shape.inverted {
                shape.height as usize - 1 - y
            } else {
                y
            };
            // SAFETY: `with_mapping` checked that every row of the buffer lies inside the mapping, and the region is inside
            // the buffer. The client may write the memory at any time, so it is copied and never referenced.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    base.add(source_row * shape.stride + region.x as usize * 4),
                    destination.as_mut_ptr(),
                    row,
                );
            }
        }
    })
}

/// Reads a linear dmabuf as an [`Image`]: red first, alpha last, the pixels as the buffer holds them (premultiplied).
///
/// # Errors
///
/// The [`DmabufError`] of a buffer that cannot be read.
pub fn read_image(dmabuf: &Dmabuf) -> Result<Image, DmabufError> {
    let shape = linear(dmabuf)?;
    let mut bytes = vec![0_u8; shape.width as usize * shape.height as usize * 4];
    let mut unflipped = shape;
    unflipped.inverted = false;
    read_region(
        dmabuf,
        &unflipped,
        Rect::new(0, 0, shape.width, shape.height),
        &mut bytes,
    )?;
    let blue = blue_first(dmabuf.format().code);
    for pixel in bytes.as_chunks_mut::<4>().0 {
        if blue {
            pixel.swap(0, 2);
        }
        if shape.layout.opaque {
            pixel[3] = u8::MAX;
        }
    }
    Image::new(Size::new(shape.width, shape.height), shape.width as usize * 4, bytes).ok_or(DmabufError::Layout)
}

/// Writes `image` into a linear dmabuf, in the byte order of the buffer's format. The image is copied where the two overlap.
///
/// # Errors
///
/// The [`DmabufError`] of a buffer that cannot be written.
pub fn write_image(dmabuf: &Dmabuf, image: &Image) -> Result<(), DmabufError> {
    let shape = linear(dmabuf)?;
    write_region(
        dmabuf,
        image,
        Rect::new(0, 0, shape.width, shape.height),
        Point::new(0, 0),
    )
}

/// Writes the `region` of a linear dmabuf from `image`: buffer pixel `(x, y)` is image pixel `(from.x + x, from.y + y)`. A
/// region that does not fit the buffer, or reaches outside the image, writes the part that does, which is what the
/// shared-memory copy (`gnoblin_render_frame::shm::copy_into`) does with the same two numbers.
///
/// # Errors
///
/// The [`DmabufError`] of a buffer that cannot be written.
pub fn write_region(dmabuf: &Dmabuf, image: &Image, region: Rect, from: Point) -> Result<(), DmabufError> {
    let shape = linear(dmabuf)?;
    let blue = blue_first(dmabuf.format().code);
    let bounds = Rect::new(0, 0, shape.width, shape.height);
    let Some(region) = region.intersection(bounds) else {
        return Ok(());
    };
    let source = image.size();
    with_mapping(dmabuf, &shape, true, |base, _| {
        for y in region.y..region.y + region.height as i32 {
            let source_y = i64::from(from.y) + i64::from(y);
            if source_y < 0 || source_y >= i64::from(source.height) {
                continue;
            }
            let target_row = if shape.inverted {
                shape.height as usize - 1 - y as usize
            } else {
                y as usize
            };
            for x in region.x..region.x + region.width as i32 {
                let source_x = i64::from(from.x) + i64::from(x);
                if source_x < 0 || source_x >= i64::from(source.width) {
                    continue;
                }
                let at = source_y as usize * image.stride() + source_x as usize * 4;
                let Some(pixel) = image.data().get(at..at + 4) else {
                    continue;
                };
                let bytes = if blue {
                    [pixel[2], pixel[1], pixel[0], pixel[3]]
                } else {
                    [pixel[0], pixel[1], pixel[2], pixel[3]]
                };
                // SAFETY: the row and column are inside the buffer, which `with_mapping` checked lies inside the mapping.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        base.add(target_row * shape.stride + x as usize * 4),
                        4,
                    )
                };
            }
        }
    })
}
