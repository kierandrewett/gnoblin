//! The animation system: easing curves, the visual properties a node exposes,
//! and the tween player that drives them over time.
//!
//! This crate is a foundation below the scene graph. It depends on
//! `gnoblin-types` alone and never names `Scene`: the tween player writes
//! through [`AnimationTarget`], which `gnoblin-render` implements for its scene.
//! That keeps the animation rules testable without a renderer, and keeps the
//! scene free of any dependency on how an animation is evaluated.

mod easing;

/// Timing curves for tracks and their validation error.
pub use easing::{Easing, EasingError};
