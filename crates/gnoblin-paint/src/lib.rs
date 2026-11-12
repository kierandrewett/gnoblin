//! The Canvas painter: the paint list a shell builds, drawn into pixels, and text measurement.
//!
//! A `Canvas` node's `content` is an encoded paint list (rectangles, text, icons, clips, opacity groups). This crate
//! owns everything about it that is not the tree:
//!
//! | Module | Owns |
//! |---|---|
//! | [`list`] | the binary encoding, [`decode`] and its refusals (the tree's `bad-paint`) |
//! | [`font`] | [`FontSet`]: system fonts found by family, plus one embedded face |
//! | [`text`] | [`TextLayout`]: where each glyph goes and how big a string is, as a [`gnoblin_types::Size`] |
//! | [`icon`] | the built-in symbolic icon set |
//! | [`raster`] | [`CanvasPainter`]: a list to a premultiplied RGBA8 [`Picture`], deterministically |
//!
//! # The text stack
//!
//! Pure Rust, no system library. `skrifa` (fontations, MIT or Apache-2.0) reads font files, maps characters to
//! glyphs and supplies advances, metrics and outlines; `tiny-skia` (BSD-3-Clause) fills the outlines with
//! anti-aliasing and also draws every other op. Both were already in the dependency graph (egui's text stack and
//! the cursor crate's SVG renderer), and both licences are on `deny.toml`'s allow-list. System fonts are found by
//! scanning the standard directories ([`FontSet::system`]); the embedded Droid Sans subset (Apache-2.0) is the
//! fallback and the face every golden test draws with (`fonts/README.md`).
//!
//! It does no shaping: no kerning, ligatures, complex scripts or bidirectional text, and no hinting. That is enough
//! for titlebars, panels, launchers and entries in Latin scripts, and it is what keeps [`CanvasPainter::measure`] and
//! [`CanvasPainter::paint`] one calculation. Text that needs more is a later change to [`text`], behind the same two
//! functions.
//!
//! # Failure modes
//!
//! 1. A malformed value reaches the painter and draws garbage: [`decode`] refuses it, and the tree reports
//!    `bad-paint` before the value is stored (`list::tests`, `tests/paint.rs`).
//! 2. `measure` answers a size the painter does not draw in:
//!    `tests::measure_is_the_extent_the_painter_lays_text_out_in` and `tests/paint.rs::the_ink_of_a_string_lies_inside_its_measured_extent`.
//! 3. Output differs between machines: the embedded set gives the same pixels everywhere, and
//!    `tests/paint.rs::painting_twice_gives_the_same_bytes` and the golden files pin them.
//! 4. A huge canvas exhausts memory: [`CanvasPainter::paint`] gives no picture over [`raster::MAX_PIXELS`].

pub mod font;
pub mod icon;
pub mod list;
pub mod raster;
pub mod text;

gnoblin_log::logger!(log = "paint");

pub use font::{FaceId, FontSet};
pub use list::{DecodeError, PaintList, PaintOp, Rgba8, decode, encode};
pub use raster::{CanvasPainter, Picture, Rendered, SharedPainter};
pub use text::{TextLayout, TextMetrics, line_height};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn measure_is_the_extent_the_painter_lays_text_out_in() {
        let painter = CanvasPainter::new(Arc::new(FontSet::embedded()));
        for (text, size) in [
            ("Hello", 14.0),
            ("Window title — v2", 13.0),
            ("", 16.0),
            ("a\nbb", 20.0),
        ] {
            let layout = painter.fonts().layout(text, size, "sans");
            assert_eq!(painter.measure(text, size, "sans"), layout.extent(), "{text:?}");
        }
    }
}
