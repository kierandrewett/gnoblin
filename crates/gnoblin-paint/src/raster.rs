//! The rasteriser: a paint list to pixels, on the CPU.
//!
//! [`CanvasPainter::paint`] draws a [`PaintList`] into a transparent picture of a given size and returns it with
//! premultiplied alpha, the form the compositor's snapshots take (`SurfaceSource::Snapshot`). It is deterministic:
//! the same list, size and fonts give the same bytes. Anti-aliasing, clip masks and opacity groups come from
//! tiny-skia, whose `simd` feature is off here so no pixel depends on the build's CPU features; glyph outlines come
//! from skrifa unhinted.
//!
//! # How ops draw
//!
//! - A rectangle is filled, then outlined inside its edge. Its corner radius is clamped to half its shorter side.
//! - Text and icons are filled paths. A glyph is placed at its fractional pen position across a line and at a whole
//!   pixel baseline, so stems are sharp vertically and not snapped horizontally.
//! - A clip is a coverage mask (anti-aliased at rounded edges); nested clips intersect. An opacity group is drawn
//!   on a transparent layer the size of the picture and composited with its alpha when the group ends.
//! - Glyphs and lines that lie wholly past the canvas's edge are not drawn, so a text of thousands of characters at a
//!   large size costs what is on the canvas and not what is in the string.
//!
//! # Scale
//!
//! [`CanvasPainter::paint_scaled`] draws the same list at a physical-per-logical `scale`: the picture is that much
//! larger and every op is drawn through the scale, so strokes and glyphs are rasterised at the output's resolution
//! and not stretched from a 1x picture.
//!
//! # Limits
//!
//! A picture is at most [`MAX_PIXELS`] pixels; a larger or empty size, or a scale that is not a finite positive
//! number, has no picture. A paint list cannot make an op fail: an op the rasteriser cannot draw (a path with no
//! area, an icon that is not built in) draws nothing.

use std::sync::{Arc, OnceLock};

use gnoblin_types::Size;
use skrifa::instance::{LocationRef, Size as Ppem};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::MetadataProvider;
use tiny_skia::{FillRule, LineCap, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

use crate::font::FontSet;
use crate::icon;
use crate::list::{MAX_NAME_BYTES, MAX_SIZE, MAX_TEXT_BYTES, PaintList, PaintOp, Rgba8};
use crate::text::TextMetrics;

/// The most pixels one picture may have: a 4096 by 4096 canvas.
pub const MAX_PIXELS: u64 = 4096 * 4096;

/// The control-point offset of a cubic that approximates a quarter circle, as a fraction of the radius.
const KAPPA: f32 = 0.552_284_8;

/// A rendered picture: premultiplied RGBA8, top left first, rows packed at `width * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixels: red, green, blue, alpha per pixel, each colour channel already multiplied by alpha.
    pub data: Vec<u8>,
}

impl Picture {
    /// The premultiplied pixel at `(x, y)`, or `None` outside the picture.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        Some([self.data[at], self.data[at + 1], self.data[at + 2], self.data[at + 3]])
    }

    /// The same pixels with straight alpha, for a consumer that blends non-premultiplied colour.
    #[must_use]
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for pixel in out.as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            if alpha != 0 && alpha != 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        out
    }
}

/// What one paint produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// The picture.
    pub picture: Picture,
    /// The names of icon ops that are not built in, which drew nothing, each once.
    pub missing_icons: Vec<String>,
}

/// Draws paint lists with a set of fonts.
#[derive(Clone, Debug)]
pub struct CanvasPainter {
    fonts: Arc<FontSet>,
}

impl CanvasPainter {
    /// A painter over `fonts`.
    #[must_use]
    pub const fn new(fonts: Arc<FontSet>) -> Self {
        Self { fonts }
    }

    /// The fonts this painter draws with.
    #[must_use]
    pub fn fonts(&self) -> &FontSet {
        &self.fonts
    }

    /// The whole-pixel size of `text` at `size` in `font`: the extent [`CanvasPainter::paint`] lays that text out in.
    #[must_use]
    pub fn measure(&self, text: &str, size: f32, font: &str) -> Size {
        self.fonts.measure(text, size, font)
    }

    /// [`CanvasPainter::measure`] for a call that comes from outside: a size that is not a finite number in
    /// `(0, MAX_SIZE]`, text over [`crate::list::MAX_TEXT_BYTES`] or a font name over [`crate::list::MAX_NAME_BYTES`]
    /// is refused with the answer's error name instead of laid out.
    ///
    /// # Errors
    ///
    /// `bad-value` for a size out of range, `too-large` for text or a name over its cap.
    pub fn measure_checked(&self, text: &str, size: f32, font: &str) -> Result<TextMetrics, &'static str> {
        if !(size.is_finite() && size > 0.0 && size <= MAX_SIZE) {
            return Err("bad-value");
        }
        if text.len() > MAX_TEXT_BYTES || font.len() > MAX_NAME_BYTES {
            return Err("too-large");
        }
        Ok(self.fonts.text_metrics(text, size, font))
    }

    /// Draw `list` into a `width` by `height` picture, or `None` when the size is empty or over [`MAX_PIXELS`].
    #[must_use]
    pub fn paint(&self, list: &PaintList, width: u32, height: u32) -> Option<Rendered> {
        self.paint_scaled(list, width, height, 1.0)
    }

    /// Draw `list`, whose coordinates are in a `width` by `height` logical canvas, at `scale` physical pixels per
    /// logical pixel: the picture is `round(width * scale)` by `round(height * scale)` pixels (at least one each) and
    /// every op is drawn scaled, so a HiDPI output shows crisp text and strokes instead of a stretched 1x picture.
    /// `None` when the logical size is empty, `scale` is not a finite positive number, or the picture would hold more
    /// than [`MAX_PIXELS`] pixels.
    #[must_use]
    pub fn paint_scaled(&self, list: &PaintList, width: u32, height: u32, scale: f64) -> Option<Rendered> {
        if width == 0 || height == 0 || !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        let pixel_width = ((f64::from(width) * scale).round() as u64).max(1);
        let pixel_height = ((f64::from(height) * scale).round() as u64).max(1);
        if pixel_width * pixel_height > MAX_PIXELS {
            return None;
        }
        let (pixel_width, pixel_height) = (pixel_width as u32, pixel_height as u32);
        let base = Pixmap::new(pixel_width, pixel_height)?;
        let mut canvas = Canvas {
            fonts: &self.fonts,
            layers: vec![OpacityLayer {
                pixmap: base,
                alpha: 1.0,
            }],
            clips: Vec::new(),
            width: pixel_width,
            height: pixel_height,
            logical: (width as f32, height as f32),
            transform: Transform::from_scale(pixel_width as f32 / width as f32, pixel_height as f32 / height as f32),
            missing: Vec::new(),
        };
        for op in &list.ops {
            canvas.draw(op);
        }
        // The decoder guarantees balance, but a hand-built list may not be: close what is left open.
        while canvas.layers.len() > 1 {
            canvas.pop_opacity();
        }
        let pixmap = canvas.layers.pop()?.pixmap;
        Some(Rendered {
            picture: Picture {
                width: pixel_width,
                height: pixel_height,
                data: pixmap.take(),
            },
            missing_icons: canvas.missing,
        })
    }
}

/// A painter shared between the tree, which measures with it, and the composer, which paints with it.
///
/// The system font scan is not free, so a compositor that never paints a canvas should not pay it: the painter is
/// built the first time [`SharedPainter::get`] is called. Clones share one painter.
#[derive(Clone, Debug)]
pub struct SharedPainter {
    cell: Arc<OnceLock<CanvasPainter>>,
    system: bool,
}

impl SharedPainter {
    /// A painter over the system fonts and the embedded face, built on first use.
    #[must_use]
    pub fn system() -> Self {
        Self {
            cell: Arc::new(OnceLock::new()),
            system: true,
        }
    }

    /// A painter over the embedded face alone, whose output is the same on every machine.
    #[must_use]
    pub fn embedded() -> Self {
        Self {
            cell: Arc::new(OnceLock::new()),
            system: false,
        }
    }

    /// The painter, built if this is the first call.
    #[must_use]
    pub fn get(&self) -> &CanvasPainter {
        self.cell.get_or_init(|| {
            let fonts = if self.system {
                FontSet::system()
            } else {
                FontSet::embedded()
            };
            CanvasPainter::new(Arc::new(fonts))
        })
    }
}

/// An opacity group being drawn.
struct OpacityLayer {
    pixmap: Pixmap,
    /// The alpha the group is composited with when it ends.
    alpha: f32,
}

struct Canvas<'a> {
    fonts: &'a FontSet,
    /// The picture first, then each open opacity group.
    layers: Vec<OpacityLayer>,
    /// The open clips, each already intersected with the ones outside it.
    clips: Vec<Mask>,
    /// The picture's size in pixels.
    width: u32,
    height: u32,
    /// The canvas's size in logical pixels, the units the ops are in.
    logical: (f32, f32),
    /// Logical to picture pixels.
    transform: Transform,
    missing: Vec<String>,
}

impl Canvas<'_> {
    fn draw(&mut self, op: &PaintOp) {
        match op {
            PaintOp::Rect {
                x,
                y,
                width,
                height,
                radius,
                fill,
                stroke,
                stroke_width,
            } => self.rect(*x, *y, *width, *height, *radius, *fill, *stroke, *stroke_width),
            PaintOp::TextLine {
                x1,
                y1,
                x2,
                y2,
                width,
                colour,
            } => self.line(*x1, *y1, *x2, *y2, *width, *colour),
            PaintOp::Text {
                x,
                y,
                size,
                colour,
                font,
                text,
            } => self.text(*x, *y, *size, *colour, font, text),
            PaintOp::Icon {
                x,
                y,
                size,
                colour,
                name,
            } => self.icon(*x, *y, *size, *colour, name),
            PaintOp::PushClip {
                x,
                y,
                width,
                height,
                radius,
            } => self.push_clip(*x, *y, *width, *height, *radius),
            PaintOp::PopClip => {
                self.clips.pop();
            }
            PaintOp::PushOpacity(alpha) => self.push_opacity(*alpha),
            PaintOp::PopOpacity => self.pop_opacity(),
        }
    }

    fn target(&mut self) -> (&mut Pixmap, Option<&Mask>) {
        let layer = self.layers.last_mut().expect("the base layer is always open");
        (&mut layer.pixmap, self.clips.last())
    }

    #[allow(clippy::too_many_arguments)]
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, fill: Rgba8, stroke: Rgba8, stroke_width: f32) {
        let transform = self.transform;
        if !fill.is_transparent()
            && let Some(path) = rounded_rect(x, y, w, h, radius)
        {
            let (pixmap, mask) = self.target();
            pixmap.fill_path(&path, &paint(fill), FillRule::Winding, transform, mask);
        }
        if !stroke.is_transparent()
            && stroke_width > 0.0
            && let Some(path) = ring(x, y, w, h, radius, stroke_width)
        {
            let (pixmap, mask) = self.target();
            pixmap.fill_path(&path, &paint(stroke), FillRule::EvenOdd, transform, mask);
        }
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, colour: Rgba8) {
        if colour.is_transparent() || width <= 0.0 {
            return;
        }
        let mut builder = PathBuilder::new();
        builder.move_to(x1, y1);
        builder.line_to(x2, y2);
        let Some(path) = builder.finish() else {
            return;
        };
        let line = Stroke {
            width,
            line_cap: LineCap::Butt,
            ..Stroke::default()
        };
        let transform = self.transform;
        let (pixmap, mask) = self.target();
        pixmap.stroke_path(&path, &paint(colour), &line, transform, mask);
    }

    fn text(&mut self, x: f32, y: f32, size: f32, colour: Rgba8, font: &str, text: &str) {
        if colour.is_transparent() || size <= 0.0 {
            return;
        }
        let layout = self.fonts.layout(text, size, font);
        let line_height = crate::text::line_height(size);
        let paint = paint(colour);
        let transform = self.transform;
        let (logical_width, logical_height) = self.logical;
        // A glyph's ink lies within about a font size of its pen position, so a glyph or a line that begins that far
        // past the canvas cannot show: a text of thousands of characters at a large size draws only what is on it.
        let reach = size * 2.0;
        for (index, line) in layout.lines.iter().enumerate() {
            let top = y + line_height * index as f32;
            if top > logical_height + reach {
                break;
            }
            if top + line_height < -reach {
                continue;
            }
            let baseline = top + layout.baseline;
            for placed in &line.glyphs {
                let pen = x + placed.x;
                if pen > logical_width + reach {
                    break;
                }
                if pen < -reach {
                    continue;
                }
                let path = self.fonts.with_font(placed.face, |font| {
                    let glyph = font.outline_glyphs().get(placed.glyph)?;
                    let mut outline = PathPen {
                        builder: PathBuilder::new(),
                        x: pen,
                        y: baseline,
                    };
                    glyph
                        .draw(
                            DrawSettings::unhinted(Ppem::new(size), LocationRef::default()),
                            &mut outline,
                        )
                        .ok()?;
                    outline.builder.finish()
                });
                if let Some(Some(path)) = path {
                    let (pixmap, mask) = self.target();
                    pixmap.fill_path(&path, &paint, FillRule::Winding, transform, mask);
                }
            }
        }
    }

    fn icon(&mut self, x: f32, y: f32, size: f32, colour: Rgba8, name: &str) {
        let Some(pieces) = icon::pieces(name) else {
            if !self.missing.iter().any(|missing| missing == name) {
                self.missing.push(name.to_owned());
            }
            return;
        };
        if colour.is_transparent() || size <= 0.0 {
            return;
        }
        let scale = size / icon::GRID;
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, x, y).post_concat(self.transform);
        let paint = paint(colour);
        for piece in pieces {
            let (pixmap, mask) = self.target();
            pixmap.stroke_path(&piece.path, &paint, &piece.stroke, transform, mask);
        }
    }

    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let mut mask = match self.clips.last() {
            Some(outer) => outer.clone(),
            None => {
                let mut full = Mask::new(self.width, self.height).expect("the picture has a size");
                // An empty mask is a clip that passes nothing; start from the whole picture instead.
                full.data_mut().fill(255);
                full
            }
        };
        match rounded_rect(x, y, w, h, radius) {
            Some(path) => mask.intersect_path(&path, FillRule::Winding, true, self.transform),
            None => mask.clear(),
        }
        self.clips.push(mask);
    }

    fn push_opacity(&mut self, alpha: f32) {
        let pixmap = Pixmap::new(self.width, self.height).expect("the picture has a size");
        self.layers.push(OpacityLayer { pixmap, alpha });
    }

    fn pop_opacity(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let group = self.layers.pop().expect("a group is open");
        let below = self.layers.last_mut().expect("the base layer is always open");
        let paint = PixmapPaint {
            opacity: group.alpha,
            ..PixmapPaint::default()
        };
        below
            .pixmap
            .draw_pixmap(0, 0, group.pixmap.as_ref(), &paint, Transform::identity(), None);
    }
}

fn paint(colour: Rgba8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(colour.r, colour.g, colour.b, colour.a);
    paint.anti_alias = true;
    paint
}

/// A rectangle with rounded corners as a path, or `None` when it has no area. A radius over half the shorter side
/// is clamped.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let r = radius.clamp(0.0, w.min(h) / 2.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    let k = r * KAPPA;
    let (right, bottom) = (x + w, y + h);
    let mut builder = PathBuilder::new();
    builder.move_to(x + r, y);
    builder.line_to(right - r, y);
    builder.cubic_to(right - r + k, y, right, y + r - k, right, y + r);
    builder.line_to(right, bottom - r);
    builder.cubic_to(right, bottom - r + k, right - r + k, bottom, right - r, bottom);
    builder.line_to(x + r, bottom);
    builder.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
    builder.line_to(x, y + r);
    builder.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    builder.close();
    builder.finish()
}

/// The outline of a rounded rectangle as a filled shape: the rectangle with its inner edge, inset by `width`,
/// cut out (fill it even-odd). When the inset leaves no inner rectangle the whole rectangle is the outline.
fn ring(x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32) -> Option<Path> {
    let outer = rounded_rect(x, y, w, h, radius)?;
    let inner_radius = (radius.min(w.min(h) / 2.0) - width).max(0.0);
    let Some(inner) = rounded_rect(x + width, y + width, w - 2.0 * width, h - 2.0 * width, inner_radius) else {
        return Some(outer);
    };
    let mut builder = PathBuilder::new();
    builder.push_path(&outer);
    builder.push_path(&inner);
    builder.finish()
}

/// Collects a glyph outline into a path at a position, flipping y from the font's up to the picture's down.
struct PathPen {
    builder: PathBuilder,
    x: f32,
    y: f32,
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(self.x + x, self.y - y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(self.x + x, self.y - y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.builder.quad_to(self.x + cx0, self.y - cy0, self.x + x, self.y - y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.builder.cubic_to(
            self.x + cx0,
            self.y - cy0,
            self.x + cx1,
            self.y - cy1,
            self.x + x,
            self.y - y,
        );
    }

    fn close(&mut self) {
        self.builder.close();
    }
}
