//! The GPU error type.

use std::fmt;

/// A failure on the GPU side. The message names the call that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// The Vulkan loader or the device is not usable, with the reason.
    Unavailable(String),
    /// A Vulkan call failed.
    Vulkan { call: &'static str, result: i32 },
    /// An OpenXR call made for the Vulkan setup failed.
    OpenXr(String),
    /// The pixels cannot be uploaded, with the reason.
    BadPixels(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "Vulkan is not available: {reason}"),
            Self::Vulkan { call, result } => write!(f, "{call} failed with VkResult {result}"),
            Self::OpenXr(reason) => write!(f, "OpenXR Vulkan setup failed: {reason}"),
            Self::BadPixels(reason) => write!(f, "cannot upload the pixels: {reason}"),
        }
    }
}

impl std::error::Error for GpuError {}
