//! Writing a captured frame into a client's shared-memory buffer.
//!
//! `wl_shm` buffers are the only destination a screencopy or image-copy-capture
//! client asks for that needs no GPU, no DMA buffer and no session, so they are what
//! capture writes into first. The buffer's pixels live in the client's pool, which
//! Smithay maps and hands over as a pointer; turning that pointer into a slice is the
//! one place in this workspace that needs `unsafe`, and it is confined to this module
//! with the length and bounds checked before the slice exists.
//!
//! # Why this is not in `gnoblin-capture`
//!
//! The pure capture crate states rules and never touches a client's memory. This is
//! the mechanism those rules hand a frame to, so it lives with the rest of the render
//! path, next to the [`Image`] it copies from.
//!
//! # Failure modes
//!
//! 1. **A region outside the buffer is written.** Every copy is clipped to the
//!    buffer's own extent, and the mapping is checked for the length the buffer
//!    declares before a slice is made, so a client cannot make the compositor write
//!    past its pool: `a_region_outside_the_buffer_writes_nothing` and the
//!    length check in [`mapped`].
//! 2. **A row is written at an assumed stride.** The buffer's stride comes from its
//!    own `BufferData`, never from `width * 4`:
//!    `a_padded_row_leaves_the_padding_alone`.
//! 3. **A format the compositor cannot encode is written as if it were ARGB.** Only
//!    the two packed 32-bit layouts are accepted, and anything else is refused before
//!    a byte is written: `an_unsupported_format_writes_nothing`.
//! 4. **An `xrgb8888` buffer keeps the frame's alpha.** A buffer that promises nothing
//!    about alpha gets an opaque one, so a client that blends its capture does not see
//!    the frame's translucency where it should see content.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::wayland::shm::{BufferAccessError, BufferData, with_buffer_contents, with_buffer_contents_mut};

use gnoblin_types::{Point, Rect, Size};

use crate::image::Image;

/// Why a frame could not be written into a client's buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShmError {
    /// The buffer is not a `wl_shm` buffer this compositor created through a pool.
    NotShared,
    /// The buffer's format is not one of the packed 32-bit layouts.
    UnsupportedFormat,
    /// Nothing of the region fits the buffer.
    Outside,
    /// The pool could not be mapped, which a client causes by shrinking it under the
    /// buffer it created from it.
    Unmapped,
}

impl std::fmt::Display for ShmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotShared => formatter.write_str("the buffer is not a shared-memory buffer"),
            Self::UnsupportedFormat => formatter.write_str("the buffer's format is not a packed 32-bit layout"),
            Self::Outside => formatter.write_str("the region does not fit the buffer"),
            Self::Unmapped => formatter.write_str("the buffer's pool could not be mapped"),
        }
    }
}

impl std::error::Error for ShmError {}

/// What a shared-memory buffer holds, read from its metadata alone.
///
/// A GPU renderer sizes its texture and its staging space from this before it reads a
/// single pixel, so a buffer it cannot use is refused without touching the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShmShape {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Whether the layout is `xrgb8888`, whose fourth byte carries no alpha.
    pub opaque: bool,
}

impl ShmShape {
    /// The bytes of the buffer's pixels with the rows packed.
    pub(crate) fn packed_bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// The shape of a client's shared-memory buffer.
///
/// # Errors
///
/// [`ShmError`] when the buffer is not shared memory, its format is not one of the packed
/// 32-bit layouts, its rows do not fit its stride or it reaches past its pool.
pub fn shape(buffer: &WlBuffer) -> Result<ShmShape, ShmError> {
    if !buffer.is_alive() {
        return Err(ShmError::NotShared);
    }
    with_buffer_contents(buffer, |_pointer, length, data| checked_shape(length, &data)).map_err(access_error)?
}

/// Copies a client's pixels into `out` with the rows packed, in the buffer's own byte order.
///
/// `out` must be exactly [`ShmShape::packed_bytes`] long. The bytes of an `argb8888` or
/// `xrgb8888` pixel on a little-endian machine are blue, green, red, then alpha or padding,
/// which is what Vulkan's `B8G8R8A8_UNORM` reads, so no pixel is converted on the way.
///
/// # Errors
///
/// The same as [`shape`], and [`ShmError::Outside`] when `out` is the wrong length.
pub fn read_packed(buffer: &WlBuffer, out: &mut [u8]) -> Result<ShmShape, ShmError> {
    if !buffer.is_alive() {
        return Err(ShmError::NotShared);
    }
    with_buffer_contents(buffer, |pointer, length, data| {
        let shape = checked_shape(length, &data)?;
        if pointer.is_null() || out.len() != shape.packed_bytes() {
            return Err(ShmError::Outside);
        }
        // The checks above and in `checked_shape` bound every row inside `length`.
        copy_rows(
            pointer,
            usize::try_from(data.offset).map_err(|_| ShmError::Outside)?,
            usize::try_from(data.stride).map_err(|_| ShmError::Outside)?,
            out,
            shape.width as usize * 4,
        );
        Ok(shape)
    })
    .map_err(access_error)?
}

/// Copies a client's whole buffer into an [`Image`], so a window that is closing keeps the
/// last picture it showed after the client destroys the buffer.
///
/// A `wl_shm` pixel is `B, G, R, A` in memory, which is what `ARGB8888` names, and an
/// [`Image`] holds `R, G, B, A`, so the red and blue bytes are swapped. Alpha stays
/// premultiplied as the client wrote it, because [`SurfaceSource::Snapshot`] is drawn
/// from premultiplied bytes.
///
/// # Errors
///
/// The same as [`read_packed`].
pub fn readback(buffer: &WlBuffer) -> Result<Image, ShmError> {
    let shape = shape(buffer)?;
    let mut bytes = vec![0; shape.packed_bytes()];
    read_packed(buffer, &mut bytes)?;
    let width = shape.width as usize;
    Image::new(
        Size::new(shape.width, shape.height),
        width * 4,
        to_image_order(shape, bytes),
    )
    .ok_or(ShmError::Outside)
}

/// Puts packed `ARGB8888` or `XRGB8888` pixels into [`Image`] order: red first, alpha last.
///
/// An `XRGB8888` buffer's last byte is padding, so it is made opaque here rather than left
/// as the client wrote it. This is separate from [`readback`] so the byte order can be
/// tested without a Wayland pool.
fn to_image_order(shape: ShmShape, mut bytes: Vec<u8>) -> Vec<u8> {
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        if shape.opaque {
            pixel[3] = u8::MAX;
        }
    }
    bytes
}

/// The one error a refused mapping becomes.
fn access_error(error: BufferAccessError) -> ShmError {
    match error {
        BufferAccessError::NotManaged => ShmError::NotShared,
        _ => ShmError::Unmapped,
    }
}

/// The checked part of [`shape`]: everything that is decided from the buffer's metadata.
fn checked_shape(length: usize, data: &BufferData) -> Result<ShmShape, ShmError> {
    let opaque = match data.format {
        wl_shm::Format::Argb8888 => false,
        wl_shm::Format::Xrgb8888 => true,
        _ => return Err(ShmError::UnsupportedFormat),
    };
    let width = u32::try_from(data.width).ok().filter(|width| *width > 0);
    let height = u32::try_from(data.height).ok().filter(|height| *height > 0);
    let (Some(width), Some(height)) = (width, height) else {
        return Err(ShmError::Outside);
    };
    let row = width as usize * 4;
    let stride = usize::try_from(data.stride).map_err(|_| ShmError::Outside)?;
    if stride < row {
        return Err(ShmError::Outside);
    }
    let offset = usize::try_from(data.offset).map_err(|_| ShmError::Outside)?;
    // The last byte the buffer addresses, so a buffer larger than its pool is refused before
    // a row is read.
    let end = stride
        .checked_mul(height as usize - 1)
        .and_then(|rows| rows.checked_add(offset))
        .and_then(|start| start.checked_add(row))
        .ok_or(ShmError::Outside)?;
    if end > length {
        return Err(ShmError::Unmapped);
    }
    Ok(ShmShape { width, height, opaque })
}

/// Copies `out.len() / row` rows of `row` bytes from the pool, `stride` bytes apart.
///
/// The rows are copied with `ptr::copy_nonoverlapping` and no reference to the pool is
/// ever made: the client may write the memory at any time, and a slice over it would be
/// undefined behaviour while it does.
///
/// # Safety
///
/// The caller has checked that `offset + stride * (rows - 1) + row` is within the mapping
/// that starts at `pointer` ([`checked_shape`]) and that `out` holds `rows * row` bytes.
#[allow(unsafe_code)]
fn copy_rows(pointer: *const u8, offset: usize, stride: usize, out: &mut [u8], row: usize) {
    for (index, destination) in out.chunks_exact_mut(row).enumerate() {
        // SAFETY: the source row lies inside the mapping by the caller's bound, and the
        // destination is a distinct `row`-byte chunk of `out`, so the ranges cannot overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(pointer.add(offset + index * stride), destination.as_mut_ptr(), row);
        }
    }
}

/// Copies the part of `image` that starts at `from` into `buffer`.
///
/// `region` is in the buffer's own pixels and is clipped to the buffer: a region that
/// overlaps it only partly writes the overlap rather than refusing the frame, because
/// a client that asked for a frame to the edge of an output may hold a buffer exactly
/// that size and one pixel short is not a reason to fail the capture. Buffer pixel
/// `(x, y)` is read from image pixel `(from.x + x, from.y + y)`, so a window's frame
/// is read from where the window sits on the output.
///
/// Returns the region that was written, in buffer pixels.
///
/// # Errors
///
/// [`ShmError`] when the buffer is not shared memory, its format is not packed 32-bit,
/// nothing of the region fits it, or the pool cannot be mapped.
pub fn copy_into(image: &Image, region: Rect, from: Point, buffer: &WlBuffer) -> Result<Rect, ShmError> {
    if !buffer.is_alive() {
        return Err(ShmError::NotShared);
    }
    with_buffer_contents_mut(buffer, |pointer, length, data| {
        let written = write(pointer, length, &data, image, region, from)?;
        Ok(written)
    })
    // The outer error is Smithay refusing to hand over the mapping at all: the pool was
    // not created through `ShmState`, or the client shrank it under its buffer.
    .map_err(|_unmapped| ShmError::Unmapped)?
}

/// The checked part of [`copy_into`]: everything that touches the mapping.
fn write(
    pointer: *mut u8,
    length: usize,
    data: &BufferData,
    image: &Image,
    region: Rect,
    from: Point,
) -> Result<Rect, ShmError> {
    let pixel = match data.format {
        wl_shm::Format::Argb8888 => Pixel::Argb,
        wl_shm::Format::Xrgb8888 => Pixel::Xrgb,
        _ => return Err(ShmError::UnsupportedFormat),
    };
    let bounds = Rect::new(
        0,
        0,
        u32::try_from(data.width).unwrap_or(0),
        u32::try_from(data.height).unwrap_or(0),
    );
    let written = region.intersection(bounds).ok_or(ShmError::Outside)?;
    let stride = usize::try_from(data.stride).map_err(|_| ShmError::Outside)?;
    if stride < written.width as usize * pixel.bytes() {
        return Err(ShmError::Outside);
    }
    let offset = usize::try_from(data.offset).map_err(|_| ShmError::Outside)?;
    // The last byte the copy addresses, so a buffer whose declared size is larger than
    // the pool it was created from cannot be written past the mapping.
    let end = offset
        .checked_add(stride.checked_mul(written.height as usize).ok_or(ShmError::Outside)?)
        .ok_or(ShmError::Outside)?;
    if end > length {
        return Err(ShmError::Unmapped);
    }
    let Some(data_slice) = mapped(pointer, length) else {
        return Err(ShmError::Unmapped);
    };
    for row in 0..written.height {
        let source_y = written.y.saturating_add_unsigned(row);
        let start = offset + (written.y as usize + row as usize) * stride + written.x as usize * pixel.bytes();
        let end = start + written.width as usize * pixel.bytes();
        let Some(destination) = data_slice.get_mut(start..end) else {
            return Err(ShmError::Outside);
        };
        for (column, slot) in destination.chunks_exact_mut(pixel.bytes()).enumerate() {
            let source_x = written.x.saturating_add_unsigned(u32::try_from(column).unwrap_or(0));
            let source = Point::new(source_x.saturating_add(from.x), source_y.saturating_add(from.y));
            let Some(pixel_value) = image.pixel(source) else {
                // The image is smaller than the buffer: the pixels it does not hold stay
                // as the client left them rather than becoming a colour the frame never
                // had.
                continue;
            };
            slot.copy_from_slice(&pixel.encode(pixel_value));
        }
    }
    Ok(written)
}

/// The client's pool as a slice.
///
/// # Safety
///
/// The pointer and length come from Smithay, which maps the pool for the duration of
/// the `with_buffer_contents_mut` closure this is called inside, so the memory is
/// alive and cannot be unmapped while the slice exists. The slice is never longer
/// than the mapping (`length` is the mapping's own byte length), and only bytes inside
/// the buffer's declared stride and height are written.
#[allow(unsafe_code)]
fn mapped<'a>(pointer: *mut u8, length: usize) -> Option<&'a mut [u8]> {
    if pointer.is_null() || length == 0 {
        return None;
    }
    // SAFETY: the pointer and length are Smithay's mapping of the pool the buffer was
    // created from, which stays mapped for the closure that calls this, and every
    // write below is bounded by the length that produced the slice.
    Some(unsafe { std::slice::from_raw_parts_mut(pointer, length) })
}

/// The packed 32-bit layouts a capture can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pixel {
    /// Alpha is used.
    Argb,
    /// Alpha is forced opaque.
    Xrgb,
}

impl Pixel {
    /// The bytes one pixel occupies.
    const fn bytes(self) -> usize {
        4
    }

    /// The bytes `pixel` is written as, in the host's byte order.
    ///
    /// The wire layout is `0xAARRGGBB` read as a native-endian word, so going through
    /// `to_ne_bytes` is what keeps red and blue in the right places on either
    /// endianness instead of relying on the machine being little-endian.
    const fn encode(self, pixel: [u8; 4]) -> [u8; 4] {
        let [red, green, blue, alpha] = pixel;
        let alpha = match self {
            Self::Argb => alpha,
            Self::Xrgb => u8::MAX,
        };
        let word: u32 = (alpha as u32) << 24 | (red as u32) << 16 | (green as u32) << 8 | (blue as u32);
        word.to_ne_bytes()
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_types::{Point, Size};

    use super::{Pixel, ShmShape, to_image_order};
    use crate::image::Image;

    /// The packed layouts put alpha in the high byte and red next to it, which is the
    /// order a client reads with.
    #[test]
    fn a_pixel_is_encoded_as_argb_in_the_high_byte() {
        assert_eq!(
            Pixel::Argb.encode([0x11, 0x22, 0x33, 0x44]),
            0x4411_2233u32.to_ne_bytes()
        );
        assert_eq!(
            Pixel::Xrgb.encode([0x11, 0x22, 0x33, 0x44]),
            0xFF11_2233u32.to_ne_bytes(),
            "a buffer that promises nothing about alpha gets an opaque one"
        );
    }

    /// A client's `ARGB8888` pixel is blue first in memory and an image holds red first, so
    /// the bytes must swap and the alpha must survive.
    #[test]
    fn an_argb_pixel_is_read_back_in_image_order() {
        let shape = ShmShape {
            width: 1,
            height: 1,
            opaque: false,
        };
        assert_eq!(
            to_image_order(shape, vec![0x33, 0x22, 0x11, 0x44]),
            vec![0x11, 0x22, 0x33, 0x44]
        );
    }

    /// An `XRGB8888` pixel's last byte is padding, which must not become the image's alpha.
    #[test]
    fn an_xrgb_pixel_is_read_back_opaque() {
        let shape = ShmShape {
            width: 1,
            height: 1,
            opaque: true,
        };
        assert_eq!(
            to_image_order(shape, vec![0x33, 0x22, 0x11, 0x00]),
            vec![0x11, 0x22, 0x33, 0xFF]
        );
    }

    /// A View's picture is copied through the same per-pixel encoding as an output's:
    /// a known RGBA colour in the picture must reach the client's ARGB bytes as the
    /// same colour, not swapped or washed out.
    #[test]
    fn a_view_picture_pixel_reaches_the_client_as_the_same_colour() {
        // One opaque red-ish pixel and one half-transparent blue one, row-packed.
        let image = Image::new(Size::new(2, 1), 8, vec![0xFF, 0x20, 0x10, 0xFF, 0x00, 0x20, 0xF0, 0x80])
            .expect("a two-pixel image");
        let red = image.pixel(Point::new(0, 0)).expect("the first pixel");
        let blue = image.pixel(Point::new(1, 0)).expect("the second pixel");
        assert_eq!(red, [0xFF, 0x20, 0x10, 0xFF]);
        assert_eq!(blue, [0x00, 0x20, 0xF0, 0x80]);
        assert_eq!(
            Pixel::Argb.encode(red),
            0xFF_FF_20_10u32.to_ne_bytes(),
            "an opaque pixel keeps its alpha and colour"
        );
        assert_eq!(
            Pixel::Argb.encode(blue),
            0x80_00_20_F0u32.to_ne_bytes(),
            "a translucent pixel keeps its alpha instead of being made opaque"
        );
    }
}
