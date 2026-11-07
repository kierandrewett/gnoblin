//! Easing curves the tween player evaluates.
//!
//! The named curves are the ones the animation guide documents, so a declaration
//! and a track cannot disagree about what `ease-out-cubic` means. Two families
//! go beyond the guide on purpose:
//!
//! - A cubic Bezier is the general form of every named curve the declaration
//!   format allows, with control points in the documented ranges (`x` in 0 to 1,
//!   `y` in -2 to 2 so a curve can overshoot).
//! - A spring is a damped harmonic oscillator, because the guide says a Bezier
//!   does not add spring physics. Its parameters are documented on
//!   [`Easing::Spring`] and are measured in normalised progress, so the same
//!   spring over a different duration is the same motion, time-scaled.
//!
//! Every curve is exact at both endpoints: `0` maps to `0` and `1` maps to `1`.
//! An overshooting curve may leave the 0 to 1 range in between, which is what
//! `ease-out-back` and a spring do, and the renderer clamps at draw time. The
//! endpoints are exact because the tween player applies the target value
//! verbatim at completion: a curve that ended at `0.9999` would leave a window
//! permanently one thousandth of a pixel from its resting place.

/// A timing curve over normalised progress.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Easing {
    /// Constant speed.
    #[default]
    Linear,
    /// Quadratic acceleration from rest.
    EaseInQuad,
    /// Quadratic deceleration to rest.
    EaseOutQuad,
    /// Cubic acceleration from rest.
    EaseInCubic,
    /// Cubic deceleration to rest.
    EaseOutCubic,
    /// Cubic acceleration and deceleration, symmetric about the midpoint.
    EaseInOutCubic,
    /// Exponential deceleration; the guide's `ease-out-expo`.
    EaseOutExpo,
    /// Cubic deceleration that overshoots before settling; the guide's
    /// `ease-out-back`.
    EaseOutBack,
    /// A cubic Bezier timing function.
    ///
    /// `x1` and `x2` are time control points in 0 to 1 and must be monotonic for
    /// the curve to be a function; `y1` and `y2` are value control points in -2
    /// to 2.
    CubicBezier {
        /// First control point's time.
        x1: f64,
        /// First control point's value.
        y1: f64,
        /// Second control point's time.
        x2: f64,
        /// Second control point's value.
        y2: f64,
    },
    /// A damped spring.
    ///
    /// The oscillator starts at rest at 0 and settles at 1. Progress is
    /// normalised, so `stiffness` is measured in inverse squared progress and
    /// `damping` in inverse progress; a spring with `stiffness = 180` and
    /// `damping = 12` overshoots slightly and settles well before the end of a
    /// 360 ms track. `mass` scales the period: a heavier mass is slower for the
    /// same stiffness and damping. Damping below the critical value oscillates,
    /// equal to it settles without overshoot, and above it creeps in.
    Spring {
        /// Stiffness, greater than zero.
        stiffness: f64,
        /// Damping coefficient, zero or greater.
        damping: f64,
        /// Mass, greater than zero.
        mass: f64,
    },
}

/// A refused easing curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EasingError {
    /// A component is NaN or infinite.
    NonFinite,
    /// A Bezier time control point lies outside 0 to 1.
    CubicBezierTimeOutOfRange,
    /// A Bezier value control point lies outside -2 to 2.
    CubicBezierValueOutOfRange,
    /// A spring parameter is outside its domain.
    SpringParameters,
}

impl std::fmt::Display for EasingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::NonFinite => "an easing component is not finite",
            Self::CubicBezierTimeOutOfRange => "cubic Bezier time control points must be between 0 and 1",
            Self::CubicBezierValueOutOfRange => "cubic Bezier value control points must be between -2 and 2",
            Self::SpringParameters => "a spring needs stiffness > 0, mass > 0 and damping >= 0",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for EasingError {}

impl Easing {
    /// Refuse a curve this build cannot evaluate.
    pub fn validate(self) -> Result<(), EasingError> {
        match self {
            Self::CubicBezier { x1, y1, x2, y2 } => {
                if ![x1, y1, x2, y2].iter().all(|value| value.is_finite()) {
                    return Err(EasingError::NonFinite);
                }
                if !((0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2)) {
                    return Err(EasingError::CubicBezierTimeOutOfRange);
                }
                if !((-2.0..=2.0).contains(&y1) && (-2.0..=2.0).contains(&y2)) {
                    return Err(EasingError::CubicBezierValueOutOfRange);
                }
                Ok(())
            }
            Self::Spring {
                stiffness,
                damping,
                mass,
            } => {
                if !stiffness.is_finite() || !damping.is_finite() || !mass.is_finite() {
                    return Err(EasingError::NonFinite);
                }
                if stiffness <= 0.0 || mass <= 0.0 || damping < 0.0 {
                    return Err(EasingError::SpringParameters);
                }
                Ok(())
            }
            Self::Linear
            | Self::EaseInQuad
            | Self::EaseOutQuad
            | Self::EaseInCubic
            | Self::EaseOutCubic
            | Self::EaseInOutCubic
            | Self::EaseOutExpo
            | Self::EaseOutBack => Ok(()),
        }
    }

    /// Evaluate the curve at `progress`, clamped to 0 to 1.
    #[must_use]
    pub fn apply(self, progress: f64) -> f64 {
        let progress = if progress.is_finite() {
            progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if progress <= 0.0 {
            return 0.0;
        }
        if progress >= 1.0 {
            return 1.0;
        }
        match self {
            Self::Linear => progress,
            Self::EaseInQuad => progress * progress,
            Self::EaseOutQuad => 1.0 - (1.0 - progress) * (1.0 - progress),
            Self::EaseInCubic => progress * progress * progress,
            Self::EaseOutCubic => {
                let inverse = 1.0 - progress;
                1.0 - inverse * inverse * inverse
            }
            Self::EaseInOutCubic => {
                if progress < 0.5 {
                    4.0 * progress * progress * progress
                } else {
                    let shifted = -2.0 * progress + 2.0;
                    1.0 - shifted * shifted * shifted / 2.0
                }
            }
            Self::EaseOutExpo => 1.0 - (-10.0 * progress).exp2(),
            Self::EaseOutBack => {
                const C1: f64 = 1.701_58;
                const C3: f64 = C1 + 1.0;
                let shifted = progress - 1.0;
                1.0 + C3 * shifted * shifted * shifted + C1 * shifted * shifted
            }
            Self::CubicBezier { x1, y1, x2, y2 } => {
                let parameter = solve_bezier_parameter(progress, x1, x2);
                bezier(parameter, y1, y2)
            }
            Self::Spring {
                stiffness,
                damping,
                mass,
            } => spring(progress, stiffness, damping, mass),
        }
    }
}

/// A cubic Bezier with endpoints at 0 and 1 and the two control points given.
fn bezier(parameter: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - parameter;
    3.0 * inverse * inverse * parameter * first
        + 3.0 * inverse * parameter * parameter * second
        + parameter * parameter * parameter
}

/// The parameter whose x coordinate is `time`, by Newton's method with a
/// bisection fallback.
///
/// Fixed iteration counts, not a tolerance loop with an unbounded count: the
/// curve must evaluate identically on every frame and in every test.
fn solve_bezier_parameter(time: f64, x1: f64, x2: f64) -> f64 {
    let mut parameter = time;
    for _ in 0..8 {
        let error = bezier(parameter, x1, x2) - time;
        if error.abs() < 1e-9 {
            return parameter;
        }
        let slope = bezier_slope(parameter, x1, x2);
        if slope.abs() < 1e-9 {
            break;
        }
        let next = parameter - error / slope;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        parameter = next;
    }
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..40 {
        parameter = (low + high) / 2.0;
        let value = bezier(parameter, x1, x2);
        if (value - time).abs() < 1e-12 {
            return parameter;
        }
        if value < time {
            low = parameter;
        } else {
            high = parameter;
        }
    }
    parameter
}

/// The derivative of the x coordinate with respect to the parameter.
fn bezier_slope(parameter: f64, x1: f64, x2: f64) -> f64 {
    let inverse = 1.0 - parameter;
    3.0 * inverse * inverse * x1 + 6.0 * inverse * parameter * (x2 - x1) + 3.0 * parameter * parameter * (1.0 - x2)
}

/// A damped harmonic oscillator from rest at 0 to rest at 1.
fn spring(time: f64, stiffness: f64, damping: f64, mass: f64) -> f64 {
    let natural = (stiffness / mass).sqrt();
    let damping_ratio = damping / (2.0 * (stiffness * mass).sqrt());
    if damping_ratio < 1.0 {
        let damped = natural * (1.0 - damping_ratio * damping_ratio).sqrt();
        let envelope = (-damping_ratio * natural * time).exp();
        1.0 - envelope * ((damped * time).cos() + (damping_ratio * natural / damped) * (damped * time).sin())
    } else if (damping_ratio - 1.0).abs() < 1e-9 {
        1.0 - (-natural * time).exp() * (1.0 + natural * time)
    } else {
        let root = (damping_ratio * damping_ratio - 1.0).sqrt();
        let first = -natural * (damping_ratio - root);
        let second = -natural * (damping_ratio + root);
        let first_weight = -second / (first - second);
        let second_weight = first / (first - second);
        1.0 - (first_weight * (first * time).exp() + second_weight * (second * time).exp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [Easing; 8] = [
        Easing::Linear,
        Easing::EaseInQuad,
        Easing::EaseOutQuad,
        Easing::EaseInCubic,
        Easing::EaseOutCubic,
        Easing::EaseInOutCubic,
        Easing::EaseOutExpo,
        Easing::EaseOutBack,
    ];

    #[test]
    fn every_easing_hits_both_endpoints() {
        let curves = CURVES.into_iter().chain([
            Easing::CubicBezier {
                x1: 0.2,
                y1: 1.5,
                x2: 0.35,
                y2: 1.0,
            },
            Easing::Spring {
                stiffness: 180.0,
                damping: 12.0,
                mass: 1.0,
            },
        ]);
        for easing in curves {
            assert_eq!(easing.apply(0.0), 0.0, "{easing:?} at 0");
            assert_eq!(easing.apply(1.0), 1.0, "{easing:?} at 1");
            assert_eq!(easing.apply(-1.0), 0.0, "{easing:?} below 0");
            assert_eq!(easing.apply(2.0), 1.0, "{easing:?} above 1");
        }
    }

    #[test]
    fn a_spring_overshoots_and_settles() {
        let spring = Easing::Spring {
            stiffness: 180.0,
            damping: 12.0,
            mass: 1.0,
        };
        let mut peak: f64 = 0.0;
        for step in 0..=100 {
            peak = peak.max(spring.apply(f64::from(step) / 100.0));
        }
        assert!(peak > 1.0, "an underdamped spring overshoots: {peak}");
        assert!((spring.apply(0.99) - 1.0).abs() < 0.05);
    }

    #[test]
    fn a_bezier_solves_its_parameter_monotonically() {
        let (x1, x2) = (0.2, 0.35);
        let mut previous = 0.0;
        for step in 0..=100 {
            let parameter = solve_bezier_parameter(f64::from(step) / 100.0, x1, x2);
            assert!(parameter >= previous - 1e-9, "the timing is a function");
            previous = parameter;
            assert!((0.0..=1.0).contains(&parameter));
        }
    }

    #[test]
    fn a_bezier_can_overshoot_in_value() {
        let curve = Easing::CubicBezier {
            x1: 0.2,
            y1: 1.5,
            x2: 0.35,
            y2: 1.0,
        };
        let mut peak: f64 = 0.0;
        for step in 0..=100 {
            peak = peak.max(curve.apply(f64::from(step) / 100.0));
        }
        assert!(peak > 1.0, "the value control points can pass the target");
    }

    #[test]
    fn a_refused_curve_names_its_problem() {
        assert_eq!(
            Easing::CubicBezier {
                x1: 1.5,
                y1: 0.0,
                x2: 0.5,
                y2: 0.0
            }
            .validate(),
            Err(EasingError::CubicBezierTimeOutOfRange)
        );
        assert_eq!(
            Easing::CubicBezier {
                x1: 0.5,
                y1: 9.0,
                x2: 0.5,
                y2: 0.0
            }
            .validate(),
            Err(EasingError::CubicBezierValueOutOfRange)
        );
        assert_eq!(
            Easing::Spring {
                stiffness: 0.0,
                damping: 1.0,
                mass: 1.0
            }
            .validate(),
            Err(EasingError::SpringParameters)
        );
        assert_eq!(
            Easing::Spring {
                stiffness: 1.0,
                damping: -1.0,
                mass: 1.0
            }
            .validate(),
            Err(EasingError::SpringParameters)
        );
    }
}
