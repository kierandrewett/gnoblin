//! Text in canvases: how a string is measured, what a text shows when it is shorter than its string, and the estimate a measurement
//! falls back on.
//!
//! Text is measured in the process that holds the toolkit, by the painter the compositor draws with (`gnoblin-paint`), so a `text`'s
//! size is known the moment it is asked: there is no pending state. A test replaces the measurer (`testing::set_measurer`) to use a
//! fixed rule instead of fonts.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_paint::SharedPainter;

use super::widget::{Cid, Metrics, Ui};

/// What a measurement is asked of: the font, the size in logical pixels and the string.
pub trait Measurer {
    /// The whole-pixel size of `text` at `size` in `font`, and the distance from its top to its first baseline; an error is the
    /// answer's error name (`bad-value`, `too-large`, `unavailable`).
    ///
    /// # Errors
    ///
    /// The text cannot be measured.
    fn measure(&self, font: &str, size: f64, text: &str) -> Result<(u32, u32, u32), &'static str>;
}

/// The painter the compositor draws with, over the fonts of the system and the embedded face.
struct PainterMeasurer(SharedPainter);

impl Measurer for PainterMeasurer {
    fn measure(&self, font: &str, size: f64, text: &str) -> Result<(u32, u32, u32), &'static str> {
        let metrics = self.0.get().measure_checked(text, size as f32, font)?;
        Ok((metrics.width, metrics.height, metrics.ascent))
    }
}

thread_local! {
    static MEASURER: RefCell<Rc<dyn Measurer>> = RefCell::new(Rc::new(PainterMeasurer(SharedPainter::system())));
}

/// Replaces the measurer of this thread.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn set_measurer(measurer: Rc<dyn Measurer>) {
    MEASURER.with(|held| *held.borrow_mut() = measurer);
}

/// What the library estimates a string at when it cannot be measured: half the font size for each character, a line of one and a half
/// font sizes and a baseline at 1.1 times the font size.
pub(crate) fn estimate(text: &str, size: f64) -> Metrics {
    let mut widest = 0.0_f64;
    let mut lines = 0.0_f64;
    for line in text.split('\n') {
        widest = widest.max(((line.chars().count() as f64) * size / 2.0).floor());
        lines += 1.0;
    }
    Metrics {
        width: widest,
        height: lines * (size * 3.0 / 2.0).floor(),
        ascent: (size * 11.0 / 10.0).floor(),
    }
}

impl Ui {
    /// The size of the text of canvas `cid` for its font, size and string, measured now.
    pub(crate) fn metrics(&self, cid: Cid) -> Metrics {
        let epoch = self.epoch.get();
        let Some((key, font, size, text, cached)) = self.read(cid, |widget| {
            let t = &widget.t;
            let key = format!("{}\u{1}{}\u{1}{}", t.font, t.size, t.text);
            let cached = t
                .measured
                .as_ref()
                .filter(|(held, at, _)| *held == key && *at == epoch)
                .map(|(_, _, metrics)| *metrics);
            (key, t.font.clone(), t.size, t.text.clone(), cached)
        }) else {
            return Metrics {
                width: 0.0,
                height: 0.0,
                ascent: 0.0,
            };
        };
        if let Some(metrics) = cached {
            return metrics;
        }
        let measurer = MEASURER.with(|held| Rc::clone(&held.borrow()));
        let metrics = match measurer.measure(&font, size, &text) {
            Ok((width, height, ascent)) => Metrics {
                width: f64::from(width),
                height: f64::from(height),
                ascent: f64::from(ascent),
            },
            Err(why) => {
                self.warn(&format!("Text.measure: Text {text:?}: measure failed: {why}"));
                estimate(&text, size)
            }
        };
        self.tweak(cid, |widget| widget.t.measured = Some((key, epoch, metrics)));
        metrics
    }

    /// The string a text draws: the whole string, or what fits its width with an ellipsis when `ellipsize` is set.
    pub(crate) fn shown(&self, cid: Cid) -> String {
        let measured = self.metrics(cid);
        let Some((text, ellipsize)) = self.read(cid, |widget| (widget.t.text.clone(), widget.t.ellipsize)) else {
            return String::new();
        };
        let width = self.width_of(cid);
        if !ellipsize || width >= measured.width || measured.width == 0.0 {
            return text;
        }
        let chars: Vec<char> = text.chars().collect();
        // One character of room is left for the ellipsis.
        let keep = (((chars.len() as f64) * width / measured.width).floor() - 1.0).max(0.0) as usize;
        let mut shown: String = chars.iter().take(keep).collect();
        shown.push('\u{2026}');
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_estimate_is_half_the_size_a_character_and_a_line_and_a_half() {
        let one = estimate("Hello", 10.0);
        assert_eq!((one.width, one.height, one.ascent), (25.0, 15.0, 11.0));
        let three = estimate("a\nbbbb\ncc", 10.0);
        assert_eq!((three.width, three.height), (20.0, 45.0));
        let empty = estimate("", 12.0);
        assert_eq!((empty.width, empty.height, empty.ascent), (0.0, 18.0, 13.0));
    }

    #[test]
    fn characters_are_counted_by_code_point() {
        assert_eq!(estimate("h\u{e9}llo", 10.0).width, 25.0);
        assert_eq!(estimate("\u{1f600}", 10.0).width, 5.0);
    }
}
