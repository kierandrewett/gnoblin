//! The GPU-facing half of an offscreen target, shared by the headless and the nested backend.
//!
//! Both backends draw an output with an [`AnyRenderer`] into an offscreen buffer and read it back, so the dmabuf
//! import, the copy into a client's buffer, the View readback and the secure-layer redaction are the same code on
//! both. They live here once: a nested session that can draw with GLES answers the dmabuf questions the way a
//! headless one does, because it is the same function, and a backend cannot grow its own variant of a rule.
//!
//! [`GpuTarget`] is what a target states (its renderer, the frame last drawn, its extent and scale) and the trait's
//! provided methods are everything built on those. [`DmabufHub`] is the backend-wide half: what clients were
//! offered, kept so every later import or copy is checked against the same list, and the optional stand-in
//! importer a test uses where no device exists.

use smithay::backend::allocator::dmabuf::Dmabuf;

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{OutputFrame, SecureTreatment};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::view_scene::ViewScene;
use gnoblin_types::{Rect, Scale};

use crate::render::renderer::{AnyRenderer, CaptureRequest, capture_redacted};
use crate::{DmabufCopy, DmabufCopyError, DmabufImportError, DmabufProvider, DmabufSource, DmabufSupport};

/// The pieces of a target a GPU-facing call reads, borrowed together because a redacted read needs the renderer
/// mutably while it reads the frame last drawn.
pub(crate) struct TargetParts<'a> {
    /// The renderer that draws the output.
    pub renderer: &'a mut AnyRenderer,
    /// The frame last drawn, kept only when it has a secure layer.
    pub display: Option<&'a OutputFrame>,
    /// The output's whole extent in logical pixels.
    pub bounds: Rect,
    /// The scale the frame is composed at, supersampling included.
    pub scale: Scale,
}

/// An offscreen target: a renderer and the frame it last drew.
pub(crate) trait GpuTarget {
    /// The target's parts.
    fn parts(&mut self) -> TargetParts<'_>;

    /// Reads the last frame back, with each secure layer drawn as `treatment` says.
    ///
    /// # Errors
    ///
    /// The renderer's [`RenderError`].
    fn read_back(&mut self, with_cursor: bool, treatment: SecureTreatment) -> Result<Image, RenderError> {
        let TargetParts {
            renderer,
            display,
            bounds,
            scale,
        } = self.parts();
        capture_redacted(
            renderer,
            CaptureRequest {
                display,
                bounds,
                scale,
                with_cursor,
                treatment,
            },
            AnyRenderer::draw,
            AnyRenderer::capture,
        )
    }

    /// What this target's renderer imports of a client's GPU buffers; see [`AnyRenderer::dmabuf_support`].
    fn dmabuf_support(&mut self) -> Option<DmabufSupport> {
        self.parts().renderer.dmabuf_support()
    }

    /// Imports a client's GPU buffer into this target's renderer.
    ///
    /// # Errors
    ///
    /// [`DmabufImportError`] when the renderer imports none or the driver refuses the buffer.
    fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        self.parts().renderer.import_dmabuf(dmabuf)
    }

    /// Copies the frame last drawn into a client's GPU buffer, with each secure layer drawn as `treatment` says.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] when the renderer draws into no client buffer, and the renderer's own error
    /// otherwise.
    fn copy_frame_into_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        copy: DmabufCopy,
        with_cursor: bool,
        treatment: SecureTreatment,
    ) -> Result<(), RenderError> {
        let TargetParts {
            renderer,
            display,
            bounds,
            scale,
        } = self.parts();
        capture_redacted(
            renderer,
            CaptureRequest {
                display,
                bounds,
                scale,
                with_cursor,
                treatment,
            },
            AnyRenderer::draw,
            |renderer, cursor| renderer.copy_frame_into_dmabuf(dmabuf, copy, cursor),
        )
    }

    /// Writes a picture the CPU holds into a client's GPU buffer.
    ///
    /// # Errors
    ///
    /// As [`GpuTarget::copy_frame_into_dmabuf`].
    fn copy_picture_into_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        picture: &Image,
        copy: DmabufCopy,
    ) -> Result<(), RenderError> {
        self.parts().renderer.copy_image_into_dmabuf(dmabuf, picture, copy)
    }

    /// Reads a client's GPU buffer back as a picture.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a renderer that imports none.
    fn read_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<Image, RenderError> {
        self.parts().renderer.read_dmabuf(dmabuf)
    }

    /// Reads a View's picture back, drawing the scene first when the renderer holds none of its current revision.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a renderer that draws a View on the CPU.
    fn read_view(&mut self, scene: &ViewScene, capture: bool, live: &[u64]) -> Result<Image, RenderError> {
        self.parts().renderer.view_pixels(scene, capture, live)
    }

    /// Gives back every View texture this target's renderer holds.
    fn release_views(&mut self) {
        self.parts().renderer.release_views();
    }

    /// The largest virtual edge this target's renderer can hold, within the compositor's own limit.
    fn max_virtual_edge(&mut self) -> u32 {
        self.parts()
            .renderer
            .max_texture_size()
            .min(gnoblin_output::supersample::MAX_VIRTUAL_EDGE)
    }
}

/// Whether a renderer's answer names the device a client allocates its buffers on.
fn has_device(support: &DmabufSupport) -> bool {
    support.main_device.is_some()
}

/// What clients were offered, and the stand-in importer a test may install.
///
/// The offer is remembered so each import and each copy is checked against the formats clients were told, before
/// the driver is asked: a buffer outside the offer is refused up front.
#[derive(Default)]
pub(crate) struct DmabufHub {
    /// A stand-in for the renderers' answers; see `Headless::with_dmabuf_provider`.
    provider: Option<Box<dyn DmabufProvider>>,
    /// What [`Backend::dmabuf`](crate::Backend::dmabuf) reported.
    support: Option<DmabufSupport>,
    /// What the capture queries reported.
    capture: Option<DmabufSupport>,
}

impl DmabufHub {
    /// A hub whose answers come from `provider` instead of the renderers.
    pub fn with_provider(provider: Box<dyn DmabufProvider>) -> Self {
        Self {
            provider: Some(provider),
            ..Self::default()
        }
    }

    /// What the session can import of a client's GPU buffers: the provider's answer, or the first output's
    /// renderer's, when that renderer has a render node. `None` when nothing imports (the software renderer, or no
    /// output) and when the renderer has no device a client could allocate on (Mesa's llvmpipe on a surfaceless
    /// display): the protocol is not served there, so a client sends shared memory and no buffer of its own reaches
    /// a software rasteriser that cannot map it.
    pub fn query<T: GpuTarget>(&mut self, first: Option<&mut T>) -> Option<DmabufSupport> {
        let support = match self.provider.as_mut() {
            Some(provider) => provider.support(),
            None => first.and_then(GpuTarget::dmabuf_support).filter(has_device),
        };
        self.support.clone_from(&support);
        support
    }

    /// Imports `dmabuf` after the checks that need no driver: a format and modifier the clients were not offered,
    /// and a plane count the modifier cannot have.
    ///
    /// # Errors
    ///
    /// [`DmabufImportError`] naming why the buffer was refused.
    pub fn import<T: GpuTarget>(&mut self, first: Option<&mut T>, dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        self.support
            .as_ref()
            .ok_or(DmabufImportError::Unsupported)?
            .check(dmabuf)?;
        match self.provider.as_mut() {
            Some(provider) => provider.import(dmabuf),
            None => first.ok_or(DmabufImportError::Unsupported)?.import_dmabuf(dmabuf),
        }
    }

    /// What a capture may be written into: the provider's answer, or the first output's renderer's, when that
    /// renderer has a render node (see [`DmabufHub::query`]). Remembered, so each copy checks a buffer against the
    /// formats a capture client was told.
    pub fn query_capture<T: GpuTarget>(&mut self, first: Option<&mut T>) -> Option<DmabufSupport> {
        let support = match self.provider.as_mut() {
            Some(provider) => provider.capture_support(),
            None => first.and_then(GpuTarget::dmabuf_support).filter(has_device),
        };
        self.capture.clone_from(&support);
        support
    }

    /// Copies `source` into `dmabuf`, after the checks that need no driver. `target` is the target the source reads
    /// through: the one of the output a frame source names, the first one for a picture, or `None` when this backend
    /// has none, which a frame answers as [`DmabufCopyError::NoFrame`].
    ///
    /// # Errors
    ///
    /// [`DmabufCopyError`] naming why nothing was written.
    pub fn copy<T: GpuTarget>(
        &mut self,
        treatment: SecureTreatment,
        target: Option<&mut T>,
        source: DmabufSource<'_>,
        dmabuf: &Dmabuf,
        copy: DmabufCopy,
    ) -> Result<(), DmabufCopyError> {
        self.capture
            .as_ref()
            .ok_or(DmabufCopyError::Unsupported)?
            .check(dmabuf)?;
        match source {
            DmabufSource::Frame { with_cursor, .. } => {
                let target = target.ok_or(DmabufCopyError::NoFrame)?;
                match self.provider.as_mut() {
                    // The stand-in has no frame of its own: it is handed the one the output's renderer drew.
                    Some(provider) => {
                        let frame = target
                            .read_back(with_cursor, treatment)
                            .map_err(DmabufCopyError::from)?;
                        provider.write(dmabuf, &frame, copy)
                    }
                    None => target
                        .copy_frame_into_dmabuf(dmabuf, copy, with_cursor, treatment)
                        .map_err(DmabufCopyError::from),
                }
            }
            DmabufSource::Picture(picture) => match self.provider.as_mut() {
                Some(provider) => provider.write(dmabuf, picture, copy),
                None => target
                    .ok_or(DmabufCopyError::Unsupported)?
                    .copy_picture_into_dmabuf(dmabuf, picture, copy)
                    .map_err(DmabufCopyError::from),
            },
        }
    }

    /// Reads a client's GPU buffer back as a picture.
    ///
    /// # Errors
    ///
    /// [`DmabufCopyError`] naming why nothing was read.
    pub fn read<T: GpuTarget>(&mut self, first: Option<&mut T>, dmabuf: &Dmabuf) -> Result<Image, DmabufCopyError> {
        match self.provider.as_mut() {
            Some(provider) => provider.read(dmabuf),
            None => first
                .ok_or(DmabufCopyError::Unsupported)?
                .read_dmabuf(dmabuf)
                .map_err(DmabufCopyError::from),
        }
    }
}
