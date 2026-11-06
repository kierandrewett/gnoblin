//! The shader compiler: WGSL in, GLSL ES 3.10 out, and a line number when it fails.
//!
//! D12's rule is that every effect and user shader is written once in one language and
//! compiled per backend. The language is WGSL (`mod.rs` records why), `naga` is the
//! compiler, and this module is the GLES half of it: the module is parsed and validated by
//! `naga`, written as GLSL ES 3.10 by `naga`'s own backend, and only then handed to the
//! driver. A shader that fails any step answers a [`ShaderError`] carrying the line of the
//! *user's* source, and never a [`RenderError`]: a broken shader disables its own pass, not
//! the frame.
//!
//! # The uniform layout
//!
//! One uniform block is shared by every pass: the fixed 96 bytes of the preamble
//! (`shaders/preamble.wgsl`), then the fields a user shader's schema declares, appended in
//! declaration order. The offsets are computed here, once, and the same bytes are written
//! for every draw.
//!
//! The rules are the ones WGSL's uniform layout and GLSL's `std140` agree on, which is why
//! one layout can serve both:
//!
//! | Type   | Alignment | Size |
//! | ------ | --------- | ---- |
//! | `f32`  | 4         | 4    |
//! | `i32`  | 4         | 4    |
//! | `vec2` | 8         | 8    |
//! | `vec3` | 16        | 12   |
//! | `vec4` | 16        | 16   |
//!
//! # Failure modes
//!
//! 1. **A shader with a syntax error kills the frame loop.** Compiling answers an error and
//!    no program, and the caller keeps drawing:
//!    `tests::a_broken_user_shader_leaves_the_frame_drawn`.
//! 2. **A uniform whose type differs from the shader's is uploaded as garbage.**
//!    [`UniformLayout::pack`] refuses a value the compiled program did not declare with that
//!    type: `tests::a_user_shader_refuses_a_mistyped_uniform`.
//! 3. **An error points at the generated preamble instead of the user's line.** The
//!    preamble's line count is subtracted, so the number a caller reports is the number the
//!    user can count to: `tests::a_syntax_error_names_the_user_line`.

use std::fmt;

use gnoblin_log::debug;
use naga::back::glsl::{Options as GlslOptions, PipelineOptions, Version, Writer};
use naga::front::wgsl::Frontend;
use naga::proc::BoundsCheckPolicies;
use naga::valid::{Capabilities, ValidationFlags, Validator};

use gnoblin_render::{Color, ResolvedUniform, UniformType, UniformValue};
use gnoblin_types::Rect;

use crate::render::RenderError;
use crate::render::gles::gl::{Device, Program};
use crate::render::shader::sources as shaders;

/// The entry point name every generated module uses, in both stages.
pub(crate) const ENTRY: &str = "gnoblin_entry";

/// The bytes of the fixed part of the uniform block, before any user field.
///
/// It is the size of `Gnoblin` in `shaders/preamble.wgsl`: five `vec4`s, two `f32`s and a
/// `vec2` of padding, which is 96 bytes — a multiple of 16, so a user field's offset is the
/// same in WGSL's uniform layout and GLSL's `std140`.
const FIXED_BYTES: usize = 96;

/// Where a shader's source came from, for a report that names it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShaderOrigin {
    /// A pass the compositor ships.
    Effect(&'static str),
    /// A shader a caller registered.
    User,
}

impl fmt::Display for ShaderOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Effect(name) => write!(formatter, "the {name} pass"),
            Self::User => write!(formatter, "the user shader"),
        }
    }
}

/// The stage of a shader that failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stage {
    /// Parsing or validating the WGSL module.
    Module,
    /// Compiling the vertex shader.
    Vertex,
    /// Compiling the fragment shader.
    Fragment,
    /// Linking the two stages.
    Link,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => write!(formatter, "the shader module"),
            Self::Vertex => write!(formatter, "the vertex stage"),
            Self::Fragment => write!(formatter, "the fragment stage"),
            Self::Link => write!(formatter, "the program link"),
        }
    }
}

/// A shader the compiler or the driver refused, at the line of the source that was written.
///
/// It is a value rather than a logged string because a caller has to be able to *report* it
/// — once, with the line — and because a failure must not be indistinguishable from a pass
/// that chose to draw nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShaderError {
    context: ShaderOrigin,
    stage: Stage,
    line: usize,
    column: usize,
    detail: String,
}

impl ShaderError {
    fn new(context: ShaderOrigin, stage: Stage, line: usize, column: usize, detail: String) -> Self {
        Self {
            context,
            stage,
            line,
            column,
            detail,
        }
    }

    /// The 1-based line of the source that was written, or 0 when the failure had no
    /// position — a uniform type mismatch fails on no line at all.
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// The 1-based column, or 0 when there is none.
    #[must_use]
    pub fn column(&self) -> usize {
        self.column
    }

    /// The compiler's or the driver's message.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// A refusal that has no line to point at: a value or a schema that disagrees with the
    /// shader the caller registered.
    pub(crate) fn refusal(detail: String) -> Self {
        Self::new(ShaderOrigin::User, Stage::Module, 0, 0, detail)
    }
}

impl fmt::Display for ShaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(formatter, "{}: {}", self.context, self.detail)
        } else if self.column == 0 {
            write!(formatter, "{} line {}: {}", self.context, self.line, self.detail)
        } else {
            write!(
                formatter,
                "{} line {}, column {}: {}",
                self.context, self.line, self.column, self.detail
            )
        }
    }
}

impl std::error::Error for ShaderError {}

/// A driver failure, before it is known which line of a source it came from.
pub(crate) struct CompileFailure {
    /// The stage that failed.
    pub stage: Stage,
    /// The driver's log.
    pub log: String,
}

impl fmt::Debug for CompileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompileFailure")
            .field("stage", &self.stage)
            .field("log", &self.log)
            .finish()
    }
}

/// The uniform values one pass uploads.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Uniforms {
    /// The destination rectangle in physical pixels.
    pub rect: [f32; 4],
    /// The texture coordinate mapping: origin and scale.
    pub src: [f32; 4],
    /// The pass's straight colour.
    pub colour: [f32; 4],
    /// The layer's multiplicative colour.
    pub tint: [f32; 4],
    /// Values whose meaning belongs to the pass.
    pub params: [f32; 4],
    /// Corner radius in physical pixels.
    pub corner: f32,
    /// Group opacity.
    pub opacity: f32,
}

impl Uniforms {
    /// The values of a solid pass: a rectangle of one colour, multiplied by its tint.
    ///
    /// A solid has a tint like every other layer, so the frame description does not have to
    /// grow a second notion of "a layer whose colour is multiplied".
    pub(crate) fn solid(rect: Rect, colour: Color, tint: Color, opacity: f32) -> Self {
        Self {
            rect: [rect.x as f32, rect.y as f32, rect.width as f32, rect.height as f32],
            src: [0.0, 0.0, 1.0, 1.0],
            colour: components(colour),
            tint: components(tint),
            params: [0.0; 4],
            corner: 0.0,
            opacity: opacity.clamp(0.0, 1.0),
        }
    }

    /// The values of a textured pass: a layer's pixels, mapped, tinted and clipped.
    pub(crate) fn surface(rect: Rect, src: [f32; 4], tint: Color, opacity: f32, corner: f32) -> Self {
        Self {
            rect: [rect.x as f32, rect.y as f32, rect.width as f32, rect.height as f32],
            src,
            colour: [0.0; 4],
            tint: components(tint),
            params: [0.0; 4],
            corner,
            opacity: opacity.clamp(0.0, 1.0),
        }
    }
}

/// A colour as the four floats a shader reads, clamped so an overshooting animation cannot
/// produce a colour outside the unit range.
fn components(colour: Color) -> [f32; 4] {
    [
        colour.r.clamp(0.0, 1.0),
        colour.g.clamp(0.0, 1.0),
        colour.b.clamp(0.0, 1.0),
        colour.a.clamp(0.0, 1.0),
    ]
}

/// Where each uniform sits in the block, and which type the shader declared for it.
#[derive(Clone, Debug)]
pub(crate) struct UniformLayout {
    fields: Vec<UniformField>,
    bytes: usize,
}

/// One user-declared uniform.
#[derive(Clone, Debug)]
struct UniformField {
    name: String,
    kind: UniformType,
    offset: usize,
}

impl UniformLayout {
    /// The layout of a program with no user uniforms: the fixed part only.
    pub(crate) fn fixed() -> Self {
        Self {
            fields: Vec::new(),
            bytes: FIXED_BYTES,
        }
    }

    /// The block's size in bytes.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// The layout a user shader's schema produces.
    ///
    /// The fields are appended in declaration order to the fixed part, so the shader's own
    /// struct is the preamble's struct plus these.
    pub(crate) fn with_schema(schema: &gnoblin_render::UniformSchema) -> Self {
        let mut fields = Vec::new();
        let mut offset = FIXED_BYTES;
        for (name, kind) in schema.entries() {
            let (alignment, size) = kind_layout(*kind);
            offset = offset.div_ceil(alignment) * alignment;
            fields.push(UniformField {
                name: name.clone(),
                kind: *kind,
                offset,
            });
            offset += size;
        }
        Self {
            fields,
            bytes: offset.max(FIXED_BYTES).div_ceil(16) * 16,
        }
    }

    /// Writes the block for one draw.
    ///
    /// # Errors
    ///
    /// [`ShaderError`] when a value's type is not the one the compiled shader declares for
    /// that name, or the name is not declared at all: the block would otherwise hold four
    /// bytes where a vector belongs, which the driver reads back as a value nobody wrote.
    pub(crate) fn pack(
        &self,
        uniforms: &Uniforms,
        values: &[ResolvedUniform],
        out: &mut Vec<u8>,
    ) -> Result<(), ShaderError> {
        out.clear();
        out.resize(self.bytes, 0);
        for (index, part) in [
            uniforms.rect,
            uniforms.src,
            uniforms.colour,
            uniforms.tint,
            uniforms.params,
        ]
        .iter()
        .enumerate()
        {
            write_floats(out, index * 16, part);
        }
        write_floats(out, 80, &[uniforms.corner, uniforms.opacity, 0.0, 0.0]);
        for value in values {
            let Some(field) = self.fields.iter().find(|field| field.name == value.name) else {
                return Err(ShaderError::new(
                    ShaderOrigin::User,
                    Stage::Module,
                    0,
                    0,
                    format!("the shader declares no uniform named `{}`", value.name),
                ));
            };
            if field.kind != value.kind {
                return Err(ShaderError::new(
                    ShaderOrigin::User,
                    Stage::Module,
                    0,
                    0,
                    format!(
                        "uniform `{}` is declared as {} but the value is {}",
                        field.name,
                        field.kind.name(),
                        value.value.kind().name()
                    ),
                ));
            }
            write_value(out, field.offset, value.value);
        }
        Ok(())
    }
}

/// The alignment and size of a declared uniform type, in the layout both languages use.
fn kind_layout(kind: UniformType) -> (usize, usize) {
    match kind {
        UniformType::Float | UniformType::Int => (4, 4),
        UniformType::Vec2 => (8, 8),
        UniformType::Vec3 => (16, 12),
        UniformType::Vec4 => (16, 16),
    }
}

/// Writes floats at `offset`, ignoring a range outside the block.
fn write_floats(out: &mut [u8], offset: usize, values: &[f32]) {
    for (index, value) in values.iter().enumerate() {
        let start = offset + index * 4;
        if let Some(bytes) = out.get_mut(start..start + 4) {
            bytes.copy_from_slice(&value.to_ne_bytes());
        }
    }
}

/// Writes one value at its offset.
fn write_value(out: &mut [u8], offset: usize, value: UniformValue) {
    match value {
        UniformValue::Float(value) => write_floats(out, offset, &[value]),
        UniformValue::Vec2(values) => write_floats(out, offset, &values),
        UniformValue::Vec3(values) => write_floats(out, offset, &values),
        UniformValue::Vec4(values) => write_floats(out, offset, &values),
        UniformValue::Int(value) => {
            if let Some(bytes) = out.get_mut(offset..offset + 4) {
                bytes.copy_from_slice(&value.to_ne_bytes());
            }
        }
    }
}

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
        debug!(%error, "[render] naga could not write GLSL");
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
        debug!(%error, "[render] an effect pass does not compile");
        RenderError::Unavailable
    })?;
    let fragment_glsl = translate(fragment, "effect pass", Stage::Fragment, Shift::NONE).map_err(|error| {
        debug!(%error, "[render] an effect pass does not compile");
        RenderError::Unavailable
    })?;
    device
        .program(&vertex_glsl, &fragment_glsl, UniformLayout::fixed())
        .map_err(|failure| {
            debug!(?failure, "[render] an effect pass does not link");
            RenderError::Unavailable
        })
}
