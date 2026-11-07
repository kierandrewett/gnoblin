//! The named reasons a Vulkan renderer cannot be created.
//!
//! [`RenderError`] is deliberately small and `Copy`, so it cannot say *why* a renderer
//! is unavailable. A caller that wants to fall back to another renderer, and a test that
//! wants to prove which failure it provoked, read a [`VulkanError`] from
//! [`Vulkan::new`](super::Vulkan::new) instead. The conversion to [`RenderError`] exists
//! for the places that only need to know that the renderer is not there. It is a method and not a
//! `From` impl on purpose: a second `From` impl for [`RenderError`] makes the `?` conversions in
//! the GLES renderer ambiguous.

use std::fmt;

use ash::vk;

use gnoblin_render_frame::error::RenderError;

use crate::log;

/// Why a Vulkan renderer could not be created.
///
/// None of these is a bug in the caller: each is a fact about the machine or the build,
/// and the compositor answers all of them the same way, by drawing with another renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VulkanError {
    /// The Vulkan loader library could not be opened.
    ///
    /// The loader is opened with `dlopen` when the renderer is created, so a machine
    /// without one reaches this error instead of failing to start.
    NoLoader {
        /// What the dynamic linker or the loader said.
        reason: String,
    },
    /// The loader is there, but no physical device can draw a frame.
    ///
    /// Either the loader lists none (no driver is installed), or every one it lists was
    /// refused; the reason names each refusal.
    NoDevice {
        /// Which devices were seen and why each was refused.
        reason: String,
    },
    /// One of the compositor's own shaders did not compile or the driver refused its module.
    ///
    /// Reported while the renderer is created, so a broken pass is a startup error and
    /// never a frame that fails at the first draw.
    Shader {
        /// The pass: `vertex`, `solid` or `surface`.
        pass: &'static str,
        /// The compiler's or driver's message, with a line when there is one.
        message: String,
    },
    /// A driver call failed while the renderer was being created.
    Driver {
        /// The Vulkan entry point.
        call: &'static str,
        /// The raw `VkResult`.
        code: i32,
    },
    /// The requested target covers no pixels on an axis.
    Degenerate {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },
}

impl VulkanError {
    /// A failed driver call.
    pub(super) fn driver(call: &'static str, result: vk::Result) -> Self {
        Self::Driver {
            call,
            code: result.as_raw(),
        }
    }

    /// A failed `vkCreateInstance`.
    ///
    /// The loader answers `INCOMPATIBLE_DRIVER` when no driver is installed or none of them
    /// supports the requested version, and `INITIALIZATION_FAILED` when the driver it found
    /// could not start. Both mean the machine has no device to draw on, which is the
    /// [`VulkanError::NoDevice`] a caller falls back on.
    pub(super) fn instance(result: vk::Result) -> Self {
        match result {
            vk::Result::ERROR_INCOMPATIBLE_DRIVER | vk::Result::ERROR_INITIALIZATION_FAILED => Self::NoDevice {
                reason: format!("the loader found no usable Vulkan driver ({result:?})"),
            },
            other => Self::driver("vkCreateInstance", other),
        }
    }
}

impl fmt::Display for VulkanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLoader { reason } => write!(formatter, "no Vulkan loader: {reason}"),
            Self::NoDevice { reason } => write!(formatter, "no suitable Vulkan device: {reason}"),
            Self::Shader { pass, message } => write!(formatter, "the {pass} shader is refused: {message}"),
            Self::Driver { call, code } => write!(formatter, "{call} failed with {:?}", vk::Result::from_raw(*code)),
            Self::Degenerate { width, height } => write!(formatter, "a {width}x{height} target covers no pixels"),
        }
    }
}

impl std::error::Error for VulkanError {}

impl VulkanError {
    /// The renderer error this becomes for a caller that does not need the reason, which is
    /// logged here instead: the renderer is unavailable, or the size was degenerate.
    #[must_use]
    pub fn into_render_error(self) -> RenderError {
        match self {
            Self::Degenerate { width, height } => RenderError::Degenerate { width, height },
            other => {
                log::warn!(area = "render", error = %other, "the Vulkan renderer could not be created");
                RenderError::Unavailable
            }
        }
    }
}

/// The renderer error a failed driver call during a frame becomes.
///
/// `DEVICE_LOST` is the one that matters: after it nothing on the device is usable, so the
/// frame is dropped and the caller is told to build a new renderer. Everything else is a
/// frame that failed and may be retried.
pub(super) fn classify(result: vk::Result) -> RenderError {
    match result {
        vk::Result::ERROR_DEVICE_LOST | vk::Result::TIMEOUT => RenderError::Lost,
        _ => RenderError::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_device_is_reported_as_lost() {
        assert_eq!(classify(vk::Result::ERROR_DEVICE_LOST), RenderError::Lost);
        assert_eq!(classify(vk::Result::TIMEOUT), RenderError::Lost);
        assert_eq!(classify(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY), RenderError::Failed);
    }

    #[test]
    fn a_loader_without_a_driver_is_no_suitable_device() {
        for result in [
            vk::Result::ERROR_INCOMPATIBLE_DRIVER,
            vk::Result::ERROR_INITIALIZATION_FAILED,
        ] {
            assert!(
                matches!(VulkanError::instance(result), VulkanError::NoDevice { .. }),
                "{result:?}"
            );
        }
        assert_eq!(
            VulkanError::instance(vk::Result::ERROR_OUT_OF_HOST_MEMORY),
            VulkanError::driver("vkCreateInstance", vk::Result::ERROR_OUT_OF_HOST_MEMORY)
        );
    }

    #[test]
    fn every_creation_failure_names_itself() {
        let loader = VulkanError::NoLoader {
            reason: "libvulkan.so.1: not found".to_owned(),
        };
        assert!(loader.to_string().starts_with("no Vulkan loader"));
        let device = VulkanError::NoDevice {
            reason: "none listed".to_owned(),
        };
        assert!(device.to_string().starts_with("no suitable Vulkan device"));
        let shader = VulkanError::Shader {
            pass: "solid",
            message: "line 3".to_owned(),
        };
        assert!(shader.to_string().contains("solid shader"));
        assert_eq!(loader.into_render_error(), RenderError::Unavailable);
        assert_eq!(
            VulkanError::Degenerate { width: 0, height: 4 }.into_render_error(),
            RenderError::Degenerate { width: 0, height: 4 }
        );
    }
}
