//! Which renderer draws an output, and the one type that holds it.
//!
//! [`RendererKind`] is the choice and [`AnyRenderer`] is the choice made: a software, GLES
//! or Vulkan renderer behind the four calls an offscreen output needs. This is the only
//! place in the crate that dispatches on the renderer or tests the `vulkan` and `gles` features to
//! do so; `render/mod.rs` explains why it is an enum and not a trait.

use gnoblin_config::{CaptureSecureTreatment, RendererChoice};
use gnoblin_types::{Rect, Scale, Size};

use gnoblin_render_frame::{
    error::RenderError,
    footprint::Footprint,
    frame::{OutputFrame, SecureTreatment},
    image::Image,
};
#[cfg(feature = "gles")]
use gnoblin_render_gles::Gles;
use gnoblin_render_software::Software;
#[cfg(feature = "vulkan")]
use gnoblin_render_vulkan::Vulkan;

#[cfg(feature = "vulkan")]
use std::rc::Rc;

use crate::log;

/// The renderers this crate can draw an output with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RendererKind {
    /// Smithay's Pixman renderer: pixels with no GPU.
    Software,
    /// Smithay's GLES renderer: effects, on a surfaceless EGL context when
    /// it draws offscreen.
    ///
    /// A build without the `gles` feature cannot create it: [`AnyRenderer::new`] refuses
    /// with [`RenderError::Unavailable`] and a log line that names the feature.
    Gles,
    /// The Vulkan renderer on the device the loader offers, which may be a software driver.
    ///
    /// A build without the `vulkan` feature cannot create it: [`AnyRenderer::new`] refuses
    /// with [`RenderError::Unavailable`] and a log line that names the feature.
    Vulkan,
}

impl RendererKind {
    /// What `compositor.renderer = auto` means.
    ///
    /// Software, because it is the reference every other renderer is compared against, it
    /// needs no GPU, no EGL and no session, and so it is the one choice that cannot fail on
    /// a machine where the session starts at all. Vulkan is not eligible until it draws
    /// effects and can scan out, and GLES is not chosen for a
    /// session that did not ask for it because a missing EGL library would then stop a
    /// session that software would have run. The rule is written here so a backend that
    /// honours the setting cannot grow its own.
    pub const AUTO: Self = Self::Software;

    /// The renderer's name, for a log line and a startup report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Gles => "gles",
            Self::Vulkan => "vulkan",
        }
    }

    /// The renderer a `compositor.renderer` setting names; `auto` is [`Self::AUTO`].
    #[must_use]
    pub const fn from_setting(setting: RendererChoice) -> Self {
        match setting {
            RendererChoice::Auto => Self::AUTO,
            RendererChoice::Software => Self::Software,
            RendererChoice::Gles => Self::Gles,
            RendererChoice::Vulkan => Self::Vulkan,
        }
    }

    /// The effect kinds the renderer draws, by [`gnoblin_render::Pass::name`].
    ///
    /// GLES draws every kind: a blur, a shadow, an inset shadow, a border, a background, a filter and a shader. The
    /// software renderer draws a blur, a shadow (outer and inset), a border, a background and a filter (its grade and
    /// its blur) by computing the pixels on the CPU (`gnoblin-render-software`'s `layers` module): the blur is the GLES
    /// kernel, reduction and grade, the shadows' falloff and the border's edge are the shaders' own arithmetic, and a
    /// filter is drawn as GLES draws it. The Vulkan renderer draws a blur, a shadow, an inset shadow, a border, a
    /// background and a filter (its grade and its blur) with the same shaders as GLES, the blur into images drawn before
    /// the frame.
    ///
    /// The `shader` kind (a shell's own pass, accepted by the tree while `compositor.user-shaders` is on) is drawn by
    /// the GLES and Vulkan renderers. The software renderer has no WGSL interpreter, so a node's shader pass is named in
    /// its `skipped` list as `shader: unsupported-by-renderer` and the renderer logs why once.
    #[must_use]
    pub const fn effect_kinds(self) -> &'static [&'static str] {
        match self {
            Self::Gles => &[
                "blur",
                "shadow",
                "inset-shadow",
                "border",
                "background",
                "filter",
                "shader",
            ],
            Self::Software => &["blur", "shadow", "inset-shadow", "border", "background", "filter"],
            Self::Vulkan => &[
                "blur",
                "shadow",
                "border",
                "background",
                "inset-shadow",
                "filter",
                "shader",
            ],
        }
    }

    /// Whether the renderer blurs what a `filter` effect paints, which it lists as `filter` only for the grade when this is
    /// false. A filter that blurs is then named in the node's `skipped` list as `unsupported-filter-blur`.
    #[must_use]
    pub const fn draws_filter_blur(self) -> bool {
        matches!(self, Self::Gles | Self::Software | Self::Vulkan)
    }

    /// Whether this renderer draws a View's source subtree into a texture on the GPU: GLES and Vulkan do, and the software
    /// renderer is the CPU reference. A Vulkan device with no depth-stencil format draws none; [`AnyRenderer::draws_views`]
    /// answers for the device a renderer is on, and a backend asks it once it has one.
    #[must_use]
    pub const fn draws_views(self) -> bool {
        matches!(self, Self::Gles | Self::Vulkan)
    }
}

impl Default for RendererKind {
    fn default() -> Self {
        Self::AUTO
    }
}

/// The treatment a `capture.secure-treatment` setting names.
///
/// This is a free function rather than a `From` impl because both types live in other crates.
#[must_use]
pub const fn secure_treatment_from_setting(setting: CaptureSecureTreatment) -> SecureTreatment {
    match setting {
        CaptureSecureTreatment::Mask => SecureTreatment::Mask,
        CaptureSecureTreatment::Omit => SecureTreatment::Omit,
    }
}

/// A renderer of one of the [`RendererKind`]s, drawing into an offscreen buffer it owns.
///
/// The variants are boxed because a GLES renderer owns a context and its programs and is
/// several times the size of a pixman one; without the box every holder would reserve the
/// larger size, and a renderer is created once per output.
pub enum AnyRenderer {
    /// Pixman, into an offscreen image buffer.
    Software(Box<Software>),
    /// GLES, into a texture on a surfaceless context.
    #[cfg(feature = "gles")]
    Gles(Box<Gles>),
    /// Vulkan, into an offscreen image on the device the loader offers.
    #[cfg(feature = "vulkan")]
    Vulkan(Box<Vulkan>),
}

/// Runs one expression against whichever renderer is installed.
///
/// It is where the `vulkan` and `gles` features are tested for dispatch: a call that every renderer
/// answers the same way is written once and the macro supplies the arms.
macro_rules! on_renderer {
    ($renderer:expr, $name:ident => $call:expr) => {
        match $renderer {
            Self::Software($name) => $call,
            #[cfg(feature = "gles")]
            Self::Gles($name) => $call,
            #[cfg(feature = "vulkan")]
            Self::Vulkan($name) => $call,
        }
    };
}

/// What the outputs of one backend share on the GPU.
///
/// Only Vulkan shares anything: its device, which owns the allocator, the pipelines and the cache of client textures. A GLES
/// context is one per renderer, and the software renderer has no GPU.
#[derive(Default)]
pub struct GpuShare {
    #[cfg(feature = "vulkan")]
    vulkan: Option<Rc<gnoblin_render_vulkan::VulkanDevice>>,
    /// The `dev_t` of the DRM node the device should be on, when the backend knows one: a host compositor's dmabuf
    /// `main_device`, so the buffers the nested session presents are ones the host can import. A machine with no device there
    /// gets the best device, and the presenter falls back to shared memory if the host refuses its buffers.
    pub node: Option<u64>,
}

impl GpuShare {
    /// A share that has no device yet and prefers the one on `node`.
    #[must_use]
    pub fn on_node(node: Option<u64>) -> Self {
        Self {
            node,
            ..Self::default()
        }
    }

    /// The Vulkan device the outputs share, once one has been built.
    #[cfg(feature = "vulkan")]
    #[must_use]
    pub fn vulkan_device(&self) -> Option<&Rc<gnoblin_render_vulkan::VulkanDevice>> {
        self.vulkan.as_ref()
    }
}

impl AnyRenderer {
    /// A renderer of `kind` with an offscreen buffer of `physical` pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] when the size covers no pixels, and
    /// [`RenderError::Unavailable`] when this machine cannot create the renderer at all:
    /// for GLES, no EGL library, no surfaceless platform or no GLES 3 context; for Vulkan,
    /// no loader, no device or a refused shader (the log names which), or a build without
    /// the `vulkan` feature; and for GLES, a build without the `gles` feature.
    pub fn new(kind: RendererKind, physical: Size) -> Result<Self, RenderError> {
        Self::new_sharing(kind, physical, &mut GpuShare::default())
    }

    /// [`AnyRenderer::new`] on the GPU `share` holds: the outputs of one backend that are built with the same share draw on one
    /// Vulkan device, so a window that spans two outputs is uploaded once and a lost device is lost once. The first Vulkan
    /// renderer built makes the device, on `share.node` when the machine has a device there.
    ///
    /// # Errors
    ///
    /// As [`AnyRenderer::new`].
    pub fn new_sharing(kind: RendererKind, physical: Size, share: &mut GpuShare) -> Result<Self, RenderError> {
        match kind {
            RendererKind::Software => Software::new(physical).map(|software| Self::Software(Box::new(software))),
            RendererKind::Gles => Self::gles(physical),
            RendererKind::Vulkan => Self::vulkan(physical, share),
        }
    }

    #[cfg(feature = "gles")]
    fn gles(physical: Size) -> Result<Self, RenderError> {
        Gles::new(physical).map(|gles| Self::Gles(Box::new(gles)))
    }

    #[cfg(not(feature = "gles"))]
    fn gles(_physical: Size) -> Result<Self, RenderError> {
        log::warn!(
            area = "render",
            "this build has no GLES renderer (the `gles` feature is off)"
        );
        Err(RenderError::Unavailable)
    }

    #[cfg(feature = "vulkan")]
    fn vulkan(physical: Size, share: &mut GpuShare) -> Result<Self, RenderError> {
        use gnoblin_render_vulkan::VulkanError;
        // A device that was lost is lost for every output on it: the next output builds a new one.
        if share.vulkan.as_ref().is_some_and(|device| device.is_lost()) {
            share.vulkan = None;
        }
        let device = match &share.vulkan {
            Some(device) => Rc::clone(device),
            None => {
                let device = Self::vulkan_device(share.node).map_err(VulkanError::into_render_error)?;
                share.vulkan = Some(Rc::clone(&device));
                device
            }
        };
        Vulkan::with_device(&device, physical)
            .map(|vulkan| Self::Vulkan(Box::new(vulkan)))
            .map_err(VulkanError::into_render_error)
    }

    /// The device outputs share: the one on `node` when the backend names one and the machine has it, the best otherwise.
    #[cfg(feature = "vulkan")]
    fn vulkan_device(
        node: Option<u64>,
    ) -> Result<Rc<gnoblin_render_vulkan::VulkanDevice>, gnoblin_render_vulkan::VulkanError> {
        use gnoblin_render_vulkan::{DeviceChoice, VulkanDevice, VulkanError, VulkanOptions};
        if let Some(node) = node {
            let options = VulkanOptions {
                device: DeviceChoice::RenderNode(node),
                ..VulkanOptions::default()
            };
            match VulkanDevice::with_options(&options) {
                Ok(device) => return Ok(device),
                Err(VulkanError::NoDevice { reason }) => {
                    log::info!(
                        area = "render",
                        node = format_args!("{node:#x}"),
                        reason,
                        "no Vulkan device on the wanted DRM node; the best device is used"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        VulkanDevice::new()
    }

    #[cfg(not(feature = "vulkan"))]
    fn vulkan(_physical: Size, _share: &mut GpuShare) -> Result<Self, RenderError> {
        log::warn!(
            area = "render",
            "this build has no Vulkan renderer (the `vulkan` feature is off)"
        );
        Err(RenderError::Unavailable)
    }

    /// Which renderer this is.
    #[must_use]
    pub fn kind(&self) -> RendererKind {
        match self {
            Self::Software(_) => RendererKind::Software,
            #[cfg(feature = "gles")]
            Self::Gles(_) => RendererKind::Gles,
            #[cfg(feature = "vulkan")]
            Self::Vulkan(_) => RendererKind::Vulkan,
        }
    }

    /// The largest edge, in pixels, this renderer's buffer can have. Only a GPU renderer has a texture limit (GLES's
    /// `GL_MAX_TEXTURE_SIZE`, Vulkan's `maxImageDimension2D`); the software renderer's buffer is bounded by memory,
    /// which the compositor's own virtual-size limit bounds.
    pub fn max_texture_size(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.max_texture_size(),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.max_image_edge(),
            _ => u32::MAX,
        }
    }

    /// The signal the renderer asks for another frame on, when it can: only the Vulkan renderer builds anything after
    /// `draw` returned. A backend may install a waker on it to wake its event loop.
    #[must_use]
    pub fn redraw_signal(&self) -> Option<gnoblin_render_frame::redraw::RedrawSignal> {
        match self {
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => Some(vulkan.redraw_signal()),
            _ => None,
        }
    }

    /// Installs what the renderer's worker thread calls when it asks for another frame, so the event loop wakes for it.
    /// A renderer that never asks ignores it.
    pub fn set_redraw_waker(&self, waker: gnoblin_render_frame::redraw::Waker) {
        if let Some(signal) = self.redraw_signal() {
            signal.set_waker(waker);
        }
    }

    /// Whether the renderer asked for another frame since the last call, which clears the request.
    pub fn take_redraw(&mut self) -> bool {
        self.redraw_signal().is_some_and(|signal| signal.take())
    }

    /// The bytes the installed renderer holds for its output, by what holds them.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        on_renderer!(self, renderer => renderer.footprint())
    }

    /// Draws a frame, repainting only its damage.
    ///
    /// # Errors
    ///
    /// The installed renderer's [`RenderError`].
    pub fn draw(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), RenderError> {
        let drawn = on_renderer!(self, renderer => renderer.draw(frame, scale));
        if drawn == Err(RenderError::Lost) {
            // The frame that was in flight when the device went held the shaders that may have hung it: the host strikes them.
            frame.shader_timings().record_loss();
        }
        drawn
    }

    /// Reads the frame back, with the cursor composed in when `with_cursor` is set.
    ///
    /// # Errors
    ///
    /// The installed renderer's [`RenderError`].
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        on_renderer!(self, renderer => renderer.capture(with_cursor))
    }

    /// Replaces the buffer with one of the given size.
    ///
    /// # Errors
    ///
    /// The installed renderer's [`RenderError`].
    pub fn resize(&mut self, physical: Size) -> Result<(), RenderError> {
        on_renderer!(self, renderer => renderer.resize(physical))
    }
}

#[cfg(feature = "gles")]
impl AnyRenderer {
    /// The GLES renderer, when this is one.
    ///
    /// A caller that wants the draw counter asks for it here; the other
    /// renderers answer [`None`], because a question about a GL device has no answer without
    /// one.
    #[must_use]
    pub fn gles_mut(&mut self) -> Option<&mut Gles> {
        match self {
            Self::Gles(gles) => Some(gles.as_mut()),
            _ => None,
        }
    }
}

#[cfg(feature = "vulkan")]
impl AnyRenderer {
    /// The Vulkan renderer, when this is one.
    #[must_use]
    pub fn vulkan_mut(&mut self) -> Option<&mut Vulkan> {
        match self {
            Self::Vulkan(vulkan) => Some(vulkan.as_mut()),
            _ => None,
        }
    }
}

/// What a client's GPU buffer, a capture into one and a View's picture ask of a renderer. GLES and Vulkan answer them; the
/// software renderer reads CPU memory and answers that it cannot.
impl AnyRenderer {
    /// Whether this renderer draws a View's source subtree into a texture on the GPU: GLES always, Vulkan when the device has
    /// a depth-stencil format to draw one with ([`RendererKind::draws_views`] says what the kind can do).
    #[must_use]
    pub fn draws_views(&self) -> bool {
        match self {
            Self::Software(_) => false,
            #[cfg(feature = "gles")]
            Self::Gles(_) => true,
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.gpu().draws_views(),
        }
    }

    /// What this renderer imports of a client's GPU buffers, or `None` when it imports none.
    ///
    /// GLES imports what its display reports importable, on the render node of the context when it has one. Vulkan imports a
    /// linear single-plane buffer in the formats its device draws (`gnoblin_render_vulkan::dmabuf_formats`), on the render
    /// node of its device when the driver names one; a software driver names none, so clients are served version 3 of the
    /// protocol, which names formats and no device. The software renderer reads CPU memory and imports none.
    #[must_use]
    pub fn dmabuf_support(&mut self) -> Option<crate::DmabufSupport> {
        match self {
            Self::Software(_) => None,
            #[cfg(feature = "gles")]
            Self::Gles(gles) => {
                let formats = gles.dmabuf_formats();
                if formats.is_empty() {
                    return None;
                }
                Some(crate::DmabufSupport::new(gles.render_node(), formats))
            }
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => Some(crate::DmabufSupport::new(vulkan.render_node(), vulkan.dmabuf_formats())),
        }
    }

    /// Imports a client's GPU buffer, so a draw of it finds the texture made.
    ///
    /// # Errors
    ///
    /// [`crate::DmabufImportError::Unsupported`] for a renderer that imports none, and
    /// [`crate::DmabufImportError::Refused`] when the renderer refuses the buffer (Vulkan: not a single linear plane in a
    /// format it draws).
    pub fn import_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<(), crate::DmabufImportError> {
        match self {
            Self::Software(_) => Err(crate::DmabufImportError::Unsupported),
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.import_dmabuf(dmabuf).map_err(crate::DmabufImportError::Refused),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.import_dmabuf(dmabuf).map_err(crate::DmabufImportError::Refused),
        }
    }

    /// The DRM device explicit sync may be served on, when this renderer can keep its promise.
    ///
    /// Only Vulkan can: its timeline says when the GPU has finished each frame ([`AnyRenderer::buffers_retired`]), so a
    /// client's release point signals after the last frame that may have read the buffer. GLES and the software renderer
    /// report no retirement, so a client of theirs keeps implicit sync. Vulkan answers when its device names a DRM render
    /// node whose kernel waits on a timeline point through an eventfd; a software driver names no node, so it answers `None`.
    #[must_use]
    pub fn explicit_sync(&self) -> Option<crate::ExplicitSync> {
        match self {
            Self::Software(_) => None,
            #[cfg(feature = "gles")]
            Self::Gles(_) => None,
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => match crate::ExplicitSync::on_node(vulkan.render_node()) {
                Ok(device) => Some(device),
                Err(refusal) => {
                    crate::log::info!(area = "explicit-sync", %refusal, "explicit sync is not offered on this Vulkan device");
                    None
                }
            },
        }
    }

    /// How far the GPU has got through this renderer's frames; all zero for a renderer that reports none.
    #[must_use]
    pub fn buffers_retired(&self) -> crate::Retirement {
        match self {
            Self::Software(_) => crate::Retirement::default(),
            #[cfg(feature = "gles")]
            Self::Gles(_) => crate::Retirement::default(),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => {
                let progress = vulkan.gpu().frame_progress();
                crate::Retirement {
                    submitted: progress.submitted,
                    retired: progress.completed,
                }
            }
        }
    }

    /// Copies the frame last drawn into a client's GPU buffer, with the cursor composed in when `with_cursor` is set.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a renderer that draws into no client buffer (the software renderer) or a buffer
    /// Vulkan does not offer, and the renderer's own error otherwise.
    pub fn copy_frame_into_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        copy: crate::DmabufCopy,
        with_cursor: bool,
    ) -> Result<(), RenderError> {
        match self {
            Self::Software(_) => Err(RenderError::Unsupported),
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.copy_frame_into_dmabuf(dmabuf, copy.region, copy.from, with_cursor),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.copy_frame_region_into_dmabuf(dmabuf, copy.region, copy.from, with_cursor),
        }
    }

    /// Writes a picture the CPU holds into a client's GPU buffer.
    ///
    /// # Errors
    ///
    /// As [`AnyRenderer::copy_frame_into_dmabuf`].
    pub fn copy_image_into_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        image: &Image,
        copy: crate::DmabufCopy,
    ) -> Result<(), RenderError> {
        match self {
            Self::Software(_) => Err(RenderError::Unsupported),
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.copy_image_into_dmabuf(dmabuf, image, copy.region, copy.from),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.copy_image_region_into_dmabuf(image, dmabuf, copy.region, copy.from),
        }
    }

    /// Reads a client's GPU buffer back as a picture.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a renderer that imports none, and the renderer's own error otherwise.
    pub fn read_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<Image, RenderError> {
        match self {
            Self::Software(_) => Err(RenderError::Unsupported),
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.read_dmabuf(dmabuf),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.read_dmabuf(dmabuf),
        }
    }

    /// Reads a View's picture back, drawing the scene into a texture first when the renderer holds none of its
    /// current revision. `live` names every View alive on the GPU path; the textures of the others are dropped.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] for a renderer that draws a View on the CPU, and the renderer's own error otherwise.
    pub fn view_pixels(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, RenderError> {
        match self {
            Self::Software(_) => Err(RenderError::Unsupported),
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.read_view(scene, capture, live),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) if vulkan.gpu().draws_views() => vulkan.read_view(scene, capture, live),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(_) => Err(RenderError::Unsupported),
        }
    }

    /// Gives back every View texture the renderer holds. A renderer that draws a View on the CPU holds none.
    pub fn release_views(&mut self) {
        match self {
            Self::Software(_) => {}
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.retire_views(&[]),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.retire_views(&[]),
        }
    }

    /// The output's target as the display shows it at `shown` pixels: a supersampled target averaged down on the GPU, with the
    /// cursor. The software renderer has no GPU pass for it and answers [`RenderError::Unsupported`]; its caller averages
    /// the capture on the CPU.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`], and the renderer's own error otherwise.
    pub fn downsampled(&mut self, shown: Size) -> Result<Image, RenderError> {
        match self {
            Self::Software(_) => Err(RenderError::Unsupported),
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.downsampled(shown),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(vulkan) => vulkan.downsampled(shown),
        }
    }
}

impl AnyRenderer {
    /// Gives back what the renderer holds only between frames (`Footprint::releasable`): the blur scratch targets of
    /// the GLES renderer and the software renderer's snapshots of each layer. The next frame makes them again. The
    /// Vulkan renderer trims its host buffers after each frame already and has nothing more to give.
    pub fn release_scratch(&mut self) {
        match self {
            #[cfg(feature = "gles")]
            Self::Gles(gles) => gles.release_scratch(),
            Self::Software(software) => software.release_scratch(),
            #[cfg(feature = "vulkan")]
            Self::Vulkan(_) => {}
        }
    }
}

/// What a redacted capture reads: the frame last drawn, where the capture covers, and how
/// its secure layers are treated.
pub(crate) struct CaptureRequest<'a> {
    /// The frame last drawn, or `None` when it has no secure layer, in which case the read
    /// is the plain one.
    pub display: Option<&'a OutputFrame>,
    /// The rectangle the capture covers.
    pub bounds: Rect,
    /// The scale the frame is drawn at.
    pub scale: Scale,
    /// Whether the cursor is drawn into the read.
    pub with_cursor: bool,
    /// How each secure layer is drawn in the read.
    pub treatment: SecureTreatment,
}

/// Reads a renderer back for a capture, with the secure layers of the frame it last drew
/// replaced by the request's treatment.
///
/// `read` takes what a capture produces from the renderer: a picture on the CPU, or nothing at all when it writes
/// into a client's buffer on the GPU. Either way the replacement is in place while it runs.
///
/// The display is drawn again with full damage so the next present shows the real layers.
/// It is restored even when the read fails.
pub(crate) fn capture_redacted<R, T>(
    renderer: &mut R,
    request: CaptureRequest<'_>,
    draw: fn(&mut R, &OutputFrame, Scale) -> Result<(), RenderError>,
    read: impl FnOnce(&mut R, bool) -> Result<T, RenderError>,
) -> Result<T, RenderError> {
    let CaptureRequest {
        display,
        bounds,
        scale,
        with_cursor,
        treatment,
    } = request;
    let Some(display) = display else {
        return read(renderer, with_cursor);
    };
    let replaced = display.redacted(treatment, bounds);
    let taken = draw(renderer, &replaced, scale).and_then(|()| read(renderer, with_cursor));
    let restored = draw(renderer, &display.fully_damaged(bounds), scale);
    let taken = taken?;
    restored?;
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_renderer_setting_selects_the_renderer_and_auto_keeps_the_default() {
        assert_eq!(
            RendererKind::from_setting(RendererChoice::Auto),
            RendererKind::default(),
            "auto is what a session that says nothing gets"
        );
        assert_eq!(RendererKind::AUTO, RendererKind::Software, "auto stays software");
        assert_eq!(
            RendererKind::from_setting(RendererChoice::Software),
            RendererKind::Software
        );
        assert_eq!(RendererKind::from_setting(RendererChoice::Gles), RendererKind::Gles);
        assert_eq!(RendererKind::from_setting(RendererChoice::Vulkan), RendererKind::Vulkan);
    }

    #[test]
    fn a_software_renderer_reports_its_kind_and_has_no_gl_device() {
        let mut renderer = AnyRenderer::new(RendererKind::Software, Size::new(8, 8)).expect("a software renderer");
        assert_eq!(renderer.kind(), RendererKind::Software);
        #[cfg(feature = "gles")]
        assert!(renderer.gles_mut().is_none(), "only a GLES renderer answers gles_mut");
        assert!(
            renderer.dmabuf_support().is_none(),
            "the software renderer imports no GPU buffer"
        );
    }

    #[cfg(not(feature = "gles"))]
    #[test]
    fn a_build_without_the_gles_feature_refuses_the_gles_renderer() {
        assert!(matches!(
            AnyRenderer::new(RendererKind::Gles, Size::new(8, 8)),
            Err(RenderError::Unavailable)
        ));
    }

    #[test]
    fn the_gles_renderer_lists_every_kind_it_draws() {
        for kind in gnoblin_render::Pass::NAMES {
            assert!(RendererKind::Gles.effect_kinds().contains(kind), "GLES draws {kind}");
        }
    }

    #[test]
    fn the_vulkan_renderer_lists_every_kind_and_blurs_a_filter() {
        for kind in gnoblin_render::Pass::NAMES {
            assert!(
                RendererKind::Vulkan.effect_kinds().contains(kind),
                "Vulkan draws {kind}"
            );
        }
        assert!(RendererKind::Vulkan.draws_filter_blur());
    }

    #[cfg(not(feature = "vulkan"))]
    #[test]
    fn a_build_without_the_vulkan_feature_refuses_the_vulkan_renderer() {
        assert!(matches!(
            AnyRenderer::new(RendererKind::Vulkan, Size::new(8, 8)),
            Err(RenderError::Unavailable)
        ));
    }
}
