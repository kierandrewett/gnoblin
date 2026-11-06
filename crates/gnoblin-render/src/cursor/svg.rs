//! SVG cursor rasterisation with resvg (pure Rust).
//!
//! An SVG cursor is authored at its nominal size and drawn onto a square canvas
//! of the physical size, so the bitmap is sharp at every scale. The canvas is
//! the only thing that is rendered: external images and fonts are not loaded,
//! so a theme file cannot make the compositor read anything but its own bytes.
//!
//! Failure modes, each a named [`SvgError`]:
//! - the bytes are not an SVG document (`Parse`);
//! - the canvas cannot be allocated (`Canvas`, only for a zero or absurd size);
//! - the canvas exceeds the frame limit (`Frame`).
//!
//! The time one SVG may take is bounded by the 1 MiB file cap in
//! [`super::files`] and by the 512 pixel canvas cap in [`super::MAX_PHYSICAL_SIZE`];
//! resvg's own filter limits bound the rest. A pathological SVG can still take
//! noticeable time to render, which is why the chain caches every result.

use std::fmt;

use resvg::{tiny_skia, usvg};

use super::{CursorFrame, ImageError, MAX_PHYSICAL_SIZE, pack_argb};

/// Why an SVG cursor frame could not be produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SvgError {
    /// The bytes are not a parseable SVG document.
    Parse(String),
    /// The canvas for the requested size could not be allocated.
    Canvas,
    /// The rendered frame was refused by the image limits.
    Frame(ImageError),
}

impl fmt::Display for SvgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(reason) => write!(f, "svg does not parse: {reason}"),
            Self::Canvas => f.write_str("svg canvas cannot be allocated"),
            Self::Frame(error) => write!(f, "svg frame refused: {error}"),
        }
    }
}

/// Renders one SVG cursor, authored at `nominal` pixels, onto a square
/// `physical` pixel canvas.
///
/// The frame's delay is zero; the caller sets timing from the theme's metadata.
pub fn rasterise(bytes: &[u8], nominal: u32, physical: u32) -> Result<CursorFrame, SvgError> {
    if nominal == 0 || physical == 0 || physical > MAX_PHYSICAL_SIZE {
        return Err(SvgError::Canvas);
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|error| SvgError::Parse(error.to_string()))?;
    let mut pixmap = tiny_skia::Pixmap::new(physical, physical).ok_or(SvgError::Canvas)?;
    let scale = physical as f32 / nominal as f32;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied RGBA, which is the layout a frame holds.
    let pixels = pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| pack_argb(px[3], px[0], px[1], px[2]))
        .collect();
    CursorFrame::new(physical, physical, pixels, 0).map_err(SvgError::Frame)
}
