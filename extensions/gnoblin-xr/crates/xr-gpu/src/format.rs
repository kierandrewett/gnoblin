//! Pixel formats and the conversion from a `wl_shm` capture to a swapchain image.
//!
//! A capture arrives as `wl_shm` Argb8888 or Xrgb8888. On a little-endian machine both are stored as the bytes
//! `B, G, R, A` (`X` for Xrgb), so a `B8G8R8A8` swapchain takes them as they are and an `R8G8B8A8` one needs the red
//! and blue bytes swapped. Xrgb has no alpha: the converted alpha is 255. The swapchain formats are the sRGB ones,
//! because the captured bytes are sRGB encoded, and a copy does not change the encoding.

use crate::error::GpuError;

/// `VK_FORMAT_R8G8B8A8_SRGB`.
const VK_R8G8B8A8_SRGB: i64 = 43;
/// `VK_FORMAT_B8G8R8A8_SRGB`.
const VK_B8G8R8A8_SRGB: i64 = 50;

/// The `wl_shm` formats a capture may have. Both are 32 bits per pixel, `B, G, R, A` in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Argb8888,
    Xrgb8888,
}

/// The swapchain formats the extension can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapchainFormat {
    /// `VK_FORMAT_R8G8B8A8_SRGB`.
    Rgba8Srgb,
    /// `VK_FORMAT_B8G8R8A8_SRGB`.
    Bgra8Srgb,
}

impl SwapchainFormat {
    /// The `VkFormat` value, as the runtime lists it.
    pub const fn vk_format(self) -> i64 {
        match self {
            Self::Rgba8Srgb => VK_R8G8B8A8_SRGB,
            Self::Bgra8Srgb => VK_B8G8R8A8_SRGB,
        }
    }

    /// The format with the `VkFormat` value `vk`, if the extension can fill it.
    pub const fn from_vk_format(vk: i64) -> Option<Self> {
        match vk {
            VK_R8G8B8A8_SRGB => Some(Self::Rgba8Srgb),
            VK_B8G8R8A8_SRGB => Some(Self::Bgra8Srgb),
            _ => None,
        }
    }
}

/// Picks the swapchain format: the first one in the runtime's list that the extension can fill. OpenXR lists formats
/// in the runtime's order of preference.
pub fn choose_format(enumerated: &[i64]) -> Option<SwapchainFormat> {
    enumerated.iter().find_map(|&vk| SwapchainFormat::from_vk_format(vk))
}

/// A captured picture: `height` rows of `stride` bytes, the first `width * 4` of each holding pixels.
#[derive(Debug, Clone, Copy)]
pub struct Pixels<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: SourceFormat,
    pub bytes: &'a [u8],
}

/// The bytes of a converted picture: `width * height * 4`, rows tightly packed.
pub const fn packed_len(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}

impl Pixels<'_> {
    /// Checks that the picture has pixels and that its bytes cover every row.
    ///
    /// # Errors
    ///
    /// The picture is empty, its stride is shorter than a row, or its bytes are too few.
    pub fn check(&self) -> Result<(), GpuError> {
        if self.width == 0 || self.height == 0 {
            return Err(GpuError::BadPixels("the picture has no pixels".to_owned()));
        }
        let row = self.width as usize * 4;
        let stride = self.stride as usize;
        if stride < row {
            return Err(GpuError::BadPixels(format!(
                "the stride {stride} is shorter than a row of {row} bytes"
            )));
        }
        let needed = stride * (self.height as usize - 1) + row;
        if self.bytes.len() < needed {
            return Err(GpuError::BadPixels(format!(
                "{} bytes cannot hold {} rows of stride {stride}",
                self.bytes.len(),
                self.height
            )));
        }
        Ok(())
    }
}

/// Writes `pixels` into `out` as `target` pixels with tightly packed rows.
///
/// # Errors
///
/// The pixels fail [`Pixels::check`], or `out` is not exactly [`packed_len`] bytes.
pub fn convert(pixels: &Pixels<'_>, target: SwapchainFormat, out: &mut [u8]) -> Result<(), GpuError> {
    pixels.check()?;
    let expected = packed_len(pixels.width, pixels.height);
    if out.len() != expected {
        return Err(GpuError::BadPixels(format!(
            "the destination holds {} bytes, the picture needs {expected}",
            out.len()
        )));
    }
    let row = pixels.width as usize * 4;
    let opaque = pixels.format == SourceFormat::Xrgb8888;
    for (y, dst_row) in out.chunks_exact_mut(row).enumerate() {
        let start = y * pixels.stride as usize;
        let src_row = &pixels.bytes[start..start + row];
        let (dst_pixels, _) = dst_row.as_chunks_mut::<4>();
        let (src_pixels, _) = src_row.as_chunks::<4>();
        for (dst, &[b, g, r, a]) in dst_pixels.iter_mut().zip(src_pixels) {
            let a = if opaque { 255 } else { a };
            *dst = match target {
                SwapchainFormat::Bgra8Srgb => [b, g, r, a],
                SwapchainFormat::Rgba8Srgb => [r, g, b, a],
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argb(width: u32, height: u32, bytes: &[u8]) -> Pixels<'_> {
        Pixels {
            width,
            height,
            stride: width * 4,
            format: SourceFormat::Argb8888,
            bytes,
        }
    }

    #[test]
    fn the_runtimes_first_supported_format_wins() {
        // 37 is VK_FORMAT_R8G8B8A8_UNORM, which the extension cannot fill.
        assert_eq!(choose_format(&[37, 50, 43]), Some(SwapchainFormat::Bgra8Srgb));
        assert_eq!(choose_format(&[43, 50]), Some(SwapchainFormat::Rgba8Srgb));
    }

    #[test]
    fn no_supported_format_is_none() {
        assert_eq!(choose_format(&[37, 44]), None);
        assert_eq!(choose_format(&[]), None);
    }

    #[test]
    fn vk_format_numbers_round_trip() {
        for format in [SwapchainFormat::Rgba8Srgb, SwapchainFormat::Bgra8Srgb] {
            assert_eq!(SwapchainFormat::from_vk_format(format.vk_format()), Some(format));
        }
        assert_eq!(SwapchainFormat::Rgba8Srgb.vk_format(), 43);
        assert_eq!(SwapchainFormat::Bgra8Srgb.vk_format(), 50);
    }

    #[test]
    fn argb_to_bgra_is_a_copy() {
        let src = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut out = [0; 8];
        convert(&argb(2, 1, &src), SwapchainFormat::Bgra8Srgb, &mut out).expect("converts");
        assert_eq!(out, src);
    }

    #[test]
    fn argb_to_rgba_swaps_red_and_blue() {
        let src = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut out = [0; 8];
        convert(&argb(2, 1, &src), SwapchainFormat::Rgba8Srgb, &mut out).expect("converts");
        assert_eq!(out, [30, 20, 10, 40, 70, 60, 50, 80]);
    }

    #[test]
    fn xrgb_gets_an_opaque_alpha() {
        let src = [10, 20, 30, 0, 50, 60, 70, 5];
        let pixels = Pixels {
            format: SourceFormat::Xrgb8888,
            ..argb(2, 1, &src)
        };
        let mut bgra = [0; 8];
        convert(&pixels, SwapchainFormat::Bgra8Srgb, &mut bgra).expect("converts");
        assert_eq!(bgra, [10, 20, 30, 255, 50, 60, 70, 255]);
        let mut rgba = [0; 8];
        convert(&pixels, SwapchainFormat::Rgba8Srgb, &mut rgba).expect("converts");
        assert_eq!(rgba, [30, 20, 10, 255, 70, 60, 50, 255]);
    }

    #[test]
    fn row_padding_is_dropped() {
        // Two rows of one pixel, each row padded to 8 bytes.
        let src = [1, 2, 3, 4, 99, 99, 99, 99, 5, 6, 7, 8];
        let pixels = Pixels {
            width: 1,
            height: 2,
            stride: 8,
            format: SourceFormat::Argb8888,
            bytes: &src,
        };
        let mut out = [0; 8];
        convert(&pixels, SwapchainFormat::Bgra8Srgb, &mut out).expect("converts");
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn short_bytes_are_refused() {
        let src = [0; 7];
        let mut out = [0; 8];
        let error = convert(&argb(2, 1, &src), SwapchainFormat::Bgra8Srgb, &mut out).expect_err("too short");
        assert!(matches!(error, GpuError::BadPixels(_)), "{error}");
    }

    #[test]
    fn an_empty_picture_is_refused() {
        let mut out = [];
        assert!(convert(&argb(0, 1, &[]), SwapchainFormat::Bgra8Srgb, &mut out).is_err());
    }

    #[test]
    fn a_stride_shorter_than_a_row_is_refused() {
        let src = [0; 16];
        let pixels = Pixels {
            stride: 4,
            ..argb(2, 2, &src)
        };
        assert!(pixels.check().is_err());
    }

    #[test]
    fn a_wrong_destination_size_is_refused() {
        let src = [0; 8];
        let mut out = [0; 4];
        assert!(convert(&argb(2, 1, &src), SwapchainFormat::Bgra8Srgb, &mut out).is_err());
    }

    #[test]
    fn packed_length_is_four_bytes_a_pixel() {
        assert_eq!(packed_len(1920, 1080), 1920 * 1080 * 4);
    }
}
