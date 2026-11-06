//! The buffer a session negotiates: its size, row length and pixel layout.
//!
//! The values are the ones a protocol advertises to its client before the client
//! allocates anything, so this module is the one place that decides what a capture
//! buffer is. Everything downstream — the copy, the damage rectangles and the
//! protocol's own buffer events — reads the same value, which is what keeps the
//! advertised stride and the stride the copy writes with from disagreeing.

use gnoblin_types::Size;

/// How the pixels of a capture buffer are laid out.
///
/// Both variants are the 32-bit packed layouts `wl_shm` names, which is what every
/// consumer of a screencopy or image-copy-capture buffer asks for: one word per
/// pixel, alpha in the high byte, native byte order. `Xrgb8888` promises nothing
/// about alpha, so a copy writes an opaque alpha rather than the frame's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 32-bit packed, alpha used.
    Argb8888,
    /// 32-bit packed, alpha ignored.
    Xrgb8888,
}

impl PixelFormat {
    /// The bytes one pixel occupies.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        4
    }

    /// The word a pixel is written as, in the host's byte order.
    ///
    /// The wire layout is `0xAARRGGBB` read as a native-endian word, so converting
    /// through `to_ne_bytes` is what makes the same code correct on a big-endian
    /// machine instead of silently swapping red and blue.
    #[must_use]
    pub const fn word(self, pixel: [u8; 4]) -> u32 {
        let [red, green, blue, alpha] = pixel;
        let alpha = match self {
            Self::Argb8888 => alpha,
            Self::Xrgb8888 => u8::MAX,
        };
        (alpha as u32) << 24 | (red as u32) << 16 | (green as u32) << 8 | (blue as u32)
    }

    /// The protocol's name for this layout, for a log line or a test.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Argb8888 => "argb8888",
            Self::Xrgb8888 => "xrgb8888",
        }
    }
}

/// The geometry of the buffer a session copies into.
///
/// A value of this type is always usable: the constructor refuses a degenerate size
/// and a stride too short for one row, so nothing downstream has to re-check what a
/// protocol already advertised to a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Format {
    /// The buffer's size in pixels.
    size: Size,
    /// The bytes between the start of one row and the next.
    stride: u32,
    /// The pixel layout.
    pixel: PixelFormat,
}

impl Format {
    /// A format, or [`None`] when the client could never allocate it.
    ///
    /// Refused: a size with no pixels on an axis (a frame of nothing), a stride
    /// shorter than one packed row (every row would overlap the next), and a size
    /// whose byte length does not fit a [`usize`] on this machine.
    #[must_use]
    pub fn new(size: Size, stride: u32, pixel: PixelFormat) -> Option<Self> {
        if size.is_empty() {
            return None;
        }
        let row = size.width.checked_mul(pixel.bytes_per_pixel())?;
        if stride < row {
            return None;
        }
        Self { size, stride, pixel }.bytes()?;
        Some(Self { size, stride, pixel })
    }

    /// The size in pixels.
    #[must_use]
    pub const fn size(self) -> Size {
        self.size
    }

    /// The bytes between the start of one row and the next.
    #[must_use]
    pub const fn stride(self) -> u32 {
        self.stride
    }

    /// The pixel layout.
    #[must_use]
    pub const fn pixel(self) -> PixelFormat {
        self.pixel
    }

    /// The bytes the whole buffer needs, if they are addressable on this machine.
    #[must_use]
    pub fn bytes(self) -> Option<usize> {
        usize::try_from(self.stride)
            .ok()?
            .checked_mul(usize::try_from(self.size.height).ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::{Format, PixelFormat};
    use gnoblin_types::Size;

    /// Failure mode: a stride that cannot hold a row is advertised anyway, and the
    /// copy writes each row over the next one.
    #[test]
    fn a_stride_that_cannot_hold_a_row_is_refused() {
        assert_eq!(Format::new(Size::new(4, 2), 15, PixelFormat::Argb8888), None);
        assert!(Format::new(Size::new(4, 2), 16, PixelFormat::Argb8888).is_some());
    }

    /// Failure mode: a frame of nothing is negotiated, and every copy afterwards
    /// addresses no pixels while looking successful.
    #[test]
    fn a_degenerate_size_is_refused() {
        assert_eq!(Format::new(Size::new(0, 10), 0, PixelFormat::Argb8888), None);
        assert_eq!(Format::new(Size::new(10, 0), 40, PixelFormat::Argb8888), None);
    }

    /// The packed layouts put alpha in the high byte and red next to it, which is
    /// the order a client reads its buffer in.
    #[test]
    fn a_pixel_word_is_argb_in_the_high_byte() {
        assert_eq!(PixelFormat::Argb8888.word([0x11, 0x22, 0x33, 0x44]), 0x4411_2233);
        assert_eq!(PixelFormat::Xrgb8888.word([0x11, 0x22, 0x33, 0x00]), 0xFF11_2233);
        assert_eq!(
            u32::from_ne_bytes(PixelFormat::Argb8888.word([1, 2, 3, 4]).to_ne_bytes()),
            0x0401_0203
        );
    }

    /// A padded stride is kept exactly: it is what the client allocated with, and a
    /// copy that assumed `width * 4` would shift every row.
    #[test]
    fn a_padded_stride_is_kept() {
        let format = Format::new(Size::new(3, 3), 20, PixelFormat::Argb8888).expect("a valid format");
        assert_eq!(format.stride(), 20);
        assert_eq!(format.bytes(), Some(60));
    }
}
