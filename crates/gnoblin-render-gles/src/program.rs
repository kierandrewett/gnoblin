//! The GLES shader compiler: WGSL in, GLSL ES 3.10 out, and a line number when it fails.
//!
//! D12's rule is that every effect is written once in one language and compiled per backend.
//! The language is WGSL (`mod.rs` records why), `naga` is the compiler, and this module is the
//! GLES half of it: the module is parsed and validated by `naga`, written as GLSL ES 3.10 by
//! `naga`'s own backend, and only then handed to the driver.
//!
//! The sources, the uniform block's layout and [`ShaderError`] are renderer-neutral and live
//! in [`gnoblin_render_frame::shader`]; what stays here is what needs a GL driver.
//!
//! # Failure modes
//!
//! 1. **A pass that does not compile draws garbage.** A shipped pass that fails any step
//!    answers [`RenderError::Unavailable`] when the renderer is built, so a broken pass is a
//!    startup failure and never a frame that fails at its first draw.

use naga::back::glsl::{Options as GlslOptions, PipelineOptions, Version, Writer};
use naga::front::wgsl::Frontend;
use naga::proc::BoundsCheckPolicies;
use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::gl::{GlContext, Program};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::shader::{ENTRY, ShaderError, Stage};

use crate::log;

/// Translates WGSL to GLSL ES 3.10.
fn translate(source: &str, pass: &'static str, stage: Stage) -> Result<String, ShaderError> {
    let module = Frontend::new().parse(source).map_err(|error| {
        let (line, column) = error.location(source).map_or((0, 0), |location| {
            (location.line_number as usize, location.line_position as usize)
        });
        ShaderError::new(pass, stage, line, column, error.message().to_owned())
    })?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|error| {
            let (line, column) = error.location(source).map_or((0, 0), |location| {
                (location.line_number as usize, location.line_position as usize)
            });
            ShaderError::new(pass, stage, line, column, format!("{error}"))
        })?;
    let options = GlslOptions {
        version: Version::Embedded {
            version: 310,
            is_webgl: false,
        },
        ..Default::default()
    };
    let pipelines = PipelineOptions {
        shader_stage: match stage {
            Stage::Vertex => naga::ShaderStage::Vertex,
            Stage::Module | Stage::Fragment | Stage::Link => naga::ShaderStage::Fragment,
        },
        entry_point: ENTRY.to_owned(),
        multiview: None,
    };
    let mut output = String::new();
    Writer::new(
        &mut output,
        &module,
        &info,
        &options,
        &pipelines,
        BoundsCheckPolicies::default(),
    )
    .and_then(|mut writer| writer.write())
    .map_err(|error| {
        log::debug!(area = "render", %error, "naga could not write GLSL");
        ShaderError::new(
            pass,
            stage,
            0,
            0,
            format!("the shader could not be written as GLSL: {error}"),
        )
    })?;
    Ok(output)
}

/// Links an effect program from its WGSL stages.
///
/// # Errors
///
/// [`RenderError::Unavailable`] when a pass the compositor ships fails to compile, which is a
/// bug in the tree rather than a caller's mistake: there is no frame to draw without it.
pub(crate) fn link(device: &mut GlContext, vertex: &str, fragment: &str) -> Result<Program, RenderError> {
    let vertex_glsl = translate(vertex, "vertex", Stage::Vertex).map_err(|error| {
        log::debug!(area = "render", %error, "an effect pass does not compile");
        RenderError::Unavailable
    })?;
    let fragment_glsl = translate(fragment, "effect", Stage::Fragment).map_err(|error| {
        log::debug!(area = "render", %error, "an effect pass does not compile");
        RenderError::Unavailable
    })?;
    device.program(&vertex_glsl, &fragment_glsl).map_err(|failure| {
        log::debug!(area = "render", ?failure, "an effect pass does not link");
        RenderError::Unavailable
    })
}
