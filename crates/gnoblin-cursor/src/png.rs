//! PNG cursor frames, decoded to premultiplied pixels.
//!
//! Only 8-bit greyscale, greyscale-alpha, RGB and RGBA images are accepted. A
//! palette (indexed) or 16-bit image is refused by name rather than converted,
//! and so is a source larger than [`MAX_IMAGE_EDGE`] on either side. A square
//! source is resampled with nearest-neighbour to the physical canvas, so the
//! frame has the same geometry as an SVG frame of the same request.

use std::fmt;
use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder};

use crate::image::{CursorFrame, CursorImages, ImageError, MAX_IMAGE_EDGE, MAX_PHYSICAL_SIZE, premultiplied};

/// Why a PNG frame is refused.
#[derive(Debug)]
pub enum PngError {
    /// The bytes do not decode as a PNG.
    Decode(String),
    /// The image uses a colour type or bit depth this provider does not accept.
    Unsupported(&'static str),
    /// The source is not square, or is larger than [`MAX_IMAGE_EDGE`].
    Dimensions { width: u32, height: u32 },
    /// The physical canvas is empty or larger than [`MAX_PHYSICAL_SIZE`].
    Canvas,
    /// The decoded frame is refused by [`CursorFrame::new`].
    Frame(ImageError),
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(reason) => write!(f, "png does not decode: {reason}"),
            Self::Unsupported(what) => write!(f, "png {what} is not supported"),
            Self::Dimensions { width, height } => write!(
                f,
                "png is {width}x{height}; it must be square and at most {MAX_IMAGE_EDGE} pixels"
            ),
            Self::Canvas => f.write_str("png canvas size is out of range"),
            Self::Frame(error) => write!(f, "png frame refused: {error}"),
        }
    }
}

/// A PNG decoded at its own size: 8-bit samples, `channels` of them to a pixel.
struct Decoded {
    width: u32,
    channels: usize,
    buffer: Vec<u8>,
}

impl Decoded {
    /// The straight-alpha colour of the pixel at `x`, `y`.
    fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let at = (y as usize * self.width as usize + x as usize) * self.channels;
        let px = &self.buffer[at..at + self.channels];
        match self.channels {
            1 => (px[0], px[0], px[0], 255),
            2 => (px[0], px[0], px[0], px[1]),
            3 => (px[0], px[1], px[2], 255),
            _ => (px[0], px[1], px[2], px[3]),
        }
    }
}

/// Reads a square 8-bit PNG of at most [`MAX_IMAGE_EDGE`] pixels an edge.
fn decode(bytes: &[u8]) -> Result<Decoded, PngError> {
    let mut reader = Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|error| PngError::Decode(error.to_string()))?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if width != height || width == 0 || width > MAX_IMAGE_EDGE {
        return Err(PngError::Dimensions { width, height });
    }
    let (color, depth) = {
        let info = reader.info();
        (info.color_type, info.bit_depth)
    };
    match (color, depth) {
        (ColorType::Indexed, _) => return Err(PngError::Unsupported("palette image")),
        (_, BitDepth::Eight) => {}
        _ => return Err(PngError::Unsupported("image deeper than 8 bits")),
    }
    let channels = match color {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => unreachable!("indexed images are refused above"),
    };
    let mut buffer = vec![0; width as usize * height as usize * channels];
    reader
        .next_frame(&mut buffer)
        .map_err(|error| PngError::Decode(error.to_string()))?;
    Ok(Decoded {
        width,
        channels,
        buffer,
    })
}

/// Decodes one PNG cursor frame onto a square `physical` pixel canvas.
///
/// The frame's delay is zero; the caller sets timing from the theme's metadata.
pub fn rasterise(bytes: &[u8], physical: u32) -> Result<CursorFrame, PngError> {
    if physical == 0 || physical > MAX_PHYSICAL_SIZE {
        return Err(PngError::Canvas);
    }
    let image = decode(bytes)?;
    let width = image.width;
    let mut pixels = Vec::with_capacity(physical as usize * physical as usize);
    for y in 0..physical {
        let sy = u64::from(y) * u64::from(width) / u64::from(physical);
        for x in 0..physical {
            let sx = u64::from(x) * u64::from(width) / u64::from(physical);
            let (r, g, b, a) = image.pixel(sx as u32, sy as u32);
            pixels.push(premultiplied(r, g, b, a));
        }
    }
    CursorFrame::new(physical, physical, pixels, 0).map_err(PngError::Frame)
}

/// Decodes a PNG a program supplies as a whole cursor at the image's own size, with `hotspot` (in image pixels, moved
/// inside the image when it lies outside) as the point that sits on the pointer.
///
/// The same PNGs [`rasterise`] takes: square, 8 bits a channel, at most [`MAX_IMAGE_EDGE`] an edge. The image is not
/// resampled, so one image pixel is one device pixel when it is drawn.
pub fn decode_image(bytes: &[u8], hotspot: (i64, i64)) -> Result<CursorImages, PngError> {
    let image = decode(bytes)?;
    let width = image.width;
    let mut pixels = Vec::with_capacity(width as usize * width as usize);
    for y in 0..width {
        for x in 0..width {
            let (r, g, b, a) = image.pixel(x, y);
            pixels.push(premultiplied(r, g, b, a));
        }
    }
    let frame = CursorFrame::new(width, width, pixels, 0).map_err(PngError::Frame)?;
    CursorImages::new(hotspot, vec![frame]).map_err(PngError::Frame)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `width` x `width` RGBA PNG whose pixels are all `rgba`.
    pub(crate) fn encode(width: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, width);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(data).expect("data");
        }
        out
    }

    #[test]
    fn an_rgba_png_decodes_to_premultiplied_pixels() {
        // Two opaque red pixels and two half-transparent white pixels.
        let data = [255, 0, 0, 255, 255, 0, 0, 255, 255, 255, 255, 128, 255, 255, 255, 128];
        let bytes = encode(2, png::ColorType::Rgba, png::BitDepth::Eight, &data);
        let frame = rasterise(&bytes, 2).expect("decodes");
        assert_eq!((frame.width(), frame.height()), (2, 2));
        assert_eq!(frame.pixels()[0], premultiplied(255, 0, 0, 255));
        assert_eq!(frame.pixels()[2], premultiplied(255, 255, 255, 128));
        assert_eq!(frame.pixels()[2] >> 24, 128);
    }

    #[test]
    fn a_greyscale_png_is_expanded_to_all_three_channels() {
        let bytes = encode(1, png::ColorType::Grayscale, png::BitDepth::Eight, &[200]);
        let frame = rasterise(&bytes, 1).expect("decodes");
        assert_eq!(frame.pixels(), &[premultiplied(200, 200, 200, 255)]);
    }

    #[test]
    fn a_source_is_resampled_to_the_physical_canvas() {
        let data = [0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255];
        let bytes = encode(2, png::ColorType::Rgba, png::BitDepth::Eight, &data);
        let frame = rasterise(&bytes, 4).expect("decodes");
        assert_eq!((frame.width(), frame.height()), (4, 4));
        assert_eq!(frame.pixels()[0], premultiplied(0, 0, 0, 255));
        assert_eq!(frame.pixels()[2], premultiplied(255, 255, 255, 255));
    }

    #[test]
    fn a_sixteen_bit_png_is_refused_by_name() {
        let bytes = encode(1, png::ColorType::Grayscale, png::BitDepth::Sixteen, &[0, 1]);
        let error = rasterise(&bytes, 1).expect_err("refused");
        assert!(matches!(error, PngError::Unsupported(_)), "{error}");
    }

    #[test]
    fn a_non_square_png_is_refused() {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&[0; 8]).expect("data");
        }
        assert!(matches!(rasterise(&out, 1), Err(PngError::Dimensions { .. })));
    }

    #[test]
    fn a_truncated_png_is_refused() {
        let bytes = encode(2, png::ColorType::Rgba, png::BitDepth::Eight, &[0; 16]);
        assert!(matches!(
            rasterise(&bytes[..bytes.len() / 2], 2),
            Err(PngError::Decode(_))
        ));
    }

    #[test]
    fn a_supplied_image_keeps_its_own_size_and_hotspot() {
        let data = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128];
        let bytes = encode(2, png::ColorType::Rgba, png::BitDepth::Eight, &data);
        let images = decode_image(&bytes, (1, 0)).expect("decodes");
        assert_eq!((images.width(), images.height()), (2, 2), "not resampled");
        assert_eq!(images.hotspot(), (1, 0));
        assert_eq!(images.frames()[0].pixels()[0], premultiplied(255, 0, 0, 255));
        assert_eq!(images.frames()[0].pixels()[3], premultiplied(255, 255, 255, 128));
    }

    #[test]
    fn a_supplied_images_hotspot_outside_it_is_moved_to_its_nearest_edge_pixel() {
        let bytes = encode(2, png::ColorType::Rgba, png::BitDepth::Eight, &[255; 16]);
        let images = decode_image(&bytes, (-5, 40)).expect("decodes");
        assert_eq!(images.hotspot(), (0, 1));
        assert!(images.hotspot_was_clamped());
    }

    #[test]
    fn a_supplied_image_that_is_not_a_png_is_refused() {
        assert!(matches!(decode_image(b"not a png", (0, 0)), Err(PngError::Decode(_))));
        assert!(matches!(decode_image(&[], (0, 0)), Err(PngError::Decode(_))));
    }
}
