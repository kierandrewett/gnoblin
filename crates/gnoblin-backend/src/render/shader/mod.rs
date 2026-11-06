//! The renderer-neutral half of the shader pipeline.
//!
//! D12's rule is that every effect and user shader is written once, in WGSL, and compiled
//! per backend. This module is the "once": nothing here names a GPU API, and the GLES and
//! Vulkan renderers both read it.
//!
//! | Module       | Owns                                                                   |
//! | ------------ | ---------------------------------------------------------------------- |
//! | `sources`    | the WGSL of the passes (`wgsl/*.wgsl`) and the preamble a user's joins |
//! | `uniforms`   | the uniform values of a pass and the byte layout of the block          |
//! | `error`      | [`ShaderError`], the line-numbered refusal a broken shader answers     |
//!
//! The *compilers* are not here, because each one ends in an API: `gles::program` turns the
//! WGSL into GLSL ES 3.10 for a driver, and `vulkan::shader` turns it into SPIR-V. Both
//! enter a module through [`ENTRY`].

mod error;
pub(crate) mod sources;
pub(crate) mod uniforms;

pub use error::ShaderError;
pub(crate) use error::{ShaderOrigin, Stage};

/// The entry point name every generated module uses, in both stages.
pub(crate) const ENTRY: &str = "gnoblin_entry";
