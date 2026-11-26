//! The GBM scan-out buffers of one output and the [`Scanout`] adapter that draws into them.
//!
//! An output owns two buffers, allocated on the device's GBM node with the plane's XRGB8888
//! modifiers. Each buffer is exported as a dmabuf and registered as a KMS framebuffer. [`crate::frame::present`]
//! chooses the slot; this module draws into it and queues its flip. Two renderers draw into a slot:
//!
//! - **GLES.** The buffer is imported into the output's GLES renderer, which composes each frame into its own
//!   persistent target and copies that over the buffer with the rows reversed, because a dmabuf's first memory row is
//!   the top scan line ([`Gles::draw_scanout`]).
//! - **Vulkan.** The buffer is allocated LINEAR and the Vulkan renderer composes into its persistent target; the
//!   copy over the buffer is the renderer's mapped LINEAR write, of only what the slot missed ([`crate::vulkan`]).
//!   The target is the composed frame before the copy, so it can be read back: the Vulkan output answers
//!   [`Buffers::capture`], which the GLES output, whose frame is only ever in the buffer, does not.
//!
//! # Supersampling
//!
//! An output composed at `mode * s` keeps one persistent virtual target in its renderer, `composed` pixels across,
//! beside the scan-out buffers, which stay at the mode: a frame's damage is drawn into the virtual target at the
//! compositing scale, and the whole target is then averaged down at the mode and copied into the buffer being flipped
//! ([`Gles::draw_scanout_downsampled`], and `downsampled` on the Vulkan renderer). The scan-out buffer is the only
//! thing the kernel sees, so a modeset, a flip and the plane's format checks are the ones of an output with no
//! supersampling. Client buffers are never scanned out directly on this backend (there is no overlay or primary-plane
//! path for them: every frame is composed), so there is no direct scan-out to turn off while a factor is set.

use gnoblin_types::{Rect, Scale, Size};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::exporter::ExportBuffer;
use smithay::backend::drm::exporter::ExportFramebuffer;
use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
use smithay::backend::egl::{EGLDevice, EGLDisplay};
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Physical, Size as PhysicalSize};

use crate::frame::Scanout;
use crate::gpu::Gpu;
use crate::modeset::{COMMITTED_SLOT, ModesetKernel, RealModeset};
use crate::present::SLOTS;
use crate::vulkan::VulkanScan;
use gnoblin_backend::render::renderer::{AnyRenderer, GpuShare, RendererKind};
use gnoblin_backend::{DmabufCopy, DmabufCopyError, DmabufImportError, DmabufSupport};
use gnoblin_output::DisplayMode;
use gnoblin_output::supersample::MAX_VIRTUAL_EDGE;
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::footprint::{COLOUR_BYTES_PER_PIXEL, Footprint, image_bytes};
use gnoblin_render_frame::frame::{OutputFrame, SecureTreatment};
use gnoblin_render_frame::image::Image;
use gnoblin_render_gles::egl;
use gnoblin_render_gles::{Gles, ScanBuffer};
use gnoblin_types::OutputId;

use crate::log;

/// What the device's renderer is: the EGL display GLES draws on, or the Vulkan device the outputs share, with the
/// 1x1 renderer that answers what a client asks of the GPU before any output draws.
enum Api {
    Gles {
        display: EGLDisplay,
        /// A 1x1 context on the display, made on the first import of a client's GPU buffer, that decides whether the
        /// driver accepts one before the client may use it.
        probe: Option<Box<Gles>>,
    },
    Vulkan {
        /// The device every output's renderer is made on, so a window on two outputs is uploaded once and a lost
        /// device is lost once.
        share: GpuShare,
        probe: Option<AnyRenderer>,
    },
}

/// The GBM device and the renderer's device every output's buffers are allocated on.
pub(crate) struct Device {
    gbm: GbmDevice<DrmDeviceFd>,
    drm: DrmDeviceFd,
    api: Api,
    /// What clients were offered, so each import checks a buffer against it.
    support: Option<DmabufSupport>,
}

impl Device {
    /// The GBM device on the DRM node `drm`, and the renderer's device `kind` names.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when GBM cannot be brought up on the node, or EGL cannot for GLES; the software
    /// renderer cannot scan out, so it is always refused.
    pub(crate) fn open(drm: DrmDeviceFd, kind: RendererKind) -> Result<Self, RenderError> {
        let gbm = GbmDevice::new(drm.clone()).map_err(|error| {
            log::warn!(area = "render", %error, "no GBM device on the DRM node");
            RenderError::Unavailable
        })?;
        let api = match kind {
            RendererKind::Gles => Api::Gles {
                display: egl::gbm_display(&gbm)?,
                probe: None,
            },
            RendererKind::Vulkan => Api::Vulkan {
                // The card the session drives names the Vulkan device that draws for it. Without a device on that node
                // the share takes the best one: the buffers are written through the CPU, so any device can.
                share: GpuShare::on_node(drm.dev_id().ok()),
                probe: None,
            },
            RendererKind::Software => {
                log::warn!(
                    area = "render",
                    "the software renderer cannot scan out a hardware output"
                );
                return Err(RenderError::Unavailable);
            }
        };
        Ok(Self {
            gbm,
            drm,
            api,
            support: None,
        })
    }

    /// Which renderer the device is for.
    pub(crate) fn kind(&self) -> RendererKind {
        match self.api {
            Api::Gles { .. } => RendererKind::Gles,
            Api::Vulkan { .. } => RendererKind::Vulkan,
        }
    }

    /// What the device imports of a client's GPU buffers: the pairs it can sample as a 2D texture, and the render node
    /// it sits on. `None` when it imports none.
    pub(crate) fn dmabuf_support(&mut self) -> Option<DmabufSupport> {
        let support = match &mut self.api {
            Api::Gles { display, .. } => {
                let formats = display.dmabuf_render_formats();
                if formats.iter().next().is_none() {
                    log::info!(area = "render", "the GPU's EGL display imports no dmabuf format");
                    return None;
                }
                let node = EGLDevice::device_for_display(display)
                    .ok()
                    .and_then(|device| device.try_get_render_node().ok().flatten())
                    .map(|node| node.dev_id());
                DmabufSupport::new(node, formats.iter().copied())
            }
            Api::Vulkan { .. } => self.vulkan_probe().ok()?.dmabuf_support()?,
        };
        self.support = Some(support.clone());
        Some(support)
    }

    /// The GLES probe context, made on first use.
    fn gles_probe(&mut self) -> Result<&mut Gles, RenderError> {
        let Api::Gles { display, probe } = &mut self.api else {
            return Err(RenderError::Unsupported);
        };
        if probe.is_none() {
            *probe = Some(Box::new(Gles::on_display(display, Size::new(1, 1))?));
        }
        probe.as_deref_mut().ok_or(RenderError::Unavailable)
    }

    /// The Vulkan probe renderer, made on first use on the shared device.
    fn vulkan_probe(&mut self) -> Result<&mut AnyRenderer, RenderError> {
        let Api::Vulkan { share, probe } = &mut self.api else {
            return Err(RenderError::Unsupported);
        };
        if probe.is_none() {
            *probe = Some(AnyRenderer::new_sharing(RendererKind::Vulkan, Size::new(1, 1), share)?);
        }
        probe.as_mut().ok_or(RenderError::Unavailable)
    }

    /// The largest edge a virtual surface of an output can have: the driver's texture limit within the compositor's
    /// own. A device whose renderer cannot be made answers the compositor's limit and lets the allocation say why it
    /// cannot go on.
    pub(crate) fn max_composed_edge(&mut self) -> u32 {
        match self.api {
            Api::Gles { .. } => self
                .gles_probe()
                .map_or(MAX_VIRTUAL_EDGE, |probe| probe.max_texture_size().min(MAX_VIRTUAL_EDGE)),
            Api::Vulkan { .. } => self
                .vulkan_probe()
                .map_or(MAX_VIRTUAL_EDGE, |probe| probe.max_texture_size().min(MAX_VIRTUAL_EDGE)),
        }
    }

    /// Imports a client's GPU buffer on the device's own renderer, after the checks that need no driver.
    pub(crate) fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        self.support
            .as_ref()
            .ok_or(DmabufImportError::Unsupported)?
            .check(dmabuf)?;
        match self.api {
            Api::Gles { .. } => self
                .gles_probe()
                .and_then(|probe| probe.import_dmabuf(dmabuf))
                .map_err(DmabufImportError::Refused),
            Api::Vulkan { .. } => self
                .vulkan_probe()
                .map_err(DmabufImportError::Refused)?
                .import_dmabuf(dmabuf),
        }
    }

    /// Reads a client's GPU buffer back as a picture, on the probe, so a View of a window that shows it has pixels. The
    /// buffer must be one [`Device::dmabuf_support`] offered.
    pub(crate) fn read_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<Image, DmabufCopyError> {
        self.support
            .as_ref()
            .ok_or(DmabufCopyError::Unsupported)?
            .check(dmabuf)?;
        match self.api {
            Api::Gles { .. } => Ok(self.gles_probe()?.read_dmabuf(dmabuf)?),
            Api::Vulkan { .. } => Ok(self.vulkan_probe()?.read_dmabuf(dmabuf)?),
        }
    }

    /// Reads a View's picture back, on the probe: the scene is drawn there when the probe holds no texture of its
    /// current revision, so a View that no output shows is still read. The outputs' own renderers draw the View they
    /// show from the scene their frame carries, and none of them is read back.
    pub(crate) fn read_view(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, RenderError> {
        match self.api {
            Api::Gles { .. } => self.gles_probe()?.read_view(scene, capture, live),
            Api::Vulkan { .. } => self.vulkan_probe()?.view_pixels(scene, capture, live),
        }
    }

    /// Gives back every View texture the probe holds. The outputs' own renderers give theirs back when they draw a
    /// frame that names no View, which is the cover a locked output shows.
    pub(crate) fn release_views(&mut self) {
        match &mut self.api {
            Api::Gles { probe, .. } => {
                if let Some(probe) = probe {
                    probe.retire_views(&[]);
                }
            }
            Api::Vulkan { probe, .. } => {
                if let Some(probe) = probe {
                    probe.release_views();
                }
            }
        }
    }

    /// What the probe holds: for GLES the textures of the Views it read back and its 1x1 target. The probe serves every
    /// output, so it belongs to none; [`crate::Drm`] counts it once. The Vulkan probe's Views and client textures are the
    /// shared device's, which every output's own footprint already counts, so it adds nothing.
    pub(crate) fn footprint(&self) -> Footprint {
        match &self.api {
            Api::Gles { probe, .. } => probe.as_ref().map(|probe| probe.footprint()).unwrap_or_default(),
            Api::Vulkan { .. } => Footprint::default(),
        }
    }

    /// Gives back what the probe holds only between uses.
    pub(crate) fn release_scratch(&mut self) {
        if let Api::Gles { probe: Some(probe), .. } = &mut self.api {
            probe.release_scratch();
        }
    }

    /// Writes a picture the CPU holds into a client's GPU buffer, on the probe.
    ///
    /// The frame an output last drew is not copied here: it is read from the output's own renderer, which only a Vulkan
    /// output keeps readable (see [`Buffers::copy_frame_into_dmabuf`]).
    pub(crate) fn copy_picture(
        &mut self,
        dmabuf: &Dmabuf,
        picture: &Image,
        copy: DmabufCopy,
    ) -> Result<(), DmabufCopyError> {
        self.support
            .as_ref()
            .ok_or(DmabufCopyError::Unsupported)?
            .check(dmabuf)?;
        match self.api {
            Api::Gles { .. } => {
                Ok(self
                    .gles_probe()?
                    .copy_image_into_dmabuf(dmabuf, picture, copy.region, copy.from)?)
            }
            Api::Vulkan { .. } => Ok(self.vulkan_probe()?.copy_image_into_dmabuf(dmabuf, picture, copy)?),
        }
    }

    /// Checks a client's capture buffer against what the device offered, before a frame is copied into it.
    pub(crate) fn check_capture(&self, dmabuf: &Dmabuf) -> Result<(), DmabufCopyError> {
        self.support
            .as_ref()
            .ok_or(DmabufCopyError::Unsupported)?
            .check(dmabuf)?;
        Ok(())
    }

    /// Forgets the probe renderer of a device that was lost, so the next question builds one on the new device.
    pub(crate) fn forget_lost_probe(&mut self) {
        if let Api::Vulkan { probe, .. } = &mut self.api {
            *probe = None;
        }
    }
}

/// What a scan-out buffer is drawn through.
enum SlotTarget {
    /// The buffer as a GL render target.
    Gles(ScanBuffer),
    /// The buffer as a LINEAR dmabuf the Vulkan renderer writes through a mapping.
    Vulkan(Dmabuf),
}

/// One scan-out buffer: the allocation, what its renderer draws it through and its KMS framebuffer.
struct ScanSlot {
    /// Owns the allocation the dmabuf, the target and the framebuffer all refer to.
    _buffer: GbmBuffer,
    target: SlotTarget,
    framebuffer: <GbmFramebufferExporter<DrmDeviceFd> as ExportFramebuffer<GbmBuffer>>::Framebuffer,
}

/// Slots of one output, allocated on first use and reused after that.
///
/// The policy is generic so it can be tested on fakes: a slot is allocated by a closure only when
/// it is first used, and a failed allocation leaves the slot empty so the next attempt retries.
pub(crate) struct SlotRing<T> {
    slots: Vec<Option<T>>,
}

impl<T> SlotRing<T> {
    /// `count` empty slots.
    pub(crate) fn new(count: usize) -> Self {
        Self {
            slots: (0..count).map(|_| None).collect(),
        }
    }

    /// Makes slot `index` hold a value, calling `allocate` only when the slot is empty.
    ///
    /// # Errors
    ///
    /// The index is outside the ring, or `allocate` failed; the slot stays empty in both cases.
    pub(crate) fn ensure_with(
        &mut self,
        index: usize,
        allocate: impl FnOnce() -> Result<T, String>,
    ) -> Result<(), String> {
        let entry = self
            .slots
            .get_mut(index)
            .ok_or_else(|| format!("scan-out slot {index} does not exist"))?;
        if entry.is_none() {
            *entry = Some(allocate()?);
        }
        Ok(())
    }

    /// How many slots hold an allocation.
    pub(crate) fn allocated(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// The value in slot `index`, which must have been allocated.
    pub(crate) fn get(&self, index: usize) -> Result<&T, String> {
        self.slots
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| format!("scan-out slot {index} is empty"))
    }

    /// The value in slot `index` for drawing, which must have been allocated.
    pub(crate) fn get_mut(&mut self, index: usize) -> Result<&mut T, String> {
        self.slots
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("scan-out slot {index} is empty"))
    }
}

/// The renderer of one output.
enum ScanRenderer {
    Gles(Box<Gles>),
    Vulkan(Box<VulkanScan>),
}

/// The [`SLOTS`] scan-out buffers of one output, created on first use.
pub(crate) struct Buffers {
    renderer: ScanRenderer,
    size: PhysicalSize<i32, Physical>,
    /// The size of the virtual surface the output is composed on, which is `size` when there is no supersampling.
    composed: Size,
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    drm: DrmDeviceFd,
    modifiers: Vec<Modifier>,
    slots: SlotRing<ScanSlot>,
    /// What the last draw wrote into its slot, in the buffer's pixels, which the flip hands the plane as damage clips.
    /// Empty for GLES, which writes whole buffers and names no damage, and for a frame that changed nothing the slot
    /// missed.
    written: Vec<Rect>,
}

impl Buffers {
    /// Buffers for an output of `size` physical pixels, composed on a virtual surface of `composed` pixels, scanned out
    /// in a format of `formats`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the plane offers no XRGB8888 format the renderer draws into (Vulkan writes only
    /// LINEAR ones), or when the renderer cannot be made on the device, which includes a virtual surface past its
    /// texture limit.
    pub(crate) fn new(
        device: &mut Device,
        size: Size,
        composed: Size,
        formats: &FormatSet,
    ) -> Result<Self, RenderError> {
        let vulkan = device.kind() == RendererKind::Vulkan;
        let modifiers: Vec<Modifier> = formats
            .iter()
            .filter(|format| format.code == Fourcc::Xrgb8888)
            // The Vulkan renderer writes the buffer through a mapping, which is the LINEAR layout.
            .filter(|format| !vulkan || format.modifier == Modifier::Linear)
            .map(|format| format.modifier)
            .collect();
        if modifiers.is_empty() {
            if vulkan {
                log::warn!(
                    area = "drm",
                    "the plane scans out no LINEAR XRGB8888 buffer, which the Vulkan renderer writes"
                );
            } else {
                log::warn!(area = "drm", "the plane scans out no XRGB8888 format");
            }
            return Err(RenderError::Unavailable);
        }
        let degenerate = || RenderError::Degenerate {
            width: size.width,
            height: size.height,
        };
        let physical = PhysicalSize::from((
            i32::try_from(size.width).map_err(|_| degenerate())?,
            i32::try_from(size.height).map_err(|_| degenerate())?,
        ));
        let renderer = match &mut device.api {
            Api::Gles { display, .. } => ScanRenderer::Gles(Box::new(Gles::on_display(display, composed)?)),
            Api::Vulkan { share, .. } => ScanRenderer::Vulkan(Box::new(VulkanScan::new(share, composed, size, SLOTS)?)),
        };
        Ok(Self {
            renderer,
            composed,
            size: physical,
            allocator: GbmAllocator::new(device.gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
            exporter: GbmFramebufferExporter::new(device.gbm.clone(), NodeFilter::None),
            drm: device.drm.clone(),
            modifiers,
            slots: SlotRing::new(SLOTS),
            written: Vec::new(),
        })
    }

    /// Whether these buffers were allocated for an output of `size` physical pixels composed on `composed`.
    #[must_use]
    pub(crate) fn fits(&self, size: Size, composed: Size) -> bool {
        size_fits((self.size.w, self.size.h), size) && self.composed == composed
    }

    /// The size of the buffers, which is what the display shows.
    fn shown(&self) -> Size {
        Size::new(self.size.w.unsigned_abs(), self.size.h.unsigned_abs())
    }

    /// Whether the output is composed on a surface other than the scan-out buffer, so every frame is averaged down.
    #[must_use]
    fn averaged(&self) -> bool {
        !size_fits((self.size.w, self.size.h), self.composed)
    }

    /// Gives the output a virtual surface of `composed` pixels, keeping the scan-out buffers.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the renderer cannot make the surface; the one it had is kept.
    pub(crate) fn recompose(&mut self, composed: Size) -> Result<(), RenderError> {
        match &mut self.renderer {
            ScanRenderer::Gles(gles) => gles.resize(composed)?,
            ScanRenderer::Vulkan(vulkan) => vulkan.resize(composed)?,
        }
        self.composed = composed;
        Ok(())
    }

    /// What this output holds: its renderer's footprint, and the scan-out buffers allocated so far, which the renderer
    /// does not count (it only imports them as render targets, or writes them through a mapping). The buffers are the
    /// frame the kernel scans out, so they are target bytes: the budget never takes them back.
    #[must_use]
    pub(crate) fn footprint(&self) -> Footprint {
        let scan_out = Footprint {
            target: scanout_bytes((self.size.w, self.size.h), self.slots.allocated()),
            ..Footprint::default()
        };
        match &self.renderer {
            ScanRenderer::Gles(gles) => gles.footprint(),
            ScanRenderer::Vulkan(vulkan) => vulkan.footprint(),
        }
        .plus(scan_out)
    }

    /// Gives back the renderer's blur scratch, which the next frame that blurs makes again.
    pub(crate) fn release_scratch(&mut self) {
        match &mut self.renderer {
            ScanRenderer::Gles(gles) => gles.release_scratch(),
            ScanRenderer::Vulkan(vulkan) => vulkan.renderer().release_scratch(),
        }
    }

    /// Gives back the textures of every View this output draws; the next frame that names a View draws it again.
    pub(crate) fn release_views(&mut self) {
        match &mut self.renderer {
            ScanRenderer::Gles(gles) => gles.retire_views(&[]),
            ScanRenderer::Vulkan(vulkan) => vulkan.renderer().release_views(),
        }
    }

    /// Whether the output's renderer lost its device: it draws nothing until [`Buffers::rebuild`].
    #[must_use]
    pub(crate) fn is_lost(&self) -> bool {
        matches!(&self.renderer, ScanRenderer::Vulkan(vulkan) if vulkan.is_lost())
    }

    /// Gives the output a renderer on a new device after a loss. The scan-out buffers and their framebuffers stay,
    /// because the one the CRTC shows must not be taken from it: the screen keeps the last frame while the renderer
    /// comes back, and the host draws the output whole afterwards.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when no renderer can be made yet; the lost one is kept and the caller tries again later.
    pub(crate) fn rebuild(&mut self, device: &mut Device) -> Result<(), RenderError> {
        device.forget_lost_probe();
        let (ScanRenderer::Vulkan(vulkan), Api::Vulkan { share, .. }) = (&mut self.renderer, &mut device.api) else {
            return Err(RenderError::Unsupported);
        };
        vulkan.rebuild(share, self.composed)
    }

    /// Reads the output's frame back, with the cursor composed in when `with_cursor` and each secure layer replaced as
    /// `treatment` says. The extent is physical: the composed surface, which a supersampled output holds at its
    /// virtual size.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a GLES output, whose frame is only ever in the buffer the kernel shows, and the
    /// renderer's own error otherwise.
    pub(crate) fn capture(
        &mut self,
        with_cursor: bool,
        scale: Scale,
        treatment: SecureTreatment,
    ) -> Result<Image, RenderError> {
        let bounds = self.logical_bounds(scale);
        match &mut self.renderer {
            ScanRenderer::Gles(_) => Err(RenderError::Unsupported),
            ScanRenderer::Vulkan(vulkan) => vulkan.capture(with_cursor, bounds, scale, treatment),
        }
    }

    /// Writes the frame the output last drew into a client's buffer, with the cursor composed in when `with_cursor`
    /// and each secure layer replaced as `treatment` says.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a GLES output, and the renderer's own error otherwise.
    pub(crate) fn copy_frame_into_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        copy: DmabufCopy,
        with_cursor: bool,
        scale: Scale,
        treatment: SecureTreatment,
    ) -> Result<(), RenderError> {
        let bounds = self.logical_bounds(scale);
        match &mut self.renderer {
            ScanRenderer::Gles(_) => Err(RenderError::Unsupported),
            ScanRenderer::Vulkan(vulkan) => {
                vulkan.copy_frame_into_dmabuf(dmabuf, copy, with_cursor, bounds, scale, treatment)
            }
        }
    }

    /// What the output's frame covers in its own (logical) space, which a capture redacts secure layers within: the
    /// composed surface over the compositing scale.
    fn logical_bounds(&self, scale: Scale) -> Rect {
        let ratio = scale.as_f64();
        let across = |pixels: u32| -> u32 {
            if ratio > 0.0 {
                (f64::from(pixels) / ratio).ceil() as u32
            } else {
                pixels
            }
        };
        Rect::new(0, 0, across(self.composed.width), across(self.composed.height))
    }

    /// Draws `frame` into the scan-out buffer of `slot`: straight into it when the output is not supersampled, and
    /// otherwise into the virtual surface, averaged down into the buffer.
    fn draw(&mut self, slot: usize, frame: &OutputFrame, scale: Scale) -> Result<(), String> {
        self.ensure(slot)?;
        let averaged = self.averaged();
        let (shown, composed) = (self.shown(), self.composed);
        let Buffers {
            renderer,
            slots,
            written,
            ..
        } = self;
        let entry = slots.get_mut(slot)?;
        match (renderer, &mut entry.target) {
            (ScanRenderer::Gles(gles), SlotTarget::Gles(scan)) => {
                written.clear();
                if averaged {
                    gles.draw_scanout_downsampled(scan, frame, scale)
                } else {
                    gles.draw_scanout(scan, frame, scale)
                }
                .map_err(|error| {
                    if error == RenderError::Lost {
                        // The frame that was in flight when the device went held the shaders that may have hung it: the host strikes them.
                        frame.shader_timings().record_loss();
                    }
                    format!("the frame cannot be drawn: {error}")
                })
            }
            (ScanRenderer::Vulkan(vulkan), SlotTarget::Vulkan(dmabuf)) => vulkan
                .compose(frame, scale, composed, shown)
                .and_then(|()| {
                    *written = vulkan.write_slot(slot, dmabuf, composed, shown)?;
                    Ok(())
                })
                .map_err(|error| format!("the frame cannot be drawn: {error}")),
            _ => Err("the scan-out buffer belongs to another renderer".to_owned()),
        }
    }

    /// Allocates slot `index` if it does not exist yet.
    fn ensure(&mut self, index: usize) -> Result<(), String> {
        let Buffers {
            renderer,
            size,
            allocator,
            exporter,
            drm,
            modifiers,
            slots,
            ..
        } = self;
        slots.ensure_with(index, || allocate(renderer, allocator, exporter, drm, *size, modifiers))
    }
}

/// Whether buffers of `allocated` physical pixels suit an output of `wanted` pixels.
#[must_use]
fn size_fits(allocated: (i32, i32), wanted: Size) -> bool {
    u32::try_from(allocated.0) == Ok(wanted.width) && u32::try_from(allocated.1) == Ok(wanted.height)
}

/// The nominal bytes of `slots` scan-out buffers of `allocated` physical pixels each. The format is XRGB8888, four
/// bytes a pixel; the allocator may pad a row or tile the buffer, so this is a floor on what the driver holds, as every
/// footprint is.
#[must_use]
fn scanout_bytes(allocated: (i32, i32), slots: usize) -> u64 {
    let edge = Size::new(allocated.0.unsigned_abs(), allocated.1.unsigned_abs());
    image_bytes(edge, COLOUR_BYTES_PER_PIXEL).saturating_mul(u64::try_from(slots).unwrap_or(u64::MAX))
}

/// Allocates one scan-out buffer of `size` and brings it up as a render target of `renderer` and a KMS framebuffer.
///
/// # Errors
///
/// The buffer, its export, its import by the renderer or its framebuffer cannot be made; the message says which.
fn allocate(
    renderer: &mut ScanRenderer,
    allocator: &mut GbmAllocator<DrmDeviceFd>,
    exporter: &GbmFramebufferExporter<DrmDeviceFd>,
    drm: &DrmDeviceFd,
    size: PhysicalSize<i32, Physical>,
    modifiers: &[Modifier],
) -> Result<ScanSlot, String> {
    let buffer = allocator
        .create_buffer(
            size.w.unsigned_abs(),
            size.h.unsigned_abs(),
            Fourcc::Xrgb8888,
            modifiers,
        )
        .map_err(|error| format!("the scan-out buffer cannot be allocated: {error}"))?;
    let dmabuf = buffer
        .export()
        .map_err(|error| format!("the scan-out buffer cannot be exported: {error}"))?;
    let target = match renderer {
        ScanRenderer::Gles(gles) => SlotTarget::Gles(
            gles.import_scanout(&dmabuf)
                .map_err(|error| format!("the scan-out buffer cannot be imported: {error}"))?,
        ),
        ScanRenderer::Vulkan(vulkan) => {
            // A buffer the renderer cannot write is refused here, with its format, and not at the first flip.
            vulkan
                .renderer()
                .import_dmabuf(&dmabuf)
                .map_err(|error| format!("the Vulkan renderer cannot write the scan-out buffer: {error:?}"))?;
            SlotTarget::Vulkan(dmabuf)
        }
    };
    let framebuffer = exporter
        .add_framebuffer(drm, ExportBuffer::Allocator(&buffer), false)
        .map_err(|error| format!("the scan-out buffer has no KMS framebuffer: {error}"))?
        .ok_or_else(|| "the scan-out buffer has no KMS framebuffer".to_string())?;
    Ok(ScanSlot {
        _buffer: buffer,
        target,
        framebuffer,
    })
}

/// The scan-out of one output for one [`crate::frame::present`] call: draws into the
/// buffers and queues flips on the CRTC.
///
/// Built fresh for each present, so it borrows the output's buffers and the GPU only for that
/// call. Nothing here outlives the frame.
pub(crate) struct Flip<'a> {
    /// The two slots of this output, borrowed mutably because drawing into one changes its state.
    pub(crate) buffers: &'a mut Buffers,
    /// The device the buffers were allocated on, used to queue the flip.
    pub(crate) gpu: &'a Gpu,
    /// The CRTC the output is driven on; the flip is queued against it.
    pub(crate) crtc: crtc::Handle,
    /// The output's compositing scale, applied when the frame is drawn: the scale times the supersample factor.
    pub(crate) scale: Scale,
}

impl Scanout for Flip<'_> {
    type Frame = OutputFrame;

    fn draw(&mut self, slot: usize, frame: &OutputFrame) -> Result<(), String> {
        self.buffers.draw(slot, frame, self.scale)
    }

    fn flip(&mut self, slot: usize) -> Result<(), String> {
        let size = self.buffers.size;
        let framebuffer = *self.buffers.slots.get(slot)?.framebuffer.as_ref();
        self.gpu.queue_flip(self.crtc, framebuffer, size, &self.buffers.written)
    }
}

/// The kernel and the allocator behind one output's mode change, for [`crate::modeset::configure`].
///
/// Built fresh for each call, so it borrows the GPU and the buffer device only for that call.
pub(crate) struct LiveModeset<'a> {
    /// The GPU whose surface is committed.
    pub(crate) gpu: &'a Gpu,
    /// The device the new buffers are allocated on, borrowed mutably because its probe renderer answers the texture limit.
    pub(crate) device: &'a mut Device,
    /// The output whose mode changes.
    pub(crate) output: OutputId,
    /// The CRTC that scans the output out, which names the plane formats the buffers must suit.
    pub(crate) crtc: crtc::Handle,
    /// What the process opted in to: a mode change on this kernel is refused unless it is on.
    pub(crate) real_modeset: RealModeset,
}

impl ModesetKernel for LiveModeset<'_> {
    type Buffers = Buffers;

    fn real_modeset(&self) -> RealModeset {
        self.real_modeset
    }

    fn allocate(&mut self, shown: Size, composed: Size) -> Result<Buffers, RenderError> {
        let formats = self.gpu.plane_formats(self.crtc).ok_or(RenderError::Unavailable)?;
        let mut buffers = Buffers::new(self.device, shown, composed, &formats)?;
        buffers.ensure(COMMITTED_SLOT).map_err(|error| {
            log::warn!(area = "drm", output = %self.output, %error, "the buffer for the new mode could not be made");
            RenderError::Unavailable
        })?;
        // The memory a buffer is made from is not defined. An empty frame is opaque black, so the first thing on the
        // new mode is a black screen and never what the memory held before.
        buffers
            .draw(COMMITTED_SLOT, &OutputFrame::new(), Scale::integral(1))
            .map_err(|error| {
                log::warn!(area = "drm", output = %self.output, %error, "the buffer for the new mode could not be cleared");
                RenderError::Unavailable
            })?;
        Ok(buffers)
    }

    fn recompose(&mut self, buffers: &mut Buffers, composed: Size) -> Result<(), RenderError> {
        buffers.recompose(composed)
    }

    fn max_composed_edge(&mut self) -> u32 {
        self.device.max_composed_edge()
    }

    fn test(&mut self, mode: DisplayMode, buffers: &Buffers) -> Result<(), String> {
        let framebuffer = *buffers.slots.get(COMMITTED_SLOT)?.framebuffer.as_ref();
        self.gpu.test_mode(self.output, mode, framebuffer, buffers.size)
    }

    fn commit(&mut self, mode: DisplayMode, buffers: &Buffers) -> Result<(), String> {
        let framebuffer = *buffers.slots.get(COMMITTED_SLOT)?.framebuffer.as_ref();
        self.gpu.commit_mode(self.output, mode, framebuffer, buffers.size)
    }

    fn unstage(&mut self) -> Result<(), String> {
        self.gpu.unstage_mode(self.output)
    }

    fn reapply_gamma(&mut self) -> Result<(), String> {
        self.gpu.reapply_gamma(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::{SlotRing, scanout_bytes, size_fits};
    use gnoblin_types::Size;
    use std::cell::Cell;

    #[test]
    fn a_slot_is_allocated_on_first_use_and_reused_after() {
        let mut slots: SlotRing<usize> = SlotRing::new(2);
        let calls = Cell::new(0);
        assert!(slots.get(0).is_err(), "nothing is allocated before the first draw");
        slots
            .ensure_with(0, || {
                calls.set(calls.get() + 1);
                Ok(10)
            })
            .unwrap();
        slots
            .ensure_with(0, || {
                calls.set(calls.get() + 1);
                Ok(99)
            })
            .unwrap();
        assert_eq!(calls.get(), 1, "a second ensure of the same slot reuses the allocation");
        assert_eq!(slots.get(0), Ok(&10));
        assert!(slots.get(1).is_err(), "slot 1 stays unallocated until it is used");
    }

    #[test]
    fn the_two_slots_alternate_without_reallocating() {
        let mut slots: SlotRing<usize> = SlotRing::new(2);
        let calls = Cell::new(0);
        for index in [0, 1, 0, 1, 0] {
            slots
                .ensure_with(index, || {
                    calls.set(calls.get() + 1);
                    Ok(index)
                })
                .unwrap();
        }
        assert_eq!(calls.get(), 2, "each of the two slots is allocated once");
        assert_eq!(slots.get(1), Ok(&1));
    }

    #[test]
    fn a_slot_outside_the_ring_is_refused_without_allocating() {
        let mut slots: SlotRing<usize> = SlotRing::new(2);
        let calls = Cell::new(0);
        let error = slots
            .ensure_with(2, || {
                calls.set(calls.get() + 1);
                Ok(0)
            })
            .unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn a_failed_allocation_surfaces_and_the_next_draw_retries() {
        let mut slots: SlotRing<usize> = SlotRing::new(2);
        let error = slots
            .ensure_with(0, || Err("the scan-out buffer cannot be allocated".to_string()))
            .unwrap_err();
        assert!(error.contains("cannot be allocated"), "{error}");
        assert!(slots.get(0).is_err(), "a failed slot stays empty rather than half set");
        slots.ensure_with(0, || Ok(7)).unwrap();
        assert_eq!(slots.get(0), Ok(&7), "the output is not wedged: the retry allocates");
    }

    #[test]
    fn only_the_slots_that_were_allocated_are_counted() {
        let mut slots: SlotRing<usize> = SlotRing::new(2);
        assert_eq!(slots.allocated(), 0, "nothing is held before the first draw");
        slots
            .ensure_with(0, || Err("the scan-out buffer cannot be allocated".to_string()))
            .unwrap_err();
        assert_eq!(slots.allocated(), 0, "a slot whose allocation failed holds nothing");
        slots.ensure_with(1, || Ok(1)).unwrap();
        slots.ensure_with(1, || Ok(2)).unwrap();
        assert_eq!(
            slots.allocated(),
            1,
            "an ensure of a slot already made makes no second one"
        );
        slots.ensure_with(0, || Ok(0)).unwrap();
        assert_eq!(slots.allocated(), 2);
    }

    #[test]
    fn a_scan_out_buffer_is_four_bytes_a_pixel_for_each_slot_allocated() {
        // 1920 x 1080 at 4 bytes is 8_294_400 bytes (7.91 MiB): two slots are 16_588_800.
        assert_eq!(scanout_bytes((1920, 1080), 0), 0);
        assert_eq!(scanout_bytes((1920, 1080), 1), 8_294_400);
        assert_eq!(scanout_bytes((1920, 1080), 2), 16_588_800);
        assert_eq!(
            scanout_bytes((-1, 1080), 2),
            scanout_bytes((1, 1080), 2),
            "a negative extent is not a huge one"
        );
        assert_eq!(
            scanout_bytes((i32::MAX, i32::MAX), usize::MAX),
            u64::MAX,
            "a size past the word saturates"
        );
    }

    #[test]
    fn buffers_fit_only_an_output_of_the_same_physical_size() {
        let full = Size {
            width: 1920,
            height: 1080,
        };
        assert!(size_fits((1920, 1080), full));
        assert!(!size_fits((1280, 1080), full), "a width change reallocates");
        assert!(!size_fits((1920, 720), full), "a height change reallocates");
        assert!(!size_fits((-1, 1080), full), "a negative extent never fits");
    }
}
