//! The GLES shader compiler: WGSL in, GLSL ES 3.10 out, and a line number when it fails.
//!
//! D12's rule is that every effect and user shader is written once in one language and
//! compiled per backend. The language is WGSL (`mod.rs` records why), `naga` is the
//! compiler, and this module is the GLES half of it: the module is parsed and validated by
//! `naga`, written as GLSL ES 3.10 by `naga`'s own backend, and only then handed to the
//! driver. A shader that fails any step answers a [`ShaderError`] carrying the line of the
//! *user's* source, and never a [`RenderError`]: a broken shader disables its own pass, not
//! the frame.
//!
//! The sources, the uniform block's layout and [`ShaderError`] are renderer-neutral and live
//! in [`crate::render::shader`]; what stays here is what needs a GL driver.
//!
//! # Failure modes
//!
//! 1. **A shader with a syntax error kills the frame loop.** Compiling answers an error and
//!    no program, and the caller keeps drawing:
//!    `tests::a_broken_user_shader_leaves_the_frame_drawn`.
//! 2. **An error points at the generated preamble instead of the user's line.** The
//!    preamble's line count is subtracted, so the number a caller reports is the number the
//!    user can count to: `tests::a_syntax_error_names_the_user_line`.

use gnoblin_log::debug_in;
use naga::back::glsl::{Options as GlslOptions, PipelineOptions, Version, Writer};
use naga::front::wgsl::Frontend;
use naga::proc::BoundsCheckPolicies;
use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::render::RenderError;
use crate::render::gles::gl::{CompileFailure, Device, Program};
use crate::render::shader::sources as shaders;
use crate::render::shader::uniforms::UniformLayout;
use crate::render::shader::{ENTRY, ShaderError, ShaderOrigin, Stage};

/// How a line in the generated module maps to a line of the source that was written.
///
/// A user's module is the preamble, then their source, then an entry point, so an error's
/// line is the generated one minus the preamble's. The effect passes are generated whole by
/// the compositor, so their lines are already the ones a reader can count to.
#[derive(Clone, Copy)]
struct Shift {
    /// The number of generated lines before the writable source starts.
    preamble: usize,
    /// The number of lines the writable source has, which is the highest line an error can
    /// point at.
    user: usize,
}

impl Shift {
    /// A module whose lines are all the compositor's.
    const NONE: Self = Self {
        preamble: 0,
        user: usize::MAX,
    };

    /// A module that ends with a caller's source.
    fn user(preamble: usize, user: usize) -> Self {
        Self {
            preamble,
            user: user.max(1),
        }
    }

    /// The line of the writable source a generated line refers to.
    fn line(self, generated: usize) -> usize {
        if generated > self.preamble {
            (generated - self.preamble).min(self.user)
        } else {
            // The failure is in the generated part, which the caller cannot see; their first
            // line is the closest true statement.
            1
        }
    }
}

/// Compiles a user shader into a program on `device`.
///
/// The module is the preamble with the schema's uniforms in its block, the user's `shade`
/// function and the generated entry point that maps the quad onto the layer's texture, so a
/// user shader samples the pixels it is shading without knowing about crops, scale or
/// orientation.
///
/// # Errors
///
/// [`ShaderError`] with the line of the user's own source. The caller keeps drawing.
pub(crate) fn compile(
    device: &mut Device,
    source: &str,
    schema: &gnoblin_render::UniformSchema,
) -> Result<Program, ShaderError> {
    let layout = UniformLayout::with_schema(schema);
    let preamble = shaders::preamble(schema);
    let mut module = String::with_capacity(preamble.len() + source.len() + shaders::USER_ENTRY.len());
    module.push_str(&preamble);
    module.push('\n');
    let offset = module.lines().count();
    module.push_str(source);
    module.push('\n');
    module.push_str(shaders::USER_ENTRY);
    let shift = Shift::user(offset, source.lines().count());
    let fragment = translate(&module, "user shader", Stage::Fragment, shift)?;
    let vertex = translate(shaders::VERTEX, "vertex stage", Stage::Vertex, Shift::NONE)?;
    device
        .program(&vertex, &fragment, layout)
        .map_err(|failure| from_failure(shift, failure))
}

/// Translates WGSL to GLSL ES 3.10.
fn translate(source: &str, context: &'static str, stage: Stage, shift: Shift) -> Result<String, ShaderError> {
    let module = Frontend::new().parse(source).map_err(|error| {
        let (line, column) = error.location(source).map_or((0, 0), |location| {
            (location.line_number as usize, location.line_position as usize)
        });
        ShaderError::new(
            ShaderOrigin::Effect(context),
            stage,
            shift.line(line),
            column,
            error.message().to_owned(),
        )
    })?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|error| {
            let (line, column) = error.location(source).map_or((0, 0), |location| {
                (location.line_number as usize, location.line_position as usize)
            });
            ShaderError::new(
                ShaderOrigin::Effect(context),
                stage,
                shift.line(line),
                column,
                format!("{error}"),
            )
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
        debug_in!("backend", %error, "[render] naga could not write GLSL");
        ShaderError::new(
            ShaderOrigin::Effect(context),
            stage,
            0,
            0,
            format!("the shader could not be written as GLSL: {error}"),
        )
    })?;
    Ok(output)
}

/// Turns a driver failure into an error at the line of the user's source.
///
/// A driver's log is `ERROR: 0:12: message`, where the middle field is the line of the
/// *generated* module; the preamble's lines come off it. A failure inside the preamble
/// itself is reported at the user's first line, because their source is the only one they
/// can act on.
fn from_failure(shift: Shift, failure: CompileFailure) -> ShaderError {
    let mut line = 0;
    for part in failure.log.split(':') {
        if let Ok(number) = part.trim().parse::<usize>() {
            line = number;
            break;
        }
    }
    line = shift.line(line);
    let detail = if failure.log.trim().is_empty() {
        format!("{} was refused by the driver", failure.stage)
    } else {
        failure.log
    };
    ShaderError::new(ShaderOrigin::User, failure.stage, line, 0, detail)
}

/// Links an effect program from its WGSL stages.
///
/// # Errors
///
/// [`RenderError::Unavailable`] when a pass the compositor ships fails to compile, which is a
/// bug in the tree rather than a caller's mistake: there is no frame to draw without it.
pub(crate) fn link(device: &mut Device, vertex: &str, fragment: &str) -> Result<Program, RenderError> {
    let vertex_glsl = translate(vertex, "vertex stage", Stage::Vertex, Shift::NONE).map_err(|error| {
        debug_in!("backend", %error, "[render] an effect pass does not compile");
        RenderError::Unavailable
    })?;
    let fragment_glsl = translate(fragment, "effect pass", Stage::Fragment, Shift::NONE).map_err(|error| {
        debug_in!("backend", %error, "[render] an effect pass does not compile");
        RenderError::Unavailable
    })?;
    device
        .program(&vertex_glsl, &fragment_glsl, UniformLayout::fixed())
        .map_err(|failure| {
            debug_in!("backend", ?failure, "[render] an effect pass does not link");
            RenderError::Unavailable
        })
}
