//! The renderer-neutral half of the shader pipeline.
//!
//! D12's rule is that every effect is written once, in WGSL, and compiled per backend. This
//! module is the "once": nothing here names a GPU API, and the GLES and Vulkan renderers
//! both read it.
//!
//! | Module       | Owns                                                                   |
//! | ------------ | ---------------------------------------------------------------------- |
//! | `sources`    | the WGSL of the passes (`wgsl/*.wgsl`)                                 |
//! | `uniforms`   | the uniform values of a pass and the byte layout of the block          |
//! | `error`      | [`ShaderError`], the line-numbered refusal a broken pass answers       |
//!
//! The *compilers* are not here, because each one ends in an API: the GLES program module
//! turns the WGSL into GLSL ES 3.10 for a driver, and the Vulkan shader module turns it
//! into SPIR-V. Both enter a module through [`ENTRY`]. The shipped passes are the only
//! shaders: there is no user shader (survey 16.6, decision O7).

mod error;
pub mod sources;
pub mod uniforms;

pub use error::ShaderError;
pub use error::Stage;

/// The entry point name every generated module uses, in both stages.
pub const ENTRY: &str = "gnoblin_entry";
