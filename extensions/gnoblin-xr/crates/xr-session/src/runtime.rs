//!
//! [`Runtime`] is the set of calls the frame loop makes. The OpenXR-backed
//! implementation is [`crate::openxr_runtime::OpenxrRuntime`]; tests use a fake.
//! Times are OpenXR `XrTime` values (nanoseconds on the runtime's clock), passed
//! as plain `i64` so the trait does not expose `openxr` types.

use std::fmt;

use xr_scene::{Pose, Ray};

/// The OpenXR session states, as `XrSessionState` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Before the first `XrSessionStateChanged` event.
    Unknown,
    /// The session exists and is not yet ready to begin.
    Idle,
    /// The application may call `begin_session`.
    Ready,
    /// The session is begun and the frame loop runs, but the user does not see it yet.
    Synchronized,
    /// The session is visible to the user.
    Visible,
    /// The session is visible and receives input.
    Focused,
    /// The runtime asks the application to call `end_session`.
    Stopping,
    /// The session is about to be lost. The runtime must not be used for frames.
    LossPending,
    /// The application must destroy the session and exit.
    Exiting,
}

/// An event the runtime reports between frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// The session moved to a new state.
    StateChanged(SessionState),
}

/// What `wait_frame` tells the application about the frame it may now render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// The predicted display time of this frame.
    pub predicted_display_time: i64,
    /// Whether the application should render this frame. It must still call
    /// `begin_frame` and `end_frame` when this is false.
    pub should_render: bool,
}

/// A failure reported by the runtime. The message is the runtime's own text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError(pub String);

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OpenXR runtime error: {}", self.0)
    }
}

impl std::error::Error for RuntimeError {}

pub type Result<T, E = RuntimeError> = std::result::Result<T, E>;

/// The calls the frame loop makes on an OpenXR runtime, in the order the spec requires.
///
/// `end_frame` submits no composition layers yet: the runtime has no swapchains.
pub trait Runtime {
    /// Returns the next pending event, or `None` when there are no more.
    fn poll_event(&mut self) -> Result<Option<SessionEvent>>;
    /// `xrBeginSession` with the primary stereo view configuration.
    fn begin_session(&mut self) -> Result<()>;
    /// `xrEndSession`.
    fn end_session(&mut self) -> Result<()>;
    /// `xrWaitFrame`. Blocks until the runtime wants the next frame.
    fn wait_frame(&mut self) -> Result<FrameTiming>;
    /// `xrBeginFrame`.
    fn begin_frame(&mut self) -> Result<()>;
    /// `xrEndFrame` for `display_time`, with no layers.
    fn end_frame(&mut self, display_time: i64) -> Result<()>;
    /// The head's pose in the local reference space at `display_time`, if the
    /// runtime tracks it.
    fn locate_head(&mut self, display_time: i64) -> Result<Option<Pose>>;
    /// The pointing ray (controller or gaze) in the local reference space at
    /// `display_time`, if the runtime tracks one. The headless runtime has none.
    fn locate_ray(&mut self, display_time: i64) -> Result<Option<Ray>> {
        let _ = display_time;
        Ok(None)
    }
}
