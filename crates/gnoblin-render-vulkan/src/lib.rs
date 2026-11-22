//! The Vulkan renderer: an [`OutputFrame`] drawn through `ash` into an offscreen image and read
//! back as an [`Image`].
//!
//! This is the third implementation of the frame description (see `gnoblin-render-frame`), built on `ash` and
//! `naga`. Slice 1 is the offscreen renderer:
//! it draws what the software renderer draws, plus the rounded corners the shared shaders
//! already clip, and it reads the result back. It does not present, import dmabufs or draw the
//! shipped effects (blur, shadow, border) yet; the design note lists what is next and in what order.
//!
//! # What it draws
//!
//! - **Solid layers** and the background, with the layer's tint and corner radius.
//! - **Client buffers**: `argb8888` and `xrgb8888` `wl_shm` buffers, cropped by the layer's
//!   source rectangle and scaled with linear filtering. The pixels are premultiplied, as the
//!   protocol says a shm buffer's are, and are blended with `ONE, ONE_MINUS_SRC_ALPHA`.
//! - **Opacity** and **tint** per layer, in the same shader the GLES renderer runs.
//! - **The portal overlay and the cursor**, nearest-sampled, with the pixels under each saved
//!   so a capture can leave the overlay (always) and the cursor (when asked) out.
//! - **Output scale**: a frame's rectangles are logical and the target is physical, converted
//!   with the one rounding rule all renderers share ([`physical_rect`](gnoblin_render_frame::geometry)).
//! - **Damage**: only the damage is repainted. Each rectangle is a scissor, and overlapping
//!   rectangles are made disjoint first so a translucent layer is never blended twice.
//!
//! Layers carrying a 3D model, the blur and shadow passes and a buffer's
//! `set_buffer_transform` are not drawn, which is what the GLES renderer does for the last of
//! them and what the design note schedules for the others. The `shader` pass is drawn (below).
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
//! build a new renderer and damage the whole output.
//!
//! # Failure modes
//!
//! 1. **No Vulkan loader.** The loader is opened with `dlopen`, so a machine without one gets
//!    [`VulkanError::NoLoader`] instead of a binary that will not start:
//!    `tests::a_missing_loader_is_a_named_error`.
//! 2. **No suitable device.** A machine whose loader lists no device, or only ones that cannot
//!    draw, gets [`VulkanError::NoDevice`] with the reason for each:
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
//!    installed, every drawing test fails on anything it reports, and a deliberate misuse proves
//!    the collector sees it: `tests::validation_reports_a_driver_misuse`.
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
//!     into a retired list that is destroyed once the frame's fence has been waited, and a lost
//!     device or a dropped renderer drains everything after waiting for the queue:
//!     `tests::shader::the_pipelines_are_kept_by_source_and_the_least_recently_used_leave_after_64`,
//!     `tests::shader::a_lost_device_drains_the_pipelines`.
//!
//! # Byte order
//!
//! A little-endian host is assumed where a client's pixels are uploaded: the bytes of an
//! `argb8888` word are blue, green, red, alpha, which is what `B8G8R8A8_UNORM` reads.

mod context;
#[cfg(test)]
mod driver_probe;
mod error;
mod frame;
#[cfg(test)]
mod harness;
mod pipeline;
mod plan;
mod shader;
#[cfg(test)]
mod tests;

gnoblin_log::logger!(log = "render");

use std::rc::Rc;
use std::time::Instant;

use gnoblin_types::{Scale, Size};

pub use context::{DeviceChoice, DeviceInfo, GpuKind, Loader, Validation, VulkanOptions};
pub use error::VulkanError;

use context::VkContext;
use frame::{FrameRecorder, RenderTarget, Saved};
use pipeline::{Compiled, Pipelines, ShaderPipelines, ShaderSources};

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::footprint::{COLOUR_BYTES_PER_PIXEL, Footprint, image_bytes};
use gnoblin_render_frame::frame::OutputFrame;
use gnoblin_render_frame::image::Image;

/// A Vulkan renderer and the offscreen image it draws into.
pub struct Vulkan {
    // The fields that hold device objects come before the context they were made on.
    recorder: FrameRecorder,
    /// The pipelines of the shell's shader passes, made when a frame first draws a source.
    shaders: ShaderPipelines,
    pipelines: Pipelines,
    target: RenderTarget,
    /// The pixels the cursor covers, saved for a capture that leaves it out.
    cursor_under: Saved,
    physical: Size,
    /// Set when the device was lost; every call after that returns [`RenderError::Lost`].
    lost: bool,
    /// Whether the 3D warning has been logged, so a steady session logs it once.
    warned_models: bool,
    /// When the renderer was made: a shader pass's time `t` is the seconds since.
    epoch: Instant,
    context: Rc<VkContext>,
}

impl Vulkan {
    /// A renderer with a target of `physical` pixels, on the best device.
    ///
    /// # Errors
    ///
    /// A [`VulkanError`] naming why there is no renderer: no loader, no device, a shader that
    /// does not compile, or a driver call that failed.
    pub fn new(physical: Size) -> Result<Self, VulkanError> {
        Self::with_options(physical, &VulkanOptions::default())
    }

    /// A renderer created with `options`.
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
        // Compiled first: a pass that does not compile is refused before any driver is asked.
        let compiled = Compiled::new(sources)?;
        let context = Rc::new(VkContext::create(options)?);
        let pipelines = Pipelines::new(&context, &compiled)?;
        let mut recorder =
            FrameRecorder::new(&context).map_err(|result| VulkanError::driver("create the recorder", result))?;
        let target = RenderTarget::new(&context, pipelines.render_pass, physical)
            .map_err(|result| VulkanError::driver("create the target", result))?;
        recorder
            .clear(&target)
            .map_err(|result| VulkanError::driver("clear the target", result))?;
        Ok(Self {
            recorder,
            shaders: ShaderPipelines::new(&context, &pipelines),
            pipelines,
            target,
            cursor_under: Saved::default(),
            physical,
            lost: false,
            warned_models: false,
            epoch: Instant::now(),
            context,
        })
    }

    /// The signal this renderer asks for another frame on: a shell's shader pipeline finished building on the worker thread,
    /// so a node drawn without its pass can be drawn with it. The backend asks the target (`Target::take_redraw`) and
    /// damages the output; it may install a waker to wake its loop.
    #[must_use]
    pub fn redraw_signal(&self) -> gnoblin_render_frame::redraw::RedrawSignal {
        self.shaders.redraw_signal()
    }

    /// The target's size in physical pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.physical
    }

    /// The bytes this renderer holds: the target image, and the host-visible buffers (staging, readback, uniforms,
    /// instances and the pixels under the cursor) it keeps between frames.
    ///
    /// The staging and readback buffers are trimmed after every frame to what one steady frame needs, so a number
    /// above that is a frame in flight and not a ratchet.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        Footprint {
            target: image_bytes(self.physical, COLOUR_BYTES_PER_PIXEL),
            staging: self.host_bytes(),
            ..Footprint::default()
        }
    }

    /// The largest edge, in pixels, of an image the device can create: what bounds the target of a supersampled output.
    #[must_use]
    pub fn max_image_edge(&self) -> u32 {
        self.context.limits.max_image_dimension2_d
    }

    /// The device the renderer draws on.
    #[must_use]
    pub fn device(&self) -> &DeviceInfo {
        &self.context.info
    }

    /// Whether the device was lost, after which every call returns [`RenderError::Lost`].
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// Whether the validation layer is on for this renderer.
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

    /// Draws `frame`, repainting only what it damaged.
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
        if self.lost {
            return Err(RenderError::Lost);
        }
        if frame.is_empty() {
            return Ok(());
        }
        let painted = self.paint(frame, scale);
        if painted.is_err() {
            // A frame that failed may still be on the queue; nothing a pipeline evicted during it is destroyed until it is not.
            self.context.wait_idle();
        }
        self.shaders.release_retired();
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
    /// # Errors
    ///
    /// [`RenderError::Lost`] when the device is gone and [`RenderError::Failed`] when the
    /// readback is refused.
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        if self.lost {
            return Err(RenderError::Lost);
        }
        self.read_target(with_cursor)
            .inspect(|_| self.trim())
            .map_err(|result| self.fail(result))
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
        if self.lost {
            return Err(RenderError::Lost);
        }
        if physical == self.physical {
            return Ok(());
        }
        let target = RenderTarget::new(&self.context, self.pipelines.render_pass, physical)
            .map_err(|result| self.fail(result))?;
        if let Err(result) = self.recorder.clear(&target) {
            return Err(self.fail(result));
        }
        self.target = target;
        self.physical = physical;
        self.cursor_under.rect = None;
        Ok(())
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        // A frame that timed out may still be running; nothing below may be destroyed under it.
        self.context.wait_idle();
    }
}

impl std::fmt::Debug for Vulkan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vulkan")
            .field("device", &self.context.info)
            .field("physical", &self.physical)
            .field("lost", &self.lost)
            .finish_non_exhaustive()
    }
}
