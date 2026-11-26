//! The Vulkan renderer: an [`OutputFrame`] drawn through `ash` into an offscreen image and read
//! back as an [`Image`].
//!
//! This is the third implementation of the frame description (see `gnoblin-render-frame`), built on `ash` and
//! `naga`. It is the offscreen renderer: it draws what the software renderer draws, plus the rounded corners the shared
//! shaders already clip, and it reads the result back. It does not present, import dmabufs or draw a blur
//! yet; `DESIGN.md` lists what is next and in what order.
//!
//! # The device and the output
//!
//! A [`VulkanDevice`] is one physical GPU: the instance and logical device, the allocator, the pipelines, the cache of client
//! textures and the timeline every submission signals. It is shared (`Rc`) by every output on the GPU. A [`Vulkan`] is one
//! output: its target image, its two frame slots and the pixels under its cursor. [`Vulkan::new`] makes a private device for
//! one output, for tests and the headless default; [`Vulkan::with_device`] makes an output on a device that already exists,
//! and a window that spans two outputs is then uploaded once.
//!
//! # What it draws
//!
//! - **Solid layers** and the background, with the layer's tint and corner radius.
//! - **Client buffers**: `argb8888` and `xrgb8888` `wl_shm` buffers, cropped by the layer's
//!   source rectangle and scaled with linear filtering. The pixels are premultiplied, as the
//!   protocol says a shm buffer's are, and are blended with `ONE, ONE_MINUS_SRC_ALPHA`.
//! - **Opacity** and **tint** per layer, in the same shader the GLES renderer runs.
//! - **Effects**: outer and inset shadows, a background, borders and a filter's grade, in the paint order of
//!   the design note (section 6.1) and with the same shaders as GLES. A solid colour is graded on the CPU by
//!   [`grade::apply`](gnoblin_render_frame::grade::apply) and a layer's pixels in the fragment stage.
//! - **The portal overlay and the cursor**, nearest-sampled, with the pixels under each saved
//!   so a capture can leave the overlay (always) and the cursor (when asked) out.
//! - **Output scale**: a frame's rectangles are logical and the target is physical, converted
//!   with the one rounding rule all renderers share ([`physical_rect`](gnoblin_render_frame::geometry)).
//! - **Damage**: only the damage is repainted. Each rectangle is a scissor, and overlapping
//!   rectangles are made disjoint first so a translucent layer is never blended twice.
//! - **Layers placed in three dimensions**: a layer with a [`Model`](gnoblin_render_frame::frame::Model) is drawn through its
//!   matrix against a depth buffer, after the flat layers and before the overlay (`spatial.rs`).
//! - **Views**: a View's source subtree is drawn once per revision into an image of its own, with a stencil for its clipping
//!   ancestors, and the layer that shows it samples that image ([`Vulkan::view_pixels`], `subtree.rs`).
//! - **The magnifier and the colour matrix**: a frame's view and matrix are applied to the finished output in a pass of
//!   their own (`view.rs`).
//! - **Supersampling**: [`Vulkan::downsampled`] averages a target composed at `mode * s` down to the mode and reads it
//!   back (`downsample.rs`).
//!
//! A buffer's `set_buffer_transform` is not drawn, which is what the GLES renderer does too. The
//! `shader` pass is drawn (below).
//!
//! # Frames in flight, textures and memory
//!
//! - **`draw` does not wait for the GPU.** It records into one of the output's two slots, submits, and returns; the next
//!   frame is recorded while the last runs. A device-wide timeline semaphore says which frames have finished, and an object a
//!   frame in flight uses is destroyed when the timeline passes that frame (`sync.rs`). A capture waits for its own
//!   submission. A frame that draws a shell's shader is waited for, because the watchdog needs its cost.
//! - **Client textures persist.** A layer that carries a [`ContentKey`](gnoblin_render_frame::frame::ContentKey) keeps its
//!   texture between frames, and a new commit uploads only the damaged rectangles (`textures.rs`). A frame that changed
//!   nothing uploads nothing.
//! - **Memory is sub-allocated.** Images and buffers are ranges of a few large device allocations (`memory.rs`), so a steady
//!   session asks the driver for none.
//! - **Pipelines are cached on disk.** The driver's pipeline cache is kept in a file between runs (`pipeline.rs`).
//! - **[`Stats`]** count the submissions, uploads, allocations and pipelines, which is what the tests of the points above read.
//!
//! # The shader pass
//!
//! A shell's own shader is a pipeline of its own for
//! each source, made when a frame first draws it from the WGSL
//! [`user::validate`](gnoblin_render_frame::shader::user::validate) returns, and kept in the shared
//! 64-entry [`ShaderCache`](gnoblin_render_frame::shader::ShaderCache). It replaces the layer's own
//! pass: the layer's pixels (a solid's colour is a one-pixel texture) are `c`, the wrapper applies
//! the layer's tint, opacity and corners after the last pass, and several passes run in list order
//! through two scratch images the size of the layer.
//!
//! A pass that calls `backdrop(uv)` reads an image of the layer's rectangle that the renderer draws
//! first: [`OutputFrame::backdrop_input`] is the layers below the layer with every secure one black,
//! drawn with the same planner into an image of its own, never the target (which shows the secure
//! layers). A layer below that reads its own backdrop has it built first, so backdrops nest; the first
//! three layers of a frame get one and a fourth is drawn without the passes that read it, as the GLES
//! renderer does. What lies outside the output is transparent in a backdrop.
//!
//! The cost of each source in a frame is told to [`OutputFrame::shader_timings`]: the GPU time of its
//! draws when the queue writes timestamps, otherwise the wall time from submit to fence. A pipeline
//! is made on the render thread at the first draw of a source, which costs milliseconds to tens of
//! milliseconds (design section 37.4).
//!
//! # Choosing it and failing over
//!
//! [`Vulkan::new`] returns a [`VulkanError`] that names why there is no renderer, so the
//! caller can fall back to another one. Once a renderer exists, a frame that cannot be drawn
//! returns a [`RenderError`]; [`RenderError::Lost`] means the device is gone and the caller must
//! build a new renderer and damage the whole output. A lost device is lost for every output on it.
//!
//! # Failure modes
//!
//! 1. **No Vulkan loader.** The loader is opened with `dlopen`, so a machine without one gets
//!    [`VulkanError::NoLoader`] instead of a binary that will not start:
//!    `tests::a_missing_loader_is_a_named_error`.
//! 2. **No suitable device.** A machine whose loader lists no device, or only ones that cannot
//!    draw (or speak Vulkan 1.2 with timeline semaphores), gets [`VulkanError::NoDevice`] with the reason for each:
//!    `tests::an_unmatched_device_is_a_named_error`.
//! 3. **The device is lost during a frame.** The frame is dropped, the call returns
//!    [`RenderError::Lost`], and every later call returns it without touching the device:
//!    `tests::a_lost_device_drops_the_frame_and_is_reported`.
//! 4. **A shader does not compile.** The passes are compiled when the renderer is created, before
//!    any driver is asked, so a broken one is [`VulkanError::Shader`] at startup and never a
//!    frame that fails at its first draw: `tests::a_broken_shader_is_a_startup_error`.
//! 5. **A texture cannot be allocated.** That layer is skipped with a warning and the others
//!    are drawn: `tests::a_texture_that_cannot_be_allocated_skips_its_layer`.
//! 6. **The driver is used wrongly.** The validation layer runs in debug builds when it is
//!    installed, every drawing test fails on anything it reports (synchronisation checks included), and a deliberate misuse
//!    proves the collector sees it: `tests::validation_reports_a_driver_misuse`.
//! 7. **A readback is upside down.** The shared vertex stage maps pixel row 0 to the top and
//!    the writer must not flip it: `tests::a_readback_is_top_left_first`.
//! 8. **A client's buffer is read after it was destroyed or shrunk.** The pool is read through
//!    Smithay's guarded mapping. A destroyed buffer is still read, so a closing window fades out
//!    with its last picture (`tests::a_destroyed_buffer_still_draws_its_last_picture`); a buffer
//!    that cannot be read skips its layer.
//! 9. **A shell's shader does not compile, or the driver refuses its pipeline.** The layer is
//!    drawn without the pass and the source is reported on the frame's shader timings with a short
//!    code (`compile-failed`, `pipeline-failed`, `backdrop-unavailable` or `image-failed`; the node's
//!    `skipped` list names it and the log has the detail). A source the validator or compiler refuses is
//!    remembered, a driver's refusal is tried again next frame:
//!    `tests::shader::a_source_that_does_not_compile_leaves_its_layer_drawn_and_is_reported`,
//!    `tests::shader::a_pipeline_the_driver_refuses_is_tried_again_next_frame`.
//! 10. **A shader pipeline is destroyed while a frame still uses it.** The 64-entry cache evicts
//!     into a list the device hands to the retire queue, which destroys it when the timeline passes the last frame, and a lost
//!     device or a dropped device drains everything after waiting for the queue:
//!     `tests::shader::the_pipelines_are_kept_by_source_and_the_least_recently_used_leave_after_64`,
//!     `tests::shader::a_lost_device_drains_the_pipelines`.
//! 11. **A frame races the one beside it.** Each slot owns what the CPU writes and the GPU reads, a slot is waited for before it
//!     is written, and every frame begins with a barrier from all earlier work: `tests::a_frame_returns_before_the_gpu_finishes`
//!     and the whole drawing suite, which runs with synchronisation validation.
//! 12. **A texture is destroyed under a frame, or shows an old picture.** The retire queue holds it until the frame that used it
//!     has finished, and a commit the cache did not see in order is uploaded whole:
//!     `tests::a_texture_is_destroyed_only_after_its_last_frame_retires`,
//!     `tests::a_commit_that_was_not_drawn_is_uploaded_whole`.
//! 13. **A corrupt pipeline cache file takes the renderer down.** A file that fails a check is deleted and ignored:
//!     `tests::a_corrupt_pipeline_cache_is_ignored_and_replaced`.
//! 14. **A View's picture cannot be had** (a size no device makes, no memory, no depth-stencil format) or **its frame fails
//!     before it is submitted.** The layer samples nothing and the rest of the frame is drawn; a frame that failed drops the
//!     pictures it meant to draw, so the next one draws them: `tests::views::a_view_the_device_refuses_leaves_the_frame_alone`,
//!     `tests::views::a_frame_that_fails_before_it_is_submitted_does_not_leave_a_view_believed_drawn`.
//! 15. **A secure node shows in a capture of a View.** A capture reads the scene's capture variant, a picture of its own,
//!     and the picture a frame samples stays the display's: `tests::views::a_capture_reads_the_scenes_redacted_eyes`.
//! 16. **A device has no depth-stencil format.** A layer with a model is not drawn and `unsupported-3d` is logged once; no
//!     View is drawn (its layer is skipped, as in 14); the view pass and the downsample are not drawn either, because their
//!     pipelines are made with the depth pass's (`Extras`). Every device the renderer has been run on has `D24S8` or `D32S8`.
//!
//! # Byte order
//!
//! A little-endian host is assumed where a client's pixels are uploaded: the bytes of an
//! `argb8888` word are blue, green, red, alpha, which is what `B8G8R8A8_UNORM` reads.

mod capture;
mod context;
mod dmabuf;
mod downsample;
#[cfg(test)]
mod driver_probe;
mod error;
mod external;
mod formats;
mod frame;
#[cfg(test)]
mod harness;
mod memory;
mod pipeline;
mod plan;
mod shader;
mod spatial;
mod subtree;
mod sync;
#[cfg(test)]
mod tests;
mod textures;
mod view;

gnoblin_log::logger!(log = "render");

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use gnoblin_types::{Scale, Size};

pub use context::{DeviceChoice, DeviceInfo, GpuKind, Loader, Validation, VulkanOptions};
pub use dmabuf::DmabufError;
pub use error::VulkanError;
pub use external::{SYNC_READ, SYNC_WRITE, export_sync_file, import_sync_file, wait_for_writer};
pub use formats::dmabuf_formats;
pub use pipeline::PipelineCacheLocation;

use context::VkContext;
use frame::{RenderTarget, Saved};
use memory::{VkBuffer, VkImage};
use pipeline::{Compiled, Pipelines, ShaderPipelines, ShaderSources};
use spatial::{DepthTarget, Extras};
use subtree::ViewStore;
use sync::{FrameRing, MIN_BUFFER, RetireQueue};
use textures::TextureCache;

use ash::vk;
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::footprint::{COLOUR_BYTES_PER_PIXEL, Footprint, image_bytes};
use gnoblin_render_frame::frame::OutputFrame;
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::redraw::RedrawSignal;

/// What the renderer has done, which a test reads to prove that a frame did nothing, or did only what it had to.
///
/// The counts are of the device: every output on it adds to them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Queue submissions: a frame, a capture, the clear of a new target. A frame that has nothing to draw submits none.
    pub submits: u64,
    /// Bytes of client pixels copied into staging for textures. A texture that is current costs none, and one that a commit
    /// damaged costs the damaged rectangles.
    pub bytes_uploaded: u64,
    /// Textures uploaded, whole or in part.
    pub uploads: u64,
    /// Times the allocator took memory from the driver. A steady session adds none.
    pub device_allocations: u64,
    /// Pipelines built: the shipped ones, then each of a shell's.
    pub pipelines_built: u64,
    /// Client textures in the cache.
    pub textures: u64,
    /// Images destroyed since the device was made.
    pub images_destroyed: u64,
    /// Frames submitted and not yet finished.
    pub frames_in_flight: u64,
    /// Objects held until the frames that used them have finished.
    pub retiring: u64,
    /// Bytes of device memory handed to buffers and images and not given back.
    pub allocated_bytes: u64,
    /// Bytes of device memory the allocator holds from the driver, used or not.
    pub device_memory: u64,
    /// Whether the pipeline cache was read from its file when the device was made.
    pub pipeline_cache_loaded: bool,
}

/// One physical GPU, shared by every output on it.
///
/// It owns what is the GPU's and not an output's: the Vulkan device, the allocator, the compiled pipelines and the shell's
/// shader worker, the cache of client textures, and the timeline every submission signals. Outputs hold it by `Rc`, so it
/// outlives them all.
pub struct VulkanDevice {
    // The fields come in the order they are dropped: what holds device objects before the context they were made on.
    /// Objects waiting for the frames that used them to finish.
    retire: RefCell<RetireQueue>,
    /// The textures of client buffers, by surface.
    textures: RefCell<TextureCache>,
    /// The pictures of Views, drawn from the scenes frames carry.
    views: RefCell<ViewStore>,
    /// The pipelines of the shell's shader passes, made when a frame first draws a source.
    shaders: RefCell<ShaderPipelines>,
    pipelines: Pipelines,
    /// The depth-stencil pass and the pipelines of 3D layers, Views and the whole-target passes; `None` on a device with no
    /// depth-stencil format to draw into.
    extras: Option<Extras>,
    /// The unit quad every draw scales to its rectangle.
    quad: VkBuffer,
    /// Set when the device was lost; every output on it returns [`RenderError::Lost`].
    lost: Cell<bool>,
    /// When the device was made: a shader pass's time `t` is the seconds since.
    epoch: Instant,
    context: Rc<VkContext>,
}

impl VulkanDevice {
    /// The best device the machine has.
    ///
    /// # Errors
    ///
    /// A [`VulkanError`] naming why there is no renderer: no loader, no device, a shader that
    /// does not compile, or a driver call that failed.
    pub fn new() -> Result<Rc<Self>, VulkanError> {
        Self::with_options(&VulkanOptions::default())
    }

    /// A device chosen and set up as `options` say.
    ///
    /// # Errors
    ///
    /// The same as [`VulkanDevice::new`].
    pub fn with_options(options: &VulkanOptions) -> Result<Rc<Self>, VulkanError> {
        Self::with_sources(options, &ShaderSources::shared())
    }

    /// A device whose passes are compiled from `sources`.
    fn with_sources(options: &VulkanOptions, sources: &ShaderSources<'_>) -> Result<Rc<Self>, VulkanError> {
        // Compiled first: a pass that does not compile is refused before any driver is asked.
        let compiled = Compiled::new(sources)?;
        let context = Rc::new(VkContext::create(options)?);
        let pipelines = Pipelines::new(&context, &compiled, &options.pipeline_cache)?;
        let mut quad = VkBuffer::new(&context, 32, vk::BufferUsageFlags::VERTEX_BUFFER, false)
            .map_err(|result| VulkanError::driver("create the quad", result))?;
        // The unit quad as a triangle strip: the vertex stage scales it to the rectangle.
        let corners: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let bytes: Vec<u8> = corners.iter().flat_map(|value| value.to_ne_bytes()).collect();
        quad.write(0, &bytes);
        let extras = match context.depth_format {
            Some(format) => Some(Extras::new(&context, format)?),
            None => None,
        };
        Ok(Rc::new(Self {
            retire: RefCell::new(RetireQueue::default()),
            textures: RefCell::new(TextureCache::default()),
            views: RefCell::new(ViewStore::default()),
            shaders: RefCell::new(ShaderPipelines::new(&context, &pipelines)),
            extras,
            pipelines,
            quad,
            lost: Cell::new(false),
            epoch: Instant::now(),
            context,
        }))
    }

    /// The device the renderer draws on.
    #[must_use]
    pub fn info(&self) -> &DeviceInfo {
        &self.context.info
    }

    /// The names of the dmabuf extensions the device has and the renderer enabled: what it could import tiled buffers or export
    /// fences with. The linear path needs none.
    #[must_use]
    pub fn dmabuf_extensions(&self) -> Vec<String> {
        self.context
            .extensions
            .iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }

    /// Whether the device was lost, after which every output on it returns [`RenderError::Lost`].
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.lost.get()
    }

    /// What the device has done: see [`Stats`].
    #[must_use]
    pub fn stats(&self) -> Stats {
        let counters = &self.context.memory.counters;
        Stats {
            submits: counters.submits.get(),
            bytes_uploaded: counters.uploaded.get(),
            uploads: counters.uploads.get(),
            device_allocations: counters.device_allocations.get(),
            pipelines_built: self.pipelines.built(),
            textures: self.textures.borrow().len() as u64,
            images_destroyed: counters.images_destroyed.get(),
            frames_in_flight: self
                .context
                .timeline
                .submitted()
                .saturating_sub(self.context.completed()),
            retiring: self.retire.borrow().len() as u64,
            allocated_bytes: counters.allocated.get(),
            device_memory: self.context.memory.capacity(),
            pipeline_cache_loaded: self.pipelines.cache_loaded(),
        }
    }

    /// The bytes the device holds that are not an output's: the textures of client buffers, as `scratch`.
    ///
    /// An output's own [`Vulkan::footprint`] includes this, so a session with one output counts it once; a backend that
    /// shares a device between outputs counts the device with this and the outputs with [`Vulkan::footprint`] less it.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        Footprint {
            scratch: self
                .textures
                .borrow()
                .bytes()
                .saturating_add(self.views.borrow().bytes()),
            ..Footprint::default()
        }
    }

    /// The largest edge, in pixels, of an image the device can create: what bounds the target of a supersampled output.
    #[must_use]
    pub fn max_image_edge(&self) -> u32 {
        self.context.limits.max_image_dimension2_d
    }

    /// Whether the device draws a View's subtree into an image: it has a depth-stencil format to draw them with. A backend
    /// answers `draws_views` from this, because a device without one skips every View layer.
    #[must_use]
    pub fn draws_views(&self) -> bool {
        self.extras.is_some()
    }

    /// Whether the validation layer is on for this device.
    #[must_use]
    pub fn validation_enabled(&self) -> bool {
        self.context.validating
    }

    /// What the validation layer reported since the last call: errors, and validation warnings.
    ///
    /// Empty when the layer is off.
    pub fn take_validation_messages(&self) -> Vec<String> {
        self.context.take_messages()
    }

    /// Gives back every cached client texture, which a session under memory pressure asks of its renderer. The next frame
    /// that draws a surface uploads it again, whole. The textures are destroyed once the frames that drew them have
    /// finished.
    pub fn release_textures(&self) {
        let mut retire = self.retire.borrow_mut();
        self.textures.borrow_mut().clear(&mut retire);
    }

    /// Marks the device lost and drops every shader pipeline: nothing is drawn on it again.
    fn lose(&self) {
        if !self.lost.replace(true) {
            self.shaders.borrow_mut().drain();
        }
    }
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        // A frame that timed out may still be running; nothing below may be destroyed under it.
        self.context.wait_idle();
        self.retire.borrow_mut().clear();
    }
}

impl std::fmt::Debug for VulkanDevice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VulkanDevice")
            .field("device", &self.context.info)
            .field("lost", &self.lost.get())
            .finish_non_exhaustive()
    }
}

/// A Vulkan renderer for one output: the offscreen image it draws into and the frame slots it records with, on a
/// [`VulkanDevice`] it shares with the other outputs of the GPU.
pub struct Vulkan {
    // The fields that hold device objects come before the device they were made on.
    ring: FrameRing,
    /// The slot of the frame being recorded.
    active: usize,
    target: RenderTarget,
    /// What a capture copies the target into, which it waits for.
    readback: VkBuffer,
    /// The pixels the cursor covers, saved for a capture that leaves it out.
    cursor_under: Saved,
    physical: Size,
    /// Set when the device was lost; every call after that returns [`RenderError::Lost`].
    lost: bool,
    /// Whether the warning that this device draws no 3D layer has been logged, so a steady session logs it once.
    warned_models: bool,
    /// Whether the filter-blur warning has been logged.
    warned_filter_blur: bool,
    /// Whether the last frame drew the magnifier or the colour matrix, which makes the next one repaint the whole output.
    filtered: bool,
    /// The depth-stencil image the output's 3D layers are tested against, made when a frame first has one.
    depth: Option<DepthTarget>,
    /// The copy of the finished output the view pass samples.
    scratch: Option<VkImage>,
    /// The target [`Vulkan::downsampled`] averages into.
    shown: Option<RenderTarget>,
    /// The signal the shell's shader worker asks this output for another frame on.
    redraw: RedrawSignal,
    device: Rc<VulkanDevice>,
}

/// The renderer of one output, under the name the design uses for it.
pub type VulkanOutput = Vulkan;

impl Vulkan {
    /// A renderer with a target of `physical` pixels, on the best device, which it opens for itself.
    ///
    /// # Errors
    ///
    /// A [`VulkanError`] naming why there is no renderer: no loader, no device, a shader that
    /// does not compile, or a driver call that failed.
    pub fn new(physical: Size) -> Result<Self, VulkanError> {
        Self::with_options(physical, &VulkanOptions::default())
    }

    /// A renderer created with `options`, on a device it opens for itself.
    ///
    /// # Errors
    ///
    /// The same as [`Vulkan::new`].
    pub fn with_options(physical: Size, options: &VulkanOptions) -> Result<Self, VulkanError> {
        Self::with_sources(physical, options, &ShaderSources::shared())
    }

    /// A renderer whose passes are compiled from `sources`.
    fn with_sources(physical: Size, options: &VulkanOptions, sources: &ShaderSources<'_>) -> Result<Self, VulkanError> {
        if physical.is_empty() {
            return Err(VulkanError::Degenerate {
                width: physical.width,
                height: physical.height,
            });
        }
        let device = VulkanDevice::with_sources(options, sources)?;
        Self::with_device(&device, physical)
    }

    /// A renderer for one output with a target of `physical` pixels, on a device other outputs may share.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Degenerate`] for a size with no pixels, and [`VulkanError::Driver`] when the target or the frame
    /// slots cannot be made.
    pub fn with_device(device: &Rc<VulkanDevice>, physical: Size) -> Result<Self, VulkanError> {
        if physical.is_empty() {
            return Err(VulkanError::Degenerate {
                width: physical.width,
                height: physical.height,
            });
        }
        let context = &device.context;
        let ring = FrameRing::new(context).map_err(|result| VulkanError::driver("create the frame slots", result))?;
        let target = RenderTarget::new(context, device.pipelines.render_pass, physical)
            .map_err(|result| VulkanError::driver("create the target", result))?;
        let readback = VkBuffer::new(context, MIN_BUFFER, vk::BufferUsageFlags::TRANSFER_DST, true)
            .map_err(|result| VulkanError::driver("create the readback", result))?;
        let redraw = RedrawSignal::new();
        device.shaders.borrow().listen(redraw.clone());
        let mut output = Self {
            ring,
            active: 0,
            target,
            readback,
            cursor_under: Saved::default(),
            physical,
            lost: false,
            warned_models: false,
            warned_filter_blur: false,
            filtered: false,
            depth: None,
            scratch: None,
            shown: None,
            redraw,
            device: Rc::clone(device),
        };
        let image = output.target.image.raw();
        output
            .clear_target(image)
            .map_err(|result| VulkanError::driver("clear the target", result))?;
        Ok(output)
    }

    /// The signal this renderer asks for another frame on: a shell's shader pipeline finished building on the worker thread,
    /// so a node drawn without its pass can be drawn with it. The backend asks the target (`Target::take_redraw`) and
    /// damages the output; it may install a waker to wake its loop.
    #[must_use]
    pub fn redraw_signal(&self) -> RedrawSignal {
        self.redraw.clone()
    }

    /// The target's size in physical pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.physical
    }

    /// The bytes this renderer holds: the target image, the host-visible buffers (the frame slots' staging, uniforms and
    /// instances, the readback and the pixels under the cursor) it keeps between frames, and the client textures its device
    /// caches (as `scratch`).
    ///
    /// The staging and readback buffers are trimmed after every frame to what one steady frame needs, so a number
    /// above that is a frame in flight and not a ratchet. The textures are the device's: an output on a device with others
    /// reports them too.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        Footprint {
            target: image_bytes(self.physical, COLOUR_BYTES_PER_PIXEL),
            staging: self.host_bytes(),
            ..self.device.footprint()
        }
    }

    /// The largest edge, in pixels, of an image the device can create: what bounds the target of a supersampled output.
    #[must_use]
    pub fn max_image_edge(&self) -> u32 {
        self.device.max_image_edge()
    }

    /// The device the renderer draws on.
    #[must_use]
    pub fn device(&self) -> &DeviceInfo {
        self.device.info()
    }

    /// The GPU this output draws on, which other outputs may share.
    #[must_use]
    pub fn gpu(&self) -> &Rc<VulkanDevice> {
        &self.device
    }

    /// What the device has done: see [`Stats`].
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.device.stats()
    }

    /// Whether the device was lost, after which every call returns [`RenderError::Lost`].
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.lost || self.device.is_lost()
    }

    /// Whether the validation layer is on for this renderer.
    #[must_use]
    pub fn validation_enabled(&self) -> bool {
        self.device.validation_enabled()
    }

    /// What the validation layer reported since the last call: errors, and validation warnings.
    ///
    /// Empty when the layer is off.
    pub fn take_validation_messages(&self) -> Vec<String> {
        self.device.take_validation_messages()
    }

    /// Draws `frame`, repainting only what it damaged, and returns when the frame is on the queue: it does not wait for the
    /// GPU, which finishes it while the next one is recorded.
    ///
    /// `scale` is the output's compositing scale: a frame's rectangles are logical and the
    /// target is physical.
    ///
    /// # Errors
    ///
    /// [`RenderError::Lost`] when the device is gone, which is permanent for this renderer, and
    /// [`RenderError::Failed`] when the driver refused the frame. A client buffer that cannot
    /// be read or a texture that cannot be allocated is not an error: that layer is skipped and
    /// the rest of the frame is drawn.
    pub fn draw(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), RenderError> {
        if self.is_lost() {
            return Err(RenderError::Lost);
        }
        if frame.is_empty() {
            return Ok(());
        }
        let painted = self.paint(frame, scale);
        self.collect();
        painted.map(|()| self.trim()).map_err(|result| self.fail(result))
    }

    /// Reads the target back as an [`Image`], without the cursor.
    ///
    /// # Errors
    ///
    /// The same as [`Vulkan::capture`].
    pub fn screenshot(&mut self) -> Result<Image, RenderError> {
        self.capture(false)
    }

    /// Reads the target back as an [`Image`], with the cursor composed in when `with_cursor`.
    ///
    /// The call waits for the read, which is every frame drawn before it and no later one.
    ///
    /// # Errors
    ///
    /// [`RenderError::Lost`] when the device is gone and [`RenderError::Failed`] when the
    /// readback is refused.
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        if self.is_lost() {
            return Err(RenderError::Lost);
        }
        let read = self.read_target(with_cursor);
        self.collect();
        read.inspect(|_| self.trim()).map_err(|result| self.fail(result))
    }

    /// Replaces the target with one of `physical` pixels.
    ///
    /// A new target holds no pixels, so the caller damages the whole output afterwards.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels, and the errors of a frame. The
    /// old target is kept on failure.
    pub fn resize(&mut self, physical: Size) -> Result<(), RenderError> {
        if physical.is_empty() {
            return Err(RenderError::Degenerate {
                width: physical.width,
                height: physical.height,
            });
        }
        if self.is_lost() {
            return Err(RenderError::Lost);
        }
        if physical == self.physical {
            return Ok(());
        }
        let target = RenderTarget::new(&self.device.context, self.device.pipelines.render_pass, physical)
            .map_err(|result| self.fail(result))?;
        // Frames submitted so far may still draw into the old target.
        let before = self.device.context.timeline.submitted();
        if let Err(result) = self.clear_target(target.image.raw()) {
            return Err(self.fail(result));
        }
        let old = std::mem::replace(&mut self.target, target);
        let mut retire = self.device.retire.borrow_mut();
        retire.push(before, old);
        // The depth image's framebuffer is over the old target's view, and the scratch copy is the old size.
        if let Some(depth) = self.depth.take() {
            retire.push(before, depth);
        }
        if let Some(scratch) = self.scratch.take() {
            retire.push(before, scratch);
        }
        self.physical = physical;
        self.cursor_under.rect = None;
        Ok(())
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        // A frame still in flight uses this output's target and buffers; nothing below may be destroyed under it. A device
        // that cannot answer is lost, and has nothing running.
        let _ = self.device.context.wait(self.ring.newest());
    }
}

impl std::fmt::Debug for Vulkan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vulkan")
            .field("device", self.device.info())
            .field("physical", &self.physical)
            .field("lost", &self.is_lost())
            .finish_non_exhaustive()
    }
}
