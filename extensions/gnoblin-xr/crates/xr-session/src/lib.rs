//! The OpenXR session and frame loop for `gnoblin-xr`.
//!
//! [`runtime`] holds the trait the frame loop drives, and [`frame_loop`] holds
//! the state machine over it. Neither has OpenXR types, so the loop is tested
//! with a fake runtime. [`openxr_runtime`] is the one OpenXR-backed `Runtime`:
//! a headless (`XR_MND_headless`) session, the only `unsafe` in the crate.
//!
//! Nothing in this crate creates swapchains yet. The owner socket client is in
//! `xr-tree`, and the binary that joins the two is `gnoblin-xr`.

pub mod frame_loop;
pub mod openxr_runtime;
pub mod runtime;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use frame_loop::{Frame, FrameLoop, Tick};
pub use runtime::{FrameTiming, Runtime, RuntimeError, SessionEvent, SessionState};
