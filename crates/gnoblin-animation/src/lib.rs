//! The animation system: easing curves, the visual properties a node exposes,
//! and the tween player that drives them over time.
//!
//! This crate is a foundation below the scene graph. It depends on
//! `gnoblin-types` alone and never names `Scene`: the tween player writes
//! through [`AnimationTarget`], which `gnoblin-render` implements for its scene.
//! That keeps the animation rules testable without a renderer, and keeps the
//! scene free of any dependency on how an animation is evaluated.

mod animated_set;
mod easing;
mod property;
mod sampled;
mod transform;

/// Sampled timing curves, the validated timing of an animated `set`, and their errors.
pub use animated_set::{AnimatedSet, MAX_SET_SECONDS, SetError};
/// Timing curves for tracks and their validation error.
pub use easing::{Easing, EasingError};
/// Typed visual property names, values and their kinds.
pub use property::{PropertyError, PropertyValue, ValueKind, VisualProperty};
pub use sampled::{CurveError, MAX_CURVE_MAGNITUDE, MAX_CURVE_SAMPLES, SampledCurve};
/// The per-node visual transform (offset, rotation, scale, depth, opacity).
pub use transform::VisualTransform;
