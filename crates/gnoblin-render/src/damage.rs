//! Damage rectangles per output.
//!
//! Damage is a set of pixel rectangles that the renderer must redraw. It is
//! allowed to be too large — a rectangle that covers unchanged pixels costs work
//! but cannot show a stale frame — and it is never allowed to be too small,
//! because a missed rectangle leaves the old pixels on screen.
//!
//! Two rules keep the set small without risking that miss:
//!
//! - Rectangles merge when their bounding union is at most twice the area they
//!   covered separately, so a merge never turns two small rectangles into a
//!   large empty one. The greedy pass is deterministic: rectangles are sorted,
//!   then merged in that order, and a merge restarts the scan because the
//!   candidate grew.
//! - If the merged set is still larger than the per-output cap, the output
//!   degrades to one full-output rectangle. A renderer with a rectangle list
//!   budget therefore pays a full repaint exactly when the list would have been
//!   unwieldy, instead of dropping entries.
//!
//! The plan's 3D rule lives here too: a node under a perspective camera falls
//! back to full damage on its output, because a projected bound is not the
//! union of the node's own rectangle. [`OutputDamage::full`] is that fallback.
//!
//! # Coordinate spaces
//!
//! Damage rectangles are in scene space, which is global: one origin shared by
//! every output. [`OutputDamage::rects`] are reported in the same space, and the
//! consumer converts them to output-local pixels by subtracting the output's
//! origin (`local = rect - output.origin`). A rectangle on an output whose origin
//! is `(1920, 0)` therefore starts at `x = 0` in that output's frame.
//!
//! # Frame timing
//!
//! The scene collects damage on every mutation and the renderer takes it once per
//! iteration with `Scene::take_damage`, through `Composer::begin_frame`. A second
//! take in the same iteration would find an empty set and leave pixels stale, so
//! the take is not repeated.

use gnoblin_types::{OutputId, Rect};

/// The damage for one output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputDamage {
    /// The output.
    pub output: OutputId,
    /// The rectangles to redraw; one rectangle covering the whole output when
    /// `full` is set.
    pub rects: Vec<Rect>,
    /// Whether the whole output must be redrawn.
    pub full: bool,
}

impl OutputDamage {
    /// Whether the damage covers `rect`.
    ///
    /// A caller asks this about a rectangle it knows changed; the scene tests use
    /// it to assert the coverage invariant.
    #[must_use]
    pub fn covers(&self, rect: Rect) -> bool {
        self.full || self.rects.iter().any(|candidate| rect.is_inside(*candidate))
    }

    /// The number of rectangles, or one when the damage is full.
    #[must_use]
    pub fn rect_count(&self) -> usize {
        if self.full { 1 } else { self.rects.len() }
    }
}

/// The damage for every output that has any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    /// Per-output damage, in output id order.
    pub outputs: Vec<OutputDamage>,
}

impl Damage {
    /// Whether no output needs redrawing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    /// The damage for one output.
    #[must_use]
    pub fn for_output(&self, output: OutputId) -> Option<&OutputDamage> {
        self.outputs.iter().find(|entry| entry.output == output)
    }

    /// The total number of rectangles across outputs.
    #[must_use]
    pub fn rect_count(&self) -> usize {
        self.outputs.iter().map(OutputDamage::rect_count).sum()
    }
}

/// Merge rectangles that cost no extra pixels to merge.
///
/// The input order does not matter: the list is sorted first, and the greedy
/// merge restarts after every successful union, so the result depends only on
/// the set of rectangles.
#[must_use]
pub fn merge_rects(rects: Vec<Rect>) -> Vec<Rect> {
    let mut sorted = rects;
    sorted.sort();
    sorted.dedup();
    let mut merged: Vec<Rect> = Vec::with_capacity(sorted.len());
    for rect in sorted {
        let mut candidate = rect;
        let mut index = 0;
        while index < merged.len() {
            if mergeable(&merged[index], &candidate) {
                candidate = merged[index].union(candidate);
                merged.remove(index);
                index = 0;
            } else {
                index += 1;
            }
        }
        merged.push(candidate);
    }
    merged.sort();
    merged
}

/// Whether merging two rectangles costs at most a doubling of the redrawn area.
///
/// Any axis-aligned union can add pixels that neither rectangle covered (the
/// corners of the bounding box), so "free" merges do not exist in general.
/// Doubling is the bound that keeps a merge worthwhile: below it the list gets
/// shorter for about the same work, above it the rectangle list is cheaper than
/// the wasted pixels and the per-output cap takes over.
fn mergeable(first: &Rect, second: &Rect) -> bool {
    let union = u128::from(first.union(*second).area());
    let separate = u128::from(first.area()) + u128::from(second.area());
    union <= 2 * separate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_rectangles_merge_into_one() {
        let merged = merge_rects(vec![Rect::new(0, 0, 20, 20), Rect::new(10, 10, 20, 20)]);
        assert_eq!(merged, vec![Rect::new(0, 0, 30, 30)]);
    }

    #[test]
    fn touching_rectangles_merge_into_one() {
        let merged = merge_rects(vec![Rect::new(0, 0, 10, 10), Rect::new(10, 0, 10, 10)]);
        assert_eq!(merged, vec![Rect::new(0, 0, 20, 10)]);
    }

    #[test]
    fn distant_rectangles_stay_separate() {
        let merged = merge_rects(vec![Rect::new(0, 0, 10, 10), Rect::new(100, 100, 10, 10)]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merging_is_order_independent() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        let c = Rect::new(200, 200, 4, 4);
        let first = merge_rects(vec![a, b, c]);
        let second = merge_rects(vec![c, b, a]);
        assert_eq!(first, second);
    }

    #[test]
    fn a_full_damage_entry_covers_anything_inside_the_output() {
        let damage = OutputDamage {
            output: OutputId::new(0),
            rects: vec![Rect::new(0, 0, 1920, 1080)],
            full: true,
        };
        assert!(damage.covers(Rect::new(100, 100, 1, 1)));
        assert_eq!(damage.rect_count(), 1);
    }
}
