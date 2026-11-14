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

    /// Rounds a solid or surface pass to `silhouette` instead of its own quad.
    ///
    /// The pass keeps drawing inside its quad; only the shape its corner radius cuts from
    /// changes. The silhouette is held in `params` as its origin relative to the quad's and
    /// its size, which the preamble's `gnoblin_corner` reads. An empty silhouette is ignored,
    /// because a zero width is how the shader tells there is none.
    pub fn round_to(&mut self, silhouette: Rect) {
        if silhouette.is_empty() {
            return;
        }
        self.params = [
            silhouette.x as f32 - self.rect[0],
            silhouette.y as f32 - self.rect[1],
            silhouette.width as f32,
            silhouette.height as f32,
        ];
    }

    /// The values of a border pass.
    ///
    /// `quad` is the silhouette grown by `grow` physical pixels on every side, and the ring is
    /// the band between `near` and `far` physical pixels of signed distance from the
    /// silhouette's edge (negative inside it). `corner` is the silhouette's radius.
    pub fn border(
        quad: Rect,
        grow: f32,
        band: (f32, f32),
        colour: Color,
        tint: Color,
        opacity: f32,
        corner: f32,
    ) -> Self {
        Self {
            rect: [quad.x as f32, quad.y as f32, quad.width as f32, quad.height as f32],
            src: [0.0, 0.0, 1.0, 1.0],
            colour: components(colour),
            tint: components(tint),
            params: [grow, grow, band.0, band.1],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silhouette_is_held_relative_to_the_quad() {
        let mut uniforms = Uniforms::solid(Rect::new(10, 40, 100, 80), Color::WHITE, Color::WHITE, 1.0);
        uniforms.round_to(Rect::new(10, 12, 100, 108));
        assert_eq!(uniforms.params, [0.0, -28.0, 100.0, 108.0]);
        let mut bytes = Vec::new();
        uniforms.pack(&mut bytes);
        assert_eq!(bytes.len(), BLOCK_BYTES);
        // `params` is the fifth vec4 of the block.
        assert_eq!(f32::from_ne_bytes(bytes[72..76].try_into().expect("four bytes")), 100.0);
    }

    #[test]
    fn an_empty_silhouette_leaves_the_quad_as_the_shape() {
        let mut uniforms = Uniforms::surface(Rect::new(0, 0, 8, 8), [0.0, 0.0, 1.0, 1.0], Color::WHITE, 1.0, 3.0);
        uniforms.round_to(Rect::new(0, 0, 0, 8));
        assert_eq!(uniforms.params, [0.0; 4]);
    }

    #[test]
    fn a_border_holds_its_growth_and_its_band() {
        let uniforms = Uniforms::border(
            Rect::new(0, 0, 20, 20),
            4.0,
            (-3.0, -1.0),
            Color::WHITE,
            Color::WHITE,
            0.5,
            6.0,
        );
        assert_eq!(uniforms.params, [4.0, 4.0, -3.0, -1.0]);
        assert_eq!((uniforms.corner, uniforms.opacity), (6.0, 0.5));
    }
}
