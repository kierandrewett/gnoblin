//! Sampled curves and the validation of an animated `set` (node-core section 6.1, F31 to F33).

use gnoblin_animation::{
    AnimatedSet, CurveError, Easing, MAX_CURVE_MAGNITUDE, MAX_CURVE_SAMPLES, MAX_SET_SECONDS, SampledCurve, SetError,
};

const EPSILON: f64 = 1e-12;

#[test]
fn sampled_curve_interpolates_linearly_between_samples() {
    let curve = SampledCurve::new(vec![0.0, 0.5, 1.0]).expect("valid curve");
    assert!((curve.apply(0.25) - 0.25).abs() < EPSILON);
    assert!((curve.apply(0.75) - 0.75).abs() < EPSILON);
}

#[test]
fn sampled_curve_hits_both_endpoints_exactly() {
    for samples in [vec![0.0, 1.0], vec![0.0, 0.3, 0.9, 1.0]] {
        let curve = SampledCurve::new(samples).expect("valid curve");
        assert_eq!(curve.apply(0.0), 0.0);
        assert_eq!(curve.apply(1.0), 1.0);
    }
}

#[test]
fn two_sample_curve_equals_linear_easing() {
    let curve = SampledCurve::new(vec![0.0, 1.0]).expect("valid curve");
    for step in 0..=100 {
        let t = f64::from(step) / 100.0;
        assert!((curve.apply(t) - Easing::Linear.apply(t)).abs() < EPSILON, "t = {t}");
    }
}

#[test]
fn empty_samples_refused_bad_curve() {
    let error = SampledCurve::new(Vec::new()).expect_err("empty refused");
    assert_eq!(error, CurveError::Empty);
    assert_eq!(error.code(), "bad-curve");
}

#[test]
fn samples_must_start_at_zero_and_end_at_one() {
    assert_eq!(SampledCurve::new(vec![0.1, 1.0]), Err(CurveError::Endpoints));
    assert_eq!(SampledCurve::new(vec![0.0, 0.9]), Err(CurveError::Endpoints));
}

#[test]
fn non_finite_sample_refused() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(SampledCurve::new(vec![0.0, bad, 1.0]), Err(CurveError::NonFinite));
    }
}

#[test]
fn overshoot_samples_allowed_up_to_the_magnitude_bound() {
    assert!(SampledCurve::new(vec![0.0, 1.5, -0.2, 1.0]).is_ok());
    assert!(SampledCurve::new(vec![0.0, MAX_CURVE_MAGNITUDE, 1.0]).is_ok());
    assert_eq!(
        SampledCurve::new(vec![0.0, MAX_CURVE_MAGNITUDE * 2.0, 1.0]),
        Err(CurveError::OutOfBounds)
    );
    assert_eq!(
        SampledCurve::new(vec![0.0, -MAX_CURVE_MAGNITUDE - 1.0, 1.0]),
        Err(CurveError::OutOfBounds)
    );
}

#[test]
fn more_than_1024_samples_refused_bad_curve() {
    assert_eq!(MAX_CURVE_SAMPLES, 1024);
    let at_limit: Vec<f64> = (0..MAX_CURVE_SAMPLES)
        .map(|i| i as f64 / (MAX_CURVE_SAMPLES - 1) as f64)
        .collect();
    assert!(SampledCurve::new(at_limit.clone()).is_ok());
    let mut over = at_limit;
    over.push(1.0);
    let error = SampledCurve::new(over).expect_err("too many refused");
    assert_eq!(error, CurveError::TooMany);
    assert_eq!(error.code(), "bad-curve");
}

#[test]
fn f31_seconds_on_unanimated_property_refused_not_animated() {
    let error = AnimatedSet::resolve(false, Some(1.0), None, false).expect_err("refused");
    assert_eq!(error, SetError::NotAnimated);
    assert_eq!(error.code(), "not-animated");
}

#[test]
fn f31_set_without_seconds_is_instant_on_any_property() {
    assert_eq!(AnimatedSet::resolve(false, None, None, false), Ok(AnimatedSet::Instant));
}

#[test]
fn f32_bad_samples_refused_bad_curve() {
    let error = AnimatedSet::resolve(true, Some(1.0), Some(Vec::new()), false).expect_err("refused");
    assert_eq!(error, SetError::Curve(CurveError::Empty));
    assert_eq!(error.code(), "bad-curve");
    let error = AnimatedSet::resolve(true, Some(1.0), Some(vec![0.0, f64::NAN, 1.0]), false).expect_err("refused");
    assert_eq!(error.code(), "bad-curve");
}

#[test]
fn f33_seconds_out_of_range_refused() {
    for bad in [-0.5, MAX_SET_SECONDS + 0.5, f64::NAN, f64::INFINITY] {
        let error = AnimatedSet::resolve(true, Some(bad), None, false).expect_err("refused");
        assert_eq!(error, SetError::OutOfRange, "seconds = {bad}");
        assert_eq!(error.code(), "out-of-range");
    }
    assert!(AnimatedSet::resolve(true, Some(MAX_SET_SECONDS), None, false).is_ok());
}

#[test]
fn zero_seconds_is_an_instant_set() {
    assert_eq!(
        AnimatedSet::resolve(true, Some(0.0), None, false),
        Ok(AnimatedSet::Instant)
    );
}

#[test]
fn a_timed_set_carries_its_duration_and_curve() {
    let set = AnimatedSet::resolve(true, Some(1.5), Some(vec![0.0, 0.5, 1.0]), false).expect("valid");
    let AnimatedSet::Track { duration_ms, curve } = set else {
        panic!("expected a track, got {set:?}");
    };
    assert_eq!(duration_ms, 1500);
    assert_eq!(curve.samples(), &[0.0, 0.5, 1.0]);
}

#[test]
fn reduced_motion_makes_a_valid_set_instant() {
    assert_eq!(
        AnimatedSet::resolve(true, Some(2.0), Some(vec![0.0, 0.5, 1.0]), true),
        Ok(AnimatedSet::Instant)
    );
}

#[test]
fn reduced_motion_still_refuses_invalid_sets() {
    assert_eq!(
        AnimatedSet::resolve(true, Some(2.0), Some(Vec::new()), true),
        Err(SetError::Curve(CurveError::Empty))
    );
    assert_eq!(
        AnimatedSet::resolve(false, Some(2.0), None, true),
        Err(SetError::NotAnimated)
    );
}
