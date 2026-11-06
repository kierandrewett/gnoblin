//! The render path: a frame description, a software renderer, a readback, and the
//! session scene that produces the description.
//!
//! This is the Renderer card of `design/rust-compositor.md` at the mechanism layer
//! (D12: GLES now, Pixman for software). It is split so the parts can be replaced
//! one at a time:
//!
//! | Module              | Owns                                                              |
//! | ------------------- | ----------------------------------------------------------------- |
//! | [`frame`]           | what an output draws, in numbers and client buffers, no renderer   |
//! | [`software`]        | Pixman: the frame into an offscreen buffer, damage-only            |
//! | [`gles`]            | Smithay's GLES renderer: the same frame with effects and shaders   |
//! | `vulkan`            | `ash`: the same frame on a GPU or lavapipe, no effects yet         |
//! | [`image`]           | the RGBA8 readback a screenshot, capture and the parity probes use |
//! | [`compose`]         | the session's `gnoblin_render::Scene` and the frame it produces    |
//!
//! # Why the frame is a value
//!
//! An [`OutputFrame`] is the seam between the compositor and a renderer, and it names no
//! renderer API: a GLES path reads the same value a Pixman path does. That is what
//! makes D12's "write every effect once, compile per backend" possible later, and
//! it is why the scene-to-frame step ([`Composer`]) can be unit-tested without a
//! device.
//!
//! # Contract with the backend
//!
//! [`Backend::render`](crate::Backend::render) asks the
//! [`Host`](crate::Host) to fill a frame for one output and hands it to that
//! output's [`Target`](crate::Target). The host takes the scene's damage once per
//! iteration ([`Host::begin_frame`](crate::Host::begin_frame)) and a target that has
//! nothing to draw draws nothing, which is what makes an idle iteration free.
//!
//! # Colour
//!
//! Every colour crosses this boundary in one form: [`gnoblin_render::Color`], linear
//! components from 0 to 1 where the stored values are display sRGB. A renderer
//! clamps at draw time, so an overshooting animation cannot produce an invalid
//! colour, and nothing here applies a transfer function: colour management is
//! Gnoblin's own pipeline, which D12 leaves for later.

pub mod compose;
pub mod error;
pub mod frame;
pub mod geometry;
pub mod gles;
pub mod image;
pub mod shader;
// The one module in this crate that needs `unsafe`: writing a frame into a client's
// `wl_shm` pool. Its own documentation says why.
pub mod shm;
pub mod software;
// The Vulkan renderer: the same frame drawn through `ash`, with its own `unsafe` shim
// modules. See `design/vulkan-backend.md`.
#[cfg(feature = "vulkan")]
pub mod vulkan;

pub use compose::{Composer, Content};
pub use error::RenderError;
// The colour a frame is described with, so a caller that paints a surface the scene
// graph does not know about names the same type the renderer does.
pub use frame::{Layer, OutputFrame, Overlay, Solid, Surface};
pub use gles::Gles;
pub use gnoblin_render::cursor;
pub use gnoblin_render::{Color, Mat4};
pub use image::Image;
pub use shm::copy_into as copy_into_shm;
pub use software::Software;
#[cfg(feature = "vulkan")]
pub use vulkan::Vulkan;
