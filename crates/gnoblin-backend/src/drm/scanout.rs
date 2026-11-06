//! The GBM scan-out buffers of one output and the [`Scanout`] adapter that draws into them.
//!
//! An output owns two buffers, allocated on the device's GBM node with the plane's XRGB8888
//! modifiers. Each buffer is exported as a dmabuf, imported into the output's GLES renderer as a
//! render target, and registered as a KMS framebuffer. [`crate::drm::frame::present`] chooses the
//! slot; this module draws into it and queues its flip.

use gnoblin_log::warn;
use gnoblin_types::{Scale, Size};
use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::exporter::ExportBuffer;
use smithay::backend::drm::exporter::ExportFramebuffer;
use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
use smithay::backend::egl::EGLDisplay;
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Physical, Size as PhysicalSize};

use crate::drm::frame::Scanout;
use crate::drm::gpu::Gpu;
use crate::drm::present::SLOTS;
use crate::render::RenderError;
use crate::render::frame::OutputFrame;
use crate::render::gles::egl;
use crate::render::gles::{Gles, ScanBuffer};

/// The GBM device and EGL display every output's buffers are allocated on.
pub(crate) struct Device {
    gbm: GbmDevice<DrmDeviceFd>,
    display: EGLDisplay,
    drm: DrmDeviceFd,
}

impl Device {
    /// The GBM device and EGL display on the DRM node `drm`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when GBM or EGL cannot be brought up on the node.
    pub(crate) fn open(drm: DrmDeviceFd) -> Result<Self, RenderError> {
        let gbm = GbmDevice::new(drm.clone()).map_err(|error| {
            warn!(%error, "[render] no GBM device on the DRM node");
            RenderError::Unavailable
        })?;
        let display = egl::gbm_display(&gbm)?;
        Ok(Self { gbm, display, drm })
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
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    drm: DrmDeviceFd,
    modifiers: Vec<Modifier>,
    slots: SlotRing<ScanSlot>,
}

impl Buffers {
    /// Buffers for an output of `size` physical pixels, scanned out in a format of `formats`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the plane offers no XRGB8888 format, or when the GLES
    /// renderer cannot be made on the device's display.
    pub(crate) fn new(device: &Device, size: Size, formats: &FormatSet) -> Result<Self, RenderError> {
        let modifiers: Vec<Modifier> = formats
            .iter()
            .filter(|format| format.code == Fourcc::Xrgb8888)
            .map(|format| format.modifier)
            .collect();
        if modifiers.is_empty() {
            warn!("[drm] the plane scans out no XRGB8888 format");
            return Err(RenderError::Unavailable);
        }
        Ok(Self {
            gles: Gles::on_display(&device.display, size)?,
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

    /// Whether these buffers were allocated for an output of `size` physical pixels.
    #[must_use]
    pub(crate) fn fits(&self, size: Size) -> bool {
        size_fits((self.size.w, self.size.h), size)
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
        } = self;
        slots.ensure_with(index, || allocate(gles, allocator, exporter, drm, *size, modifiers))
    }
}

/// Whether buffers of `allocated` physical pixels suit an output of `wanted` pixels.
#[must_use]
fn size_fits(allocated: (i32, i32), wanted: Size) -> bool {
    u32::try_from(allocated.0) == Ok(wanted.width) && u32::try_from(allocated.1) == Ok(wanted.height)
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

/// The scan-out of one output for one [`crate::drm::frame::present`] call: draws into the
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
    /// The output's scale, applied when the frame is drawn into the buffer.
    pub(crate) scale: Scale,
}

impl Scanout for Flip<'_> {
    type Frame = OutputFrame;

    fn draw(&mut self, slot: usize, frame: &OutputFrame) -> Result<(), String> {
        self.buffers.ensure(slot)?;
        let Buffers { gles, slots, .. } = &mut *self.buffers;
        let entry = slots.get_mut(slot)?;
        gles.draw_scanout(&mut entry.scan, frame, self.scale)
            .map_err(|error| format!("the frame cannot be drawn: {error}"))
    }

    fn flip(&mut self, slot: usize) -> Result<(), String> {
        let size = self.buffers.size;
        let framebuffer = *self.buffers.slots.get(slot)?.framebuffer.as_ref();
        self.gpu.queue_flip(self.crtc, framebuffer, size)
    }
}

#[cfg(test)]
mod tests {
    use super::{SlotRing, size_fits};
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
