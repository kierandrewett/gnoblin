#![allow(unsafe_code)]
//! The downsample pass of a supersampled output: the virtual surface, averaged down to the mode the display shows.
//!
//! An output composed at `mode * s` is drawn into a target of that size by [`Vulkan::draw`] as any other. [`Vulkan::downsampled`]
//! then averages the target down into an image of the mode and reads that back, which is the picture the display shows. The
//! pass has its own uniform block of 32 bytes, laid out as `GnoblinDownsample` in `wgsl/downsample.wgsl`: two `vec4`s. It is
//! never the 112-byte layer block, so a layer pass cannot reach it. `gnoblin-render-gles`'s `downsample.rs` packs the same
//! layout; this one sets `taps.z`, which says the source is top-down.
//!
//! Whether the pass is an exact box average or a bilinear sample is decided by [`Image::box_ratio`], the same rule the CPU path
//! ([`Image::downsample`]) follows, so a Vulkan frame and a software frame of the same picture agree. The averaged image is kept
//! between calls and made again when the shown size changes.

use ash::vk;
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::image::Image;
use gnoblin_types::Size;

use super::Vulkan;
use super::frame::{RenderTarget, Resting};
use super::plan::DrawBatch;
use super::spatial::Replace;
use super::view::push_block;

/// The bytes of the downsample block: two `vec4`s.
pub(super) const DOWNSAMPLE_BYTES: usize = 32;

/// The uniform block of the downsample pass, packed as the shader reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DownsampleUniforms {
    /// The target's width and height, then the source's.
    sizes: [f32; 4],
    /// The source pixels per target pixel across and down, both zero for a bilinear sample, then `1` for a top-down source.
    taps: [f32; 4],
}

impl DownsampleUniforms {
    /// The block for averaging a `source` surface down to `shown`.
    pub(super) fn new(source: Size, shown: Size) -> Self {
        let taps = Image::box_ratio(source, shown).map_or([0.0, 0.0, 1.0, 0.0], |(across, down)| {
            [across as f32, down as f32, 1.0, 0.0]
        });
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
    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DOWNSAMPLE_BYTES);
        for vector in [self.sizes, self.taps] {
            for value in vector {
                out.extend_from_slice(&value.to_ne_bytes());
            }
        }
        out
    }
}

impl Vulkan {
    /// The target as the display shows it at `shown` pixels: [`Vulkan::capture`] averaged down on the GPU, with the cursor,
    /// which is part of the picture a display shows.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels, [`RenderError::Lost`] when the device is gone, and
    /// [`RenderError::Failed`] when the device refuses the pass or the read.
    pub fn downsampled(&mut self, shown: Size) -> Result<Image, RenderError> {
        if shown.is_empty() {
            return Err(RenderError::Degenerate {
                width: shown.width,
                height: shown.height,
            });
        }
        if self.is_lost() {
            return Err(RenderError::Lost);
        }
        let averaged = self.average_into_shown(shown);
        self.collect();
        let image = averaged.map_err(|result| self.fail(result))?;
        let bytes = self
            .read_back(image, shown, Resting::attachment())
            .map_err(|result| self.fail(result))?;
        self.trim();
        Image::new(shown, shown.width as usize * 4, bytes).ok_or(RenderError::Failed)
    }

    /// Runs the pass: the target is read whole and written over all of the averaged image, which is returned.
    fn average_into_shown(&mut self, shown: Size) -> Result<vk::Image, vk::Result> {
        let device = std::rc::Rc::clone(&self.device);
        let Some(extras) = device.extras.as_ref() else {
            return Err(vk::Result::ERROR_FEATURE_NOT_PRESENT);
        };
        let pipeline = extras
            .replacing(&device.pipelines, Replace::Downsample)
            .map_err(|_| vk::Result::ERROR_INITIALIZATION_FAILED)?;
        if self
            .shown
            .as_ref()
            .is_none_or(|held| Size::new(held.image.width(), held.image.height()) != shown)
        {
            let made = RenderTarget::new(&device.context, device.pipelines.render_pass, shown)?;
            if let Some(old) = self.shown.replace(made) {
                // A frame in flight cannot be using it: it is only ever drawn and read inside one call. The queue may
                // not have finished the read, though.
                device
                    .retire
                    .borrow_mut()
                    .push(device.context.timeline.submitted(), old);
            }
        }
        let mut batch = DrawBatch::default();
        let pass = push_block(
            &mut batch,
            &DownsampleUniforms::new(self.physical, shown).bytes(),
            shown,
        );
        self.active = self.ring.acquire();
        self.ring.slot_mut(self.active).begin()?;
        self.upload_batch(&batch)?;
        let uniform_set = self.uniform_set()?;
        // A whole-ratio average reads texels one by one, so the sampler only matters to the bilinear case.
        let source = self.texture_set(self.target.image.view(), device.pipelines.linear)?;
        let (shown_image, shown_framebuffer) = match &self.shown {
            Some(held) => (held.image.raw(), held.framebuffer),
            None => return Err(vk::Result::ERROR_UNKNOWN),
        };
        let target = self.target.image.raw();
        self.to_sampled(target);
        self.to_drawn(shown_image);
        self.whole_pass((shown_framebuffer, shown), pipeline, (uniform_set, source), pass);
        self.to_attached(target);
        self.ring.slot_mut(self.active).submit()?;
        Ok(shown_image)
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
            [1920.0, 1080.0, 3840.0, 2160.0, 2.0, 2.0, 1.0, 0.0],
            "target size, source size, two taps each way and the top-down flag"
        );
    }

    #[test]
    fn a_ratio_that_is_not_whole_has_no_taps_so_the_shader_samples_bilinearly() {
        let bytes = DownsampleUniforms::new(Size::new(2880, 1620), Size::new(1920, 1080)).bytes();
        assert_eq!(floats(&bytes)[4..], [0.0, 0.0, 1.0, 0.0]);
    }
}
