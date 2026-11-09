//! The validated timing of an animated `set` (node-core section 6.1, F31 to F33).
//!
//! A `set` carries an optional `seconds` and an optional sample list. [`AnimatedSet::resolve`]
//! checks them in the order the protocol names the refusals: the property must be
//! animatable (F31), the duration must be in range (F33), and the samples must form a
//! valid curve (F32). Only then does reduced motion turn a valid timed set into an
//! instant one, so reduced motion never hides a bad request.

use std::fmt;

use crate::sampled::{CurveError, SampledCurve};

/// The longest duration a `set` may request, in seconds.
pub const MAX_SET_SECONDS: f64 = 3600.0;

/// How a `set` lands on its property.
#[derive(Clone, Debug, PartialEq)]
pub enum AnimatedSet {
    /// The value lands at once. This is a `set` without `seconds`, a zero duration,
    /// a duration under one millisecond, or any `set` under reduced motion.
    Instant,
    /// The value is reached over `duration_ms` along `curve`.
    Track {
        /// The whole-millisecond duration, at least 1.
        duration_ms: u32,
        /// The validated curve.
        curve: SampledCurve,
    },
}

impl AnimatedSet {
    /// Check a `set` and decide how it lands.
    ///
    /// `animated` says whether the property accepts a duration. `seconds` is `None`
    /// for an instant set. `samples` is `None` for the linear curve. A `seconds` of
    /// exactly `0` is an instant set. A negative or non-finite `seconds` is refused.
    ///
    /// # Errors
    ///
    /// Returns [`SetError::NotAnimated`] for a duration on a property without
    /// `animated`, [`SetError::OutOfRange`] for a duration outside `[0, 3600]`, and
    /// [`SetError::Curve`] for an invalid sample list.
    pub fn resolve(
        animated: bool,
        seconds: Option<f64>,
        samples: Option<Vec<f64>>,
        reduced_motion: bool,
    ) -> Result<Self, SetError> {
        let Some(seconds) = seconds else {
            return Ok(Self::Instant);
        };
        if !animated {
            return Err(SetError::NotAnimated);
        }
        if !(0.0..=MAX_SET_SECONDS).contains(&seconds) {
            return Err(SetError::OutOfRange);
        }
        let curve = match samples {
            Some(samples) => SampledCurve::new(samples)?,
            None => SampledCurve::linear(),
        };
        let duration_ms = (seconds * 1000.0).round() as u32;
        if reduced_motion || duration_ms == 0 {
            return Ok(Self::Instant);
        }
        Ok(Self::Track { duration_ms, curve })
    }
}

/// Why a `set` is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetError {
    /// `seconds` names a property without `animated` (F31).
    NotAnimated,
    /// The sample list is not a valid curve (F32).
    Curve(CurveError),
    /// `seconds` is negative, above [`MAX_SET_SECONDS`] or not finite (F33).
    OutOfRange,
}

impl SetError {
    /// The refusal code (node-core section 4.4).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAnimated => "not-animated",
            Self::Curve(error) => error.code(),
            Self::OutOfRange => "out-of-range",
        }
    }
}

impl From<CurveError> for SetError {
    fn from(error: CurveError) -> Self {
        Self::Curve(error)
    }
}

impl fmt::Display for SetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnimated => formatter.write_str("the property is not animated"),
            Self::Curve(error) => fmt::Display::fmt(error, formatter),
            Self::OutOfRange => formatter.write_str("seconds must be between 0 and 3600"),
        }
    }
}

impl std::error::Error for SetError {}
