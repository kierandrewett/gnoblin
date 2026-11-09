//! Sampled timing curves: validated samples of normalised progress.
//!
//! A sampled curve is a list of values evenly spaced over progress `[0, 1]`. The
//! first sample is `0` and the last is `1`, so the curve is exact at both ends, and
//! progress between two neighbouring samples is interpolated linearly. A sample may
//! leave the 0 to 1 range, which lets a sampled curve overshoot like a spring, but
//! no sample may exceed [`MAX_CURVE_MAGNITUDE`] in magnitude.
//!
//! The list is validated once, when it is built. Evaluation never fails.

use std::fmt;

/// The most samples one curve may carry.
pub const MAX_CURVE_SAMPLES: usize = 1024;

/// The largest magnitude a single sample may have.
pub const MAX_CURVE_MAGNITUDE: f64 = 1.0e6;

/// A timing curve given as samples over progress `[0, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct SampledCurve {
    samples: Vec<f64>,
}

impl SampledCurve {
    /// Build a curve from samples, refusing any list that cannot be evaluated.
    ///
    /// # Errors
    ///
    /// Returns [`CurveError`] when the list is empty, has more than
    /// [`MAX_CURVE_SAMPLES`] entries, holds a non-finite value, holds a value above
    /// [`MAX_CURVE_MAGNITUDE`], or does not start at `0` and end at `1`.
    pub fn new(samples: Vec<f64>) -> Result<Self, CurveError> {
        let (Some(&first), Some(&last)) = (samples.first(), samples.last()) else {
            return Err(CurveError::Empty);
        };
        if samples.len() > MAX_CURVE_SAMPLES {
            return Err(CurveError::TooMany);
        }
        if samples.iter().any(|value| !value.is_finite()) {
            return Err(CurveError::NonFinite);
        }
        if samples.iter().any(|value| value.abs() > MAX_CURVE_MAGNITUDE) {
            return Err(CurveError::OutOfBounds);
        }
        if first != 0.0 || last != 1.0 {
            return Err(CurveError::Endpoints);
        }
        Ok(Self { samples })
    }

    /// The straight line from `0` to `1`: two samples, identical to [`crate::Easing::Linear`].
    #[must_use]
    pub fn linear() -> Self {
        Self {
            samples: vec![0.0, 1.0],
        }
    }

    /// The validated samples, first `0`, last `1`.
    #[must_use]
    pub fn samples(&self) -> &[f64] {
        &self.samples
    }

    /// The curve's value at `progress`, clamped to `[0, 1]`.
    ///
    /// Progress `0` and `1` return the first and last sample exactly. A
    /// non-finite progress is treated as `0`.
    #[must_use]
    pub fn apply(&self, progress: f64) -> f64 {
        let first = self.samples[0];
        let last_sample = self.samples[self.samples.len() - 1];
        if !progress.is_finite() || progress <= 0.0 {
            return first;
        }
        if progress >= 1.0 {
            return last_sample;
        }
        let last = self.samples.len() - 1;
        let position = progress * last as f64;
        let index = (position.floor() as usize).min(last - 1);
        let fraction = position - index as f64;
        let (from, to) = (self.samples[index], self.samples[index + 1]);
        from + (to - from) * fraction
    }
}

/// Why a sampled curve is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveError {
    /// There are no samples.
    Empty,
    /// There are more than [`MAX_CURVE_SAMPLES`] samples.
    TooMany,
    /// A sample is NaN or infinite.
    NonFinite,
    /// A sample's magnitude exceeds [`MAX_CURVE_MAGNITUDE`].
    OutOfBounds,
    /// The first sample is not `0` or the last is not `1`.
    Endpoints,
}

impl CurveError {
    /// The refusal code the protocol reports for any curve error (node-core F32).
    #[must_use]
    pub const fn code(self) -> &'static str {
        "bad-curve"
    }
}

impl fmt::Display for CurveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "a sampled curve needs at least one sample",
            Self::TooMany => "a sampled curve has at most 1024 samples",
            Self::NonFinite => "a sampled curve sample is not finite",
            Self::OutOfBounds => "a sampled curve sample exceeds 1e6 in magnitude",
            Self::Endpoints => "a sampled curve starts at 0 and ends at 1",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CurveError {}
