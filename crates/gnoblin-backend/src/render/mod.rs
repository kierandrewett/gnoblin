//! The render path: a frame description, three renderers, the shaders they share, a
//! readback, and the session scene that produces the description.
//!
//! This is the Renderer card of `design/rust-compositor.md` at the mechanism layer
//! (D12: GLES now, Pixman for software). It is split so the parts can be replaced
//! one at a time:
//!
//! | Module              | Owns                                                              |
//! | ------------------- | ----------------------------------------------------------------- |
//! | [`frame`]           | what an output draws, in numbers and client buffers, no renderer   |
//! | [`geometry`]        | the one rule that turns a logical rectangle into pixels            |
//! | [`shader`]          | the WGSL passes and the uniform block: written once, no GPU API    |
//! | [`software`]        | Pixman: the frame into an offscreen buffer, damage-only            |
//! | [`gles`]            | Smithay's GLES renderer: the same frame with effects and shaders   |
//! | `vulkan`            | `ash`: the same frame on a GPU or lavapipe, no effects yet (`vulkan` feature) |
//! | [`renderer`]        | [`RendererKind`], the choice, and [`AnyRenderer`], the choice made |
//! | [`image`]           | the RGBA8 readback a screenshot, capture and the parity probes use |
//! | [`compose`]         | the session's `gnoblin_render::Scene` and the frame it produces    |
//! | [`error`]           | [`RenderError`], the small `Copy` reason a renderer could not draw |
//! | [`shm`]             | the one writer of a client's `wl_shm` pool                         |
//!
//! # Why the frame is a value
//!
//! An [`OutputFrame`] is the seam between the compositor and a renderer, and it names no
//! renderer API: a GLES path reads the same value a Pixman path does. That is what
//! makes D12's "write every effect once, compile per backend" possible later, and
//! it is why the scene-to-frame step ([`Composer`]) can be unit-tested without a
//! device.
//!
//! # Choosing a renderer
//!
//! The three renderers share four calls (`new`, `draw`, `capture`, `resize`) over an
//! [`OutputFrame`], and the choice is [`RendererKind`], with [`RendererKind::AUTO`] as the
//! single statement of what `compositor.renderer = auto` means. Each backend picks its
//! renderer in one place:
//!
//! | Backend  | Picks                                                                          |
//! | -------- | ------------------------------------------------------------------------------ |
//! | headless | `HeadlessConfig::renderer`, any kind, built by [`AnyRenderer::new`]            |
//! | nested   | [`Software`], hard-wired in `nested/surface.rs`; the compositor refuses others |
//! | DRM      | [`Gles`], hard-wired in `drm/scanout.rs` on the device's EGL display           |
//!
//! Nested and DRM do not go through [`AnyRenderer`] yet: DRM needs to draw into a scan-out
//! buffer the device owns, which the offscreen constructor does not offer (see "Target
//! ownership" in `design/vulkan-backend.md`).
//!
//! ## Why an enum and not a trait
//!
//! [`AnyRenderer`] is an enum with one `match` per call, written once by a macro. A trait
//! would name the same four calls, but they are all the three renderers have in common:
//! GLES alone has an EGL display and context, a scan-out buffer, user shaders, a shader
//! error and a draw counter; Vulkan owns its device and target; Pixman has neither. Every
//! caller that wants one of those (a test reading [`Gles::draws`], DRM calling
//! `draw_scanout`) would have to downcast out of a `Box<dyn ...>` through an `as_any`
//! escape hatch, or the trait would grow methods two of the three implement as no-ops.
//! The set is closed and chosen at build time, so an enum costs nothing a trait buys:
//! [`AnyRenderer::gles_mut`] is a plain `Option`, and adding a renderer is a compile error
//! at every `match` instead of a silent default.
//!
//! What it replaced, in `headless.rs`: a private enum over the three renderers and a public
//! choice enum beside it, with six `match` expressions over the renderers and nine
//! `cfg(feature = "vulkan")` attributes. The feature is now tested in `renderer.rs` for
//! dispatch, so a build without it has one named refusal (`Unavailable` and a log line)
//! instead of an arm per method.
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

/// The scene and the drawing of one output's frame from it. See [`compose::Composer`].
pub mod compose;
/// Why a renderer could not draw.
pub mod error;
/// The description of one output's frame that a renderer draws.
pub mod frame;
/// The one rule every renderer converts logical rectangles to pixels with.
pub mod geometry;
/// The GLES renderer, built on Smithay's glow renderer and EGL.
pub mod gles;
/// A frame's pixels, copied out of a renderer.
pub mod image;
/// Which renderer draws an output, and the one type that holds it.
pub mod renderer;
/// The renderer-neutral half of the shader pipeline.
pub mod shader;
// The one module in this crate that needs `unsafe`: writing a frame into a client's
// `wl_shm` pool. Its own documentation says why.
/// Writes rendered pixels into a client's `wl_shm` buffer.
pub mod shm;
/// The CPU renderer, built on Pixman.
pub mod software;
// The Vulkan renderer: the same frame drawn through `ash`, with its own `unsafe` shim
// modules. See `design/vulkan-backend.md`.
/// The Vulkan renderer, enabled by the `vulkan` feature.
#[cfg(feature = "vulkan")]
pub mod vulkan;

/// The scene composer and the per-window content it draws.
pub use compose::{Composer, Content};
/// The error type of every renderer.
pub use error::RenderError;
// The colour a frame is described with, so a caller that paints a surface the scene
// graph does not know about names the same type the renderer does.
/// The layers, overlays and solid fills that make up an output frame.
pub use frame::{Layer, OutputFrame, Overlay, Solid, Surface};
/// The GLES renderer.
pub use gles::Gles;
/// Colour and matrix types shared with the scene crate.
pub use gnoblin_render::{BlurParams, Color, Effects, MAX_PASSES, Mat4, Pass, Shadow};
/// The readback pixels of a frame.
pub use image::Image;
/// The enum that dispatches to each backend, and its kind selector.
pub use renderer::{AnyRenderer, RendererKind};
/// Copies an [`Image`] into a client's `wl_shm` buffer.
pub use shm::copy_into as copy_into_shm;
/// The Pixman CPU renderer.
pub use software::Software;
#[cfg(feature = "vulkan")]
/// The Vulkan renderer.
pub use vulkan::Vulkan;
