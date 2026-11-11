//! The GPU side of `gnoblin-xr`.
//!
//! A captured View arrives as `wl_shm` pixels. The runtime owns the swapchain images the headset shows. This crate
//! copies the first into the second:
//!
//! - [`format`] is pure: it chooses the swapchain format from the runtime's list and converts `wl_shm` pixels to it.
//! - [`presenter`] is the [`Presenter`] trait, and a recording `FakePresenter` for tests (feature `testing`).
//! - [`vulkan`] is the Vulkan implementation, behind `XR_KHR_vulkan_enable2`. It is the only `unsafe` in the crate.

pub mod error;
pub mod format;
pub mod presenter;
pub mod vulkan;

pub use error::GpuError;
pub use format::{Pixels, SourceFormat, SwapchainFormat, choose_format, convert, packed_len};
#[cfg(any(test, feature = "testing"))]
pub use presenter::{FakePresenter, Upload};
pub use presenter::{Presenter, Target};
pub use vulkan::{VulkanDevice, VulkanPresenter};
