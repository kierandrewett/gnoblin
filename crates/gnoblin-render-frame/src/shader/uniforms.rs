//! The uniform block every pass shares: its values and the bytes they pack to.
//!
//! One block is shared by every pass: the fixed 96 bytes of the preamble
//! (`wgsl/preamble.wgsl`). The offsets are written here, once, and the same bytes are written
//! for every draw, whichever renderer uploads them.
//!
//! The rules are the ones WGSL's uniform layout and GLSL's `std140` agree on, which is why
//! one layout can serve both, and Vulkan's `std140`-compatible blocks too: every member is a
//! `vec4` except the trailing `corner` and `opacity` scalars and the `vec2` of padding, which
//! is 96 bytes, a multiple of 16.

use gnoblin_types::{Color, Rect};

/// The size of the uniform block in bytes.
///
/// It is the size of `Gnoblin` in `wgsl/preamble.wgsl`: five `vec4`s, two `f32`s and a
/// `vec2` of padding.
pub const BLOCK_BYTES: usize = 96;

/// The uniform values one pass uploads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Uniforms {
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
    pub fn solid(rect: Rect, colour: Color, tint: Color, opacity: f32) -> Self {
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
    pub fn surface(rect: Rect, src: [f32; 4], tint: Color, opacity: f32, corner: f32) -> Self {
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

    /// Writes the block for one draw into `out`, replacing what it held.
    pub fn pack(&self, out: &mut Vec<u8>) {
        out.clear();
        out.resize(BLOCK_BYTES, 0);
        for (index, part) in [self.rect, self.src, self.colour, self.tint, self.params]
            .iter()
            .enumerate()
        {
            write_floats(out, index * 16, part);
        }
        write_floats(out, 80, &[self.corner, self.opacity, 0.0, 0.0]);
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

/// Writes floats at `offset`, ignoring a range outside the block.
fn write_floats(out: &mut [u8], offset: usize, values: &[f32]) {
    for (index, value) in values.iter().enumerate() {
        let start = offset + index * 4;
        if let Some(bytes) = out.get_mut(start..start + 4) {
            bytes.copy_from_slice(&value.to_ne_bytes());
        }
    }
}
