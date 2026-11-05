//! The GLES renderer: Smithay's GLES context, `glow` for the drawing, one frame in and an
//! [`Image`] out.
//!
//! This is D12's GPU half — "GLES now, Vulkan later" — drawn on Smithay's GLES renderer
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
//! `naga`, the compiler D12 chose, has no GLSL ES 1.00 output. D12 also says every effect
//! and user shader is written *once* in one language and compiled per backend, so the
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
//! 3. Its uniform layout rules agree with GLSL's `std140` for every type the uniform schema
//!    declares, so one set of byte offsets describes both targets. `program.rs` is the CPU
//!    side of that agreement and `tests::a_user_shader_reads_its_uniforms` is the proof.
//!
//! The camera is orthographic and drawing is in painter's order, which is the 2D subset the
//! scene accepts today; the transform is a rectangle, exactly as the plan states for this
//! phase.
//!
//! # What a pass is
//!
//! A pass draws one quad. The vertex stage is shared ([`shaders::VERTEX`]) and takes the
//! destination rectangle and the target's size as attributes, so the fragment stage is the
//! only stage with a uniform block: one block per program, bound once, overwritten per
//! draw. That is what makes "uniforms typed-checked and uploaded" a single small write of a
//! 96-byte buffer plus the user's own fields.
//!
//! | Pass    | Shader                 | Draws                                        |
//! | ------- | ---------------------- | -------------------------------------------- |
//! | solid   | [`shaders::SOLID`]     | the background, a solid layer, the cursor     |
//! | surface | [`shaders::SURFACE`]   | a client buffer, tinted and corner-clipped    |
//! | shadow  | [`shaders::SHADOW`]    | a [`Pass::Shadow`], behind its layer          |
//! | blur    | [`shaders::BLUR`]      | one axis of a Gaussian over the backdrop      |
//! | shader  | a user's WGSL          | the layer through the user's `shade`          |
//!
//! # Where the pixels go
//!
//! The target is an `RGBA8` texture. The vertex stage maps pixel `y` downwards while clip
//! space is upwards, so row 0 of the target is the output's top row; a pass that samples the
//! target (the blur) therefore states its source with a negative v scale
//! ([`flipped_mapping`]). `glReadPixels` counts from the *bottom* of a framebuffer, so
//! [`Gles::screenshot`] reverses the rows it gets. That flip is the one place orientation
//! can go wrong, and `tests::a_readback_is_top_left_first` is what catches it.
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
//! 5. **A broken user shader kills the frame loop.** A shader that fails to compile disables
//!    that pass, is reported once, and the frame is still drawn:
//!    `tests::a_broken_user_shader_leaves_the_frame_drawn`.
//! 6. **A uniform of the wrong type is uploaded as garbage.** A value whose type the
//!    registered schema does not declare is refused before a byte of it is uploaded:
//!    `tests::a_user_shader_refuses_a_mistyped_uniform`.
//! 7. **GL state leaks from one frame into the next.** Every frame begins from the state
//!    [`Device::begin`] sets and every pass resets what it changes:
//!    `tests::a_frame_does_not_inherit_the_last_frames_state`.
//! 8. **Damage is ignored and the whole output is repainted.** Only the damage is scissored
//!    in, so an undamaged pixel keeps what it had:
//!    `tests::damage_limits_what_is_repainted`.
//! 9. **A resize keeps the old target.** [`Gles::resize`] replaces the target and the new one
//!    holds no pixels: `tests::a_resize_replaces_the_target`.

pub mod egl;
pub(crate) mod gl;
mod program;
mod shaders;

use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::glow::GlowRenderer;
use smithay::backend::renderer::{ImportMem, ImportMemWl};
use smithay::utils::Rectangle;
use tracing::warn;

use gnoblin_render::{Color, Effects, Mat4, Pass, UniformSchema};
use gnoblin_types::{Rect, RectF, Scale, Size};

use crate::render::RenderError;
use crate::render::frame::{Layer, Model, OutputFrame, Overlay, Solid, Surface};
use crate::render::image::Image;
use crate::render::physical_rect;
use crate::render::software::READBACK_FORMAT;

pub use program::ShaderError;

use gl::{Depth, Device, Framebuffer, Placement, Program, Texture};
use program::Uniforms;

/// The largest corner radius or blur radius a pass draws, in physical pixels.
///
/// A radius larger than half the shorter side is clamped by the shader anyway; this bound
/// keeps the buffer arithmetic far from an overflow.
const MAX_RADIUS: f32 = 4096.0;

/// A user shader: the WGSL a caller registered and the uniforms it declares.
///
/// The source defines one function,
///
/// ```wgsl
/// fn shade(uv: vec2<f32>) -> vec4<f32>
/// ```
///
/// whose `uv` is the layer's texture coordinate — already cropped and scaled — and whose
/// return value is the layer's premultiplied colour. The shader may read the uniform block
/// (`gnoblin.<field>`), sample the layer (`textureSample(gnoblin_tex, gnoblin_sampler, uv)`)
/// and use the framework's own members, such as `gnoblin.rect` and `gnoblin.opacity`.
///
/// The schema is the contract between the shader and the scene: every field it declares is
/// appended to the uniform block in declaration order, and a [`Pass::Shader`] whose values do
/// not match those declarations is refused rather than uploaded.
#[derive(Clone, PartialEq, Debug)]
pub struct UserShader {
    /// The WGSL source.
    pub source: String,
    /// The uniforms the source declares.
    pub schema: UniformSchema,
}

/// A GLES renderer and the offscreen target it draws into.
pub struct Gles {
    /// The GL context and the pass machinery.
    device: Device,
    /// The output-sized target, replaced by [`Gles::resize`].
    target: Framebuffer,
    /// The two scratch targets the blur passes need, made on first use.
    scratch: Option<[Framebuffer; 2]>,
    /// The effect programs, compiled once.
    passes: Passes,
    /// The registered user shader and the program its source compiled to.
    user: Option<Registered>,
    /// A shader failure that has not been reported yet.
    ///
    /// It is reported once and then dropped: a shader that fails every frame would otherwise
    /// log a line per frame for the life of the session.
    error: Option<ShaderError>,
    /// The target's size in physical pixels.
    physical: Size,
    /// How many frames were drawn into the target. A frame with no damage leaves this alone,
    /// which is what makes an idle iteration free.
    draws: u64,
    /// The frame's damage in physical pixels, reused across frames so a steady frame
    /// allocates nothing.
    damage: Vec<Rect>,
}

impl std::fmt::Debug for Gles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Gles")
            .field("physical", &self.physical)
            .field("draws", &self.draws)
            .field("user", &self.user.is_some())
            .finish_non_exhaustive()
    }
}

/// The effect programs a frame can draw with.
struct Passes {
    solid: Program,
    surface: Program,
    shadow: Program,
    blur: Program,
}

/// One scan-out buffer of an output: the dmabuf imported as a GL texture and the framebuffer
/// that draws into it. Both are GL objects of the output's context, dropped with it.
pub(crate) struct ScanBuffer {
    /// The framebuffer the frame is drawn into.
    framebuffer: Framebuffer,
    /// The texture behind the framebuffer. Kept alive for as long as the framebuffer is used.
    _texture: GlesTexture,
}

/// A registered user shader and the program it compiled to, if it compiled.
struct Registered {
    /// The source and schema the caller registered.
    shader: UserShader,
    /// The program, or [`None`] when the source was refused.
    program: Option<Program>,
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
        let mut device = Device::on(renderer)?;
        let target = device.target(physical)?;
        let passes = Passes::new(&mut device)?;
        Ok(Self {
            device,
            target,
            scratch: None,
            passes,
            user: None,
            error: None,
            physical,
            draws: 0,
            damage: Vec::new(),
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
    pub(crate) fn on_display(display: &smithay::backend::egl::EGLDisplay, physical: Size) -> Result<Self, RenderError> {
        Self::on(egl::context_on(display)?, physical)
    }

    /// The target's size in physical pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.physical
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
        let target = self.device.target(physical)?;
        self.target = target;
        self.physical = physical;
        // The scratch targets are as big as the output they copy, so a resize invalidates
        // them; they are rebuilt on the first blur of the new size.
        self.scratch = None;
        Ok(())
    }

    /// Registers the user shader a layer's [`Pass::Shader`] runs, or clears it.
    ///
    /// The source is compiled once here, at registration, so a broken shader is known before
    /// the first frame that would draw it and costs that frame nothing. A failure leaves no
    /// program registered, is reported once (see [`Gles::take_shader_error`]) and leaves the
    /// layer drawn by the default surface pass: one window's shader must never be the reason
    /// a session stops drawing.
    pub fn set_user_shader(&mut self, shader: Option<UserShader>) {
        let Some(shader) = shader else {
            self.user = None;
            return;
        };
        match program::compile(&mut self.device, &shader.source, &shader.schema) {
            Ok(program) => {
                self.user = Some(Registered {
                    shader,
                    program: Some(program),
                });
            }
            Err(error) => {
                warn!(
                    line = error.line(),
                    "[render] a user shader was refused; its pass is disabled"
                );
                self.error = Some(error);
                self.user = Some(Registered { shader, program: None });
            }
        }
    }

    /// The shader failure that has not been reported yet, cleared by the call.
    ///
    /// The frame loop drains this once per iteration and reports it; a caller that ignores it
    /// does not accumulate failures, because the next one replaces this.
    pub fn take_shader_error(&mut self) -> Option<ShaderError> {
        self.error.take()
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
    /// of the frame still has to appear. A broken user shader is not an error either — it is
    /// reported once and its pass is skipped.
    pub fn draw(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), RenderError> {
        if frame.is_empty() {
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
        if damage.is_empty() {
            // Everything the frame wanted to redraw is outside the output.
            self.damage = damage;
            return Ok(());
        }
        let outcome = self.paint(frame, scale, &damage);
        self.damage = damage;
        outcome?;
        self.draws = self.draws.saturating_add(1);
        Ok(())
    }

    /// Imports a dmabuf as a scan-out buffer this renderer can draw into.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver cannot import the buffer, and
    /// [`RenderError::Unavailable`] when the import has no GL name. The reason is logged.
    pub(crate) fn import_scanout(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<ScanBuffer, RenderError> {
        use smithay::backend::renderer::ImportDma as _;
        let texture = self.device.renderer().import_dmabuf(dmabuf, None).map_err(|error| {
            warn!(%error, "[render] a scan-out buffer cannot be imported");
            RenderError::Failed
        })?;
        let framebuffer = self.device.framebuffer_over(&texture)?;
        Ok(ScanBuffer {
            framebuffer,
            _texture: texture,
        })
    }

    /// Draws `frame` into a scan-out buffer, repainting all of it.
    ///
    /// A scan-out buffer holds the picture of an earlier flip, not the one this frame damaged, so
    /// the whole output is painted and the frame's damage is not used. A frame with no layers paints
    /// its background.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses an operation.
    pub(crate) fn draw_scanout(
        &mut self,
        buffer: &mut ScanBuffer,
        frame: &OutputFrame,
        scale: Scale,
    ) -> Result<(), RenderError> {
        let full = [Rect::new(0, 0, self.physical.width, self.physical.height)];
        std::mem::swap(&mut self.target, &mut buffer.framebuffer);
        let outcome = self.paint(frame, scale, &full);
        std::mem::swap(&mut self.target, &mut buffer.framebuffer);
        outcome?;
        self.draws = self.draws.saturating_add(1);
        Ok(())
    }

    /// Reads the target back as an [`Image`].
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the read or returns fewer bytes than
    /// the target holds.
    pub fn screenshot(&mut self) -> Result<Image, RenderError> {
        let size = self.physical;
        let mut bytes = self.device.read(&self.target)?;
        let stride = size.width as usize * 4;
        let expected = stride.saturating_mul(size.height as usize);
        if bytes.len() < expected {
            warn!(
                got = bytes.len(),
                expected, "[render] the GLES readback is short of the target"
            );
            return Err(RenderError::Failed);
        }
        bytes.truncate(expected);
        // `glReadPixels` starts at the framebuffer's bottom-left corner, so the rows arrive
        // bottom first. An `Image` is top-left first.
        flip_rows(&mut bytes, stride);
        Image::new(size, stride, bytes).ok_or(RenderError::Failed)
    }

    /// Draws every layer of the frame, damage rectangle by damage rectangle.
    fn paint(&mut self, frame: &OutputFrame, scale: Scale, damage: &[Rect]) -> Result<(), RenderError> {
        // Textures first: importing needs the renderer mutably, and a pass borrows it for the
        // whole of the draw.
        let mut textures: Vec<Option<Texture>> = Vec::with_capacity(frame.layers().len());
        for layer in frame.layers() {
            textures.push(match layer {
                Layer::Solid(_) => None,
                Layer::Surface(surface) => self.import(surface),
            });
        }
        // The imported texture owns the GL name, so it lives until the frame is painted.
        let overlay_texture = frame.overlay().and_then(|overlay| self.import_overlay(overlay.image()));
        let overlay = frame
            .overlay()
            .zip(overlay_texture.as_ref().and_then(Texture::from_smithay));
        let background = frame.background();
        let Self {
            device,
            target,
            scratch,
            passes,
            user,
            error,
            physical,
            ..
        } = self;
        let order = spatial_order(frame.layers(), *physical, scale);
        device.begin(target)?;
        let mut painter = Painter {
            device,
            target,
            scratch,
            passes,
            user,
            error,
            physical: *physical,
            scale,
            background,
            bytes: Vec::new(),
            clip: Rect::new(0, 0, 0, 0),
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
                painter.layer(layer, texture, Placement::FLAT)?;
            }
            if !order.is_empty() {
                painter.device.clear_depth(painter.target);
            }
            for (index, placement) in &order {
                let layer = &frame.layers()[*index];
                let texture = textures.get(*index).and_then(|texture| *texture);
                painter.layer(layer, texture, *placement)?;
            }
            if let Some((overlay, texture)) = overlay {
                painter.overlay(overlay, texture)?;
            }
            if let Some(cursor) = frame.cursor() {
                painter.cursor(cursor)?;
            }
        }
        painter.finish()
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
        self.passes.surface.pack_fixed(&uniforms, &mut bytes);
        self.device
            .draw(&self.passes.surface, &bytes, quad, Some(texture), Placement::FLAT)?;
        self.device.scissor(None);
        self.device.end()?;
        self.draws = self.draws.saturating_add(1);
        Ok(())
    }

    /// Imports a surface layer's client buffer.
    fn import(&mut self, surface: &Surface) -> Option<Texture> {
        let texture = self
            .device
            .renderer()
            .import_shm_buffer(&surface.buffer, None, &[])
            .map_err(|error| {
                warn!(
                    ?error,
                    "[render] a client buffer could not be imported; the layer is skipped"
                );
            })
            .ok()?;
        Texture::from_smithay(&texture)
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
                warn!(?error, "[render] the overlay could not be imported; it is not drawn");
            })
            .ok()?;
        Some(texture)
    }
}

/// One frame's mutable state, so a pass's parameters stay a method's arguments.
struct Painter<'a> {
    device: &'a mut Device,
    target: &'a Framebuffer,
    scratch: &'a mut Option<[Framebuffer; 2]>,
    passes: &'a mut Passes,
    user: &'a mut Option<Registered>,
    error: &'a mut Option<ShaderError>,
    physical: Size,
    scale: Scale,
    background: Color,
    /// The packed uniforms of the pass in progress, reused across passes.
    bytes: Vec<u8>,
    /// The damage rectangle every pass is scissored to.
    clip: Rect,
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
        passes.solid.pack_fixed(&uniforms, bytes);
        device.draw(&passes.solid, bytes, rect, None, Placement::FLAT)
    }

    /// Draws a layer: its shadow, its backdrop effect, then its own pixels.
    fn layer(&mut self, layer: &Layer, texture: Option<Texture>, placement: Placement) -> Result<(), RenderError> {
        let (dst, corner_radius, effects, colour, tint, opacity, src) = match layer {
            Layer::Solid(solid) => (
                solid.dst,
                solid.corner_radius,
                &solid.effects,
                Some(solid.colour),
                solid.tint,
                1.0,
                [0.0, 0.0, 1.0, 1.0],
            ),
            Layer::Surface(surface) => (
                surface.dst,
                surface.corner_radius,
                &surface.effects,
                None,
                surface.tint,
                surface.opacity,
                texture.map_or([0.0, 0.0, 1.0, 1.0], |texture| texture_mapping(surface, texture)),
            ),
        };
        let dst = physical_rect(dst, self.scale);
        let quad = Rect::new(dst.loc.x, dst.loc.y, dst.size.w.max(0) as u32, dst.size.h.max(0) as u32);
        if quad.is_empty() {
            return Ok(());
        }
        let radius = (corner_radius * self.scale.as_f64() as f32).clamp(0.0, MAX_RADIUS);
        for pass in effects.passes() {
            match pass {
                Pass::Shadow(_) | Pass::Blur(_) if placement.clip.is_some() => {
                    warn!("unsupported-effect-on-3d: a shadow or blur on a transformed layer is not drawn");
                }
                Pass::Shadow(shadow) if shadow.opacity > 0.0 && shadow.color.a > 0.0 => {
                    self.shadow(quad, radius, shadow)?;
                }
                Pass::Blur(blur) if blur.radius > 0.0 => {
                    self.blur(quad, blur.radius * self.scale.as_f64() as f32)?;
                }
                Pass::Shadow(_) | Pass::Blur(_) | Pass::Shader(_) => {}
            }
        }
        let uniforms = match (colour, texture) {
            (Some(colour), _) => {
                let mut uniforms = Uniforms::solid(quad, colour, tint, opacity);
                uniforms.corner = radius;
                uniforms
            }
            (None, _) => Uniforms::surface(quad, src, tint, opacity, radius),
        };
        // A shader pass replaces the layer's own fragment stage; a refusal is reported once
        // and the default pass draws the layer instead, so one window's shader cannot blank
        // the output.
        let mut refusal = None;
        let drawn = {
            let Self {
                device,
                passes,
                user,
                bytes,
                ..
            } = self;
            match resolve(user, effects) {
                Ok(Some((program, values))) => match program.pack(&uniforms, &values, bytes) {
                    Ok(()) => device.draw(program, bytes, quad, texture, placement),
                    Err(error) => {
                        refusal = Some(error);
                        let default = if texture.is_some() {
                            &passes.surface
                        } else {
                            &passes.solid
                        };
                        default.pack_fixed(&uniforms, bytes);
                        device.draw(default, bytes, quad, texture, placement)
                    }
                },
                Ok(None) => {
                    let default = if texture.is_some() {
                        &passes.surface
                    } else {
                        &passes.solid
                    };
                    default.pack_fixed(&uniforms, bytes);
                    device.draw(default, bytes, quad, texture, placement)
                }
                Err(error) => {
                    refusal = Some(error);
                    let default = if texture.is_some() {
                        &passes.surface
                    } else {
                        &passes.solid
                    };
                    default.pack_fixed(&uniforms, bytes);
                    device.draw(default, bytes, quad, texture, placement)
                }
            }
        };
        if let Some(error) = refusal {
            self.report(error);
        }
        drawn
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
        passes.surface.pack_fixed(&uniforms, bytes);
        device.draw(&passes.surface, bytes, quad, Some(texture), Placement::FLAT)
    }

    /// The cursor, above every layer.
    fn cursor(&mut self, cursor: &Solid) -> Result<(), RenderError> {
        let Self {
            device,
            passes,
            bytes,
            scale,
            ..
        } = self;
        let dst = physical_rect(cursor.dst, *scale);
        let quad = Rect::new(dst.loc.x, dst.loc.y, dst.size.w.max(0) as u32, dst.size.h.max(0) as u32);
        if quad.is_empty() {
            return Ok(());
        }
        let mut uniforms = Uniforms::solid(quad, cursor.colour, cursor.tint, 1.0);
        uniforms.corner = (cursor.corner_radius * scale.as_f64() as f32).clamp(0.0, MAX_RADIUS);
        passes.solid.pack_fixed(&uniforms, bytes);
        device.draw(&passes.solid, bytes, quad, None, Placement::FLAT)
    }

    /// Draws a shadow behind a layer.
    fn shadow(&mut self, dst: Rect, radius: f32, shadow: &gnoblin_render::Shadow) -> Result<(), RenderError> {
        let Self {
            device,
            passes,
            bytes,
            scale,
            ..
        } = self;
        let blur = (shadow.blur * scale.as_f64() as f32).clamp(1.0, MAX_RADIUS);
        let inset = blur.round() as i32;
        let offset_x = (shadow.offset.x as f32 * scale.as_f64() as f32).round() as i32;
        let offset_y = (shadow.offset.y as f32 * scale.as_f64() as f32).round() as i32;
        let quad = Rect::new(
            dst.x + offset_x - inset,
            dst.y + offset_y - inset,
            dst.width + 2 * inset as u32,
            dst.height + 2 * inset as u32,
        );
        let mut uniforms = Uniforms::solid(quad, shadow.color, Color::WHITE, shadow.opacity);
        uniforms.corner = radius;
        uniforms.params = [blur, blur, blur, 0.0];
        passes.shadow.pack_fixed(&uniforms, bytes);
        device.draw(&passes.shadow, bytes, quad, None, Placement::FLAT)
    }

    /// Blurs the pixels the layer is about to cover.
    ///
    /// Two passes of the separable Gaussian in [`shaders::BLUR`]: horizontal into a scratch
    /// target, vertical back into the frame. Both read the *backdrop* — the pixels already
    /// drawn — so what a window shows behind itself is what was behind it, not what it
    /// covers.
    ///
    /// The copied region is the layer's rectangle grown by the kernel's reach, so the taps
    /// near the layer's edge read blurred pixels rather than the scratch target's clearing
    /// colour; the final write is scissored to the layer's rectangle, so that ring is the
    /// only price of it.
    fn blur(&mut self, dst: Rect, radius: f32) -> Result<(), RenderError> {
        let radius = radius.clamp(1.0, MAX_RADIUS);
        let Some(rect) = dst.intersection(Rect::new(0, 0, self.physical.width, self.physical.height)) else {
            return Ok(());
        };
        let reach = (radius / 3.0 * 4.0).ceil() as u32 + 1;
        let region = Rect::new(
            rect.x.saturating_sub(reach as i32),
            rect.y.saturating_sub(reach as i32),
            rect.width + 2 * reach,
            rect.height + 2 * reach,
        )
        .intersection(Rect::new(0, 0, self.physical.width, self.physical.height))
        .unwrap_or(rect);
        self.ensure_scratch()?;
        let Self {
            device,
            target,
            scratch,
            passes,
            bytes,
            physical,
            background,
            clip,
            ..
        } = self;
        let Some([first, second]) = scratch.as_mut() else {
            return Ok(());
        };
        // The scratch target starts as the frame's background everywhere the copy does not
        // reach, so a blur that samples past the output repeats a colour the frame defined.
        device.clear(first, *background);
        device.copy(target, first, region)?;
        let step = (radius / 3.0).max(0.5);
        let mut uniforms = Uniforms::surface(region, flipped_mapping(region, *physical), Color::WHITE, 1.0, 0.0);
        uniforms.params = [step / physical.width as f32, 0.0, 0.0, 0.0];
        device.begin(second)?;
        device.scissor(Some(region));
        passes.blur.pack_fixed(&uniforms, bytes);
        device.draw(&passes.blur, bytes, region, Some(first.texture()), Placement::FLAT)?;
        device.end()?;
        // The vertical pass writes back into the frame, scissored to the layer's rectangle:
        // that is the region the pass is for, and the ring exists only to give its taps a
        // neighbourhood.
        device.begin(target)?;
        let write = rect.intersection(*clip).unwrap_or(rect);
        device.scissor(Some(write));
        let uniforms = Uniforms {
            params: [0.0, step / physical.height as f32, 0.0, 0.0],
            ..uniforms
        };
        passes.blur.pack_fixed(&uniforms, bytes);
        device.draw(&passes.blur, bytes, region, Some(second.texture()), Placement::FLAT)?;
        // Back to the damage scissor, which the rest of this rectangle's passes expect.
        device.scissor(Some(*clip));
        Ok(())
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

    /// Records a refusal, once per frame at most: a shader that fails every frame must not
    /// log a line per frame for the life of the session.
    fn report(&mut self, error: ShaderError) {
        if self.error.is_none() {
            warn!(%error, "[render] a user shader was refused; its pass is disabled");
            *self.error = Some(error);
        }
    }

    /// Lifts the scissor and sends the frame to the driver.
    fn finish(&mut self) -> Result<(), RenderError> {
        let Self { device, .. } = self;
        device.scissor(None);
        device.end()
    }
}

/// The program and uniform values a layer's shader pass runs with.
///
/// A pass whose schema is not the registered shader's is a refusal: the values belong to the
/// pass, the declaration belongs to the source, and only agreement between them makes an
/// upload meaningful.
fn resolve<'a>(
    user: &'a mut Option<Registered>,
    effects: &Effects,
) -> Result<Option<(&'a Program, Vec<gnoblin_render::ResolvedUniform>)>, ShaderError> {
    let Some(pass) = effects.passes().iter().find_map(|pass| match pass {
        Pass::Shader(shader) => Some(shader),
        Pass::Blur(_) | Pass::Shadow(_) => None,
    }) else {
        return Ok(None);
    };
    let Some(registered) = user.as_mut() else {
        // A shader pass with no registered source draws the layer normally: the caller asked
        // for a pass the session has not provided a shader for.
        return Ok(None);
    };
    if pass.schema() != &registered.shader.schema {
        return Err(ShaderError::refusal(
            "a pass declares a uniform schema the registered shader does not have".to_owned(),
        ));
    }
    Ok(registered.program.as_ref().map(|program| (program, pass.resolved())))
}

impl Passes {
    /// Compiles the effect programs.
    fn new(device: &mut Device) -> Result<Self, RenderError> {
        Ok(Self {
            solid: program::link(device, shaders::VERTEX, shaders::SOLID)?,
            surface: program::link(device, shaders::VERTEX, shaders::SURFACE)?,
            shadow: program::link(device, shaders::VERTEX, shaders::SHADOW)?,
            blur: program::link(device, shaders::VERTEX, shaders::BLUR)?,
        })
    }
}

/// The layer's model placement, `None` for a flat layer.
fn layer_model(layer: &Layer) -> Option<&Model> {
    match layer {
        Layer::Solid(solid) => solid.model.as_ref(),
        Layer::Surface(surface) => surface.model.as_ref(),
    }
}

/// The projected layers in draw order: opaque ones front to back, then transparent ones back to
/// front, so every pair that can overlap is drawn with the right depth test.
///
/// Each entry is the layer's index in the frame and its placement. The clip matrix maps the unit
/// quad point `(x, y, 0, 1)` through the layer's model onto the output's device pixels, which
/// are y-down as the flat path's are; the depth is the one the vertex stage writes for the quad's
/// centre.
fn spatial_order(layers: &[Layer], physical: Size, scale: Scale) -> Vec<(usize, Placement)> {
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
            Placement {
                depth_value: depth,
                ..Placement::projected(columns, kind)
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
    let (dst, effects) = match layer {
        Layer::Solid(solid) => (solid.dst, &solid.effects),
        Layer::Surface(surface) => (surface.dst, &surface.effects),
    };
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
    let (width, height) = texture.size();
    [
        surface.src.x as f32 / width as f32,
        surface.src.y as f32 / height as f32,
        surface.src.width as f32 / width as f32,
        surface.src.height as f32 / height as f32,
    ]
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

#[cfg(test)]
mod tests;
