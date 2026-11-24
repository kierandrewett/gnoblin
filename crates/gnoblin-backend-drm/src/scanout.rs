//! The GBM scan-out buffers of one output and the [`Scanout`] adapter that draws into them.
//!
//! An output owns two buffers, allocated on the device's GBM node with the plane's XRGB8888
//! modifiers. Each buffer is exported as a dmabuf, imported into the output's GLES renderer, and
//! registered as a KMS framebuffer. The renderer composes each frame into its own persistent target
//! and copies that over the buffer with the rows reversed, because a dmabuf's first memory row is
//! the top scan line ([`Gles::draw_scanout`]). [`crate::frame::present`] chooses the slot; this
//! module draws into it and queues its flip.
//!
//! # Supersampling
//!
//! An output composed at `mode * s` keeps one persistent virtual target in its renderer, `composed` pixels across,
//! beside the scan-out buffers, which stay at the mode: a frame's damage is drawn into the virtual target at the
//! compositing scale, and the whole target is then averaged down at the mode and copied, rows reversed, into the buffer
//! being flipped ([`Gles::draw_scanout_downsampled`]). The scan-out buffer is the only thing the kernel sees, so a
//! modeset, a flip and the plane's format checks are the ones of an output with no supersampling. Client buffers are
//! never scanned out directly on this backend (there is no overlay or primary-plane path for them: every frame is
//! composed), so there is no direct scan-out to turn off while a factor is set.

use gnoblin_types::{Scale, Size};
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
use gnoblin_backend::{DmabufCopy, DmabufCopyError, DmabufImportError, DmabufSupport};
use gnoblin_output::DisplayMode;
use gnoblin_output::supersample::MAX_VIRTUAL_EDGE;
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::footprint::{COLOUR_BYTES_PER_PIXEL, Footprint, image_bytes};
use gnoblin_render_frame::frame::OutputFrame;
use gnoblin_render_frame::image::Image;
use gnoblin_render_gles::egl;
use gnoblin_render_gles::{Gles, ScanBuffer};
use gnoblin_types::OutputId;

use crate::log;

/// The GBM device and EGL display every output's buffers are allocated on.
pub(crate) struct Device {
    gbm: GbmDevice<DrmDeviceFd>,
    display: EGLDisplay,
    drm: DrmDeviceFd,
    /// A 1x1 context on the display, made on the first import of a client's GPU buffer, that decides
    /// whether the driver accepts one before the client may use it.
    probe: Option<Gles>,
    /// What clients were offered, so each import checks a buffer against it.
    support: Option<DmabufSupport>,
}

impl Device {
    /// The GBM device and EGL display on the DRM node `drm`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when GBM or EGL cannot be brought up on the node.
    pub(crate) fn open(drm: DrmDeviceFd) -> Result<Self, RenderError> {
        let gbm = GbmDevice::new(drm.clone()).map_err(|error| {
            log::warn!(area = "render", %error, "no GBM device on the DRM node");
            RenderError::Unavailable
        })?;
        let display = egl::gbm_display(&gbm)?;
        Ok(Self {
            gbm,
            display,
            drm,
            probe: None,
            support: None,
        })
    }

    /// What the display imports of a client's GPU buffers: the pairs it can sample as a 2D texture, and
    /// the render node it sits on. `None` when it has no `EGL_EXT_image_dma_buf_import`.
    pub(crate) fn dmabuf_support(&mut self) -> Option<DmabufSupport> {
        let formats = self.display.dmabuf_render_formats();
        if formats.iter().next().is_none() {
            log::info!(area = "render", "the GPU's EGL display imports no dmabuf format");
            return None;
        }
        let node = EGLDevice::device_for_display(&self.display)
            .ok()
            .and_then(|device| device.try_get_render_node().ok().flatten())
            .map(|node| node.dev_id());
        let support = DmabufSupport::new(node, formats.iter().copied());
        self.support = Some(support.clone());
        Some(support)
    }

    /// The probe context, made on first use.
    fn probe(&mut self) -> Result<&mut Gles, RenderError> {
        if self.probe.is_none() {
            self.probe = Some(Gles::on_display(&self.display, Size::new(1, 1))?);
        }
        self.probe.as_mut().ok_or(RenderError::Unavailable)
    }

    /// The largest edge a virtual surface of an output can have: the driver's texture limit within the compositor's
    /// own. A display whose context cannot be made answers the compositor's limit and lets the allocation say why it
    /// cannot go on.
    pub(crate) fn max_composed_edge(&mut self) -> u32 {
        self.probe()
            .map_or(MAX_VIRTUAL_EDGE, |probe| probe.max_texture_size().min(MAX_VIRTUAL_EDGE))
    }

    /// Imports a client's GPU buffer on the device's own display, after the checks that need no driver.
    pub(crate) fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        self.support
            .as_ref()
            .ok_or(DmabufImportError::Unsupported)?
            .check(dmabuf)?;
        self.probe()
            .and_then(|probe| probe.import_dmabuf(dmabuf))
            .map_err(DmabufImportError::Refused)
    }

    /// Reads a client's GPU buffer back as a picture, on the probe context, so a View of a window that shows it has
    /// pixels. The buffer must be one [`Device::dmabuf_support`] offered.
    pub(crate) fn read_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<Image, DmabufCopyError> {
        self.support
            .as_ref()
            .ok_or(DmabufCopyError::Unsupported)?
            .check(dmabuf)?;
        Ok(self.probe()?.read_dmabuf(dmabuf)?)
    }

    /// Reads a View's picture back, on the probe context: the scene is drawn there when the probe holds no texture of
    /// its current revision, so a View that no output shows is still read. The outputs' own contexts draw the View
    /// they show from the scene their frame carries, and none of them is read back.
    pub(crate) fn read_view(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, RenderError> {
        self.probe()?.read_view(scene, capture, live)
    }

    /// Gives back every View texture the probe context holds. The outputs' own contexts give theirs back when they
    /// draw a frame that names no View, which is the cover a locked output shows.
    pub(crate) fn release_views(&mut self) {
        if let Some(probe) = self.probe.as_mut() {
            probe.retire_views(&[]);
        }
    }

    /// What the probe context holds: the textures of the Views it read back and its 1x1 target. The probe serves every
    /// output, so it belongs to none; [`crate::Drm`] counts it once.
    pub(crate) fn footprint(&self) -> Footprint {
        self.probe.as_ref().map(Gles::footprint).unwrap_or_default()
    }

    /// Gives back what the probe context holds only between uses.
    pub(crate) fn release_scratch(&mut self) {
        if let Some(probe) = self.probe.as_mut() {
            probe.release_scratch();
        }
    }

    /// Writes a picture the CPU holds into a client's GPU buffer, on the probe context.
    ///
    /// The frame an output last drew is not copied: a scan-out buffer is the frame, and it is handed to the display,
    /// so there is no readable frame of an output here (see `Backend::capture` of this backend).
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
        Ok(self
            .probe()?
            .copy_image_into_dmabuf(dmabuf, picture, copy.region, copy.from)?)
    }
}

/// One scan-out buffer: the allocation, its GL render target and its KMS framebuffer.
struct ScanSlot {
    /// Owns the allocation the dmabuf, the texture and the framebuffer all refer to.
    _buffer: GbmBuffer,
    scan: ScanBuffer,
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

/// The [`SLOTS`] scan-out buffers of one output, created on first use.
pub(crate) struct Buffers {
    gles: Gles,
    size: PhysicalSize<i32, Physical>,
    /// The size of the virtual surface the output is composed on, which is `size` when there is no supersampling.
    composed: Size,
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    drm: DrmDeviceFd,
    modifiers: Vec<Modifier>,
    slots: SlotRing<ScanSlot>,
}

impl Buffers {
    /// Buffers for an output of `size` physical pixels, composed on a virtual surface of `composed` pixels, scanned out
    /// in a format of `formats`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the plane offers no XRGB8888 format, or when the GLES
    /// renderer cannot be made on the device's display, which includes a virtual surface past its texture limit.
    pub(crate) fn new(device: &Device, size: Size, composed: Size, formats: &FormatSet) -> Result<Self, RenderError> {
        let modifiers: Vec<Modifier> = formats
            .iter()
            .filter(|format| format.code == Fourcc::Xrgb8888)
            .map(|format| format.modifier)
            .collect();
        if modifiers.is_empty() {
            log::warn!(area = "drm", "the plane scans out no XRGB8888 format");
            return Err(RenderError::Unavailable);
        }
        Ok(Self {
            gles: Gles::on_display(&device.display, composed)?,
            composed,
            size: PhysicalSize::from((
                i32::try_from(size.width).map_err(|_| RenderError::Degenerate {
                    width: size.width,
                    height: size.height,
                })?,
                i32::try_from(size.height).map_err(|_| RenderError::Degenerate {
                    width: size.width,
                    height: size.height,
                })?,
            )),
            allocator: GbmAllocator::new(device.gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
            exporter: GbmFramebufferExporter::new(device.gbm.clone(), NodeFilter::None),
            drm: device.drm.clone(),
            modifiers,
            slots: SlotRing::new(SLOTS),
        })
    }

    /// Whether these buffers were allocated for an output of `size` physical pixels composed on `composed`.
    #[must_use]
    pub(crate) fn fits(&self, size: Size, composed: Size) -> bool {
        size_fits((self.size.w, self.size.h), size) && self.composed == composed
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
        self.gles.resize(composed)?;
        self.composed = composed;
        Ok(())
    }

    /// What this output holds: its renderer's footprint, and the scan-out buffers allocated so far, which the renderer
    /// does not count (it only imports them as render targets). The buffers are the frame the kernel scans out, so they
    /// are target bytes: the budget never takes them back.
    #[must_use]
    pub(crate) fn footprint(&self) -> Footprint {
        let scan_out = Footprint {
            target: scanout_bytes((self.size.w, self.size.h), self.slots.allocated()),
            ..Footprint::default()
        };
        self.gles.footprint().plus(scan_out)
    }

    /// Gives back the renderer's blur scratch, which the next frame that blurs makes again.
    pub(crate) fn release_scratch(&mut self) {
        self.gles.release_scratch();
    }

    /// Gives back the textures of every View this output draws; the next frame that names a View draws it again.
    pub(crate) fn release_views(&mut self) {
        self.gles.retire_views(&[]);
    }

    /// Draws `frame` into the scan-out buffer of `slot`: straight into it when the output is not supersampled, and
    /// otherwise into the virtual surface, averaged down into the buffer.
    fn draw(&mut self, slot: usize, frame: &OutputFrame, scale: Scale) -> Result<(), String> {
        self.ensure(slot)?;
        let averaged = self.averaged();
        let Buffers { gles, slots, .. } = self;
        let entry = slots.get_mut(slot)?;
        if averaged {
            gles.draw_scanout_downsampled(&mut entry.scan, frame, scale)
        } else {
            gles.draw_scanout(&mut entry.scan, frame, scale)
        }
        .map_err(|error| {
            if error == RenderError::Lost {
                // The frame that was in flight when the device went held the shaders that may have hung it: the host strikes them.
                frame.shader_timings().record_loss();
            }
            format!("the frame cannot be drawn: {error}")
        })
    }

    /// Allocates slot `index` if it does not exist yet.
    fn ensure(&mut self, index: usize) -> Result<(), String> {
        let Buffers {
            gles,
            size,
            allocator,
            exporter,
            drm,
            modifiers,
            slots,
            ..
        } = self;
        slots.ensure_with(index, || allocate(gles, allocator, exporter, drm, *size, modifiers))
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

/// Allocates one scan-out buffer of `size` and brings it up as a GL render target and a KMS
/// framebuffer.
///
/// # Errors
///
/// The buffer, its export, its GL import or its framebuffer cannot be made; the message says which.
fn allocate(
    gles: &mut Gles,
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
    let scan = gles
        .import_scanout(&dmabuf)
        .map_err(|error| format!("the scan-out buffer cannot be imported: {error}"))?;
    let framebuffer = exporter
        .add_framebuffer(drm, ExportBuffer::Allocator(&buffer), false)
        .map_err(|error| format!("the scan-out buffer has no KMS framebuffer: {error}"))?
        .ok_or_else(|| "the scan-out buffer has no KMS framebuffer".to_string())?;
    Ok(ScanSlot {
        _buffer: buffer,
        scan,
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
        self.gpu.queue_flip(self.crtc, framebuffer, size)
    }
}

/// The kernel and the allocator behind one output's mode change, for [`crate::modeset::configure`].
///
/// Built fresh for each call, so it borrows the GPU and the buffer device only for that call.
pub(crate) struct LiveModeset<'a> {
    /// The GPU whose surface is committed.
    pub(crate) gpu: &'a Gpu,
    /// The device the new buffers are allocated on, borrowed mutably because its probe context answers the texture limit.
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
