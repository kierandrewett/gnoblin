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
use smithay::wayland::shm::{BufferData, with_buffer_contents_mut};

use gnoblin_types::{Point, Rect};

use crate::render::Image;

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
    use super::Pixel;

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
}
