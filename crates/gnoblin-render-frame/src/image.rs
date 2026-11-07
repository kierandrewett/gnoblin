//! A frame's pixels, copied out of a renderer.
//!
//! This is the software path's readback and the shape every renderer's readback
//! will have, because the plan's parity rule compares *semantic pixel probes* and
//! a probe is a colour at a point of an [`Image`]. Capture, the diagnostics
//! overlay and the tests all read the same type.
//!
//! # Orientation and colour space
//!
//! - **Origin top left.** Pixel `(0, 0)` is the output's top-left corner and `y`
//!   grows downwards, matching every rectangle in `gnoblin-types`, `wl_output`
//!   and the Wayland surface coordinate space. A readback that came out bottom-up
//!   is flipped before it becomes an [`Image`], so a caller never has to know
//!   which way a particular backend's memory runs.
//! - **RGBA8, straight alpha.** Four bytes per pixel in red, green, blue, alpha
//!   order, one byte per channel, no premultiplication. The composited frame has
//!   an opaque background, so alpha is 255 in every pixel today; the promise that
//!   the values are not premultiplied is what makes a future translucent target
//!   readable without a second type.
//! - **sRGB, unmanaged.** Channels are the stored sRGB values with no transfer
//!   function applied and no colour profile: D12 puts colour management on
//!   Gnoblin's own pipeline, which does not exist yet. A probe is therefore
//!   comparable with the 8-bit colour a client put in its buffer, which is
//!   exactly what a parity check needs.
//!
//! # Failure modes
//!
//! 1. **A row is read from the wrong offset because the stride was assumed.**
//!    [`Image::pixel`] steps by the recorded stride, never by `width * 4`, so a
//!    target padded to a wider row still addresses the right bytes:
//!    `tests::a_padded_stride_addresses_the_right_pixel`.
//! 2. **A read past the end of the buffer panics or reads garbage.** A coordinate
//!    outside the image is reported as [`None`] and a buffer too short for its
//!    declared size is refused by [`Image::new`]: `tests::a_short_buffer_is_refused`
//!    and `tests::a_point_outside_the_image_has_no_colour`.

use gnoblin_types::{Point, Size};

/// A frame's pixels in RGBA8, top left first.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    size: Size,
    stride: usize,
    data: Vec<u8>,
}

impl Image {
    /// A transparent image of the given size with a packed stride.
    ///
    /// Every pixel is opaque black until something draws on it, because a
    /// zero-filled RGBA8 buffer would be transparent, and a transparent screenshot
    /// of "nothing" would be indistinguishable from a capture that failed.
    #[must_use]
    pub fn blank(size: Size) -> Self {
        let stride = row_bytes(size.width);
        let length = stride.saturating_mul(size.height as usize);
        let mut image = Self {
            size,
            stride,
            data: vec![0; length],
        };
        image.fill_opaque_black();
        image
    }

    /// An image over a pixel buffer read back from a renderer.
    ///
    /// `stride` is the number of bytes between the start of one row and the next,
    /// which is at least `width * 4`: a renderer pads rows to an alignment, and
    /// assuming `width * 4` would shift every row after the first.
    ///
    /// Returns [`None`] when the buffer cannot hold `height` rows of `stride`
    /// bytes, because reading it would either panic or read another allocation's
    /// memory.
    #[must_use]
    pub fn new(size: Size, stride: usize, data: Vec<u8>) -> Option<Self> {
        if stride < row_bytes(size.width) {
            return None;
        }
        if data.len() < stride.checked_mul(size.height as usize)? {
            return None;
        }
        Some(Self { size, stride, data })
    }

    /// The image's size in pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.size
    }

    /// The bytes between the start of one row and the next.
    #[must_use]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The pixels, row by row.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The colour of one pixel in RGBA8 order, or [`None`] outside the image.
    #[must_use]
    pub fn pixel(&self, point: Point) -> Option<[u8; 4]> {
        if point.x < 0 || point.y < 0 {
            return None;
        }
        let (x, y) = (usize::try_from(point.x).ok()?, usize::try_from(point.y).ok()?);
        if x >= self.size.width as usize || y >= self.size.height as usize {
            return None;
        }
        let start = y.checked_mul(self.stride)?.checked_add(x.checked_mul(4)?)?;
        let end = start.checked_add(4)?;
        let bytes = self.data.get(start..end)?;
        Some([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    /// The colour of one pixel, for a caller that has the coordinates rather than
    /// a [`Point`].
    #[must_use]
    pub fn colour_at(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        self.pixel(Point::new(x, y))
    }

    /// Overwrites every pixel with opaque black.
    fn fill_opaque_black(&mut self) {
        for row in self.data.chunks_mut(self.stride) {
            for pixel in row.chunks_mut(4) {
                pixel.copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
}

/// The bytes one packed row of `width` RGBA8 pixels occupies.
fn row_bytes(width: u32) -> usize {
    (width as usize).saturating_mul(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_padded_stride_addresses_the_right_pixel() {
        // Two rows of three pixels (twelve bytes packed) padded to sixteen: the
        // second row starts at byte 16, not at byte 12.
        let mut data = vec![0; 32];
        data[16..20].copy_from_slice(&[9, 8, 7, 6]);
        let image = Image::new(Size::new(3, 2), 16, data).expect("the buffer holds two padded rows");
        assert_eq!(image.pixel(Point::new(0, 0)), Some([0, 0, 0, 0]));
        assert_eq!(
            image.colour_at(0, 1),
            Some([9, 8, 7, 6]),
            "the second row starts at the stride, not at width * 4"
        );
    }

    #[test]
    fn a_short_buffer_is_refused() {
        // Two rows of two pixels need twelve bytes at the packed stride.
        assert!(Image::new(Size::new(2, 2), 8, vec![0; 15]).is_none());
        assert!(
            Image::new(Size::new(2, 2), 4, vec![0; 16]).is_none(),
            "a stride below the row width"
        );
        assert!(Image::new(Size::new(2, 2), 8, vec![0; 16]).is_some());
    }

    #[test]
    fn a_point_outside_the_image_has_no_colour() {
        let image = Image::new(Size::new(2, 2), 8, vec![7; 16]).expect("a packed image");
        assert!(image.colour_at(-1, 0).is_none());
        assert!(image.colour_at(0, -1).is_none());
        assert!(image.colour_at(2, 0).is_none());
        assert!(image.colour_at(0, 2).is_none());
        assert_eq!(image.colour_at(1, 1), Some([7, 7, 7, 7]));
    }

    #[test]
    fn a_blank_image_is_opaque_black_rather_than_transparent() {
        let image = Image::blank(Size::new(3, 2));
        assert_eq!(image.stride(), 12);
        assert_eq!(image.data().len(), 24);
        assert_eq!(image.colour_at(2, 1), Some([0, 0, 0, 255]));
    }
}
