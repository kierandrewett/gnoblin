//! The OpenXR session and frame loop for `gnoblin-xr`.
//!
//! [`runtime`] holds the trait the frame loop drives, and [`frame_loop`] holds
//! the state machine over it. Neither has OpenXR types, so the loop is tested
//! with a fake runtime.
//!
//! Nothing in this crate creates swapchains or connects to the Gnoblin owner
//! socket yet.

pub mod frame_loop;
pub mod runtime;
#[cfg(test)]
mod testing;

pub use frame_loop::{Frame, FrameLoop, Tick};
pub use runtime::{FrameTiming, Runtime, RuntimeError, SessionEvent, SessionState};
