//! The five named easing curves: the CSS timing functions, as `data/js/gnoblin-std/transition/easing.ts` computes them.
//!
//! A curve maps progress in time (0 to 1) onto eased progress in value (also 0 to 1). The four non-linear ones are cubic Beziers
//! through (0, 0) and (1, 1); the solver is the TypeScript one step for step (Newton's method, then bisection where the slope is flat),
//! so a sample here is the sample there.

use gnoblin_std_abi as abi;

/// How many steps the curve a timed write carries is sampled in: 33 samples with the start.
pub(crate) const CURVE_STEPS: usize = 32;

/// One of the CSS timing functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Easing {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Easing {
    pub(crate) fn from_abi(value: u32) -> Option<Easing> {
        Some(match value {
            abi::STD_EASING_LINEAR => Easing::Linear,
            abi::STD_EASING_EASE => Easing::Ease,
            abi::STD_EASING_EASE_IN => Easing::EaseIn,
            abi::STD_EASING_EASE_OUT => Easing::EaseOut,
            abi::STD_EASING_EASE_IN_OUT => Easing::EaseInOut,
            _ => return None,
        })
    }

    /// The control points `(x1, y1, x2, y2)` of the CSS Bezier, or `None` for the identity.
    fn points(self) -> Option<(f64, f64, f64, f64)> {
        match self {
            Easing::Linear => None,
            Easing::Ease => Some((0.25, 0.1, 0.25, 1.0)),
            Easing::EaseIn => Some((0.42, 0.0, 1.0, 1.0)),
            Easing::EaseOut => Some((0.0, 0.0, 0.58, 1.0)),
            Easing::EaseInOut => Some((0.42, 0.0, 0.58, 1.0)),
        }
    }

    /// The eased value at progress `t`.
    pub(crate) fn sample(self, t: f64) -> f64 {
        match self.points() {
            None => t,
            Some((x1, y1, x2, y2)) => cubic_bezier(x1, y1, x2, y2, t),
        }
    }

    /// The curve a timed write carries: progress from 0 to 1 over [`CURVE_STEPS`] steps, 33 samples.
    pub(crate) fn samples(self) -> Vec<f64> {
        (0..=CURVE_STEPS)
            .map(|step| self.sample(step as f64 / CURVE_STEPS as f64))
            .collect()
    }
}

/// One coordinate of the Bezier at parameter `s`, for control values `a` and `b`.
fn coordinate(a: f64, b: f64, s: f64) -> f64 {
    3.0 * a * (1.0 - s).powi(2) * s + 3.0 * b * (1.0 - s) * s.powi(2) + s.powi(3)
}

/// The slope of [`coordinate`] at `s`.
fn slope(a: f64, b: f64, s: f64) -> f64 {
    3.0 * a * (1.0 - s).powi(2) + 6.0 * (b - a) * (1.0 - s) * s + 3.0 * (1.0 - b) * s.powi(2)
}

/// The CSS timing function of a Bezier through (0, 0) and (1, 1): solves `x(s) = t` for `s`, and answers `y(s)`.
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let mut s = t;
    for _ in 0..8 {
        let error = coordinate(x1, x2, s) - t;
        if error.abs() < 1e-7 {
            return coordinate(y1, y2, s);
        }
        let d = slope(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s -= error / d;
    }
    let mut low = 0.0;
    let mut high = 1.0;
    s = t;
    for _ in 0..40 {
        let x = coordinate(x1, x2, s);
        if (x - t).abs() < 1e-7 {
            break;
        }
        if x < t {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    coordinate(y1, y2, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for easing in [
            Easing::Linear,
            Easing::Ease,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
        ] {
            let samples = easing.samples();
            assert_eq!(samples.len(), CURVE_STEPS + 1);
            assert_eq!(samples[0], 0.0);
            assert_eq!(samples[CURVE_STEPS], 1.0);
            assert!(samples.windows(2).all(|pair| pair[0] <= pair[1] + 1e-9), "{easing:?}");
        }
    }

    #[test]
    fn ease_out_is_the_css_curve() {
        // cubic-bezier(0, 0, 0.58, 1) at the middle of the time axis is well above the middle of the value axis: x(s) = 0.5 at about
        // s = 0.625, where y(s) = 0.68. The TypeScript curve is proven equal by `tests/anim_style.rs`.
        let middle = Easing::EaseOut.sample(0.5);
        assert!((middle - 0.68).abs() < 0.02, "{middle}");
        assert_eq!(Easing::Linear.sample(0.25), 0.25);
    }
}
