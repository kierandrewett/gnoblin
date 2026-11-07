//! The uniform block every pass shares: its values and where each one sits.
//!
//! One block is shared by every pass: the fixed 96 bytes of the preamble
//! (`wgsl/preamble.wgsl`), then the fields a user shader's schema declares, appended in
//! declaration order. The offsets are computed here, once, and the same bytes are written
//! for every draw, whichever renderer uploads them.
//!
//! The rules are the ones WGSL's uniform layout and GLSL's `std140` agree on, which is why
//! one layout can serve both, and Vulkan's `std140`-compatible blocks too:
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
//! 1. **A uniform whose type differs from the shader's is uploaded as garbage.**
//!    [`UniformLayout::pack`] refuses a value the compiled program did not declare with that
//!    type: `gles::tests::a_user_shader_refuses_a_mistyped_uniform`.

use gnoblin_render::{ResolvedUniform, UniformType, UniformValue};
use gnoblin_types::{Color, Rect};

use super::error::{ShaderError, ShaderOrigin, Stage};

/// The bytes of the fixed part of the uniform block, before any user field.
///
/// It is the size of `Gnoblin` in `wgsl/preamble.wgsl`: five `vec4`s, two `f32`s and a
/// `vec2` of padding, which is 96 bytes — a multiple of 16, so a user field's offset is the
/// same in WGSL's uniform layout and GLSL's `std140`.
const FIXED_BYTES: usize = 96;

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
