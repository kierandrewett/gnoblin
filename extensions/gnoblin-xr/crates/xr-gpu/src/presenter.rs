//! The [`Presenter`] trait: copies a captured picture into a swapchain image.

use crate::error::GpuError;
use crate::format::{Pixels, SwapchainFormat};

/// A swapchain image the runtime handed out, ready to be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The raw `VkImage` handle the runtime reported for the image.
    pub image: u64,
    pub width: u32,
    pub height: u32,
    pub format: SwapchainFormat,
}

/// Writes pictures into swapchain images.
///
/// The caller owns the swapchain: it acquires the image and waits for it, then calls [`Presenter::upload`], then
/// releases the image. `upload` returns once the copy has finished on the GPU, so the release is safe.
pub trait Presenter {
    /// Converts `pixels` to the target's format and copies them into `target`.
    ///
    /// # Errors
    ///
    /// The pixels do not match the target, or the GPU refuses a call.
    fn upload(&mut self, target: Target, pixels: &Pixels<'_>) -> Result<(), GpuError>;
}

/// What a [`FakePresenter`] recorded for one upload.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub target: Target,
    /// The converted bytes, tightly packed, in the target's format.
    pub bytes: Vec<u8>,
}

/// A [`Presenter`] that records every upload and does no GPU work.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Default)]
pub struct FakePresenter {
    pub uploads: Vec<Upload>,
    /// Makes the upload that is this many (1-based) uploads in fail.
    pub fail_on_upload: Option<usize>,
}

#[cfg(any(test, feature = "testing"))]
impl Presenter for FakePresenter {
    fn upload(&mut self, target: Target, pixels: &Pixels<'_>) -> Result<(), GpuError> {
        if self.fail_on_upload == Some(self.uploads.len() + 1) {
            // A failed upload is still counted, with no bytes, so a test sees the attempt.
            self.uploads.push(Upload {
                target,
                bytes: Vec::new(),
            });
            return Err(GpuError::Unavailable("injected failure".to_owned()));
        }
        let mut bytes = vec![0; crate::format::packed_len(pixels.width, pixels.height)];
        crate::format::convert(pixels, target.format, &mut bytes)?;
        self.uploads.push(Upload { target, bytes });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::SourceFormat;

    #[test]
    fn the_fake_records_converted_pixels() {
        let mut fake = FakePresenter::default();
        let src = [1, 2, 3, 4];
        let pixels = Pixels {
            width: 1,
            height: 1,
            stride: 4,
            format: SourceFormat::Argb8888,
            bytes: &src,
        };
        let target = Target {
            image: 7,
            width: 1,
            height: 1,
            format: SwapchainFormat::Rgba8Srgb,
        };
        fake.upload(target, &pixels).expect("uploads");
        assert_eq!(fake.uploads.len(), 1);
        assert_eq!(fake.uploads[0].target, target);
        assert_eq!(fake.uploads[0].bytes, vec![3, 2, 1, 4]);
    }
}
