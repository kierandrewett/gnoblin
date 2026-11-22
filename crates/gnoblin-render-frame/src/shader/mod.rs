//! The renderer-neutral half of the shader pipeline.
//!
//! The rule is that every effect is written once, in WGSL, and compiled per backend. This
//! module is the "once": nothing here names a GPU API, and the GLES and Vulkan renderers
//! both read it.
//!
//! | Module       | Owns                                                                   |
//! | ------------ | ---------------------------------------------------------------------- |
//! | `sources`    | the WGSL of the passes (`wgsl/*.wgsl`)                                 |
//! | `uniforms`   | the uniform values of a pass and the byte layout of the blocks         |
//! | `error`      | [`ShaderError`], the line-numbered refusal a broken pass answers       |
//! | `user`       | the validator of a shell's own shader and the module it compiles to    |
//! | `cache`      | [`ShaderCache`], the 64-entry least-recently-used compiled-shader cache |
//! | `timings`    | [`ShaderTimings`], where a renderer records what each shader pass cost  |
//! | `watchdog`   | the strikes and refusals the recorded costs lead to                    |
//!
//! The *compilers* are not here, because each one ends in an API: the GLES program module
//! turns the WGSL into GLSL ES 3.10 for a driver, and the Vulkan shader module turns it
//! into SPIR-V. Both enter a module through [`ENTRY`]. Besides the shipped passes there is
//! one pass a shell writes: `user::validate` checks its source and returns the module
//! (`Validated::wgsl`) that the same compilers take, with `sources::USER_PREFIX` and
//! `sources::USER_SUFFIX` around the shell's text and `uniforms::UserBlock` as its second
//! uniform block.

pub mod cache;
mod error;
pub mod sources;
pub mod timings;
pub mod uniforms;
pub mod user;
pub mod watchdog;

pub use cache::{SHADER_CACHE_ENTRIES, ShaderCache};
pub use error::ShaderError;
pub use error::Stage;
pub use timings::{
    FRAMES_OVER_BUDGET, SHADER_FRAME_BUDGET, STRIKES_TO_REFUSE, ShaderCost, ShaderReport, ShaderTimings,
};

/// The entry point name every generated module uses, in both stages.
pub const ENTRY: &str = "gnoblin_entry";
