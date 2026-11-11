//! Client dmabufs: what the context can import, whether one buffer can be, and the texture a
//! layer is drawn from.
//!
//! `zwp_linux_dmabuf_v1` lets a client hand the compositor a GPU buffer it rendered into. The
//! compositor serves the protocol only when this renderer can import such a buffer, so the
//! three questions below are the whole of what the protocol layer asks of the GPU:
//!
//! 1. [`Gles::dmabuf_formats`] — which format and modifier pairs may a client allocate in? These
//!    are the pairs the EGL display reports as importable *and* sampleable as a plain 2D
//!    texture. A pair the display can only import as `GL_TEXTURE_EXTERNAL_OES` (a YUV layout, a
//!    vendor compression) is left out, because the surface pass samples a `sampler2D`; offering
//!    it would let a client attach a buffer that draws as garbage.
//! 2. [`Gles::render_node`] — which device is the buffer's home? The protocol's main device, so
//!    a client allocates on the GPU the compositor renders with.
//! 3. [`Gles::import_dmabuf`] — can this exact buffer be imported? The answer arrives before the
//!    client may use the buffer, so a refusal is a protocol failure and never a black window.
//!
//! # Failure modes
//!
//! 1. **A buffer the driver cannot import reaches the draw.** [`Gles::import_dmabuf`] runs the
//!    same import the draw runs and caches the texture on the buffer, so the draw finds it;
//!    a draw whose import still fails logs and skips the layer: `tests::a_dmabuf_the_driver_refuses_is_skipped`.
//! 2. **A buffer the client allocated upside down is drawn flipped.** The `y-invert` flag of the
//!    buffer reverses the texture's v axis: `tests::an_inverted_dmabuf_reverses_the_v_axis`.
//! 3. **A format only importable as an external texture is offered.** The advertised set is the
//!    display's render set, which excludes external-only modifiers.

use smithay::backend::allocator::Buffer as _;
use smithay::backend::allocator::Format;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::ImportDma as _;
use smithay::backend::renderer::gles::GlesTexture;

use gnoblin_render_frame::error::RenderError;

use crate::Gles;
use crate::log;

impl Gles {
    /// The format and modifier pairs a client may share, in the display's own order.
    ///
    /// Empty when the display has no `EGL_EXT_image_dma_buf_import`: then the compositor does not
    /// serve the protocol at all.
    pub fn dmabuf_formats(&mut self) -> Vec<Format> {
        self.device
            .renderer()
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect()
    }

    /// The device number of the render node this context draws on, or `None` when the context
    /// is not backed by a DRM node (a software rasteriser on a surfaceless display).
    pub fn render_node(&mut self) -> Option<u64> {
        let display = self.device.renderer().egl_context().display();
        let device = EGLDevice::device_for_display(display)
            .map_err(|error| log::debug!(area = "render", %error, "the EGL display names no device"))
            .ok()?;
        match device.try_get_render_node() {
            Ok(node) => node.map(|node| node.dev_id()),
            Err(error) => {
                log::debug!(area = "render", %error, "the EGL device names no render node");
                None
            }
        }
    }

    /// Imports `dmabuf` and keeps the texture on the buffer, so a draw of it finds the import
    /// done.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the buffer. The reason is logged.
    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<(), RenderError> {
        self.dmabuf_texture(dmabuf).map(drop).ok_or(RenderError::Failed)
    }

    /// The texture of `dmabuf`: the cached import, or a new one. The reason of a failure is
    /// logged, so a caller that skips the layer need not say it again.
    pub(crate) fn dmabuf_texture(&mut self, dmabuf: &Dmabuf) -> Option<GlesTexture> {
        self.device
            .renderer()
            .import_dmabuf(dmabuf, None)
            .map_err(|error| {
                log::warn!(
                    area = "render",
                    ?error,
                    format = ?dmabuf.format(),
                    "a client dmabuf could not be imported"
                );
            })
            .ok()
    }
}

/// The texture-space rectangle `[u, v, width, height]` of a layer's crop of a dmabuf.
///
/// A buffer whose first memory row is its bottom row (`y_inverted`) is read from the bottom up:
/// the crop starts at `(height - y) / height` and the v extent is negative, the way `flipped_mapping`
/// in the crate root states a rectangle of the renderer's own target.
pub(crate) fn mapping(src: [f64; 4], size: (i32, i32), y_inverted: bool) -> [f32; 4] {
    let (width, height) = (size.0 as f32, size.1 as f32);
    let [x, y, w, h] = [src[0] as f32, src[1] as f32, src[2] as f32, src[3] as f32];
    if y_inverted {
        [x / width, (height - y) / height, w / width, -h / height]
    } else {
        [x / width, y / height, w / width, h / height]
    }
}
