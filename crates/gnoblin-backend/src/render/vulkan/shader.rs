//! WGSL to SPIR-V, with `naga`.
//!
//! The passes the Vulkan renderer draws are the ones the GLES renderer draws: the same
//! WGSL sources (`render/shader/wgsl/`), the same uniform block, the same vertex stage.
//! This file is the only place they become SPIR-V. Nothing is generated at build time and
//! no `.spv` file is checked in: the sources are compiled when the renderer is created,
//! which takes milliseconds, and a source that does not compile is a named startup error
//! ([`VulkanError::Shader`]) instead of a frame that fails at its first draw.
//!
//! # The clip-space convention
//!
//! The shared vertex stage is written for Vulkan: pixel row 0 maps to clip y of -1, which is
//! the top of a Vulkan framebuffer, and depth runs from 0 to w. `naga`'s default for SPIR-V
//! is to flip y and rewrite depth for an OpenGL-style source, which would draw every frame
//! upside down, so [`options`] clears every writer flag. The test
//! `a_readback_is_top_left_first` is what catches a change to this.

use naga::back::spv::{Options, PipelineOptions, WriterFlags, write_vec};
use naga::front::wgsl::Frontend;
use naga::valid::{Capabilities, ValidationFlags, Validator};

use super::error::VulkanError;
use gnoblin_render_frame::shader::ENTRY;

/// Which stage of a module to emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StageKind {
    /// The vertex stage.
    Vertex,
    /// The fragment stage.
    Fragment,
}

impl StageKind {
    fn naga(self) -> naga::ShaderStage {
        match self {
            Self::Vertex => naga::ShaderStage::Vertex,
            Self::Fragment => naga::ShaderStage::Fragment,
        }
    }
}

/// The SPIR-V writer's options: Vulkan 1.0 SPIR-V and no coordinate rewrite.
fn options() -> Options<'static> {
    Options {
        lang_version: (1, 0),
        flags: WriterFlags::empty(),
        ..Options::default()
    }
}

/// Compiles one stage of a WGSL module to SPIR-V words.
///
/// `pass` names the pass in the error. The entry point is the one every pass shares.
///
/// # Errors
///
/// [`VulkanError::Shader`] with the parser's or the validator's message and its line.
pub(super) fn compile(pass: &'static str, source: &str, stage: StageKind) -> Result<Vec<u32>, VulkanError> {
    let refused = |message: String| VulkanError::Shader { pass, message };
    let module = Frontend::new().parse(source).map_err(|error| {
        let at = error
            .location(source)
            .map_or_else(String::new, |place| format!("line {}: ", place.line_number));
        refused(format!("{at}{}", error.message()))
    })?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| refused(format!("{error}")))?;
    let pipeline = PipelineOptions {
        shader_stage: stage.naga(),
        entry_point: ENTRY.to_owned(),
    };
    write_vec(&module, &info, &options(), Some(&pipeline))
        .map_err(|error| refused(format!("the module could not be written as SPIR-V: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render_frame::shader::sources::{SOLID, SURFACE, VERTEX};

    /// The SPIR-V magic number every module starts with.
    const MAGIC: u32 = 0x0723_0203;

    #[test]
    fn the_shared_passes_compile_to_spirv() {
        for (pass, source, stage) in [
            ("vertex", VERTEX, StageKind::Vertex),
            ("solid", SOLID, StageKind::Fragment),
            ("surface", SURFACE, StageKind::Fragment),
        ] {
            let words = compile(pass, source, stage).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(words.first(), Some(&MAGIC), "{pass} starts with the SPIR-V magic");
        }
    }

    #[test]
    fn a_source_that_does_not_compile_is_a_named_shader_error() {
        let error = compile("solid", "fn shade( {", StageKind::Fragment).expect_err("a syntax error is refused");
        match error {
            VulkanError::Shader { pass, message } => {
                assert_eq!(pass, "solid");
                assert!(message.contains("line 1"), "the message names the line: {message}");
            }
            other => panic!("expected a shader error, got {other}"),
        }
    }

    #[test]
    fn a_module_without_the_entry_point_is_refused() {
        let source = "@fragment fn other() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }";
        assert!(
            matches!(
                compile("solid", source, StageKind::Fragment),
                Err(VulkanError::Shader { .. })
            ),
            "a stage without `gnoblin_entry` has nothing to draw with"
        );
    }
}
