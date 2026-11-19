//! The WGSL sources of the passes, as the compiler receives them.
//!
//! Every fragment stage is the preamble plus a body plus an entry point, concatenated at
//! compile time, so every shipped pass goes through exactly one path.

/// The vertex stage every pass shares.
pub const VERTEX: &str = include_str!("wgsl/quad.wgsl");

/// A rectangle of one colour, with the layer's corner radius and opacity.
pub const SOLID: &str = concat!(
    include_str!("wgsl/preamble.wgsl"),
    include_str!("wgsl/solid.wgsl"),
    include_str!("wgsl/effect.wgsl"),
);

/// A layer's own pixels: cropped, scaled, tinted and clipped to its corner radius.
pub const SURFACE: &str = concat!(
    include_str!("wgsl/preamble.wgsl"),
    include_str!("wgsl/surface.wgsl"),
    include_str!("wgsl/effect.wgsl"),
);

/// A drop shadow behind a layer.
pub const SHADOW: &str = concat!(
    include_str!("wgsl/preamble.wgsl"),
    include_str!("wgsl/shadow.wgsl"),
    include_str!("wgsl/effect.wgsl"),
);

/// A border: a ring along the edge of a layer's rounded silhouette, drawn over the layer.
pub const BORDER: &str = concat!(
    include_str!("wgsl/preamble.wgsl"),
    include_str!("wgsl/border.wgsl"),
    include_str!("wgsl/effect.wgsl"),
);

/// One axis of the Gaussian blur of the backdrop behind a layer.
pub const BLUR: &str = concat!(
    include_str!("wgsl/preamble.wgsl"),
    include_str!("wgsl/blur.wgsl"),
    include_str!("wgsl/effect.wgsl"),
);

/// The whole-output view pass: the magnifier and the colour matrix, applied to a copy of the
/// finished output. It has its own uniform block.
pub const VIEW: &str = include_str!("wgsl/view.wgsl");

/// The whole-output downsample pass of a supersampled output: the virtual surface averaged down to the real mode. It
/// has its own uniform block.
pub const DOWNSAMPLE: &str = include_str!("wgsl/downsample.wgsl");

/// What goes in front of a user shader's source: the shared preamble, then the user pass's own uniform block, the backdrop's
/// bindings and the `backdrop` function. The text comes first so that a refusal's line number is the user's, counted from
/// the end of this prefix (`shader::user`).
pub const USER_PREFIX: &str = concat!(include_str!("wgsl/preamble.wgsl"), include_str!("wgsl/user_block.wgsl"),);

/// What goes after a user shader's source: the `shade` body that calls the shell's `effect`, and the shared entry point.
pub const USER_SUFFIX: &str = concat!(include_str!("wgsl/user_shade.wgsl"), include_str!("wgsl/effect.wgsl"),);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pass_declares_the_entry_point() {
        for pass in [SOLID, SURFACE, SHADOW, BORDER, BLUR] {
            assert!(pass.contains("fn shade("), "a pass defines its body");
            assert!(pass.contains("fn gnoblin_entry("), "and the shared entry point");
        }
        assert!(VERTEX.contains("fn gnoblin_entry("));
    }

    #[test]
    fn the_preamble_block_is_the_packed_size() {
        let preamble = include_str!("wgsl/preamble.wgsl");
        assert!(
            preamble.contains("padding: vec2<f32>,"),
            "the fixed members are declared"
        );
        assert_eq!(super::super::uniforms::BLOCK_BYTES, 96);
    }
}
