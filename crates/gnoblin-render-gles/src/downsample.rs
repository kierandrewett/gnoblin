//! The downsample pass of a supersampled output: the virtual surface, averaged down to the mode the display shows.
//!
//! The pass has its own uniform block of 32 bytes, laid out as `GnoblinDownsample` in `wgsl/downsample.wgsl`: two
//! `vec4`s, so std140 and WGSL agree on the offsets without padding. It is never the 96-byte layer block, so a layer
//! pass cannot reach it and this pass cannot change what a layer pass reads.
//!
//! Whether the pass is an exact box average or a bilinear sample is decided by [`Image::box_ratio`], the same rule the
//! CPU path ([`Image::downsample`]) follows, so a GPU frame and a software frame of the same picture agree.

use gnoblin_render_frame::image::Image;
use gnoblin_types::Size;

/// The bytes of the downsample block: two `vec4`s.
pub(crate) const DOWNSAMPLE_BYTES: usize = 32;

/// The uniform block of the downsample pass, packed as the shader reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DownsampleUniforms {
    /// The target's width and height, then the source's.
    sizes: [f32; 4],
    /// The source pixels per target pixel across and down, both zero for a bilinear sample, then two unused slots.
    taps: [f32; 4],
}

impl DownsampleUniforms {
    /// The block for averaging a `source` surface down to `shown`.
    pub(crate) fn new(source: Size, shown: Size) -> Self {
        let taps =
            Image::box_ratio(source, shown).map_or([0.0; 4], |(across, down)| [across as f32, down as f32, 0.0, 0.0]);
        Self {
            sizes: [
                shown.width as f32,
                shown.height as f32,
                source.width as f32,
                source.height as f32,
            ],
            taps,
        }
    }

    /// The block's bytes, in the order the shader reads them.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DOWNSAMPLE_BYTES);
        for vector in [self.sizes, self.taps] {
            for value in vector {
                out.extend_from_slice(&value.to_ne_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_ne_bytes(*chunk))
            .collect()
    }

    #[test]
    fn a_whole_ratio_packs_its_taps_and_the_block_is_the_declared_size() {
        let bytes = DownsampleUniforms::new(Size::new(3840, 2160), Size::new(1920, 1080)).bytes();
        assert_eq!(bytes.len(), DOWNSAMPLE_BYTES);
        assert_eq!(
            floats(&bytes),
            [1920.0, 1080.0, 3840.0, 2160.0, 2.0, 2.0, 0.0, 0.0],
            "target size, source size, then two taps each way"
        );
    }

    #[test]
    fn a_ratio_that_is_not_whole_has_no_taps_so_the_shader_samples_bilinearly() {
        let bytes = DownsampleUniforms::new(Size::new(2880, 1620), Size::new(1920, 1080)).bytes();
        assert_eq!(floats(&bytes)[4..], [0.0; 4]);
        let uneven = DownsampleUniforms::new(Size::new(2880, 1621), Size::new(1920, 1080)).bytes();
        assert_eq!(floats(&uneven)[4..], [0.0; 4], "one uneven axis is enough");
    }
}
