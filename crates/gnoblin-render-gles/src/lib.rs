//! The GLES renderer: Smithay's GLES context, `glow` for the drawing, one frame in and an
//! [`Image`] out.
//!
//! This is the GPU renderer, drawn on Smithay's GLES renderer
//! ([`GlowRenderer`], the `renderer_gl` and `renderer_glow` features of the pinned
//! revision) against a *surfaceless* EGL context, so it runs with no display and no
//! session. It implements the same contract as the software path: it draws an
//! [`OutputFrame`] into an offscreen target, respects the frame's damage, and reads the
//! result back as an [`Image`] in the same orientation and colour space.
//!
//! # Why the frame is drawn here rather than through Smithay's frame API
//!
//! Smithay's `GlesFrame` compiles the fragment source it is handed against its own
//! `#version 100` vertex shader, so it can only run a shader written in GLSL ES 1.00 — and
//! `naga`, the shader compiler, has no GLSL ES 1.00 output. Every effect
//! is written *once* in one language and compiled per backend, so the
//! language is WGSL and the passes are compiled here: `naga` validates a module and emits
//! GLSL ES 3.10, and `glow` (the safe GL API Smithay's `renderer_glow` feature hands out)
//! compiles and draws it. Smithay still owns the EGL context and the import of a client's
//! shared-memory buffer, which is where the protocol rules and the unsafe FFI live.
//!
//! # The shader language: WGSL
//!
//! Chosen in phase 0 for three reasons, in order:
//!
//! 1. It is `naga`'s native language, so validation is complete rather than partial: a type
//!    mismatch, an unbound identifier or a bad swizzle is refused with a line before the
//!    driver ever sees it.
//! 2. It is the language a Vulkan backend takes unchanged (`naga` emits SPIR-V from the same
//!    module), which is what "written once, compiled per backend" needs.
//! 3. Its uniform layout rules agree with GLSL's `std140` for every member of the block, so
//!    one set of byte offsets describes both targets. `Uniforms::pack` is the CPU side of
//!    that agreement.
//!
//! The camera is orthographic and drawing is in painter's order, which is the 2D subset the
//! scene accepts today; the transform is a rectangle, exactly as the plan states for this
//! phase.
//!
//! # What a pass is
//!
//! A pass draws one quad. The vertex stage is shared ([`shaders::VERTEX`]) and takes the
//! destination rectangle and the target's size as attributes, so the fragment stage is the
//! only stage with a uniform block: one block per shipped program, bound once, overwritten per
//! draw. That is what makes "uniforms typed-checked and uploaded" a single small write of a
//! 112-byte buffer. A shell's shader program has a second block, its own, beside it.
//!
//! | Pass    | Shader                 | Draws                                        |
//! | ------- | ---------------------- | -------------------------------------------- |
//! | solid   | [`shaders::SOLID`]     | the background, a solid layer, the cursor     |
//! | surface | [`shaders::SURFACE`]   | a client buffer, tinted and corner-clipped    |
//! | shadow  | [`shaders::SHADOW`]    | a [`Pass::Shadow`], behind its layer          |
//! | inset shadow | [`shaders::INSET_SHADOW`] | a [`Pass::Shadow`] with `inset`, inside its layer |
//! | blur    | [`shaders::BLUR`]      | one axis of a Gaussian over the backdrop, or over a filtered layer's picture |
//! | downsample | [`shaders::DOWNSAMPLE`] | a supersampled target, averaged to the mode |
//! | shader  | a shell's source        | a [`Pass::Shader`], in place of the layer's pixels |
//!
//! A layer's effects (shadows, backdrop blur, background, inset shadows, filter and borders) are drawn in a fixed order by
//! kind, whatever order they are listed in; [`effects`] has the order and the rules of the filter's picture and the
//! blur's downscale and grain.
//!
//! # Where the pixels go
//!
//! The target is an `RGBA8` texture. The vertex stage maps pixel `y` downwards while clip
//! space is upwards, so row 0 of the target is the output's top row; a pass that samples the
//! target (the blur) therefore states its source with a negative v scale
//! ([`flipped_mapping`]). `glReadPixels` counts from the *bottom* of a framebuffer, so
//! [`Gles::screenshot`] reverses the rows it gets. That flip is where a readback can come out
//! upside down, and `tests::a_readback_is_top_left_first` is what catches it. A scan-out buffer
//! is the other side of the same rule: it is a dmabuf whose GL row 0 is its first memory row,
//! which the display scans out as the top line, so [`Gles::draw_scanout`] copies the target into
//! it with the rows reversed (`downsample_tests::a_frame_drawn_into_a_scan_out_buffer_is_top_first`).
//!
//! # Colour
//!
//! Every pass blends with `ONE, ONE_MINUS_SRC_ALPHA`, so every pass outputs *premultiplied*
//! colour. A client's `wl_shm` buffer is already premultiplied (that is what the protocol
//! says a shm buffer holds) and a straight colour is premultiplied in its shader. No
//! transfer function is applied: the readback is the stored sRGB values, which is what makes
//! a probe comparable with the software path's.
//!
//! # Failure modes
//!
//! 1. **A readback is upside down.** `glReadPixels` returns the bottom row first; the image
//!    is flipped back: `tests::a_readback_is_top_left_first`.
//! 2. **An idle frame costs a device round trip.** [`Gles::draw`] returns before any GL call
//!    for a frame with no damage, and the draw counter proves it:
//!    `tests::an_idle_frame_draws_nothing`.
//! 3. **A shadow leaks outside the damage.** Every pass is scissored to the damage it is
//!    allowed to touch, so a shadow whose padding reaches beyond it leaves those pixels
//!    alone: `tests::a_shadow_does_not_leave_the_damage`.
//! 4. **A blur samples outside the output and reads garbage.** The scratch copy is cleared
//!    to the frame's background, the copy is clamped to the output and the texture clamps at
//!    its edge, so a blur at the boundary repeats the boundary pixel:
//!    `tests::a_blur_at_the_edge_repeats_the_edge`.
//! 5. **GL state leaks from one frame into the next.** Every frame begins from the state
//!    [`GlContext::begin`] sets and every pass resets what it changes:
//!    `tests::a_frame_does_not_inherit_the_last_frames_state`.
//! 6. **Damage is ignored and the whole output is repainted.** Only the damage is scissored
//!    in, so an undamaged pixel keeps what it had:
//!    `tests::damage_limits_what_is_repainted`.
//! 7. **A resize keeps the old target.** [`Gles::resize`] replaces the target and the new one
//!    holds no pixels: `tests::a_resize_replaces_the_target`.
//! 8. **A supersampled output shows an unpainted or half-averaged target.** The downsample pass
//!    ([`shaders::DOWNSAMPLE`]) reads the whole virtual target on every flip, so a target that was
//!    just made is painted whole before it is averaged ([`Gles::draw_scanout_downsampled`]), and the
//!    scan-out buffer is written whole because it holds an old frame:
//!    `downsample_tests::a_supersampled_frame_is_averaged_into_the_scan_out_buffer`.
//! 9. **A target is asked for past the driver's texture limit.** The size is checked against
//!    `GL_MAX_TEXTURE_SIZE` before any object is made, and a refused resize keeps the old target:
//!    `downsample_tests::a_target_past_the_drivers_limit_is_refused_and_the_old_one_stays`.
//! 10. **A shell's shader reads a secure window.** A shader that calls `backdrop` samples a texture this renderer drew
//!     from [`OutputFrame::backdrop_input`] into a target of its own, never the frame's target, which shows the secure
//!     layers: `shader_tests::a_secure_window_reads_as_black_in_a_backdrop` and
//!     `shader_tests::the_lock_card_reads_as_black_in_a_backdrop`.
//! 11. **A shader that does not compile stops the frame.** The failure is cached with the source's key, the layer is
//!     drawn without the pass and the pass is reported refused:
//!     `shader_tests::a_broken_user_shader_leaves_the_frame_drawn`.
//! 12. **A shader that was compiled by a lost context is used.** A failed frame drops every compiled program
//!     ([`Gles::drain_shaders`]), and the next frame that needs one compiles it again:
//!     `shader_tests::a_lost_device_drops_every_compiled_shader_and_the_next_frame_compiles_again`.
//! 13. **A shader's cost is measured by stalling the compositor.** A draw is measured by a timer query or a polled fence
//!     ([`timing`]) and a frame with shaders makes no `glFinish` and no readback:
//!     `shader_tests::a_frame_with_shaders_never_waits_for_the_gpu`.
//!
//! # The shell's shader pass
//!
//! A [`Pass::Shader`] draws in place of the layer's own pixels with a program compiled from the shell's source
//! ([`program::link_user`]): two uniform blocks (group 0, bindings 0 and 1), the layer's pixels on unit 0 (group 1), and
//! the backdrop on unit 1 (group 2) when the source calls `backdrop`. `naga` is told these numbers through
//! [`gl::binding_map`], so the GLSL it writes states them and a draw only uploads bytes. Programs live in a
//! [`ShaderCache`] keyed by [`gnoblin_render::ShaderKey`]; unlike the shipped passes they are deleted when the cache
//! evicts one.
//!
//! The backdrop is built before the frame's own layers are painted ([`Gles::build_backdrops`]): the layers below the
//! shader's layer are drawn into an output-sized target of their own and the layer's rectangle is copied, top row first,
//! into a texture of that size. A layer below that reads a backdrop in turn is built the same way, to
//! [`MAX_BACKDROP_DEPTH`] levels. Targets for backdrops and for the pictures between chained passes come from the
//! context's pool ([`gl::GlContext::lease`]).

gnoblin_log::logger!(log = "render");

pub mod egl;
pub(crate) mod gl;
pub(crate) mod program;

use gnoblin_render_frame::shader::sources as shaders;

use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::glow::GlowRenderer;
use smithay::backend::renderer::{ImportMem, ImportMemWl};
use smithay::utils::{Physical, Rectangle};

use effects::Paint;
use gnoblin_render::{Mat4, Pass, ShaderPass};
use gnoblin_types::{Color, Rect, RectF, Scale, Size};

use gl::{Depth, Framebuffer, GlContext, Program, QuadPlacement, Texture, UserInputs};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::footprint::Footprint;
use gnoblin_render_frame::frame::{ColourMatrix, Layer, Model, OutputFrame, Overlay, Surface, SurfaceSource, View};
use gnoblin_render_frame::geometry::physical_rect;
use gnoblin_render_frame::grade::Grade;
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::image::READBACK_FORMAT;
use gnoblin_render_frame::shader::uniforms::{Uniforms, UserBlock};
use gnoblin_render_frame::shader::{ShaderCache, ShaderError, ShaderTimings};
use std::time::Instant;
use timing::{ShaderClock, TimerMode};

/// The largest corner radius or blur radius a pass draws, in physical pixels.
///
/// A radius larger than half the shorter side is clamped by the shader anyway; this bound
/// keeps the buffer arithmetic far from an overflow.
const MAX_RADIUS: f32 = 4096.0;

/// How many backdrops a frame builds inside one another. A layer whose shader reads the backdrop paints the layers
/// below it first; if one of those reads its own, that is a second picture, and each level multiplies the work by the
/// layers below it. Past this depth a layer's shader that reads the backdrop is not drawn.
const MAX_BACKDROP_DEPTH: u8 = 3;

/// A GLES renderer and the offscreen target it draws into.
pub struct Gles {
    /// The GL context and the pass machinery.
    device: GlContext,
    /// The output-sized target, replaced by [`Gles::resize`].
    target: Framebuffer,
    /// The two scratch targets the blur passes need, made on first use.
    scratch: Option<[Framebuffer; 2]>,
    /// The effect programs, compiled once.
    passes: Passes,
    /// The target's size in physical pixels.
    physical: Size,
    /// How many frames were drawn into the target. A frame with no damage leaves this alone,
    /// which is what makes an idle iteration free.
    draws: u64,
    /// The frame's damage in physical pixels, reused across frames so a steady frame
    /// allocates nothing.
    damage: Vec<Rect>,
    /// The pixels under the cursor, top first over the whole output, so a capture can
    /// leave the cursor out.
    cursor_under: Vec<u8>,
    /// The cursor's visible rectangle in the last frame, or `None` when it had none.
    cursor_under_rect: Option<Rectangle<i32, Physical>>,
    /// Whether the last frame left magnified or tinted pixels in the target.
    filtered: bool,
    /// The textures of the Views this renderer draws on the GPU.
    subtrees: subtree::Subtrees,
    /// Whether the target holds no frame yet: it was just made, so it is not damage that is missing from it but all
    /// of its pixels. A supersampled output averages the whole target down on every flip, so it must not average a
    /// target nobody painted; the first such frame repaints it whole.
    stale: bool,
    /// The target [`Gles::downsampled`] averages into and reads back, made on first use and replaced when the size
    /// asked for changes.
    shown: Option<Framebuffer>,
    /// The user shader passes' programs.
    shaders: UserShaders,
    /// How many backdrops deep the frame being painted is: a layer whose shader reads the backdrop paints the layers
    /// below it first, and one of those may read its own. See [`MAX_BACKDROP_DEPTH`].
    backdrop_depth: u8,
}

impl std::fmt::Debug for Gles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Gles")
            .field("physical", &self.physical)
            .field("draws", &self.draws)
            .finish_non_exhaustive()
    }
}

/// The effect programs a frame can draw with.
struct Passes {
    solid: Program,
    surface: Program,
    shadow: Program,
    border: Program,
    inset_shadow: Program,
    blur: Program,
    /// The whole-output view pass, run only when the frame has a view or a matrix.
    view: Program,
    /// The whole-output downsample pass of a supersampled output.
    downsample: Program,
}

/// The compiled shader programs of the shells' user shader passes and the clock they read.
struct UserShaders {
    /// Each source's program, or why it has none: a source that does not compile is compiled once and not once a frame.
    cache: ShaderCache<Result<Program, ShaderError>>,
    /// When this renderer was made: a shader's `t` is the seconds since.
    started: Instant,
    /// The measurements of the shader draws in flight.
    clock: ShaderClock<GlContext>,
}

/// One scan-out buffer of an output: the dmabuf imported as a GL texture and the framebuffer
/// that draws into it. Both are GL objects of the output's context, dropped with it.
pub struct ScanBuffer {
    /// The framebuffer the frame is drawn into.
    framebuffer: Framebuffer,
    /// The texture behind the framebuffer. Kept alive for as long as the framebuffer is used.
    _texture: GlesTexture,
}

impl Gles {
    /// A renderer with a target of `physical` pixels on a surfaceless EGL context.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels on an axis, and
    /// [`RenderError::Unavailable`] when no GLES context can be created on this machine — no
    /// EGL library, no surfaceless platform, or a driver that offers no GLES 3 context. The
    /// reason is logged where it happened.
    pub fn new(physical: Size) -> Result<Self, RenderError> {
        Self::on(egl::context()?, physical)
    }

    /// A renderer on `renderer`'s context, with a target of `physical` pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels on an axis, and
    /// [`RenderError::Unavailable`] when the context cannot hold the target or the passes.
    pub(crate) fn on(renderer: GlowRenderer, physical: Size) -> Result<Self, RenderError> {
        if physical.is_empty() {
            return Err(RenderError::Degenerate {
                width: physical.width,
                height: physical.height,
            });
        }
        let mut device = GlContext::on(renderer)?;
        Self::check_limit(&mut device, physical)?;
        let target = device.target(physical)?;
        let passes = Passes::new(&mut device)?;
        let timer_mode = if device.has_timer_queries() {
            TimerMode::Queries
        } else {
            TimerMode::Fences
        };
        Ok(Self {
            device,
            target,
            scratch: None,
            passes,
            physical,
            draws: 0,
            damage: Vec::new(),
            cursor_under: Vec::new(),
            cursor_under_rect: None,
            filtered: false,
            subtrees: subtree::Subtrees::default(),
            stale: true,
            shown: None,
            shaders: UserShaders {
                cache: ShaderCache::new(),
                started: Instant::now(),
                clock: ShaderClock::new(timer_mode),
            },
            backdrop_depth: 0,
        })
    }

    /// A renderer on a context made on `display`, with a target of `physical` pixels.
    ///
    /// The hardware path names the GBM display its scan-out buffers were allocated on; see
    /// [`egl::gbm_display`].
    ///
    /// # Errors
    ///
    /// As [`Gles::on`], plus [`RenderError::Unavailable`] when no GLES 3 context can be made on
    /// `display`.
    pub fn on_display(display: &smithay::backend::egl::EGLDisplay, physical: Size) -> Result<Self, RenderError> {
        Self::on(egl::context_on(display)?, physical)
    }

    /// The target's size in physical pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.physical
    }

    /// The bytes this renderer holds for its output: the target, the blur scratch pair and the downsample target
    /// when they were made, the pixels kept under the cursor, and the textures of the Views it draws.
    ///
    /// The sizes are what was asked of the driver (colour plus the depth or stencil buffer), not what the driver
    /// reports: GL has no portable query for that. A client's buffers are Smithay's and are not counted here.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        let blur = self
            .scratch
            .iter()
            .flatten()
            .map(Framebuffer::bytes)
            .fold(0, u64::saturating_add)
            .saturating_add(self.device.spare_bytes());
        let scratch = blur.saturating_add(self.cursor_under.capacity() as u64);
        Footprint {
            target: self
                .target
                .bytes()
                .saturating_add(self.shown.as_ref().map_or(0, Framebuffer::bytes)),
            scratch,
            staging: 0,
            views: self.subtree_bytes(),
            // The blur pair is made on first use and again after a resize (`ensure_scratch`), and holds nothing a
            // later frame reads; nor do the targets the user shader passes leased.
            releasable: blur,
        }
    }

    /// Gives back the blur scratch targets and the pool of backdrop and shader targets. The next frame that needs
    /// them makes them again.
    pub fn release_scratch(&mut self) {
        for scratch in self.scratch.take().into_iter().flatten() {
            self.device.release(scratch);
        }
        self.device.release_spare();
    }

    /// Deletes every compiled user shader, which the next frame that needs one compiles again. It is what a lost device
    /// asks for: nothing the old context made can be used, and a program of the new one is not the old one's.
    fn drain_shaders(&mut self) {
        let Self { device, shaders, .. } = self;
        for (_, entry) in shaders.cache.drain() {
            if let Ok(program) = entry {
                device.delete_program(program);
            }
        }
        shaders.clock.clear(device);
    }

    /// Hands the watchdog what the shader draws measured so far, to the frame's `shader_timings()`: a measurement is read
    /// without waiting, so a draw's cost is reported by the frame after it, or the one after that. Nothing here blocks.
    fn report_shader_costs(&mut self, frame: &OutputFrame) {
        let Self { device, shaders, .. } = self;
        for (key, elapsed) in shaders.clock.poll(device, Instant::now()) {
            frame.shader_timings().record(key, elapsed);
        }
    }

    /// How many frames this renderer has drawn.
    ///
    /// A frame with no damage draws nothing and does not count, which is how a test proves
    /// that an idle session costs no device work.
    #[must_use]
    pub fn draws(&self) -> u64 {
        self.draws
    }

    /// Whether the driver reports a software rasteriser.
    ///
    /// CI runs on llvmpipe and a developer's machine runs on a real GPU; a test that needs a
    /// tolerance says which one it ran on in its failure message.
    pub fn is_software(&mut self) -> bool {
        self.device.is_software()
    }

    /// The largest texture edge the driver accepts, in pixels: the most a target of this renderer can be across.
    pub fn max_texture_size(&mut self) -> u32 {
        self.device.max_texture_size()
    }

    /// The rasteriser the driver reports, for a log line and a test's failure message.
    pub fn driver(&mut self) -> String {
        self.device.renderer_name()
    }

    /// Replaces the target with one of the given size.
    ///
    /// The new target holds no pixels, so the caller must damage the whole output afterwards;
    /// the composer does that from the scene's own resize path.
    ///
    /// # Errors
    ///
    /// The same two errors as [`Gles::new`]. The old target is kept on failure, so a failed
    /// resize does not lose the frame that is on screen.
    pub fn resize(&mut self, physical: Size) -> Result<(), RenderError> {
        if physical.is_empty() {
            return Err(RenderError::Degenerate {
                width: physical.width,
                height: physical.height,
            });
        }
        if physical == self.physical {
            return Ok(());
        }
        Self::check_limit(&mut self.device, physical)?;
        let target = self.device.target(physical)?;
        // The old target's GL objects are given back now: an output that changes its supersample factor replaces a
        // target of hundreds of megabytes, and the context would otherwise keep every one it ever had.
        let old = std::mem::replace(&mut self.target, target);
        self.device.release(old);
        self.physical = physical;
        self.stale = true;
        // The scratch targets are as big as the output they copy, so a resize invalidates
        // them; they are rebuilt on the first blur of the new size.
        for scratch in self.scratch.take().into_iter().flatten() {
            self.device.release(scratch);
        }
        Ok(())
    }

    /// Refuses a target past the driver's texture limit, naming the numbers.
    ///
    /// The driver would refuse it too, but only as an incomplete framebuffer, which says nothing about a size. A
    /// supersampled output is where this matters: its target is the mode times the factor.
    fn check_limit(device: &mut GlContext, physical: Size) -> Result<(), RenderError> {
        let limit = device.max_texture_size();
        if physical.width > limit || physical.height > limit {
            log::warn!(
                area = "render",
                width = physical.width,
                height = physical.height,
                limit,
                "a {}x{} target is above the {limit}x{limit} the driver's textures allow",
                physical.width,
                physical.height
            );
            return Err(RenderError::Unavailable);
        }
        Ok(())
    }

    /// Draws `frame`, repainting only what it damaged.
    ///
    /// `scale` is the output's compositing scale: a frame's rectangles are logical and the
    /// target is physical.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses an operation. A client buffer that
    /// cannot be imported is not an error: a client may destroy it at any time, and the rest
    /// of the frame still has to appear.
    pub fn draw(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), RenderError> {
        self.compose(frame, scale, false)
    }

    /// Draws `frame` into the target, all of it when `whole` and only what the frame damaged otherwise.
    fn compose(&mut self, frame: &OutputFrame, scale: Scale, whole: bool) -> Result<(), RenderError> {
        if frame.is_empty() && !whole {
            return Ok(());
        }
        let bounds = Rectangle::new(
            (0, 0).into(),
            (self.physical.width as i32, self.physical.height as i32).into(),
        );
        let mut damage = std::mem::take(&mut self.damage);
        damage.clear();
        for rect in frame.damage() {
            if let Some(clipped) = physical_rect(*rect, scale).intersection(bounds) {
                damage.push(Rect::new(
                    clipped.loc.x,
                    clipped.loc.y,
                    clipped.size.w.max(0) as u32,
                    clipped.size.h.max(0) as u32,
                ));
            }
        }
        if damage.is_empty() && !whole {
            // Everything the frame wanted to redraw is outside the output.
            self.damage = damage;
            return Ok(());
        }
        // The magnifier and the colour matrix read and rewrite the whole output. A pixel that is
        // not repainted would be filtered again on the next frame, and an unfiltered pixel left
        // behind after a filter was turned off would stay wrong. So whenever the filter is on now
        // or was on last frame, repaint the whole output.
        let filtering = view::is_active(frame.view(), frame.colour_matrix());
        if filtering || self.filtered || whole {
            damage.clear();
            damage.push(Rect::new(0, 0, self.physical.width, self.physical.height));
        }
        self.prepare_subtrees(frame);
        let outcome = self.paint(frame, scale, &damage, None);
        // A target is defined again once a frame has painted all of it, and not before.
        let covered = damage == [Rect::new(0, 0, self.physical.width, self.physical.height)];
        self.damage = damage;
        self.filtered = filtering;
        outcome?;
        if covered {
            self.stale = false;
        }
        self.draws = self.draws.saturating_add(1);
        Ok(())
    }

    /// Imports a dmabuf as a scan-out buffer this renderer can draw into.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver cannot import the buffer, and
    /// [`RenderError::Unavailable`] when the import has no GL name. The reason is logged.
    pub fn import_scanout(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<ScanBuffer, RenderError> {
        use smithay::backend::renderer::ImportDma as _;
        let texture = self.device.renderer().import_dmabuf(dmabuf, None).map_err(|error| {
            log::warn!(area = "render", %error, "a scan-out buffer cannot be imported");
            RenderError::Failed
        })?;
        let framebuffer = self.device.framebuffer_over(&texture)?;
        Ok(ScanBuffer {
            framebuffer,
            _texture: texture,
        })
    }

    /// Draws `frame` into the target and copies the target into a scan-out buffer: the frame of an output the display
    /// scans out.
    ///
    /// The target persists between frames, so only the frame's damage is repainted into it, exactly as [`Gles::draw`]
    /// does; a target nobody has painted yet is painted whole first. The scan-out buffer holds the picture of an
    /// earlier flip, so the copy is written over all of it. The buffer is a dmabuf whose GL row 0 is its first memory
    /// row, the top scan line, while the target's row 0 is the bottom of the picture, so the copy reverses the rows.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses an operation.
    pub fn draw_scanout(
        &mut self,
        buffer: &mut ScanBuffer,
        frame: &OutputFrame,
        scale: Scale,
    ) -> Result<(), RenderError> {
        let whole = self.stale;
        self.compose(frame, scale, whole)?;
        let Self { device, target, .. } = self;
        scan_out(device, target, &buffer.framebuffer)
    }

    /// Draws `frame` into the target, which is `mode * s` pixels, and averages the target down into a scan-out buffer
    /// of the mode: the frame of a supersampled output.
    ///
    /// The target is composed as [`Gles::draw_scanout`] composes it. The average is made in a target of the mode and
    /// copied into the scan-out buffer with its rows reversed, for the reason [`Gles::draw_scanout`] gives. `scale` is
    /// the compositing scale, which includes the factor.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the averaged target cannot be made, and [`RenderError::Failed`] when the
    /// driver refuses an operation.
    pub fn draw_scanout_downsampled(
        &mut self,
        buffer: &mut ScanBuffer,
        frame: &OutputFrame,
        scale: Scale,
    ) -> Result<(), RenderError> {
        let whole = self.stale;
        self.compose(frame, scale, whole)?;
        let averaged = self.shown_target(buffer.framebuffer.size())?;
        let outcome = self
            .downsample_into(&averaged)
            .and_then(|()| scan_out(&mut self.device, &averaged, &buffer.framebuffer));
        self.shown = Some(averaged);
        outcome
    }

    /// The target as the display shows it at `shown` pixels: [`Gles::capture`] averaged down on the GPU, with the
    /// cursor, which is part of the picture a display shows.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels, [`RenderError::Unavailable`] when the averaged target
    /// cannot be made, and [`RenderError::Failed`] when the driver refuses the pass or the read.
    pub fn downsampled(&mut self, shown: Size) -> Result<Image, RenderError> {
        if shown.is_empty() {
            return Err(RenderError::Degenerate {
                width: shown.width,
                height: shown.height,
            });
        }
        let framebuffer = self.shown_target(shown)?;
        let outcome = self.downsample_into(&framebuffer).and_then(|()| {
            let mut bytes = self.device.read(&framebuffer)?;
            let stride = shown.width as usize * 4;
            bytes.truncate(stride.saturating_mul(shown.height as usize));
            flip_rows(&mut bytes, stride);
            Image::new(shown, stride, bytes).ok_or(RenderError::Failed)
        });
        self.shown = Some(framebuffer);
        outcome
    }

    /// The target the average of a supersampled output is made in, `shown` pixels: the one kept from the last
    /// average when it has that size, a new one otherwise. The caller puts it back in `self.shown`.
    fn shown_target(&mut self, shown: Size) -> Result<Framebuffer, RenderError> {
        match self.shown.take() {
            Some(framebuffer) if framebuffer.size() == shown => Ok(framebuffer),
            old => {
                if let Some(old) = old {
                    self.device.release(old);
                }
                self.device.target(shown)
            }
        }
    }

    /// Runs the downsample pass: the target is read whole and written over all of `destination`.
    fn downsample_into(&mut self, destination: &Framebuffer) -> Result<(), RenderError> {
        let Self {
            device, target, passes, ..
        } = self;
        let shown = destination.size();
        let bytes = downsample::DownsampleUniforms::new(target.size(), shown).bytes();
        device.begin(destination)?;
        device.scissor(None);
        device.blend_replace()?;
        let drawn = device.draw(
            &passes.downsample,
            &bytes,
            Rect::new(0, 0, shown.width, shown.height),
            Some(target.texture()),
            QuadPlacement::FLAT,
        );
        device.blend_premultiplied()?;
        device.end()?;
        drawn
    }

    /// Reads the target back as an [`Image`].
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the read or returns fewer bytes than
    /// the target holds.
    pub fn screenshot(&mut self) -> Result<Image, RenderError> {
        self.capture(false)
    }

    /// Reads the target back as an [`Image`], with the cursor composed in when `with_cursor`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the read or returns fewer bytes than
    /// the target holds.
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        let size = self.physical;
        let mut bytes = self.device.read(&self.target)?;
        let stride = size.width as usize * 4;
        let expected = stride.saturating_mul(size.height as usize);
        if bytes.len() < expected {
            log::warn!(
                area = "render",
                got = bytes.len(),
                expected,
                "the GLES readback is short of the target"
            );
            return Err(RenderError::Failed);
        }
        bytes.truncate(expected);
        // `glReadPixels` starts at the framebuffer's bottom-left corner, so the rows arrive
        // bottom first. An `Image` is top-left first.
        flip_rows(&mut bytes, stride);
        if let Some(rect) = self.cursor_under_rect.filter(|_| !with_cursor) {
            let x = rect.loc.x as usize * 4;
            let width = rect.size.w as usize * 4;
            for y in rect.loc.y as usize..(rect.loc.y + rect.size.h) as usize {
                let start = y * stride + x;
                bytes[start..start + width].copy_from_slice(&self.cursor_under[start..start + width]);
            }
        }
        Image::new(size, stride, bytes).ok_or(RenderError::Failed)
    }

    /// Draws every layer of the frame, damage rectangle by damage rectangle.
    fn paint(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        damage: &[Rect],
        glass: Option<&LayerParts<'_>>,
    ) -> Result<(), RenderError> {
        self.report_shader_costs(frame);
        self.shaders.clock.next_frame();
        let painted = self.paint_layers(frame, scale, damage, glass);
        // Every texture the frame imported was dropped when `paint_layers` returned, on every path out of it, so their
        // names are given back here, once per frame.
        self.device.collect_textures();
        self.report_shader_costs(frame);
        if matches!(painted, Err(RenderError::Failed | RenderError::Lost)) {
            // A frame that failed may have lost the context, and a program of a lost context draws nothing.
            self.drain_shaders();
        }
        painted
    }

    /// Paints the layers and, for each whose shader reads the backdrop, the picture it reads, built first and given back
    /// after.
    fn paint_layers(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        damage: &[Rect],
        glass: Option<&LayerParts<'_>>,
    ) -> Result<(), RenderError> {
        let backdrops = self.build_backdrops(frame, scale, damage)?;
        let painted = self.paint_layers_with(frame, scale, damage, glass, &backdrops);
        for backdrop in backdrops.into_iter().flatten() {
            self.device.give_back(backdrop);
        }
        painted
    }

    /// The picture each layer whose shader reads the backdrop reads, indexed like the frame's layers (a layer with none,
    /// or one this frame does not draw a shader on, has `None`).
    ///
    /// A layer on a 3D placement has none, because no shader is drawn on it. A layer none of the damage reaches has
    /// none either. Past [`MAX_BACKDROP_DEPTH`] nested backdrops the list is empty and a shader that needs one is not
    /// drawn.
    fn build_backdrops(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        damage: &[Rect],
    ) -> Result<Vec<Option<Framebuffer>>, RenderError> {
        let mut built: Vec<Option<Framebuffer>> = Vec::new();
        if self.backdrop_depth >= MAX_BACKDROP_DEPTH || !frame.layers().iter().any(reads_backdrop) {
            return Ok(built);
        }
        for (index, layer) in frame.layers().iter().enumerate() {
            let wanted = reads_backdrop(layer)
                && layer_model(layer).is_none()
                && damage.iter().any(|rect| intersects(layer, *rect, scale));
            let picture = if wanted {
                self.backdrop_of(frame, index, scale)
            } else {
                Ok(None)
            };
            match picture {
                Ok(picture) => built.push(picture),
                Err(error) => {
                    for backdrop in built.into_iter().flatten() {
                        self.device.give_back(backdrop);
                    }
                    return Err(error);
                }
            }
        }
        Ok(built)
    }

    /// The backdrop of the layer at `index`: [`OutputFrame::backdrop_input`] drawn into a target and copied, top row
    /// first, into a texture the size of the layer's own rectangle, which is what `backdrop(uv)` reads. What lies off
    /// the output stays transparent.
    ///
    /// The layers below are drawn into their own output-sized target and never into the frame's: the frame's target
    /// shows secure windows, and the input has them black.
    fn backdrop_of(
        &mut self,
        frame: &OutputFrame,
        index: usize,
        scale: Scale,
    ) -> Result<Option<Framebuffer>, RenderError> {
        let Some(input) = frame.backdrop_input(index) else {
            return Ok(None);
        };
        let region = physical_rect(input.region, scale);
        let quad = Rect::new(
            region.loc.x,
            region.loc.y,
            region.size.w.max(0) as u32,
            region.size.h.max(0) as u32,
        );
        if quad.is_empty() {
            return Ok(None);
        }
        let mut below = self.device.lease(self.physical)?;
        let texture = match self.device.lease(Size::new(quad.width, quad.height)) {
            Ok(texture) => texture,
            Err(error) => {
                self.device.give_back(below);
                return Err(error);
            }
        };
        let drawn = self.draw_backdrop(&input.frame, scale, quad, &mut below, &texture);
        self.device.give_back(below);
        match drawn {
            Ok(()) => Ok(Some(texture)),
            Err(error) => {
                self.device.give_back(texture);
                Err(error)
            }
        }
    }

    /// Draws `frame` over `quad` into `below` and copies that rectangle into `texture`, flipped so its first row is the
    /// top of the layer.
    fn draw_backdrop(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        quad: Rect,
        below: &mut Framebuffer,
        texture: &Framebuffer,
    ) -> Result<(), RenderError> {
        self.device.clear(texture, Color::TRANSPARENT);
        let Some(visible) = quad.intersection(Rect::from_size(self.physical)) else {
            return Ok(());
        };
        std::mem::swap(&mut self.target, below);
        self.backdrop_depth += 1;
        let painted = self.paint_layers(frame, scale, &[visible], None);
        self.backdrop_depth -= 1;
        std::mem::swap(&mut self.target, below);
        painted?;
        let within = Rect::new(visible.x - quad.x, visible.y - quad.y, visible.width, visible.height);
        self.device.blit_between(below, texture, visible, within, true)
    }

    /// [`Gles::paint_layers`] with the backdrops its layers read already built.
    fn paint_layers_with(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        damage: &[Rect],
        glass: Option<&LayerParts<'_>>,
        backdrops: &[Option<Framebuffer>],
    ) -> Result<(), RenderError> {
        // Textures first: importing needs the renderer mutably, and a pass borrows it for the
        // whole of the draw.
        let mut textures: Vec<Option<Texture>> = Vec::with_capacity(frame.layers().len());
        // The imported textures own their GL names, so they live until the frame is painted.
        let mut owned: Vec<GlesTexture> = Vec::new();
        for layer in frame.layers() {
            textures.push(match layer {
                Layer::Solid(_) => None,
                Layer::Overlay(overlay) => {
                    let texture = self.import_overlay(overlay.image());
                    let handle = texture.as_ref().and_then(Texture::from_smithay);
                    owned.extend(texture);
                    handle
                }
                Layer::Surface(surface) => self.import(surface, &mut owned),
            });
        }
        // The overlay is the frame's last layer, so it is drawn after the model pass and the glass.
        let overlay = frame
            .layers()
            .iter()
            .zip(&textures)
            .find_map(|(layer, texture)| match layer {
                Layer::Overlay(overlay) => texture.map(|texture| (overlay, texture)),
                _ => None,
            });
        let background = frame.background();
        // The cursor is imported like the overlay: the texture owns the GL name until the frame is painted.
        let cursor_texture = frame.cursor().and_then(|cursor| self.import_overlay(cursor.image()));
        let cursor = frame
            .cursor()
            .zip(cursor_texture.as_ref().and_then(Texture::from_smithay));
        let Self {
            device,
            target,
            scratch,
            passes,
            shaders,
            physical,
            cursor_under,
            cursor_under_rect,
            ..
        } = self;
        let output: Rectangle<i32, Physical> =
            Rectangle::new((0, 0).into(), (physical.width as i32, physical.height as i32).into());
        let cursor_rect = frame
            .cursor()
            .and_then(|cursor| physical_rect(cursor.dst(), scale).intersection(output));
        *cursor_under_rect = cursor_rect;
        if cursor_rect.is_some() {
            cursor_under.resize(physical.width as usize * 4 * physical.height as usize, 0);
        }
        let order = spatial_order(frame.layers(), *physical, scale);
        device.begin(target)?;
        let backdrop_of = |index: usize| backdrops.get(index).and_then(Option::as_ref).map(Framebuffer::texture);
        let mut painter = Painter {
            device,
            target,
            scratch,
            passes,
            shaders: Some(shaders),
            timings: Some(frame.shader_timings().clone()),
            user_bytes: Vec::new(),
            physical: *physical,
            scale,
            background,
            bytes: Vec::new(),
            clip: Rect::new(0, 0, 0, 0),
            cursor_under,
            canvas: None,
        };
        for rect in damage {
            painter.clip = *rect;
            painter.device.scissor(Some(*rect));
            painter.background(*rect)?;
            for (index, layer) in frame.layers().iter().enumerate() {
                if layer_model(layer).is_some() || !intersects(layer, *rect, scale) {
                    continue;
                }
                let texture = textures.get(index).and_then(|texture| *texture);
                painter.layer(layer, texture, QuadPlacement::FLAT, backdrop_of(index))?;
            }
            if !order.is_empty() {
                painter.device.clear_depth(painter.target);
            }
            for (index, placement) in &order {
                let layer = &frame.layers()[*index];
                let texture = textures.get(*index).and_then(|texture| *texture);
                painter.layer(layer, texture, *placement, None)?;
            }
            if let Some(parts) = glass {
                painter.draw_parts(parts, None, QuadPlacement::FLAT)?;
            }
            if let Some((overlay, texture)) = overlay {
                painter.overlay(overlay, texture)?;
            }
            if let Some((cursor, texture)) = cursor {
                if let Some(dst) = cursor_rect {
                    let damaged: Rectangle<i32, Physical> =
                        Rectangle::new((rect.x, rect.y).into(), (rect.width as i32, rect.height as i32).into());
                    if let Some(captured) = damaged.intersection(dst) {
                        painter.capture_under(captured)?;
                    }
                }
                painter.overlay(cursor, texture)?;
            }
        }
        painter.finish()?;
        // The view and the colour matrix act on the finished output, so they run after every
        // layer is drawn: the output is copied into scratch, then the view pass samples that copy
        // over the whole output. A frame with neither is skipped and draws as it always did.
        let view = frame.view();
        let matrix = frame.colour_matrix();
        if view::is_active(view, matrix) {
            painter.draw_view(view, matrix)?;
        }
        Ok(())
    }

    /// Draws a texture the test made, through the surface pass.
    ///
    /// A client-buffer layer needs a live `wl_shm` client to test, which is the session
    /// tests' job; this is the seam that lets the surface pass — its texture mapping, its
    /// premultiplied tint and its corner clip — be probed without one.
    #[cfg(test)]
    fn draw_texture(
        &mut self,
        texture: Texture,
        quad: Rect,
        src: [f32; 4],
        tint: Color,
        corner: f32,
    ) -> Result<(), RenderError> {
        let physical = self.physical;
        self.device.begin(&self.target)?;
        self.device
            .scissor(Some(Rect::new(0, 0, physical.width, physical.height)));
        let uniforms = Uniforms::surface(quad, src, tint, 1.0, corner);
        let mut bytes = Vec::new();
        self.passes.surface.pack(&uniforms, &mut bytes);
        self.device
            .draw(&self.passes.surface, &bytes, quad, Some(texture), QuadPlacement::FLAT)?;
        self.device.scissor(None);
        self.device.end()?;
        self.draws = self.draws.saturating_add(1);
        Ok(())
    }

    /// Imports a surface layer's pixels: its client buffer, or the snapshot a closing window
    /// keeps once the client has let the buffer go. A snapshot's texture is pushed to `owned`,
    /// which keeps its GL name alive until the frame is painted.
    fn import(&mut self, surface: &Surface, owned: &mut Vec<GlesTexture>) -> Option<Texture> {
        let buffer = match &surface.source {
            SurfaceSource::Buffer(buffer) => buffer,
            SurfaceSource::Snapshot(image) => {
                let texture = self.import_overlay(image)?;
                let handle = Texture::from_smithay(&texture);
                owned.push(texture);
                return handle;
            }
            // The picture of a View is the texture this renderer drew from the scene the frame carries.
            SurfaceSource::View(_) => return self.view_source(surface),
            // The GPU buffer is sampled where it is: the texture is the driver's view of the client's
            // memory, so nothing is copied, and Smithay's cache on the buffer keeps it across frames.
            SurfaceSource::Dmabuf(dmabuf) => {
                let texture = self.dmabuf_texture(dmabuf)?;
                let handle = Texture::from_smithay(&texture);
                owned.push(texture);
                return handle;
            }
        };
        let texture = self
            .device
            .renderer()
            .import_shm_buffer(buffer, None, &[])
            .map_err(|error| {
                log::warn!(
                    area = "render",
                    ?error,
                    "a client buffer could not be imported; the layer is skipped"
                );
            })
            .ok()?;
        let handle = Texture::from_smithay(&texture);
        // The texture owns the GL name: dropped here, the name would be queued for deletion and a collection before the
        // frame is painted would delete it from under the pass.
        owned.push(texture);
        handle
    }

    /// Imports the overlay's premultiplied pixels as a texture.
    ///
    /// An import failure is logged and the overlay is skipped for this frame, the same way a
    /// client buffer is.
    fn import_overlay(&mut self, image: &Image) -> Option<GlesTexture> {
        let size = image.size();
        let texture = self
            .device
            .renderer()
            .import_memory(
                image.data(),
                READBACK_FORMAT,
                (size.width as i32, size.height as i32).into(),
                false,
            )
            .map_err(|error| {
                log::warn!(
                    area = "render",
                    ?error,
                    "the overlay could not be imported; it is not drawn"
                );
            })
            .ok()?;
        Some(texture)
    }
}

/// One layer's geometry and paint: a layer of the frame, or the glass a test draws over it.
#[derive(Clone, Copy)]
struct LayerParts<'a> {
    /// The physical rectangle the layer covers.
    quad: Rect,
    radius: f32,
    /// The physical rectangle the corner radius rounds, the shadow follows when it asks for the frame and a
    /// border runs along, when the layer is one part of a larger frame. `None` is `quad`.
    silhouette: Option<Rect>,
    effects: &'a gnoblin_render::Effects,
    colour: Option<Color>,
    tint: Color,
    opacity: f32,
    src: [f32; 4],
    /// The picture a shader on the layer reads with `backdrop(uv)`, built by [`Gles::build_backdrops`].
    backdrop: Option<Texture>,
}

/// One frame's mutable state, so a pass's parameters stay a method's arguments.
struct Painter<'a> {
    device: &'a mut GlContext,
    target: &'a Framebuffer,
    scratch: &'a mut Option<[Framebuffer; 2]>,
    passes: &'a mut Passes,
    /// The user shader programs, or `None` where a shader is not drawn: inside a View, whose layers are drawn through
    /// an eye's viewport that a shader's intermediate pictures would lose.
    shaders: Option<&'a mut UserShaders>,
    /// Where the frame's shader draws report their cost, for the watchdog.
    timings: Option<ShaderTimings>,
    /// The packed `gnoblin_user` block of the draw in progress, reused across draws.
    user_bytes: Vec<u8>,
    physical: Size,
    scale: Scale,
    background: Color,
    /// The packed uniforms of the pass in progress, reused across passes.
    bytes: Vec<u8>,
    /// The damage rectangle every pass is scissored to.
    clip: Rect,
    /// The pixels under the cursor before it was drawn, kept for a capture without it.
    cursor_under: &'a mut Vec<u8>,
    /// The picture of a filtered layer while its pieces are drawn into it, in place of the frame's target.
    canvas: Option<Framebuffer>,
}
impl Painter<'_> {
    /// Fills the damage rectangle with the frame's background.
    fn background(&mut self, rect: Rect) -> Result<(), RenderError> {
        let Self {
            device,
            passes,
            background,
            bytes,
            ..
        } = self;
        let uniforms = Uniforms::solid(rect, *background, Color::WHITE, 1.0);
        passes.solid.pack(&uniforms, bytes);
        device.draw(&passes.solid, bytes, rect, None, QuadPlacement::FLAT)
    }

    /// Draws a layer: its shadow, its backdrop effect, then its own pixels.
    ///
    /// A layer with a blur is drawn as a group. The blur and the layer's pixels are composed at
    /// full opacity inside the group, and the finished group is faded over the frame by the
    /// layer's opacity, so the blur fades with the layer as a browser's `backdrop-filter` does
    /// under `opacity`.
    fn layer(
        &mut self,
        layer: &Layer,
        texture: Option<Texture>,
        placement: QuadPlacement,
        backdrop: Option<Texture>,
    ) -> Result<(), RenderError> {
        let (dst, corner_radius, silhouette, effects, colour, tint, opacity, src) = match layer {
            Layer::Solid(solid) => (
                solid.dst,
                solid.corner_radius,
                solid.silhouette,
                &solid.effects,
                Some(solid.colour),
                solid.tint,
                1.0,
                [0.0, 0.0, 1.0, 1.0],
            ),
            Layer::Surface(surface) => (
                surface.dst,
                surface.corner_radius,
                surface.silhouette,
                &surface.effects,
                None,
                surface.tint,
                surface.opacity,
                texture.map_or([0.0, 0.0, 1.0, 1.0], |texture| texture_mapping(surface, texture)),
            ),
            // The overlay has its own pass, so no layer loop draws it.
            Layer::Overlay(_) => return Ok(()),
        };
        // A surface whose pixels could not be imported draws nothing: the rest of the frame still appears, and a
        // pass that sampled no texture would paint a flat colour over the layer's place.
        if matches!(layer, Layer::Surface(_)) && texture.is_none() {
            return Ok(());
        }
        let dst = physical_rect(dst, self.scale);
        let quad = Rect::new(dst.loc.x, dst.loc.y, dst.size.w.max(0) as u32, dst.size.h.max(0) as u32);
        if quad.is_empty() {
            return Ok(());
        }
        let silhouette = silhouette.map(|shape| {
            let shape = physical_rect(shape, self.scale);
            Rect::new(
                shape.loc.x,
                shape.loc.y,
                shape.size.w.max(0) as u32,
                shape.size.h.max(0) as u32,
            )
        });
        let radius = (corner_radius * self.scale.as_f64() as f32).clamp(0.0, MAX_RADIUS);
        self.draw_parts(
            &LayerParts {
                quad,
                radius,
                silhouette,
                effects,
                colour,
                tint,
                opacity,
                src,
                backdrop,
            },
            texture,
            placement,
        )
    }

    /// Draws one layer's parts: its shadows, its blur group, then its own pixels, in the order `effects.rs` states.
    fn draw_parts(
        &mut self,
        parts: &LayerParts<'_>,
        texture: Option<Texture>,
        placement: QuadPlacement,
    ) -> Result<(), RenderError> {
        let LayerParts {
            quad,
            radius,
            silhouette,
            effects,
            colour,
            tint,
            opacity,
            src,
            backdrop,
        } = *parts;
        // The shape the corner radius rounds, the shadow follows when it asks for the frame and a border runs
        // along: the layer's silhouette when it is one part of a larger frame, else its own quad. A layer drawn
        // through a model matrix is not on the flat plane the silhouette is stated in, so it keeps its own quad.
        let flat = placement.clip.is_none();
        let shape = silhouette.filter(|_| flat).unwrap_or(quad);
        let rounded_to = silhouette.filter(|_| flat && radius > 0.0);
        let ready = if flat {
            self.ready_shaders(effects, backdrop.is_some())
        } else {
            Vec::new()
        };
        let filter = effects::filter_of(effects);
        // A filter that blurs is drawn as a picture of the layer's pieces; a grade is put on each piece as it is drawn.
        let picture = flat && filter.blur > 0.0;
        let paint = Paint {
            quad,
            shape,
            radius,
            rounded_to,
            colour,
            texture,
            src,
            tint,
            placement,
            backdrop,
            ready: &ready,
            effects,
            grade: if flat && !picture {
                effects::grade_of(&filter)
            } else {
                Grade::IDENTITY
            },
        };
        if !flat {
            if !effects.passes().is_empty() {
                log::warn!(
                    "unsupported-effect-on-3d: a shadow, blur, border, background, filter or shader on a transformed layer is not drawn"
                );
            }
            return self.content(&paint, opacity, false);
        }
        if !picture {
            self.shadows(&paint)?;
        }
        let mut grouped = false;
        for pass in effects.passes() {
            if let Pass::Blur(blur) = pass
                && blur.radius > 0.0
            {
                grouped |= self.blur(quad, *blur, grouped)?;
            }
        }
        if picture {
            // The glass is drawn first, outside the picture, so the filter does not blur it.
            if grouped {
                self.enter_group()?;
                self.fade_group(quad, opacity, radius, rounded_to)?;
            }
            return self.picture(&paint, &filter, opacity);
        }
        // A grouped layer is drawn at full opacity into its group, which then fades as one.
        let (group_opacity, fade) = if grouped { (1.0, Some(opacity)) } else { (opacity, None) };
        if grouped {
            self.enter_group()?;
        }
        self.backgrounds(&paint, group_opacity)?;
        self.inset_shadows(&paint, group_opacity)?;
        let drawn = self.content(&paint, group_opacity, grouped);
        match (drawn, fade) {
            (Ok(()), Some(factor)) => self.fade_group(quad, factor, radius, rounded_to)?,
            (drawn, _) => drawn?,
        }
        // A border is a stroke over everything else, outside the group the blur's corner clip cuts.
        self.borders(&paint, opacity)
    }

    /// The shader passes of `effects` this frame can draw, in list order: each one's program is compiled the first time
    /// its source is met and kept, and a source that does not compile is a pass left undrawn with its reason reported
    /// (`shader: compile-failed` in the node's `skipped` list), so the layer still draws.
    ///
    /// A pass that reads the backdrop needs `has_backdrop`; without one (the layer is nested too deep in backdrops, or
    /// the picture could not be made) it is left undrawn as `shader: backdrop-unavailable`.
    fn ready_shaders<'e>(&mut self, effects: &'e gnoblin_render::Effects, has_backdrop: bool) -> Vec<&'e ShaderPass> {
        let Self {
            device,
            shaders,
            timings,
            ..
        } = self;
        let Some(shaders) = shaders.as_deref_mut() else {
            return Vec::new();
        };
        let mut ready = Vec::new();
        for pass in effects.passes() {
            let Pass::Shader(pass) = pass else {
                continue;
            };
            let key = pass.key();
            if pass.backdrop() && !has_backdrop {
                if let Some(timings) = timings.as_ref() {
                    timings.record_refused(key, "backdrop-unavailable");
                }
                continue;
            }
            if !shaders.cache.contains(key) {
                let program = program::link_user(device, pass.source(), pass.backdrop());
                if let Err(error) = &program {
                    log::warn!(
                        area = "render",
                        %error,
                        shader = key.get(),
                        "a user shader does not compile; the layer is drawn without it"
                    );
                }
                if let Some((evicted_key, evicted)) = shaders.cache.insert(key, program) {
                    shaders.clock.forget(evicted_key);
                    if let Ok(evicted) = evicted {
                        device.delete_program(evicted);
                    }
                }
            }
            match shaders.cache.get(key) {
                Some(Ok(_)) => ready.push(pass),
                Some(Err(_)) => {
                    if let Some(timings) = timings.as_ref() {
                        timings.record_refused(key, "compile-failed");
                    }
                }
                None => {}
            }
        }
        ready
    }

    /// Draws a layer's own pixels through its shader passes: each pass but the last draws into a picture the size of the
    /// layer, which the next reads as the node's colour, and the last draws into the frame with the layer's tint,
    /// opacity and rounded corners.
    fn draw_shaded(&mut self, ready: &[&ShaderPass], node: &Shaded) -> Result<(), RenderError> {
        let Some((last, earlier)) = ready.split_last() else {
            return Ok(());
        };
        // The node's own colour is a texture: the layer's pixels, or for a solid the one pixel of its colour.
        let (mut input, mut src) = match (node.colour, node.texture) {
            (Some(colour), _) => (self.device.swatch(premultiplied_bytes(colour))?, [0.0, 0.0, 1.0, 1.0]),
            (None, Some(texture)) => (texture, node.src),
            (None, None) => return Ok(()),
        };
        let mut leased: Vec<Framebuffer> = Vec::new();
        let drawn = self
            .shade_earlier(earlier, node, &mut input, &mut src, &mut leased)
            .and_then(|()| {
                if node.grouped {
                    self.enter_group()?;
                } else if !leased.is_empty() {
                    // The pictures were drawn into other targets, so the canvas is bound again.
                    self.rebind_canvas()?;
                }
                let mut uniforms = Uniforms::surface(node.quad, src, node.tint, node.opacity, node.radius);
                uniforms.grade = node.grade.to_array();
                if let Some(frame) = node.rounded_to {
                    uniforms.round_to(frame);
                }
                self.shader_draw(last, &uniforms, input, node.backdrop, node.quad)
            });
        for picture in leased {
            self.device.give_back(picture);
        }
        drawn
    }

    /// Draws the passes before the last, each into a picture of the layer's size with the identity tint, opacity and
    /// corner. `input` and `src` become the last picture and how to read it; the pictures are pushed to `leased`, for
    /// the caller to give back on every path.
    fn shade_earlier(
        &mut self,
        earlier: &[&ShaderPass],
        node: &Shaded,
        input: &mut Texture,
        src: &mut [f32; 4],
        leased: &mut Vec<Framebuffer>,
    ) -> Result<(), RenderError> {
        let size = Size::new(node.quad.width, node.quad.height);
        let local = Rect::from_size(size);
        for pass in earlier {
            leased.push(self.device.lease(size)?);
            let Some(picture) = leased.last() else {
                return Err(RenderError::Failed);
            };
            self.device.begin(picture)?;
            self.device.blend_replace()?;
            let uniforms = Uniforms::surface(local, *src, Color::WHITE, 1.0, 0.0);
            let drawn = self.shader_draw(pass, &uniforms, *input, node.backdrop, local);
            self.device.blend_premultiplied()?;
            drawn?;
            *input = picture.texture();
            // A target's texture starts at its bottom row.
            *src = flipped_mapping(local, size);
        }
        Ok(())
    }

    /// One shader draw: the pass's program over `quad`, reading `input` as the node's colour.
    ///
    /// The draw is measured without waiting for it ([`timing`]): a query or a fence is started around it, and the result
    /// is reported by a later [`Gles::report_shader_costs`], when the driver has it.
    fn shader_draw(
        &mut self,
        pass: &ShaderPass,
        uniforms: &Uniforms,
        input: Texture,
        backdrop: Option<Texture>,
        quad: Rect,
    ) -> Result<(), RenderError> {
        let Self {
            device,
            shaders,
            bytes,
            user_bytes,
            ..
        } = self;
        let Some(shaders) = shaders.as_deref_mut() else {
            return Ok(());
        };
        let time = shaders.started.elapsed().as_secs_f32();
        let key = pass.key();
        let Some(Ok(program)) = shaders.cache.get(key) else {
            return Ok(());
        };
        program.pack(uniforms, bytes);
        UserBlock::new(pass.params_padded(), time).pack(user_bytes);
        let user = UserInputs {
            block: &user_bytes[..],
            backdrop: backdrop.filter(|_| pass.backdrop()),
        };
        let started = shaders.clock.begin(device, key);
        let drawn = device.draw_with(program, bytes, Some(user), quad, Some(input), QuadPlacement::FLAT);
        shaders.clock.end(device, started, key, Instant::now());
        drawn
    }

    /// Keeps the pixels of `rect` before the cursor is drawn over them, read back from the
    /// target and stored top first in `cursor_under`.
    fn capture_under(&mut self, rect: Rectangle<i32, Physical>) -> Result<(), RenderError> {
        let stride = self.physical.width as usize * 4;
        let rows = self.physical.height as usize;
        let full = self.device.read(self.target)?;
        // The read leaves the default framebuffer bound; the cursor is drawn next, into the target.
        self.device.bind(self.target)?;
        if full.len() < stride * rows {
            return Err(RenderError::Failed);
        }
        let x = rect.loc.x as usize * 4;
        let width = rect.size.w as usize * 4;
        for y in rect.loc.y as usize..(rect.loc.y + rect.size.h) as usize {
            let source = (rows - 1 - y) * stride + x;
            let start = y * stride + x;
            self.cursor_under[start..start + width].copy_from_slice(&full[source..source + width]);
        }
        Ok(())
    }

    /// The overlay, above every layer and below the cursor.
    fn overlay(&mut self, overlay: &Overlay, texture: Texture) -> Result<(), RenderError> {
        let dst = physical_rect(overlay.dst(), self.scale);
        let quad = Rect::new(dst.loc.x, dst.loc.y, dst.size.w.max(0) as u32, dst.size.h.max(0) as u32);
        if quad.is_empty() {
            return Ok(());
        }
        let Self {
            device, passes, bytes, ..
        } = self;
        let uniforms = Uniforms::surface(quad, [0.0, 0.0, 1.0, 1.0], Color::WHITE, 1.0, 0.0);
        passes.surface.pack(&uniforms, bytes);
        device.draw(&passes.surface, bytes, quad, Some(texture), QuadPlacement::FLAT)
    }

    /// Starts the layer's group: the layer's own pixels are drawn over the blur it holds.
    fn enter_group(&mut self) -> Result<(), RenderError> {
        let Self {
            device, scratch, clip, ..
        } = self;
        let Some([group, _]) = scratch.as_mut() else {
            return Err(RenderError::Failed);
        };
        device.begin(group)?;
        device.scissor(Some(*clip));
        Ok(())
    }

    /// Composes the finished group over the frame at the layer's opacity, `group * opacity +
    /// frame * (1 - opacity)`, so the blur and the layer's pixels fade together.
    fn fade_group(
        &mut self,
        dst: Rect,
        opacity: f32,
        radius: f32,
        silhouette: Option<Rect>,
    ) -> Result<(), RenderError> {
        let Some(rect) = dst.intersection(Rect::new(0, 0, self.physical.width, self.physical.height)) else {
            return Ok(());
        };
        let Self {
            device,
            target,
            scratch,
            passes,
            bytes,
            physical,
            clip,
            ..
        } = self;
        let Some([group, _]) = scratch.as_ref() else {
            return Err(RenderError::Failed);
        };
        device.begin(target)?;
        device.scissor(Some(*clip));
        device.blend_group(opacity)?;
        // The group holds a blurred backdrop that fills its whole quad, so the corner is cut here, from the
        // layer's own radius and silhouette, rather than leaving the blur square behind a rounded window.
        let mut uniforms = Uniforms::surface(rect, flipped_mapping(rect, *physical), Color::WHITE, 1.0, radius);
        if let Some(frame) = silhouette {
            uniforms.round_to(frame);
        }
        passes.surface.pack(&uniforms, bytes);
        let drawn = device.draw(&passes.surface, bytes, rect, Some(group.texture()), QuadPlacement::FLAT);
        device.blend_premultiplied()?;
        drawn
    }

    /// The two scratch targets a blur needs, made on first use and after a resize.
    fn ensure_scratch(&mut self) -> Result<(), RenderError> {
        let current = self.scratch.as_ref().map(|[first, _]| first.size());
        if current == Some(self.physical) {
            return Ok(());
        }
        let first = self.device.target(self.physical)?;
        let second = self.device.target(self.physical)?;
        *self.scratch = Some([first, second]);
        Ok(())
    }

    /// Lifts the scissor and sends the frame to the driver.
    fn finish(&mut self) -> Result<(), RenderError> {
        let Self { device, .. } = self;
        device.scissor(None);
        device.end()
    }

    /// Applies the magnified view and colour matrix to the whole output.
    ///
    /// The output is copied to the first scratch target, then the view pass draws over the whole
    /// output with blending replaced, so each pixel is the copy's pixel at its source position
    /// rather than a blend over the pixel already there.
    fn draw_view(&mut self, view: Option<View>, matrix: Option<ColourMatrix>) -> Result<(), RenderError> {
        let Self {
            device,
            target,
            scratch,
            passes,
            bytes,
            physical,
            ..
        } = self;
        // The scratch targets are the output's size; a resize drops them, so make them again here.
        if scratch.as_ref().map(|[first, _]| first.size()) != Some(*physical) {
            let first = device.target(*physical)?;
            let second = device.target(*physical)?;
            **scratch = Some([first, second]);
        }
        let Some([first, _]) = scratch.as_mut() else {
            return Err(RenderError::Failed);
        };
        let full = Rect::new(0, 0, physical.width, physical.height);
        device.copy(target, first, full)?;
        device.begin(target)?;
        device.scissor(None);
        device.blend_replace()?;
        *bytes = view::ViewUniforms::new(view, matrix, physical.width, physical.height).bytes();
        let drawn = device.draw(&passes.view, bytes, full, Some(first.texture()), QuadPlacement::FLAT);
        device.blend_premultiplied()?;
        device.end()?;
        drawn
    }
}

impl Passes {
    /// Compiles the effect programs.
    fn new(device: &mut GlContext) -> Result<Self, RenderError> {
        Ok(Self {
            solid: program::link(device, shaders::VERTEX, shaders::SOLID)?,
            surface: program::link(device, shaders::VERTEX, shaders::SURFACE)?,
            shadow: program::link(device, shaders::VERTEX, shaders::SHADOW)?,
            border: program::link(device, shaders::VERTEX, shaders::BORDER)?,
            inset_shadow: program::link(device, shaders::VERTEX, shaders::INSET_SHADOW)?,
            blur: program::link(device, shaders::VERTEX, shaders::BLUR)?,
            view: program::link(device, shaders::VERTEX, shaders::VIEW)?,
            downsample: program::link(device, shaders::VERTEX, shaders::DOWNSAMPLE)?,
        })
    }
}

/// What [`Painter::draw_shaded`] needs of a layer: the quad and the paint [`Painter::draw_parts`] would have drawn.
struct Shaded {
    quad: Rect,
    /// The solid's colour, straight; `None` for a layer of pixels.
    colour: Option<Color>,
    texture: Option<Texture>,
    src: [f32; 4],
    tint: Color,
    /// The opacity the draw carries: the group's 1 when the layer is drawn into a group, else the layer's own.
    opacity: f32,
    radius: f32,
    rounded_to: Option<Rect>,
    backdrop: Option<Texture>,
    /// The filter's grade, which the last pass puts on its result.
    grade: Grade,
    /// Whether the layer is drawn into its blur group, which the last pass binds.
    grouped: bool,
}

/// Whether a layer has a shader pass that samples the backdrop.
fn reads_backdrop(layer: &Layer) -> bool {
    let effects = match layer {
        Layer::Solid(solid) => &solid.effects,
        Layer::Surface(surface) => &surface.effects,
        Layer::Overlay(_) => return false,
    };
    effects
        .passes()
        .iter()
        .any(|pass| matches!(pass, Pass::Shader(shader) if shader.backdrop()))
}

/// A solid's colour as the premultiplied bytes a shader reads it from.
fn premultiplied_bytes(colour: Color) -> [u8; 4] {
    let alpha = colour.a.clamp(0.0, 1.0);
    let byte = |value: f32| (value.clamp(0.0, 1.0) * alpha * 255.0).round() as u8;
    [
        byte(colour.r),
        byte(colour.g),
        byte(colour.b),
        (alpha * 255.0).round() as u8,
    ]
}

/// The layer's model placement, `None` for a flat layer.
fn layer_model(layer: &Layer) -> Option<&Model> {
    match layer {
        Layer::Solid(solid) => solid.model.as_ref(),
        Layer::Surface(surface) => surface.model.as_ref(),
        Layer::Overlay(_) => None,
    }
}

/// The projected layers in draw order: opaque ones front to back, then transparent ones back to
/// front, so every pair that can overlap is drawn with the right depth test.
///
/// Each entry is the layer's index in the frame and its placement. The clip matrix maps the unit
/// quad point `(x, y, 0, 1)` through the layer's model onto the output's device pixels, which
/// are y-down as the flat path's are; the depth is the one the vertex stage writes for the quad's
/// centre.
fn spatial_order(layers: &[Layer], physical: Size, scale: Scale) -> Vec<(usize, QuadPlacement)> {
    let s = scale.as_f64();
    let (w, h) = (f64::from(physical.width), f64::from(physical.height));
    let screen = Mat4 {
        m: [
            [2.0 * s / w, 0.0, 0.0, -1.0],
            [0.0, 2.0 * s / h, 0.0, -1.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    for (index, layer) in layers.iter().enumerate() {
        let (dst, model) = match layer {
            Layer::Solid(solid) => (solid.dst, solid.model.as_ref()),
            Layer::Surface(surface) => (surface.dst, surface.model.as_ref()),
            Layer::Overlay(_) => continue,
        };
        let Some(model) = model else {
            continue;
        };
        let matrix = screen
            .mul(&model.matrix)
            .mul(&Mat4::scale(f64::from(dst.width), f64::from(dst.height), 1.0));
        let column = |point: [f64; 4]| {
            let v = matrix.apply(point);
            [v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
        };
        let columns = [
            column([1.0, 0.0, 0.0, 0.0]),
            column([0.0, 1.0, 0.0, 0.0]),
            column([0.0, 0.0, 0.0, 1.0]),
        ];
        let centre = matrix.apply([0.5, 0.5, 0.0, 1.0]);
        let depth = ((centre[2] + centre[3]) * 0.5 / centre[3]) as f32;
        let (kind, bucket) = if model.opaque {
            (Depth::Opaque, &mut opaque)
        } else {
            (Depth::Transparent, &mut transparent)
        };
        bucket.push((
            index,
            QuadPlacement {
                depth_value: depth,
                ..QuadPlacement::projected(columns, kind)
            },
        ));
    }
    opaque.sort_by(|a, b| a.1.depth_value.total_cmp(&b.1.depth_value));
    transparent.sort_by(|a, b| b.1.depth_value.total_cmp(&a.1.depth_value));
    opaque.extend(transparent);
    opaque
}

/// Whether a layer can touch `damage` at all.
///
/// It is the cheap test that lets a frame skip a layer nowhere near the damaged rectangle,
/// including the padding a shadow draws into.
fn intersects(layer: &Layer, damage: Rect, scale: Scale) -> bool {
    let (dst, silhouette, effects) = match layer {
        Layer::Solid(solid) => (solid.dst, solid.silhouette, &solid.effects),
        Layer::Surface(surface) => (surface.dst, surface.silhouette, &surface.effects),
        // The overlay has its own pass, so no layer loop reaches it.
        Layer::Overlay(_) => return false,
    };
    // A shadow that follows the frame and a border that runs along it reach out from the silhouette, which can
    // lie beyond the layer's own rectangle (a titlebar's room above the content).
    let dst = silhouette.map_or(dst, |shape| {
        let left = dst.x.min(shape.x);
        let top = dst.y.min(shape.y);
        let right = (dst.x + dst.width as i32).max(shape.x + shape.width as i32);
        let bottom = (dst.y + dst.height as i32).max(shape.y + shape.height as i32);
        Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
    });
    let padded = effects.expand(RectF::new(
        f64::from(dst.x),
        f64::from(dst.y),
        f64::from(dst.width),
        f64::from(dst.height),
    ));
    let rect = physical_rect(
        Rect::new(
            padded.x.round() as i32,
            padded.y.round() as i32,
            padded.width.round().max(0.0) as u32,
            padded.height.round().max(0.0) as u32,
        ),
        scale,
    );
    rect.intersection(Rectangle::new(
        (damage.x, damage.y).into(),
        (damage.width as i32, damage.height as i32).into(),
    ))
    .is_some()
}

/// The texture coordinate mapping of a layer's source rectangle.
///
/// A client's shared-memory texture starts at its own top row, and the vertex stage's `local`
/// coordinate also starts at the top, so the mapping is the source rectangle divided by the
/// texture's size with no flip.
fn texture_mapping(surface: &Surface, texture: Texture) -> [f32; 4] {
    if let SurfaceSource::View(_) = &surface.source {
        return subtree::picture_mapping(
            [surface.src.x, surface.src.y, surface.src.width, surface.src.height],
            texture,
        );
    }
    let y_inverted = matches!(&surface.source, SurfaceSource::Dmabuf(dmabuf) if dmabuf.y_inverted());
    dmabuf::mapping(
        [surface.src.x, surface.src.y, surface.src.width, surface.src.height],
        texture.size(),
        y_inverted,
    )
}

/// The mapping of a rectangle of the renderer's own target, whose texture starts at the
/// bottom row.
///
/// The v axis is flipped by stating a negative height and an origin shifted by the height, so
/// a pass that adds a texture-space step to the coordinate steps the right way on screen.
fn flipped_mapping(rect: Rect, physical: Size) -> [f32; 4] {
    let height = physical.height as f32;
    let width = physical.width as f32;
    let top = height - (rect.y + rect.height as i32) as f32;
    [
        rect.x as f32 / width,
        (top + rect.height as f32) / height,
        rect.width as f32 / width,
        -(rect.height as f32) / height,
    ]
}

/// Reverses the rows of a bottom-up readback in place.
fn flip_rows(bytes: &mut [u8], stride: usize) {
    if stride == 0 {
        return;
    }
    let height = bytes.len() / stride;
    let mut buffer = vec![0u8; stride];
    for row in 0..height / 2 {
        let first = row * stride;
        let second = (height - 1 - row) * stride;
        buffer.copy_from_slice(&bytes[first..first + stride]);
        bytes.copy_within(second..second + stride, first);
        bytes[second..second + stride].copy_from_slice(&buffer);
    }
}

/// Copies all of `picture`, a target of this renderer whose GL row 0 is the bottom of the picture, over all of a
/// scan-out buffer, whose GL row 0 is its first memory row and so the top scan line: a blit with the rows reversed.
fn scan_out(device: &mut GlContext, picture: &Framebuffer, buffer: &Framebuffer) -> Result<(), RenderError> {
    let (from, to) = (picture.size(), buffer.size());
    device.blit_between(
        picture,
        buffer,
        Rect::new(0, 0, from.width, from.height),
        Rect::new(0, 0, to.width, to.height),
        true,
    )
}

mod capture;
mod dmabuf;
mod downsample;
mod effects;
mod subtree;
mod timing;
mod view;

#[cfg(test)]
mod downsample_tests;
#[cfg(test)]
mod effect_tests;
#[cfg(test)]
mod grade_tests;
#[cfg(test)]
mod shader_tests;
#[cfg(test)]
mod shape_tests;
#[cfg(test)]
mod tests;
