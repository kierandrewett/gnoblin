//! Text layout: where each glyph of a string goes, and how big the string is.
//!
//! The painter draws exactly what [`FontSet::layout`] returns and [`FontSet::measure`] reports the size of exactly
//! that layout, so the extent the shell is told is the extent the painter draws in.
//!
//! # Rules
//!
//! - A string is split at `\n` into lines. Control characters other than `\n` and `\t` are dropped; a tab is a
//!   space.
//! - A glyph's advance is the font's unhinted advance at the size, in fractional pixels, summed along the line
//!   from the line's left edge. There is no kerning, shaping or bidirectional reordering: this is a stock-widget
//!   text path, and text that needs them is not what a titlebar or a panel shows.
//! - A line box is [`line_height`] tall: one and a half times the size, rounded down, the figure `ui.lua` uses for
//!   `Text.height`. The baseline sits where the font's ascent-to-descent span is centred in that box, rounded to a
//!   whole pixel so horizontal stems land on pixel rows.
//! - The extent of a string, a [`gnoblin_types::Size`], is its widest line's advance rounded up to a whole pixel by
//!   the lines' total height.

use gnoblin_types::Size;
use skrifa::instance::{LocationRef, Size as Ppem};
use skrifa::prelude::{GlyphId, MetadataProvider};

use crate::font::{FaceId, FontSet};

/// One placed glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// The face the glyph is drawn from, which may be the embedded face when the requested one lacks it.
    pub face: FaceId,
    /// The glyph in that face.
    pub glyph: GlyphId,
    /// The pen position of the glyph's origin from the line's left edge, in pixels.
    pub x: f32,
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    /// The glyphs, left to right.
    pub glyphs: Vec<PlacedGlyph>,
    /// The line's advance in pixels.
    pub width: f32,
}

/// A laid-out string.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    /// The lines, top to bottom; at least one.
    pub lines: Vec<TextLine>,
    /// The font size the layout was made at.
    pub size: f32,
    /// The distance from the top of a line box to its baseline, in whole pixels.
    pub baseline: f32,
}

/// The height of one line box at `size`.
#[must_use]
pub fn line_height(size: f32) -> f32 {
    (size * 1.5).floor()
}

impl TextLayout {
    /// The layout's size.
    #[must_use]
    pub fn extent(&self) -> Size {
        let widest = self.lines.iter().map(|line| line.width).fold(0.0_f32, f32::max);
        Size::new(
            widest.ceil() as u32,
            (line_height(self.size) * self.lines.len() as f32) as u32,
        )
    }
}

impl FontSet {
    /// Lay out `text` in the face `font` names at `size` pixels.
    #[must_use]
    pub fn layout(&self, text: &str, size: f32, font: &str) -> TextLayout {
        let mut face = self.resolve(font);
        if self.with_font(face, |_| ()).is_none() {
            // The file went away or does not parse: the embedded face is always readable.
            face = self.embedded_face();
        }
        let metrics = self
            .with_font(face, |f| f.metrics(Ppem::new(size), LocationRef::default()))
            .map(|m| (m.ascent, m.descent));
        let (ascent, descent) = metrics.unwrap_or((size * 0.9, -size * 0.2));
        let baseline = ((line_height(size) - (ascent - descent)) / 2.0 + ascent).round();
        let lines = text
            .split('\n')
            .map(|line| self.layout_line(line, size, face))
            .collect();
        TextLayout { lines, size, baseline }
    }

    /// The whole-pixel size `text` takes at `size` in `font`.
    #[must_use]
    pub fn measure(&self, text: &str, size: f32, font: &str) -> Size {
        self.layout(text, size, font).extent()
    }

    fn layout_line(&self, line: &str, size: f32, face: FaceId) -> TextLine {
        let mut glyphs = Vec::new();
        let mut pen = 0.0_f32;
        for character in line.chars() {
            let character = match character {
                '\t' => ' ',
                c if c.is_control() => continue,
                c => c,
            };
            let Some((from, glyph)) = self.glyph(face, character) else {
                continue;
            };
            let advance = self
                .with_font(from, |font| {
                    font.glyph_metrics(Ppem::new(size), LocationRef::default())
                        .advance_width(glyph)
                })
                .flatten()
                .unwrap_or(0.0);
            glyphs.push(PlacedGlyph {
                face: from,
                glyph,
                x: pen,
            });
            pen += advance;
        }
        TextLine { glyphs, width: pen }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_box_is_one_and_a_half_times_the_size_rounded_down() {
        assert_eq!(line_height(13.0), 19.0);
        assert_eq!(line_height(14.0), 21.0);
        assert_eq!(line_height(15.0), 22.0);
        assert_eq!(line_height(16.0), 24.0);
    }

    #[test]
    fn an_empty_string_is_one_empty_line() {
        let fonts = FontSet::embedded();
        assert_eq!(fonts.measure("", 14.0, "sans"), Size::new(0, 21));
    }

    #[test]
    fn a_longer_string_is_wider_and_a_bigger_size_is_taller_and_wider() {
        let fonts = FontSet::embedded();
        let short = fonts.measure("Hi", 14.0, "sans");
        let long = fonts.measure("Hi there", 14.0, "sans");
        let big = fonts.measure("Hi", 28.0, "sans");
        assert!(long.width > short.width);
        assert!(big.width > short.width && big.height > short.height);
        assert_eq!(short.height, 21);
    }

    #[test]
    fn newlines_make_lines_and_the_widest_one_sets_the_width() {
        let fonts = FontSet::embedded();
        let one = fonts.measure("wide line", 14.0, "sans");
        let two = fonts.measure("wide line\nab", 14.0, "sans");
        assert_eq!(two.width, one.width);
        assert_eq!(two.height, one.height * 2);
    }

    #[test]
    fn glyph_positions_are_the_running_sum_of_the_advances() {
        let fonts = FontSet::embedded();
        let layout = fonts.layout("Hello", 20.0, "sans");
        let line = &layout.lines[0];
        assert_eq!(line.glyphs.len(), 5);
        assert_eq!(line.glyphs[0].x, 0.0);
        assert!(line.glyphs.windows(2).all(|pair| pair[1].x > pair[0].x));
        let last = line.glyphs[4].x;
        assert!(line.width > last, "the line is wider than the last glyph's origin");
        assert_eq!(layout.extent().width, line.width.ceil() as u32);
    }

    #[test]
    fn controls_are_dropped_and_a_tab_is_a_space() {
        let fonts = FontSet::embedded();
        assert_eq!(
            fonts.measure("a\u{7}b", 14.0, "sans"),
            fonts.measure("ab", 14.0, "sans")
        );
        assert_eq!(fonts.measure("a\tb", 14.0, "sans"), fonts.measure("a b", 14.0, "sans"));
    }

    #[test]
    fn the_baseline_sits_inside_the_line_box() {
        let fonts = FontSet::embedded();
        for size in [10.0, 13.0, 14.0, 24.0] {
            let layout = fonts.layout("x", size, "sans");
            assert!(layout.baseline > 0.0 && layout.baseline < line_height(size), "{size}");
            assert_eq!(layout.baseline, layout.baseline.round());
        }
    }
}
