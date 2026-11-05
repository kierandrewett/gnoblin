//! The GBM scan-out buffers of one output and the [`Scanout`] adapter that draws into them.
//!
//! An output owns two buffers, allocated on the device's GBM node with the plane's XRGB8888
//! modifiers. Each buffer is exported as a dmabuf, imported into the output's GLES renderer as a
//! render target, and registered as a KMS framebuffer. [`crate::drm::frame::present`] chooses the
//! slot; this module draws into it and queues its flip.

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
use tracing::warn;

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

/// The [`SLOTS`] scan-out buffers of one output, created on first use.
pub(crate) struct Buffers {
    gles: Gles,
    size: PhysicalSize<i32, Physical>,
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    drm: DrmDeviceFd,
    modifiers: Vec<Modifier>,
    slots: Vec<Option<ScanSlot>>,
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
            slots: (0..SLOTS).map(|_| None).collect(),
        })
    }

    /// Allocates slot `index` if it does not exist yet.
    fn ensure(&mut self, index: usize) -> Result<(), String> {
        let Some(entry) = self.slots.get(index) else {
            return Err(format!("scan-out slot {index} does not exist"));
        };
        if entry.is_none() {
            let slot = self.allocate()?;
            self.slots[index] = Some(slot);
        }
        Ok(())
    }

    fn allocate(&mut self) -> Result<ScanSlot, String> {
        let buffer = self
            .allocator
            .create_buffer(
                self.size.w.unsigned_abs(),
                self.size.h.unsigned_abs(),
                Fourcc::Xrgb8888,
                &self.modifiers,
            )
            .map_err(|error| format!("the scan-out buffer cannot be allocated: {error}"))?;
        let dmabuf = buffer
            .export()
            .map_err(|error| format!("the scan-out buffer cannot be exported: {error}"))?;
        let scan = self
            .gles
            .import_scanout(&dmabuf)
            .map_err(|error| format!("the scan-out buffer cannot be imported: {error}"))?;
        let framebuffer = self
            .exporter
            .add_framebuffer(&self.drm, ExportBuffer::Allocator(&buffer), false)
            .map_err(|error| format!("the scan-out buffer has no KMS framebuffer: {error}"))?
            .ok_or_else(|| "the scan-out buffer has no KMS framebuffer".to_string())?;
        Ok(ScanSlot {
            _buffer: buffer,
            scan,
            framebuffer,
        })
    }
}

/// The scan-out of one output for one [`crate::drm::frame::present`] call: draws into the
/// buffers and queues flips on the CRTC.
pub(crate) struct Flip<'a> {
    pub(crate) buffers: &'a mut Buffers,
    pub(crate) gpu: &'a Gpu,
    pub(crate) crtc: crtc::Handle,
    pub(crate) scale: Scale,
}

impl Scanout for Flip<'_> {
    type Frame = OutputFrame;

    fn draw(&mut self, slot: usize, frame: &OutputFrame) -> Result<(), String> {
        self.buffers.ensure(slot)?;
        let Buffers { gles, slots, .. } = &mut *self.buffers;
        let entry = slots[slot]
            .as_mut()
            .ok_or_else(|| format!("scan-out slot {slot} is empty"))?;
        gles.draw_scanout(&mut entry.scan, frame, self.scale)
            .map_err(|error| format!("the frame cannot be drawn: {error}"))
    }

    fn flip(&mut self, slot: usize) -> Result<(), String> {
        let size = self.buffers.size;
        let framebuffer = {
            let slot = self
                .buffers
                .slots
                .get(slot)
                .and_then(Option::as_ref)
                .ok_or_else(|| format!("scan-out slot {slot} was never drawn"))?;
            *slot.framebuffer.as_ref()
        };
        self.gpu.queue_flip(self.crtc, framebuffer, size)
    }
}
