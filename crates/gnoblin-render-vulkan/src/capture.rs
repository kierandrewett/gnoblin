//! Copies between the renderer and a client's dmabuf: the frame into a buffer a capture client gave, an image into one, and a
//! buffer out as an image.
//!
//! Each is the linear path of `dmabuf.rs`: the buffer is mapped, bracketed with the kernel's sync ioctl and its rows are
//! copied in the byte order of its format. A buffer that is not a single linear plane in an offered format is refused with
//! [`RenderError::Unsupported`] before anything is read or written, so a capture client is told, and the compositor falls back
//! to shared memory.
//!
//! # Failure modes
//!
//! 1. **A frame is copied before it is drawn.** [`Vulkan::capture`] waits for every frame submitted before it, so the copy is
//!    of what the output shows: `tests::a_frame_copied_into_a_dmabuf_reads_back_equal`.
//! 2. **A refused buffer is half written.** The buffer is checked first and written only when the whole frame fits.

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::image::Image;
use smithay::backend::allocator::Format;
use smithay::backend::allocator::dmabuf::Dmabuf;

use super::Vulkan;
use super::dmabuf::{DmabufError, read_image, write_image};
use super::formats::dmabuf_formats;
use crate::log;

/// The error a refused or unmappable dmabuf becomes for the caller, and the reason in the log.
fn refused(error: &DmabufError) -> RenderError {
    log::warn!(area = "render", %error, "a dmabuf could not be used by the Vulkan renderer");
    match error {
        DmabufError::Map(_) => RenderError::Failed,
        _ => RenderError::Unsupported,
    }
}

impl Vulkan {
    /// The format and modifier pairs a client's buffer may have to be imported, or a capture written to.
    #[must_use]
    pub fn dmabuf_formats(&self) -> Vec<Format> {
        dmabuf_formats()
    }

    /// Draws what the output shows, with the cursor when `with_cursor`, into `dmabuf`. The copy has finished when it returns.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a buffer that is not linear, single-plane and in an offered format, and the errors of
    /// [`Vulkan::capture`].
    pub fn copy_frame_into_dmabuf(&mut self, dmabuf: &Dmabuf, with_cursor: bool) -> Result<(), RenderError> {
        super::dmabuf::linear(dmabuf).map_err(|error| refused(&error))?;
        let image = self.capture(with_cursor)?;
        write_image(dmabuf, &image).map_err(|error| refused(&error))
    }

    /// Writes `image` into `dmabuf`, which `with_device` renderers share.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a buffer the renderer does not offer.
    pub fn copy_image_into_dmabuf(&self, image: &Image, dmabuf: &Dmabuf) -> Result<(), RenderError> {
        write_image(dmabuf, image).map_err(|error| refused(&error))
    }

    /// Reads a client's `dmabuf` as an [`Image`], waiting for whoever is writing it.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a buffer the renderer does not offer.
    pub fn read_dmabuf(&self, dmabuf: &Dmabuf) -> Result<Image, RenderError> {
        read_image(dmabuf).map_err(|error| refused(&error))
    }
}
