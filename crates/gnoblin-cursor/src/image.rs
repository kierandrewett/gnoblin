//! Cursor bitmaps: premultiplied ARGB frames, hotspots and resampling.
//!
//! Every provider hands back a [`CursorImages`] in this one shape, so the
//! renderer never needs to know which theme format a pointer came from. Pixels
//! are premultiplied `0xAARRGGBB` words, which is the layout the GPU upload and
//! the software blend both read without a conversion.

use std::fmt;

/// The largest edge, in physical pixels, a single decoded frame may have.
///
/// A theme frame larger than this is refused as a [`ImageError::TooLarge`]
/// failure, which bounds the memory one corrupt or hostile file can take.
pub const MAX_IMAGE_EDGE: u32 = 1024;

/// The largest physical cursor size the compositor will ask a provider for.
///
/// 256 logical pixels at scale 2 is the largest legal request; anything above
/// this is clamped before lookup, so a provider never rasterises past it.
pub const MAX_PHYSICAL_SIZE: u32 = 512;

/// Why an image could not be built from its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// A frame has a zero width or height.
    ZeroSize,
    /// A frame has an edge above [`MAX_IMAGE_EDGE`].
    TooLarge,
    /// The pixel count does not equal `width * height`.
    PixelCount,
    /// An animation has no frames.
    NoFrames,
    /// The frames of one animation do not share one size.
    FrameSizeMismatch,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroSize => "image has a zero-size frame",
            Self::TooLarge => "image frame is larger than the decode limit",
            Self::PixelCount => "pixel count does not match the frame size",
            Self::NoFrames => "animation has no frames",
            Self::FrameSizeMismatch => "animation frames have different sizes",
        })
    }
}

/// One frame of a cursor: a premultiplied ARGB bitmap and how long it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorFrame {
    width: u32,
    height: u32,
    /// Row-major, top row first; each word is `0xAARRGGBB`, premultiplied.
    pixels: Vec<u32>,
    /// How long this frame stays on screen, in milliseconds. Zero for a
    /// static image; an animated frame always shows for at least one.
    delay_ms: u32,
}

impl CursorFrame {
    /// Builds a frame, checking its size and pixel count.
    ///
    /// # Errors
    ///
    /// Returns [`ImageError::ZeroSize`] for an empty edge,
    /// [`ImageError::TooLarge`] past [`MAX_IMAGE_EDGE`], and
    /// [`ImageError::PixelCount`] when the pixels do not fill the frame.
    pub fn new(width: u32, height: u32, pixels: Vec<u32>, delay_ms: u32) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::ZeroSize);
        }
        if width > MAX_IMAGE_EDGE || height > MAX_IMAGE_EDGE {
            return Err(ImageError::TooLarge);
        }
        if pixels.len() != width as usize * height as usize {
            return Err(ImageError::PixelCount);
        }
        Ok(Self {
            width,
            height,
            pixels,
            delay_ms,
        })
    }

    /// The width in physical pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in physical pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The premultiplied `0xAARRGGBB` pixels, row-major, top row first.
    #[must_use]
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// How long the frame shows, in milliseconds.
    #[must_use]
    pub fn delay_ms(&self) -> u32 {
        self.delay_ms
    }

    /// The same frame with another delay.
    #[must_use]
    pub fn with_delay(mut self, delay_ms: u32) -> Self {
        self.delay_ms = delay_ms;
        self
    }
}

/// A cursor: one or more frames and the point that is the click position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorImages {
    hotspot: (u32, u32),
    hotspot_clamped: bool,
    frames: Vec<CursorFrame>,
}

impl CursorImages {
    /// Builds a cursor from frames of one size and a hotspot.
    ///
    /// A hotspot outside the image is clamped to its nearest edge pixel and
    /// recorded, so [`CursorImages::hotspot_was_clamped`] can report it.
    ///
    /// # Errors
    ///
    /// Returns [`ImageError::NoFrames`] for an empty list and
    /// [`ImageError::FrameSizeMismatch`] when frames differ in size.
    pub fn new(hotspot: (i64, i64), frames: Vec<CursorFrame>) -> Result<Self, ImageError> {
        let first = frames.first().ok_or(ImageError::NoFrames)?;
        let (width, height) = (first.width, first.height);
        if frames
            .iter()
            .any(|frame| frame.width != width || frame.height != height)
        {
            return Err(ImageError::FrameSizeMismatch);
        }
        let clamp = |value: i64, edge: u32| value.clamp(0, i64::from(edge) - 1);
        let (x, y) = (clamp(hotspot.0, width), clamp(hotspot.1, height));
        Ok(Self {
            hotspot: (x as u32, y as u32),
            hotspot_clamped: x != hotspot.0 || y != hotspot.1,
            frames,
        })
    }

    /// The click position, in pixels from the top-left corner.
    #[must_use]
    pub fn hotspot(&self) -> (u32, u32) {
        self.hotspot
    }

    /// Whether the theme's hotspot lay outside the image and was moved inside.
    #[must_use]
    pub fn hotspot_was_clamped(&self) -> bool {
        self.hotspot_clamped
    }

    /// The frames, in playback order.
    #[must_use]
    pub fn frames(&self) -> &[CursorFrame] {
        &self.frames
    }

    /// The width of every frame, in physical pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.frames[0].width
    }

    /// The height of every frame, in physical pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.frames[0].height
    }

    /// The frame to show `elapsed_ms` after the animation started.
    ///
    /// A static cursor always returns its one frame. An animation loops; a
    /// frame's delay of zero counts as one millisecond so the loop always moves.
    #[must_use]
    pub fn frame_at(&self, elapsed_ms: u64) -> (usize, &CursorFrame) {
        if self.frames.len() == 1 {
            return (0, &self.frames[0]);
        }
        let total: u64 = self.frames.iter().map(|frame| u64::from(frame.delay_ms.max(1))).sum();
        let mut remaining = elapsed_ms % total;
        for (index, frame) in self.frames.iter().enumerate() {
            let delay = u64::from(frame.delay_ms.max(1));
            if remaining < delay {
                return (index, frame);
            }
            remaining -= delay;
        }
        (self.frames.len() - 1, &self.frames[self.frames.len() - 1])
    }

    /// The pixel bytes this cursor holds, for cache accounting.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.frames.iter().map(|frame| frame.pixels.len() * 4).sum()
    }

    /// The same cursor at another size, with its hotspot scaled to match.
    ///
    /// Each frame is resampled bilinearly in premultiplied space, so transparent
    /// edges do not pick up dark fringes. A target equal to the source size is
    /// returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`ImageError::ZeroSize`] or [`ImageError::TooLarge`] when the
    /// target size is not a legal frame size.
    pub fn resampled(&self, width: u32, height: u32) -> Result<Self, ImageError> {
        let (src_w, src_h) = (self.width(), self.height());
        if (width, height) == (src_w, src_h) {
            return Ok(self.clone());
        }
        let frames = self
            .frames
            .iter()
            .map(|frame| resample_frame(frame, width, height))
            .collect::<Result<Vec<_>, _>>()?;
        let hotspot = (
            scale_coordinate(self.hotspot.0, src_w, width),
            scale_coordinate(self.hotspot.1, src_h, height),
        );
        Self::new((i64::from(hotspot.0), i64::from(hotspot.1)), frames)
    }
}

/// Packs four 8-bit channels into one `0xAARRGGBB` word.
#[must_use]
pub const fn pack_argb(alpha: u8, red: u8, green: u8, blue: u8) -> u32 {
    ((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}

/// Premultiplies straight RGBA channels and packs them as one `0xAARRGGBB`
/// word, rounding each colour channel to the nearest value.
#[must_use]
pub fn premultiplied(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    let scale = |channel: u8| ((u32::from(channel) * u32::from(alpha) + 127) / 255) as u8;
    pack_argb(alpha, scale(red), scale(green), scale(blue))
}

/// Maps a hotspot coordinate from one edge length to another, rounding to the
/// nearest pixel and keeping the result inside the new edge.
fn scale_coordinate(value: u32, from: u32, to: u32) -> u32 {
    let scaled = (u64::from(value) * u64::from(to) + u64::from(from) / 2) / u64::from(from.max(1));
    (scaled.min(u64::from(to.max(1)) - 1)) as u32
}

/// Bilinear resampling of one premultiplied frame.
fn resample_frame(frame: &CursorFrame, width: u32, height: u32) -> Result<CursorFrame, ImageError> {
    let (src_w, src_h) = (frame.width as usize, frame.height as usize);
    let (dst_w, dst_h) = (width as usize, height as usize);
    let mut pixels = Vec::with_capacity(dst_w * dst_h);
    for y in 0..dst_h {
        let sy = ((y as f64 + 0.5) * src_h as f64 / dst_h as f64 - 0.5).clamp(0.0, (src_h - 1) as f64);
        let (y0, fy) = (sy.floor() as usize, sy - sy.floor());
        let y1 = (y0 + 1).min(src_h - 1);
        for x in 0..dst_w {
            let sx = ((x as f64 + 0.5) * src_w as f64 / dst_w as f64 - 0.5).clamp(0.0, (src_w - 1) as f64);
            let (x0, fx) = (sx.floor() as usize, sx - sx.floor());
            let x1 = (x0 + 1).min(src_w - 1);
            let at = |px: usize, py: usize| frame.pixels[py * src_w + px];
            let corners = [
                (at(x0, y0), (1.0 - fx) * (1.0 - fy)),
                (at(x1, y0), fx * (1.0 - fy)),
                (at(x0, y1), (1.0 - fx) * fy),
                (at(x1, y1), fx * fy),
            ];
            let mut channels = [0.0_f64; 4];
            for (word, weight) in corners {
                for (shift, channel) in [24, 16, 8, 0].into_iter().enumerate() {
                    channels[shift] += f64::from((word >> channel) & 0xff) * weight;
                }
            }
            let byte = |value: f64| value.round().clamp(0.0, 255.0) as u8;
            pixels.push(pack_argb(
                byte(channels[0]),
                byte(channels[1]),
                byte(channels[2]),
                byte(channels[3]),
            ));
        }
    }
    CursorFrame::new(width, height, pixels, frame.delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, word: u32, delay: u32) -> CursorFrame {
        CursorFrame::new(width, height, vec![word; (width * height) as usize], delay).unwrap()
    }

    #[test]
    fn a_zero_size_or_mismatched_frame_is_refused_by_name() {
        assert_eq!(CursorFrame::new(0, 4, Vec::new(), 0), Err(ImageError::ZeroSize));
        assert_eq!(CursorFrame::new(2, 2, vec![0; 3], 0), Err(ImageError::PixelCount));
        assert_eq!(
            CursorFrame::new(MAX_IMAGE_EDGE + 1, 1, vec![0; 1025], 0),
            Err(ImageError::TooLarge)
        );
        assert_eq!(CursorImages::new((0, 0), Vec::new()), Err(ImageError::NoFrames));
        let mismatch = vec![solid(2, 2, 0, 0), solid(3, 2, 0, 0)];
        assert_eq!(CursorImages::new((0, 0), mismatch), Err(ImageError::FrameSizeMismatch));
    }

    #[test]
    fn an_absurd_hotspot_is_clamped_into_the_image_and_reported() {
        let image = CursorImages::new((-5, 900), vec![solid(4, 3, 0xff00_0000, 0)]).unwrap();
        assert_eq!(image.hotspot(), (0, 2));
        assert!(image.hotspot_was_clamped());
        let inside = CursorImages::new((3, 2), vec![solid(4, 3, 0, 0)]).unwrap();
        assert_eq!(inside.hotspot(), (3, 2));
        assert!(!inside.hotspot_was_clamped());
    }

    #[test]
    fn an_animation_advances_by_frame_delay_and_loops() {
        let frames = vec![solid(1, 1, 1, 10), solid(1, 1, 2, 30), solid(1, 1, 3, 0)];
        let image = CursorImages::new((0, 0), frames).unwrap();
        let index = |ms| image.frame_at(ms).0;
        assert_eq!(index(0), 0);
        assert_eq!(index(9), 0);
        assert_eq!(index(10), 1);
        assert_eq!(index(39), 1);
        assert_eq!(index(40), 2);
        assert_eq!(
            index(41),
            0,
            "the zero-delay frame shows for one millisecond, then the loop restarts"
        );
        assert_eq!(index(10_000_000), index(10_000_000 % 41));
    }

    #[test]
    fn a_static_cursor_is_always_its_one_frame() {
        let image = CursorImages::new((0, 0), vec![solid(2, 2, 7, 0)]).unwrap();
        assert_eq!(image.frame_at(123_456).0, 0);
    }

    #[test]
    fn resampling_scales_the_size_and_the_hotspot_and_keeps_colour() {
        let image = CursorImages::new((2, 4), vec![solid(4, 8, 0xff11_2233, 0)]).unwrap();
        let doubled = image.resampled(8, 16).unwrap();
        assert_eq!((doubled.width(), doubled.height()), (8, 16));
        assert_eq!(doubled.hotspot(), (4, 8));
        assert!(doubled.frames()[0].pixels().iter().all(|&p| p == 0xff11_2233));
        assert_eq!(image.resampled(4, 8).unwrap(), image);
    }
}
