//! What a renderer tells the compositor about the shader passes it drew: the renderer-facing half of the frame-cost watchdog.
//!
//! A renderer that draws a user shader measures what the pass cost and says so on the frame it was handed:
//!
//! ```ignore
//! // GLES, Vulkan: after the shader pass of a layer is submitted and its time is known.
//! frame.shader_timings().record(pass.key(), elapsed);
//! ```
//!
//! `elapsed` is the GPU time of the pass where the renderer has timestamps, and otherwise the wall time from submit to the
//! fence. A key drawn by more than one layer of the frame is the sum, so the number is what the key cost the frame. The
//! compositor reads the report after the draw (`Host::drawn` in `gnoblin-backend`) and the watchdog
//! (`shader::watchdog`) acts on it; the renderer does not decide anything. It does not skip a pass either:
//! a pass the watchdog has switched off is not in the frame it draws.
//!
//! The report rides the frame, so no renderer signature changes. The handle is shared by every clone of the frame (the
//! redacted frame of a capture, the sub-frame of a backdrop), so the passes those draw count too.
//!
//! A device loss is reported by the code that sees [`RenderError::Lost`](crate::error::RenderError::Lost) from a draw
//! (`AnyRenderer::draw` and the DRM scan-out draw): the renderer returns the error as it always has and the watchdog strikes
//! the shaders of the frame that was in flight. A renderer calls [`ShaderTimings::record_loss`] itself only when it recovers
//! from a loss inside a draw and so never returns the error.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gnoblin_render::ShaderKey;

/// The longest one shader may take in one frame before the frame counts against it.
pub const SHADER_FRAME_BUDGET: Duration = Duration::from_millis(50);

/// How many frames in a row over [`SHADER_FRAME_BUDGET`] switch a shader pass off and cost its source a strike.
pub const FRAMES_OVER_BUDGET: u32 = 3;

/// How many strikes make the compositor refuse a source for the rest of the session.
pub const STRIKES_TO_REFUSE: u32 = 3;

/// What one shader source cost one draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderCost {
    /// The source's key: [`ShaderKey::get`] is the hash the log and the refusal list name.
    pub key: ShaderKey,
    /// The time every pass of this source took in the draw.
    pub elapsed: Duration,
}

/// What a draw reported about its shader passes, taken with [`ShaderTimings::take`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderReport {
    /// One entry per source drawn, in the order each was first recorded.
    pub costs: Vec<ShaderCost>,
    /// The sources the renderer could not draw (a compile or pipeline failure), with the reason, the first one per key.
    /// It is not a strike: the node's `skipped` list names the pass with this reason, and nothing else changes.
    pub refused: Vec<(ShaderKey, String)>,
    /// Whether the device was lost during the draw.
    pub lost: bool,
}

/// The place a renderer records shader costs: a handle on an [`OutputFrame`](crate::frame::OutputFrame), shared by its clones.
#[derive(Clone, Debug, Default)]
pub struct ShaderTimings(Arc<Mutex<ShaderReport>>);

impl ShaderTimings {
    /// Records that the passes of `key` took `elapsed`, added to what the draw has recorded for the key already.
    pub fn record(&self, key: ShaderKey, elapsed: Duration) {
        let mut report = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match report.costs.iter_mut().find(|cost| cost.key == key) {
            Some(cost) => cost.elapsed = cost.elapsed.saturating_add(elapsed),
            None => report.costs.push(ShaderCost { key, elapsed }),
        }
    }

    /// Records that the renderer could not build a program for `key` and left its passes undrawn. The first reason for a key
    /// in a draw is kept. The compositor names the reason in the `skipped` list of every node that has the pass.
    pub fn record_refused(&self, key: ShaderKey, reason: &str) {
        let mut report = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !report.refused.iter().any(|(held, _)| *held == key) {
            report.refused.push((key, reason.to_owned()));
        }
    }

    /// Records that the device was lost during the draw.
    pub fn record_loss(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).lost = true;
    }

    /// Takes what has been recorded since the last call, leaving the handle empty for the next draw.
    pub fn take(&self) -> ShaderReport {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u32) -> ShaderKey {
        ShaderKey::of(&format!("fn effect{n}() {{}}"))
    }

    #[test]
    fn a_source_drawn_twice_in_a_frame_reports_the_sum_and_a_take_empties_the_handle() {
        let timings = ShaderTimings::default();
        timings.record(key(1), Duration::from_millis(20));
        timings.record(key(2), Duration::from_millis(5));
        timings.record(key(1), Duration::from_millis(40));
        let report = timings.take();
        assert_eq!(
            report.costs,
            [
                ShaderCost {
                    key: key(1),
                    elapsed: Duration::from_millis(60)
                },
                ShaderCost {
                    key: key(2),
                    elapsed: Duration::from_millis(5)
                },
            ]
        );
        assert!(!report.lost);
        assert_eq!(timings.take(), ShaderReport::default(), "the next draw starts empty");
    }

    #[test]
    fn a_clone_shares_the_handle_so_a_redrawn_frame_reports_to_the_same_place() {
        let timings = ShaderTimings::default();
        let clone = timings.clone();
        clone.record(key(1), Duration::from_millis(1));
        clone.record_loss();
        let report = timings.take();
        assert_eq!(report.costs.len(), 1);
        assert!(report.lost);
    }

    #[test]
    fn a_refusal_keeps_the_first_reason_for_its_key() {
        let timings = ShaderTimings::default();
        timings.record_refused(key(1), "the pipeline could not be created");
        timings.record_refused(key(1), "another reason");
        timings.record_refused(key(2), "no backdrop target");
        assert_eq!(
            timings.take().refused,
            [
                (key(1), "the pipeline could not be created".to_owned()),
                (key(2), "no backdrop target".to_owned())
            ]
        );
    }
}
