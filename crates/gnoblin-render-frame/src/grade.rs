//! The colour grade and the grain, on the CPU.
//!
//! `gnoblin_grade` and `gnoblin_grain` in `shader/wgsl/preamble.wgsl` are the GPU's version; this is the same arithmetic
//! written once more in Rust. A solid colour (a background, a border, a shadow, a solid layer) is graded here, before it
//! reaches any renderer, so it is exact everywhere, and the software renderer, which has no shader, gets the grade for
//! free. The GPU test in each renderer draws 256 colours through the shader and compares them with [`apply`].
//!
//! The order is fixed: saturate, then brightness, then contrast, on straight sRGB values, each step clamped to 0..1.
//! No transfer function is applied, as for every other pass.
//!
//! | Step         | Formula                                                               |
//! | ------------ | --------------------------------------------------------------------- |
//! | saturate `s` | CSS `feColorMatrix type="saturate"`: luma weights 0.213, 0.715, 0.072 |
//! | brightness `b` | `c * b`                                                             |
//! | contrast `k` | `(c - 0.5) * k + 0.5`                                                 |

use gnoblin_types::Color;

/// How much one unit of grain moves a channel: `noise` of 1 is a swing of one sixteenth either way.
pub const GRAIN_SPAN: f32 = 0.0625;

/// A colour grade: the three steps and the amount of grain that follows them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    /// 1 leaves colour alone, 0 is grey, above 1 is more vivid.
    pub saturate: f32,
    /// 1 leaves colour alone; the colour is multiplied by it.
    pub brightness: f32,
    /// 1 leaves colour alone; the distance from mid grey is multiplied by it.
    pub contrast: f32,
    /// The grain amount, 0 for none; see [`grain`].
    pub noise: f32,
}

impl Grade {
    /// The grade that changes nothing.
    pub const IDENTITY: Self = Self {
        saturate: 1.0,
        brightness: 1.0,
        contrast: 1.0,
        noise: 0.0,
    };

    /// The grade as the four floats of the uniform block's `grade` member.
    #[must_use]
    pub const fn to_array(self) -> [f32; 4] {
        [self.saturate, self.brightness, self.contrast, self.noise]
    }

    /// Whether the colour steps change nothing. Grain is not a colour step: see [`grain`].
    #[must_use]
    pub fn is_identity(self) -> bool {
        self.saturate == 1.0 && self.brightness == 1.0 && self.contrast == 1.0
    }
}

impl Default for Grade {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Grades a straight (not premultiplied) colour; the alpha is kept.
///
/// An identity grade returns the colour as it was, including a channel outside 0..1, so a pass that is not graded does
/// not change under this call.
#[must_use]
pub fn apply(grade: Grade, colour: Color) -> Color {
    if grade.is_identity() {
        return colour;
    }
    let luma = 0.213 * colour.r + 0.715 * colour.g + 0.072 * colour.b;
    let step = |channel: f32| {
        let saturated = (luma + (channel - luma) * grade.saturate).clamp(0.0, 1.0);
        let lit = (saturated * grade.brightness).clamp(0.0, 1.0);
        ((lit - 0.5) * grade.contrast + 0.5).clamp(0.0, 1.0)
    };
    Color::new(step(colour.r), step(colour.g), step(colour.b), colour.a)
}

/// One round of the PCG hash, as `gnoblin_pcg` computes it.
const fn pcg(value: u32) -> u32 {
    let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// The grain added to every colour channel of the pixel at (`x`, `y`), counted from the layer's top-left.
///
/// It is `(h - 0.5) * noise * GRAIN_SPAN`, where `h` in 0..1 is the PCG hash of the pixel's position. The position is
/// the layer's own, so the grain moves with the layer, and it is the same on every renderer and every frame. It is added
/// after the grade and before the colour is premultiplied.
#[must_use]
pub fn grain(noise: f32, x: u32, y: u32) -> f32 {
    let hash = pcg(x ^ pcg(y));
    (hash as f32 / 4_294_967_295.0 - 0.5) * noise * GRAIN_SPAN
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(colour: Color) -> [f32; 3] {
        [colour.r, colour.g, colour.b]
    }

    #[test]
    fn the_identity_grade_changes_nothing() {
        let colour = Color::new(0.2, 1.4, -0.1, 0.5);
        assert_eq!(apply(Grade::IDENTITY, colour), colour);
    }

    #[test]
    fn zero_saturation_is_the_luma_grey() {
        let grade = Grade {
            saturate: 0.0,
            ..Grade::IDENTITY
        };
        let grey = apply(grade, Color::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(rgb(grey), [0.213; 3]);
    }

    #[test]
    fn the_steps_run_saturate_then_brightness_then_contrast_each_clamped() {
        let grade = Grade {
            saturate: 2.0,
            brightness: 1.5,
            contrast: 2.0,
            noise: 0.0,
        };
        // Saturate pushes red to 1.0 after the clamp, brightness leaves it at 1.0, contrast keeps it there.
        let out = apply(grade, Color::new(0.8, 0.2, 0.2, 0.25));
        let luma = 0.213 * 0.8 + 0.715 * 0.2 + 0.072 * 0.2;
        let green = (luma + (0.2 - luma) * 2.0_f32).clamp(0.0, 1.0);
        let green = ((green * 1.5).clamp(0.0, 1.0) - 0.5) * 2.0 + 0.5;
        assert_eq!(out.r, 1.0);
        assert_eq!(out.g, green.clamp(0.0, 1.0));
        assert_eq!(out.a, 0.25, "the alpha is not graded");
    }

    #[test]
    fn contrast_turns_about_mid_grey() {
        let grade = Grade {
            contrast: 2.0,
            ..Grade::IDENTITY
        };
        let out = apply(grade, Color::new(0.5, 0.25, 0.75, 1.0));
        assert_eq!(rgb(out), [0.5, 0.0, 1.0]);
    }

    #[test]
    fn grain_is_zero_at_no_noise_and_bounded_by_the_span() {
        let mut seen_low = false;
        let mut seen_high = false;
        for y in 0..64 {
            for x in 0..64 {
                assert_eq!(grain(0.0, x, y), 0.0);
                let value = grain(1.0, x, y);
                assert!(value.abs() <= GRAIN_SPAN / 2.0 + 1e-6, "({x}, {y}) gave {value}");
                seen_low |= value < -GRAIN_SPAN / 4.0;
                seen_high |= value > GRAIN_SPAN / 4.0;
            }
        }
        assert!(seen_low && seen_high, "the grain spreads over its range");
    }

    #[test]
    fn grain_is_a_function_of_the_pixel_alone() {
        assert_eq!(grain(0.7, 5, 9), grain(0.7, 5, 9));
        assert_ne!(grain(0.7, 5, 9), grain(0.7, 9, 5));
    }

    #[test]
    fn the_hash_is_the_pcg_output_function() {
        // Computed from the published PCG output function (Jarzynski and Olano, "Hash Functions for GPU Rendering"),
        // the program `gnoblin_pcg` is written from.
        assert_eq!(
            [pcg(0), pcg(1), pcg(2)],
            [0x7bb2fe2_u32, 0xa8beea3c_u32, 0x7a7ecc88_u32]
        );
    }
}
