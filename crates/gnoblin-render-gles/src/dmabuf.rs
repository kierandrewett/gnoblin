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
//! 4. **A plane that is no GPU memory reaches the driver.** Mesa's llvmpipe takes a plane such as `/dev/null` or a
//!    short file at import and dies of SIGSEGV on its rasteriser thread when the surface pass samples it, so a
//!    client could end the compositor. [`unmappable`] asks the kernel first: every plane must be a dma-buf, whose
//!    size is fixed for the buffer's life, and must hold the rows the buffer says it has. Every path that makes a
//!    texture of a client's buffer (import, draw, capture, readback) goes through [`Gles::dmabuf_texture`], so none
//!    skips the question.

use std::os::fd::BorrowedFd;

use smithay::backend::allocator::Buffer as _;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Format, Modifier};
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::ImportDma as _;
use smithay::backend::renderer::gles::GlesTexture;

use gnoblin_render_frame::error::RenderError;

use crate::Gles;
use crate::log;

/// `DMA_BUF_MAGIC` of `linux/magic.h`: the `f_type` of the pseudo filesystem every dma-buf lives on, which no other
/// file does.
const DMA_BUF_MAGIC: i128 = 0x444d_4142;

/// Why a plane of a client's buffer is not memory the driver may map.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PlaneFault {
    /// The kernel cannot say what the descriptor is.
    Unreadable,
    /// The descriptor is not a dma-buf: a regular file, a pipe, `/dev/null`.
    NotDmabuf,
    /// The dma-buf is smaller than the rows the buffer claims.
    TooSmall {
        /// The bytes the plane needs.
        need: u64,
        /// The bytes the dma-buf has.
        have: u64,
    },
}

impl std::fmt::Display for PlaneFault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("the kernel cannot describe the descriptor"),
            Self::NotDmabuf => formatter.write_str("the descriptor is not a dma-buf"),
            Self::TooSmall { need, have } => {
                write!(formatter, "the dma-buf holds {have} bytes and the plane needs {need}")
            }
        }
    }
}

/// The first reason a plane of `dmabuf` cannot be mapped, or `None` when every plane is a dma-buf at least as large as
/// the rows the buffer says it has.
///
/// A plane needs one row (`offset + stride`). The first plane of a linear or implicit-modifier buffer needs all
/// `height` of them: a vendor modifier lays its planes out in tiles and compression metadata that only the driver
/// can size, and a driver that is given a real dma-buf judges those itself. A dma-buf cannot be resized, so the answer
/// holds for the buffer's life; a regular file or memfd could be truncated under the driver, which is why none is let
/// through.
pub(crate) fn unmappable(dmabuf: &Dmabuf) -> Option<(usize, PlaneFault)> {
    let height = u64::try_from(dmabuf.size().h).unwrap_or(0).max(1);
    let whole = matches!(dmabuf.format().modifier, Modifier::Linear | Modifier::Invalid);
    let planes = dmabuf.handles().zip(dmabuf.offsets()).zip(dmabuf.strides());
    for (index, ((fd, offset), stride)) in planes.enumerate() {
        let rows = if index == 0 && whole { height } else { 1 };
        let need = u64::from(offset).saturating_add(u64::from(stride).saturating_mul(rows));
        if let Err(fault) = plane_holds(fd, need) {
            return Some((index, fault));
        }
    }
    None
}

/// Whether `fd` is a dma-buf of at least `need` bytes.
fn plane_holds(fd: BorrowedFd<'_>, need: u64) -> Result<(), PlaneFault> {
    let kind = rustix::fs::fstatfs(fd).map_err(|_| PlaneFault::Unreadable)?.f_type;
    if i128::from(kind) != DMA_BUF_MAGIC {
        return Err(PlaneFault::NotDmabuf);
    }
    let have = rustix::fs::fstat(fd).map_err(|_| PlaneFault::Unreadable)?.st_size;
    let have = u64::try_from(have).unwrap_or(0);
    if have < need {
        return Err(PlaneFault::TooSmall { need, have });
    }
    Ok(())
}

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
    /// logged, so a caller that skips the layer need not say it again. A buffer with a plane the
    /// driver cannot map ([`unmappable`]) never reaches the driver.
    pub(crate) fn dmabuf_texture(&mut self, dmabuf: &Dmabuf) -> Option<GlesTexture> {
        if let Some((plane, fault)) = unmappable(dmabuf) {
            log::warn!(
                area = "render",
                plane = plane,
                %fault,
                format = ?dmabuf.format(),
                "a client dmabuf is refused before the driver sees it"
            );
            return None;
        }
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
