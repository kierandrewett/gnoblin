//!
//! [`Runtime`] is the set of calls the frame loop makes. The OpenXR-backed
//! implementation is [`crate::openxr_runtime::OpenxrRuntime`]; tests use a fake.
//! Times are OpenXR `XrTime` values (nanoseconds on the runtime's clock), passed
//! as plain `i64` so the trait does not expose `openxr` types.

use std::fmt;

use xr_input::Hands;
use xr_layers::{SubmittedLayer, SwapchainId};
use xr_scene::Pose;

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

impl RuntimeError {
    /// The error for a call the session cannot make because it has no graphics binding.
    pub fn no_graphics(call: &str) -> Self {
        Self(format!(
            "{call} needs a graphics session: this session is headless, and has no XR_KHR_vulkan_enable2 binding"
        ))
    }
}

/// What the session offers beyond the frame loop. The default offers nothing, as a headless session does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Whether swapchains can be created: the session has a graphics binding.
    pub swapchains: bool,
    /// The runtime's `maxLayerCount`.
    pub max_layer_count: u32,
    /// Whether `XR_KHR_composition_layer_color_scale_bias` is enabled, so a layer can carry a colour scale.
    pub color_scale_bias: bool,
    /// The swapchain formats the runtime lists, as `VkFormat` values, in its order of preference.
    pub swapchain_formats: Vec<i64>,
}

pub type Result<T, E = RuntimeError> = std::result::Result<T, E>;

/// The calls the frame loop makes on an OpenXR runtime, in the order the spec requires.
///
/// The swapchain calls have defaults that fail with [`RuntimeError::no_graphics`], so a headless session needs none
/// of them. A session with a graphics binding implements all of them, and reports `swapchains` in its
/// [`Capabilities`].
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
    /// `xrEndFrame` for `display_time` with `layers`, in submission order.
    fn end_frame(&mut self, display_time: i64, layers: &[SubmittedLayer]) -> Result<()>;
    /// The head's pose in the local reference space at `display_time`, if the
    /// runtime tracks it.
    fn locate_head(&mut self, display_time: i64) -> Result<Option<Pose>>;
    /// Both controllers at `display_time`: each one's aim pose in the local reference space and its inputs, and the
    /// head pose as the gaze the pointer falls back on when the right hand has no aim pose. A hand that is `None` has
    /// no controller the runtime reports as active. A runtime with no input source, as a headless session is, reports
    /// [`Hands::default`]: no controller, no gaze, nothing pressed.
    fn locate_hands(&mut self, display_time: i64) -> Result<Hands> {
        let _ = display_time;
        Ok(Hands::default())
    }
    /// What the session offers. The default is nothing.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    /// `xrCreateSwapchain` for a colour image of `width` by `height` in the `VkFormat` `format`.
    fn create_swapchain(&mut self, width: u32, height: u32, format: i64) -> Result<SwapchainId> {
        let _ = (width, height, format);
        Err(RuntimeError::no_graphics("xrCreateSwapchain"))
    }
    /// `xrEnumerateSwapchainImages`: the raw `VkImage` handle of each image of the swapchain, by image index.
    fn swapchain_images(&mut self, id: SwapchainId) -> Result<Vec<u64>> {
        let _ = id;
        Err(RuntimeError::no_graphics("xrEnumerateSwapchainImages"))
    }
    /// `xrAcquireSwapchainImage` then `xrWaitSwapchainImage`: the index of the image to write.
    fn acquire_image(&mut self, id: SwapchainId) -> Result<u32> {
        let _ = id;
        Err(RuntimeError::no_graphics("xrAcquireSwapchainImage"))
    }
    /// `xrReleaseSwapchainImage` for the image `acquire_image` returned.
    fn release_image(&mut self, id: SwapchainId) -> Result<()> {
        let _ = id;
        Err(RuntimeError::no_graphics("xrReleaseSwapchainImage"))
    }
    /// `xrDestroySwapchain`.
    fn destroy_swapchain(&mut self, id: SwapchainId) -> Result<()> {
        let _ = id;
        Err(RuntimeError::no_graphics("xrDestroySwapchain"))
    }
}
