//! The renderer-neutral frame: the description of one output's frame that every renderer
//! draws, the one rounding rule they convert logical rectangles with, the shader sources
//! they share, the renderer error type, the readback image, and the writer that copies a
//! readback into a client's `wl_shm` buffer.
//!
//! This crate is the seam of `design/rust-compositor.md` (D12): a renderer depends on this
//! crate and on nothing else in the render path, so a GLES, Vulkan or Pixman renderer is
//! one crate that reads the same value. It names no GPU API. The modules are:
//!
//! | Module       | Owns                                                                   |
//! | ------------ | ---------------------------------------------------------------------- |
//! | [`frame`]    | what an output draws, in numbers and client buffers, no renderer        |
//! | [`geometry`] | the one rule that turns a logical rectangle into pixels                 |
//! | [`shader`]   | the WGSL passes and the uniform block: written once, no GPU API         |
//! | [`image`]    | the RGBA8 readback a screenshot, capture and the parity probes use      |
//! | [`error`]    | [`error::RenderError`], the small `Copy` reason a renderer could not draw |
//! | [`shm`]      | the one writer of a client's `wl_shm` pool                              |
//!
//! Items are named through their module (`gnoblin_render_frame::frame::OutputFrame`): the
//! crate re-exports nothing, so every caller states where a type lives.
//!
//! # Where this crate has to use unsafe
//!
//! The workspace forbids `unsafe`, and this crate opts out with the table `Cargo.toml`
//! declares. The only `unsafe` is in [`shm`], which maps a client's pool to copy pixels
//! into it; that module opens with `#[allow(unsafe_code)]` and a `SAFETY:` comment per
//! block. Nothing else in the crate contains `unsafe`.

/// Why a renderer could not draw.
pub mod error;
/// The description of one output's frame that a renderer draws.
pub mod frame;
/// The one rule every renderer converts logical rectangles to pixels with.
pub mod geometry;
/// A frame's pixels, copied out of a renderer.
pub mod image;
/// The renderer-neutral half of the shader pipeline.
pub mod shader;
/// Writes rendered pixels into a client's `wl_shm` buffer.
pub mod shm;
