//! The OpenXR session and frame loop for `gnoblin-xr`.
//!
//! This crate will hold the frame loop state machine and the OpenXR-backed
//! runtime. For now it holds [`runtime`], the trait the frame loop drives. It
//! has no OpenXR types, so the loop can be tested with a fake runtime.
//!
//! Nothing in this crate creates swapchains or connects to the Gnoblin owner
//! socket yet.

pub mod runtime;

pub use runtime::{FrameTiming, Runtime, RuntimeError, SessionEvent, SessionState};
