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
pub mod frame;
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

use smithay::utils::{Physical, Rectangle};

use gnoblin_types::{Rect, Scale};

/// A logical rectangle in physical pixels.
///
/// Both edges are converted and rounded rather than the origin and the extent, so
/// two rectangles that share an edge still share it after scaling: rounding the
/// extent on its own would leave a one-pixel gap that nothing ever repaints. Both
/// renderers use it, so a frame's damage lands on the same pixels whichever one is
/// installed.
fn physical_rect(rect: Rect, scale: Scale) -> Rectangle<i32, Physical> {
    if scale.is_identity() {
        return Rectangle::new((rect.x, rect.y).into(), (rect.width as i32, rect.height as i32).into());
    }
    let ratio = scale.as_f64();
    let left = (f64::from(rect.x) * ratio).round() as i32;
    let top = (f64::from(rect.y) * ratio).round() as i32;
    let right = (rect.right() as f64 * ratio).round() as i32;
    let bottom = (rect.bottom() as f64 * ratio).round() as i32;
    Rectangle::new(
        (left, top).into(),
        ((right - left).max(0), (bottom - top).max(0)).into(),
    )
}

use std::fmt;

/// Why a renderer could not draw.
///
/// It is deliberately small and `Copy`: a frame failure must never be the reason a
/// session stops, so the compositor logs it and keeps the previous pixels. The
/// detail of a pixman failure is in the log line where it happened, not in the
/// value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderError {
    /// The renderer or its buffer could not be created.
    Unavailable,
    /// The requested buffer or output covers no pixels.
    Degenerate {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },
    /// The renderer refused a drawing operation.
    Failed,
    /// The device behind the renderer is gone, so the frame was dropped.
    ///
    /// Nothing the renderer created can be used again: the caller builds a new renderer
    /// and damages the whole output. Only a GPU renderer reports it.
    Lost,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => write!(formatter, "the renderer could not be created"),
            Self::Degenerate { width, height } => write!(formatter, "a {width}x{height} target covers no pixels"),
            Self::Failed => write!(formatter, "the renderer refused a drawing operation"),
            Self::Lost => write!(formatter, "the renderer's device was lost"),
        }
    }
}

impl std::error::Error for RenderError {}
