//! Animation: easing curves, staged writes and the transition engine.
//!
//! `data/js/gnoblin-std/transition/` is the reference. The compositor plays the animations; the library decides which writes are
//! timed, how they are paced (the five CSS curves, sampled 33 times), which `tween` node they join, and what ended.

pub(crate) mod easing;
pub(crate) mod engine;
pub(crate) mod objects;
pub(crate) mod staging;
pub(crate) mod table;
