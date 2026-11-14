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
