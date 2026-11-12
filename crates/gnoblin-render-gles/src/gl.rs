//! The crate's one OpenGL ES shim.
//!
//! D12 puts the GPU path on Smithay's GLES renderer, whose safe half is the EGL
//! context (see [`super::egl`]) and whose drawing half is `glow`, the safe-API wrapper
//! Smithay's `renderer_glow` feature hands out. `glow`'s calls are still `unsafe` —
//! each one takes the caller's word that the GL state it is about to change is not in
//! use elsewhere — so every one of them lives in this module, and the rest of the
//! renderer is ordinary Rust that names [`GlContext`] and never a `glow` type.
//!
//! That split is what the workspace's unsafe policy asks for: one FFI shim module,
//! opted out with `#[allow(unsafe_code)]`, a `// SAFETY:` comment per block, and no
//! `unsafe` anywhere else in the crate. Every block here is of one kind: the renderer
//! owns the context, all of its GL calls are made from one thread through this module,
//! and each pass that changes state sets what it needs and resets what it changed.
//!
//! # Objects are freed by the context, not one by one
//!
//! A [`Framebuffer`] and a [`Program`] own GL names but do not delete them: the context that
//! created them is destroyed with the renderer, and destroying a context frees every
//! object it made. Deleting them individually would need a current context at drop
//! time, which is exactly the state this module cannot promise on a failure path.
#![allow(unsafe_code)]

use std::fmt;
use std::num::NonZeroU32;

use glow::HasContext;

use smithay::backend::renderer::Texture as _;
use smithay::backend::renderer::gles::{GlesError, GlesTexture};
use smithay::backend::renderer::glow::GlowRenderer;

use gnoblin_types::{Color, Rect, Size};

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::shader::Stage;
use gnoblin_render_frame::shader::uniforms::{BLOCK_BYTES, Uniforms};

use crate::log;

/// A driver failure, before it is known which line of a source it came from.
pub(crate) struct CompileFailure {
    /// The stage that failed.
    pub stage: Stage,
    /// The driver's log.
    pub log: String,
}

impl fmt::Debug for CompileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompileFailure")
            .field("stage", &self.stage)
            .field("log", &self.log)
            .finish()
    }
}

/// The binding the uniform buffer is uploaded to.
const UNIFORM_BINDING: u32 = 0;

/// The texture unit every pass samples from.
const TEXTURE_UNIT: u32 = 0;

/// The floats one quad vertex carries: the unit position, the destination rectangle and the
/// target's size, then the three columns of the placement's clip matrix, then the placement's
/// parameters.
const VERTEX_FLOATS: usize = 24;

/// How a quad takes part in the depth test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Depth {
    /// No depth test and no depth write: painter's order, the flat path.
    Off,
    /// Tested and written: an opaque surface, drawn front to back.
    Opaque,
    /// Tested but not written: a transparent surface, drawn back to front after the opaque ones.
    Transparent,
}

/// How a quad takes part in the stencil test a View's clipping ancestors use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stencil {
    /// No stencil test: the quad is drawn wherever it lands.
    Off,
    /// Drawn only where the stencil holds this clip depth, which is where every clip of the chain covers.
    Test(u8),
    /// The rectangle of a clip that begins at this depth: it raises the stencil by one where the chain before it
    /// covered, and draws no colour.
    Push(u8),
    /// The rectangle of the clip that ends at this depth: it lowers the stencil by one where the chain covers, and
    /// draws no colour.
    Pop(u8),
}

/// Where a quad sits in a frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct QuadPlacement {
    /// Columns 0, 1 and 3 of the output's clip matrix, applied to the unit quad point `(x, y, 0, 1)`.
    /// `None` draws the quad flat, in the output's logical pixels.
    pub(crate) clip: Option<[[f32; 4]; 3]>,
    /// The quad's normalised device depth when it is flat. A projected quad takes its depth from `clip`.
    pub(crate) depth_value: f32,
    /// How the quad takes part in the depth test.
    pub(crate) depth: Depth,
    /// How the quad takes part in the stencil test of a View's clipping ancestors.
    pub(crate) stencil: Stencil,
}

impl QuadPlacement {
    /// The flat path: painter's order, no depth test.
    pub(crate) const FLAT: Self = Self {
        clip: None,
        depth_value: 0.0,
        depth: Depth::Off,
        stencil: Stencil::Off,
    };

    /// A quad drawn through `columns`, the clip matrix's columns 0, 1 and 3, with the given depth
    /// behaviour.
    pub(crate) const fn projected(columns: [[f32; 4]; 3], depth: Depth) -> Self {
        Self {
            clip: Some(columns),
            depth_value: 0.0,
            depth,
            stencil: Stencil::Off,
        }
    }
}

/// The GL context and the calls every pass is made of.
pub(crate) struct GlContext {
    /// The Smithay renderer that owns the EGL context and imports client buffers.
    renderer: GlowRenderer,
    /// The one-pixel white texture a pass binds when it has no layer texture.
    ///
    /// A solid layer carries no pixels, but every pass declares `gnoblin_tex`, so it needs a bound texture;
    /// white is the identity for a multiply, so the pass's own colour is the only
    /// thing that shows.
    white: Texture,
    /// The size of the target last bound, which the vertex stage needs as an attribute.
    viewport: (i32, i32),
    /// The one framebuffer a copy into, or a read of, a client's texture attaches that texture to, made on
    /// first use. Framebuffers are not freed one by one (see the module docs), so a copy cannot make one per
    /// call.
    scratch: Option<glow::Framebuffer>,
}

impl GlContext {
    /// The device on `renderer`'s context, which the caller has made current or will have
    /// made current by the time this returns.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the context cannot draw a 1×1 texture.
    pub(crate) fn on(mut renderer: GlowRenderer) -> Result<Self, RenderError> {
        let white = renderer
            .with_context(|gl| unsafe {
                // SAFETY: the context is current (that is what `with_context` establishes)
                // and the one name created here is stored in `GlContext`, which the context
                // outlives.
                let name = gl.create_texture().map_err(|error| {
                    log::debug!(area = "render", %error, "no white texture for the GLES renderer");
                    RenderError::Unavailable
                })?;
                gl.bind_texture(glow::TEXTURE_2D, Some(name));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    1,
                    1,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&[255, 255, 255, 255])),
                );
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
                gl.bind_texture(glow::TEXTURE_2D, None);
                Ok(Texture {
                    handle: name,
                    size: (1, 1),
                })
            })
            .map_err(context_lost)??;
        Ok(Self {
            renderer,
            white,
            viewport: (0, 0),
            scratch: None,
        })
    }

    /// The Smithay renderer, for the client-buffer import only.
    pub(crate) fn renderer(&mut self) -> &mut GlowRenderer {
        &mut self.renderer
    }

    /// The renderer the driver reports.
    pub(crate) fn renderer_name(&mut self) -> String {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: reading a string parameter needs only a current context, which
                // `with_context` establishes, and changes no state.
                gl.get_parameter_string(glow::RENDERER)
            })
            .unwrap_or_else(|_| "unknown".to_owned())
    }

    /// Whether the driver is a software rasteriser.
    pub(crate) fn is_software(&mut self) -> bool {
        let name = self.renderer_name().to_ascii_lowercase();
        name.contains("llvmpipe") || name.contains("softpipe") || name.contains("swrast")
    }

    /// Links a program from GLSL sources.
    ///
    /// # Errors
    ///
    /// [`CompileFailure`] with the driver's log.
    pub(crate) fn program(&mut self, vertex: &str, fragment: &str) -> Result<Program, CompileFailure> {
        let built = self.renderer.with_context(|gl| unsafe {
            // SAFETY: compiling and linking only create and destroy names this closure owns;
            // nothing outside it refers to them until `Program` takes ownership of the linked
            // program and its buffers.
            let vertex_shader = compile(gl, true, vertex)?;
            let fragment_shader = match compile(gl, false, fragment) {
                Ok(shader) => shader,
                Err(failure) => {
                    gl.delete_shader(vertex_shader);
                    return Err(failure);
                }
            };
            let program = gl.create_program().map_err(|log| CompileFailure {
                stage: Stage::Link,
                log,
            })?;
            gl.attach_shader(program, vertex_shader);
            gl.attach_shader(program, fragment_shader);
            gl.link_program(program);
            let linked = gl.get_program_link_status(program);
            let log = gl.get_program_info_log(program);
            gl.detach_shader(program, vertex_shader);
            gl.detach_shader(program, fragment_shader);
            gl.delete_shader(vertex_shader);
            gl.delete_shader(fragment_shader);
            if !linked {
                return Err(CompileFailure {
                    stage: Stage::Link,
                    log,
                });
            }
            // Every pass declares exactly one uniform block, and binding it here is what
            // lets a draw only upload its bytes.
            let blocks = gl.get_program_parameter_i32(program, glow::ACTIVE_UNIFORM_BLOCKS);
            gl.uniform_block_binding(program, 0, UNIFORM_BINDING);
            let buffer = gl.create_buffer().map_err(|log| CompileFailure {
                stage: Stage::Link,
                log,
            })?;
            let geometry = gl.create_buffer().map_err(|log| CompileFailure {
                stage: Stage::Link,
                log,
            })?;
            Ok((program, buffer, geometry, blocks))
        });
        let (program, buffer, geometry, blocks) = match built {
            Ok(Ok(built)) => built,
            Ok(Err(failure)) => return Err(failure),
            Err(error) => return Err(context_compile_lost(error)),
        };
        if blocks != 1 {
            // The shaders are written so that the fragment stage is the only one with a
            // uniform block; a second block would be a program that silently ignored half of
            // its uniforms.
            log::debug!(
                area = "render",
                blocks,
                "a pass declares an unexpected number of uniform blocks"
            );
            return Err(CompileFailure {
                stage: Stage::Link,
                log: format!("the shader declares {blocks} uniform blocks, not one"),
            });
        }
        Ok(Program {
            program,
            buffer,
            geometry,
        })
    }

    /// A target of `size` physical pixels, holding no defined pixel.
    pub(crate) fn target(&mut self, size: Size) -> Result<Framebuffer, RenderError> {
        let (width, height) = (size.width as i32, size.height as i32);
        if width <= 0 || height <= 0 {
            return Err(RenderError::Degenerate {
                width: size.width,
                height: size.height,
            });
        }
        let texture = self
            .renderer
            .with_context(|gl| unsafe {
                // SAFETY: the name created here belongs to this context and is stored in the
                // framebuffer that `framebuffer` builds over it, which the context outlives.
                let texture = gl.create_texture().map_err(|error| {
                    log::debug!(area = "render", %error, "no target texture");
                    RenderError::Unavailable
                })?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width,
                    height,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                // A blur at the output's boundary repeats the boundary pixel instead of
                // reading whatever lies outside the target.
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                gl.bind_texture(glow::TEXTURE_2D, None);
                Ok(texture)
            })
            .map_err(context_lost)??;
        self.framebuffer(texture, width, height)
    }

    /// A target that draws into `texture`, an imported dmabuf: the scan-out buffer of an output.
    ///
    /// The caller keeps `texture` alive for as long as the framebuffer is used.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the texture has no GL name or the framebuffer is
    /// incomplete.
    pub(crate) fn framebuffer_over(&mut self, texture: &GlesTexture) -> Result<Framebuffer, RenderError> {
        let handle = Texture::from_smithay(texture).ok_or(RenderError::Unavailable)?;
        let (width, height) = handle.size;
        self.framebuffer(handle.handle, width, height)
    }

    /// A depth-backed framebuffer over `texture`, cleared once.
    fn framebuffer(&mut self, texture: glow::Texture, width: i32, height: i32) -> Result<Framebuffer, RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: every name created here belongs to this context and is stored in the
                // returned `Framebuffer`, which the context outlives.
                let framebuffer = gl.create_framebuffer().map_err(|error| {
                    log::debug!(area = "render", %error, "no target framebuffer");
                    RenderError::Unavailable
                })?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                // The depth buffer is read only by a frame that draws in three dimensions; a flat
                // frame never enables the depth test.
                let depth = gl.create_renderbuffer().map_err(|error| {
                    log::debug!(area = "render", %error, "no target depth buffer");
                    RenderError::Unavailable
                })?;
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
                gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, width, height);
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(depth),
                );
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.depth_mask(true);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                if status != glow::FRAMEBUFFER_COMPLETE {
                    log::debug!(area = "render", width, height, "the target framebuffer is incomplete");
                    return Err(RenderError::Unavailable);
                }
                Ok(Framebuffer {
                    framebuffer,
                    texture: Texture {
                        handle: texture,
                        size: (width, height),
                    },
                    renderbuffer: depth,
                })
            })
            .map_err(context_lost)?
    }

    /// A target for a View's subtree: an `RGBA8` texture the renderer owns, cleared to opaque black, with a stencil
    /// buffer for the clipping ancestors and no depth buffer, because a View is drawn in painter's order.
    ///
    /// Unlike the output's target it is given back with [`Self::release`] when its View goes away, because Views come
    /// and go for as long as the session runs and the context outlives all of them.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels, [`RenderError::Unavailable`] when the driver refuses the
    /// texture, the stencil buffer or the framebuffer (a size past the driver's limit is such a refusal), and
    /// [`RenderError::Failed`] when the context is not current.
    pub(crate) fn subtree_target(&mut self, size: Size) -> Result<Framebuffer, RenderError> {
        let (width, height) = (size.width as i32, size.height as i32);
        if width <= 0 || height <= 0 {
            return Err(RenderError::Degenerate {
                width: size.width,
                height: size.height,
            });
        }
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: every name created here belongs to this context. A name made before a failure is deleted
                // before returning, so a refused target leaks nothing; a name that is kept is stored in the returned
                // `Framebuffer`, which `release` deletes.
                let texture = gl.create_texture().map_err(|error| {
                    log::debug!(area = "render", %error, "no subtree texture");
                    RenderError::Unavailable
                })?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width,
                    height,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                gl.bind_texture(glow::TEXTURE_2D, None);
                let framebuffer = gl.create_framebuffer().ok();
                let stencil = gl.create_renderbuffer().ok();
                let (Some(framebuffer), Some(stencil)) = (framebuffer, stencil) else {
                    gl.delete_texture(texture);
                    if let Some(framebuffer) = framebuffer {
                        gl.delete_framebuffer(framebuffer);
                    }
                    if let Some(stencil) = stencil {
                        gl.delete_renderbuffer(stencil);
                    }
                    log::debug!(area = "render", "no subtree framebuffer");
                    return Err(RenderError::Unavailable);
                };
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(stencil));
                gl.renderbuffer_storage(glow::RENDERBUFFER, glow::STENCIL_INDEX8, width, height);
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(stencil),
                );
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                if status != glow::FRAMEBUFFER_COMPLETE {
                    gl.delete_framebuffer(framebuffer);
                    gl.delete_renderbuffer(stencil);
                    gl.delete_texture(texture);
                    log::debug!(area = "render", width, height, "the subtree framebuffer is incomplete");
                    return Err(RenderError::Unavailable);
                }
                Ok(Framebuffer {
                    framebuffer,
                    texture: Texture {
                        handle: texture,
                        size: (width, height),
                    },
                    renderbuffer: stencil,
                })
            })
            .map_err(context_lost)?
    }

    /// Deletes a target made by [`Self::subtree_target`]: its framebuffer, texture and stencil buffer.
    ///
    /// When the context is not current the names are dropped without a GL call, which is what a lost context has done
    /// to them already: the driver freed every object the context made.
    pub(crate) fn release(&mut self, target: Framebuffer) {
        let _ = self.renderer.with_context(|gl| unsafe {
            // SAFETY: the three names belong to this context and are not used again, because `target` is consumed.
            gl.delete_framebuffer(target.framebuffer);
            gl.delete_renderbuffer(target.renderbuffer);
            gl.delete_texture(target.texture.handle);
        });
    }

    /// Binds `target` and clears it to opaque black and its stencil to zero, the picture a View starts from.
    pub(crate) fn clear_subtree(&mut self, target: &Framebuffer) -> Result<(), RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the framebuffer belongs to this context. The scissor is lifted first so the clear covers
                // the whole target.
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.disable(glow::SCISSOR_TEST);
                gl.color_mask(true, true, true, true);
                gl.stencil_mask(0xff);
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear_stencil(0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
            })
            .map_err(context_lost)
    }

    /// Draws into `rect` of the bound target from now on, top-left first: the viewport maps the clip volume onto
    /// the rectangle and the scissor keeps every pass inside it. [`Self::begin`] sets the whole target again.
    pub(crate) fn eye(&mut self, rect: Rect) {
        let (_, height) = self.viewport;
        let _ = self.renderer.with_context(|gl| unsafe {
            // SAFETY: both calls only set rasteriser state of the current context, in framebuffer coordinates, which
            // is why the y axis is flipped here exactly as `scissor` flips it.
            let y = (height - (rect.y + rect.height as i32)).max(0);
            gl.viewport(rect.x, y, rect.width as i32, rect.height as i32);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(rect.x, y, rect.width as i32, rect.height as i32);
        });
    }

    /// Binds a target and sets the state a frame starts from.
    ///
    /// Setting the state here rather than per pass is what makes "a frame does not inherit
    /// the last frame's state" true: whatever a pass left behind, the next frame begins
    /// from this.
    pub(crate) fn begin(&mut self, target: &Framebuffer) -> Result<(), RenderError> {
        let (width, height) = target.texture.size;
        self.viewport = (width, height);
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the framebuffer belongs to this context and outlives the frame. The
                // state set here is the state every pass expects.
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, width, height);
                gl.disable(glow::SCISSOR_TEST);
                gl.enable(glow::BLEND);
                // Every pass outputs premultiplied colour, which is the only blend that agrees
                // with the software path's premultiplied compositing.
                gl.blend_func_separate(
                    glow::ONE,
                    glow::ONE_MINUS_SRC_ALPHA,
                    glow::ONE,
                    glow::ONE_MINUS_SRC_ALPHA,
                );
            })
            .map_err(context_lost)
    }

    /// Binds `target` again after a call that read it, which leaves the default framebuffer bound, so the passes that
    /// follow draw into the target and not into nothing.
    pub(crate) fn bind(&mut self, target: &Framebuffer) -> Result<(), RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the framebuffer belongs to this context and outlives the frame; binding changes no other
                // state, so the scissor and the blend the frame set are kept.
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            })
            .map_err(context_lost)
    }

    /// Ends the frame: sends the passes to the driver so a readback sees them.
    pub(crate) fn end(&mut self) -> Result<(), RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: flushing sends the commands already issued and changes no state.
                gl.flush();
            })
            .map_err(context_lost)
    }

    /// Blends the next pass as a layer group: its colour and alpha are summed with the
    /// backdrop as `source * factor + destination * (1 - factor)`, so a composed group fades
    /// over what is behind it by the layer's opacity, once, however many passes made it.
    /// [`Self::blend_premultiplied`] restores the blend every pass expects.
    pub(crate) fn blend_group(&mut self, factor: f32) -> Result<(), RenderError> {
        let factor = factor.clamp(0.0, 1.0);
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the blend state belongs to this context; the caller restores it with
                // `blend_premultiplied` before the next frame.
                gl.blend_color(0.0, 0.0, 0.0, factor);
                gl.blend_func_separate(
                    glow::CONSTANT_ALPHA,
                    glow::ONE_MINUS_CONSTANT_ALPHA,
                    glow::CONSTANT_ALPHA,
                    glow::ONE_MINUS_CONSTANT_ALPHA,
                );
            })
            .map_err(context_lost)
    }

    /// Replaces the destination rather than blending over it: the next pass writes its colour and
    /// alpha as they are. [`Self::blend_premultiplied`] restores the blend every pass expects.
    pub(crate) fn blend_replace(&mut self) -> Result<(), RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the blend state belongs to this context; the caller restores it with
                // `blend_premultiplied` before the next pass.
                gl.blend_func_separate(glow::ONE, glow::ZERO, glow::ONE, glow::ZERO);
            })
            .map_err(context_lost)
    }

    /// Restores the premultiplied blend every pass expects after [`Self::blend_group`].
    pub(crate) fn blend_premultiplied(&mut self) -> Result<(), RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: restores the blend state `begin` sets, changing nothing else.
                gl.blend_func_separate(
                    glow::ONE,
                    glow::ONE_MINUS_SRC_ALPHA,
                    glow::ONE,
                    glow::ONE_MINUS_SRC_ALPHA,
                );
            })
            .map_err(context_lost)
    }

    /// Restricts every following pass to `rect`, or lifts the restriction.
    ///
    /// A frame's damage is enforced here rather than by intersecting each quad, so a pass
    /// that draws outside its own rectangle — a shadow's falloff, a blur's taps — still
    /// cannot touch a pixel the frame did not damage.
    pub(crate) fn scissor(&mut self, rect: Option<Rect>) {
        let (_, height) = self.viewport;
        let _ = self.renderer.with_context(|gl| unsafe {
            // SAFETY: the scissor rectangle is stated in framebuffer coordinates, which is why
            // the y axis is flipped here: the compositor counts from the top, GL from the
            // bottom. A disabled scissor is also the state the next `begin` sets.
            match rect {
                Some(rect) => {
                    gl.enable(glow::SCISSOR_TEST);
                    let y = height - (rect.y + rect.height as i32);
                    gl.scissor(rect.x, y.max(0), rect.width as i32, rect.height as i32);
                }
                None => gl.disable(glow::SCISSOR_TEST),
            }
        });
    }

    /// Clears a whole target to `colour`.
    pub(crate) fn clear(&mut self, target: &Framebuffer, colour: Color) {
        let _ = self.renderer.with_context(|gl| unsafe {
            // SAFETY: the target belongs to this context; the clear is unrestricted because
            // the scissor is disabled first.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(colour.r, colour.g, colour.b, colour.a);
            gl.clear(glow::COLOR_BUFFER_BIT);
        });
    }

    /// Clears the depth buffer of `target` to the far plane, within the current scissor.
    pub(crate) fn clear_depth(&mut self, target: &Framebuffer) {
        let _ = self.renderer.with_context(|gl| unsafe {
            // SAFETY: the target belongs to this context; the clear honours the scissor the caller
            // set, so a damage-limited frame clears only its damage rectangles.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
        });
    }

    /// Copies `rect` of `from` into the same rectangle of `to`.
    pub(crate) fn copy(&mut self, from: &Framebuffer, to: &Framebuffer, rect: Rect) -> Result<(), RenderError> {
        let (_, height) = from.texture.size;
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: both framebuffers belong to this context and are ordinary colour
                // attachments of the same size, which is the requirement `glBlitFramebuffer`
                // states. The y axis is flipped here exactly as `scissor` flips it.
                let top = height - (rect.y + rect.height as i32);
                let bottom = top + rect.height as i32;
                let left = rect.x;
                let right = left + rect.width as i32;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(from.framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(to.framebuffer));
                gl.blit_framebuffer(
                    left,
                    top,
                    right,
                    bottom,
                    left,
                    top,
                    right,
                    bottom,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            })
            .map_err(context_lost)
    }

    /// Copies `src` of `from` into `dst` of `to`, a texture the caller owns, and waits for the copy.
    ///
    /// `src` is in the pixels of `from` as an output is described, top-left first. `dst` is in the rows of
    /// `to` as GL counts them: row 0 is the first row of the texture's storage, which for a client's dmabuf is its
    /// first memory row. `reverse` flips the copy vertically, which is what turns the top-first rows of a frame into
    /// the top-first memory rows of a buffer, because a framebuffer's own row 0 is the bottom of the picture it holds.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when `to` cannot be a framebuffer's colour attachment, and
    /// [`RenderError::Failed`] when the context is lost.
    pub(crate) fn blit_into(
        &mut self,
        from: &Framebuffer,
        to: Texture,
        src: Rect,
        dst: Rect,
        reverse: bool,
    ) -> Result<(), RenderError> {
        let (_, source_height) = from.texture.size;
        let Self { renderer, scratch, .. } = self;
        renderer
            .with_context(|gl| unsafe {
                // SAFETY: `from` and `to` belong to this context, `to` is kept alive by the caller for the whole
                // call, and the scratch framebuffer is made here and kept. The scissor test is disabled because
                // it limits a blit as it limits a draw; the framebuffer bindings are restored before returning.
                let framebuffer = scratch_over(gl, scratch, to)?;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(from.framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(framebuffer));
                gl.disable(glow::SCISSOR_TEST);
                let source_top = source_height - src.y;
                let source_bottom = source_top - src.height as i32;
                let (first, last) = (dst.y, dst.y + dst.height as i32);
                let (destination_start, destination_end) = if reverse { (last, first) } else { (first, last) };
                gl.blit_framebuffer(
                    src.x,
                    source_bottom,
                    src.x + src.width as i32,
                    source_top,
                    dst.x,
                    destination_start,
                    dst.x + dst.width as i32,
                    destination_end,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.framebuffer_texture_2d(
                    glow::DRAW_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    None,
                    0,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                // The consumer of a client's buffer may read it as soon as it hears `ready`.
                gl.finish();
                Ok(())
            })
            .map_err(context_lost)?
    }

    /// Writes `bytes`, `rect.height` rows of `rect.width` RGBA pixels, into `rect` of `to`, and waits for the write.
    ///
    /// `rect` is in the rows of `to` as GL counts them, and the first row of `bytes` is written to `rect.y`.
    pub(crate) fn upload_into(&mut self, to: Texture, rect: Rect, bytes: &[u8]) -> Result<(), RenderError> {
        let wanted = rect.width as usize * rect.height as usize * 4;
        if bytes.len() < wanted {
            return Err(RenderError::Failed);
        }
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: `to` belongs to this context and is kept alive by the caller; `bytes` holds at least the
                // `width * height * 4` bytes the upload reads, which is checked above, and the unpack alignment
                // (four bytes by default) is what an RGBA8 row satisfies.
                gl.bind_texture(glow::TEXTURE_2D, Some(to.handle));
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    rect.x,
                    rect.y,
                    rect.width as i32,
                    rect.height as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&bytes[..wanted])),
                );
                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.finish();
            })
            .map_err(context_lost)
    }

    /// Reads a whole texture back as RGBA8, first storage row first.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when `from` cannot be a framebuffer's colour attachment, and
    /// [`RenderError::Failed`] when the context is lost.
    pub(crate) fn read_texture(&mut self, from: Texture) -> Result<Vec<u8>, RenderError> {
        let (width, height) = from.size;
        let mut bytes = vec![0u8; width.max(0) as usize * 4 * height.max(0) as usize];
        let Self { renderer, scratch, .. } = self;
        renderer
            .with_context(|gl| unsafe {
                // SAFETY: `from` belongs to this context and is kept alive by the caller; the buffer is exactly
                // the size the read needs, and the pack alignment is what an RGBA8 row satisfies. The texture is
                // detached again, so the scratch framebuffer holds no reference to a client's buffer.
                let framebuffer = scratch_over(gl, scratch, from)?;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
                gl.read_pixels(
                    0,
                    0,
                    width,
                    height,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(bytes.as_mut_slice())),
                );
                gl.framebuffer_texture_2d(
                    glow::READ_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    None,
                    0,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                Ok(())
            })
            .map_err(context_lost)??;
        Ok(bytes)
    }

    /// Draws one quad with `program`.
    ///
    /// The caller has already packed the program's uniforms, so this is the whole of a
    /// pass: upload the geometry and the uniforms, draw six vertices, and leave nothing
    /// enabled that the next pass does not expect.
    pub(crate) fn draw(
        &mut self,
        program: &Program,
        bytes: &[u8],
        quad: Rect,
        texture: Option<Texture>,
        placement: QuadPlacement,
    ) -> Result<(), RenderError> {
        let (width, height) = self.viewport;
        let texture = texture.unwrap_or(self.white);
        // Two triangles, each vertex carrying the unit position, the destination, the viewport,
        // the clip columns and the parameters. Per-vertex rather than a uniform so the fragment
        // stage is the only stage with a uniform block. A flat quad sends zero columns and a
        // clear `projected` flag, so the vertex stage keeps the flat formula.
        let (projected, columns) = match placement.clip {
            Some(columns) => (1.0f32, columns),
            None => (0.0, [[0.0f32; 4]; 3]),
        };
        let mut geometry: Vec<u8> = Vec::with_capacity(6 * VERTEX_FLOATS * 4);
        for (x, y) in [
            (0.0f32, 0.0f32),
            (1.0, 0.0),
            (0.0, 1.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (1.0, 1.0),
        ] {
            let head = [
                x,
                y,
                quad.x as f32,
                quad.y as f32,
                quad.width as f32,
                quad.height as f32,
                width as f32,
                height as f32,
            ];
            let tail = [projected, placement.depth_value, 0.0, 0.0];
            for value in head.into_iter().chain(columns.iter().flatten().copied()).chain(tail) {
                geometry.extend_from_slice(&value.to_ne_bytes());
            }
        }
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: every name used here belongs to this context. The attribute pointers
                // are set with the stride and offsets of the geometry just built, and every
                // attribute and buffer this changes is reset before returning, so the next
                // pass — whatever program it uses — starts from the same state.
                gl.use_program(Some(program.program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(program.geometry));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &geometry, glow::DYNAMIC_DRAW);
                let stride = (VERTEX_FLOATS * 4) as i32;
                // Attribute offsets in floats: position 0, quad 2, viewport 6, the clip columns
                // 8, 12 and 16, and the parameters 20.
                for (index, size, offset) in [
                    (0u32, 2i32, 0i32),
                    (1, 4, 2),
                    (2, 2, 6),
                    (3, 4, 8),
                    (4, 4, 12),
                    (5, 4, 16),
                    (6, 4, 20),
                ] {
                    gl.enable_vertex_attrib_array(index);
                    gl.vertex_attrib_pointer_f32(index, size, glow::FLOAT, false, stride, offset * 4);
                }

                gl.bind_buffer(glow::UNIFORM_BUFFER, Some(program.buffer));
                gl.buffer_data_u8_slice(glow::UNIFORM_BUFFER, bytes, glow::DYNAMIC_DRAW);
                gl.bind_buffer_base(glow::UNIFORM_BUFFER, UNIFORM_BINDING, Some(program.buffer));

                gl.active_texture(glow::TEXTURE0 + TEXTURE_UNIT);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture.handle));
                // The filter is set per draw rather than once per texture: a texture this
                // renderer did not create — a client's imported buffer — keeps GL's default
                // `NEAREST_MIPMAP_LINEAR`, and a texture without mipmaps sampled with a
                // mipmap filter is *incomplete*, which samples as opaque black. Setting it
                // here is the same thing Smithay's own texture draw does, and it is why a
                // client's pixels show.
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                match placement.stencil {
                    Stencil::Off => gl.disable(glow::STENCIL_TEST),
                    Stencil::Test(depth) => {
                        gl.enable(glow::STENCIL_TEST);
                        gl.stencil_func(glow::EQUAL, i32::from(depth), 0xff);
                        gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    }
                    Stencil::Push(depth) => {
                        gl.enable(glow::STENCIL_TEST);
                        gl.stencil_func(glow::EQUAL, i32::from(depth) - 1, 0xff);
                        gl.stencil_op(glow::KEEP, glow::KEEP, glow::INCR);
                        gl.color_mask(false, false, false, false);
                    }
                    Stencil::Pop(depth) => {
                        gl.enable(glow::STENCIL_TEST);
                        gl.stencil_func(glow::EQUAL, i32::from(depth), 0xff);
                        gl.stencil_op(glow::KEEP, glow::KEEP, glow::DECR);
                        gl.color_mask(false, false, false, false);
                    }
                }
                match placement.depth {
                    Depth::Off => gl.disable(glow::DEPTH_TEST),
                    Depth::Opaque => {
                        gl.enable(glow::DEPTH_TEST);
                        gl.depth_func(glow::LESS);
                        gl.depth_mask(true);
                    }
                    Depth::Transparent => {
                        gl.enable(glow::DEPTH_TEST);
                        gl.depth_func(glow::LEQUAL);
                        gl.depth_mask(false);
                    }
                }
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                gl.color_mask(true, true, true, true);

                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                gl.bind_buffer(glow::UNIFORM_BUFFER, None);
                gl.disable_vertex_attrib_array(0);
                gl.disable_vertex_attrib_array(1);
                gl.disable_vertex_attrib_array(2);
                for index in 3..=6 {
                    gl.disable_vertex_attrib_array(index);
                }
            })
            .map_err(context_lost)
    }

    /// Reads a whole target back, bottom row first, as RGBA8.
    pub(crate) fn read(&mut self, target: &Framebuffer) -> Result<Vec<u8>, RenderError> {
        let (width, height) = target.texture.size;
        let mut bytes = vec![0u8; width as usize * 4 * height as usize];
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the framebuffer belongs to this context; the buffer is exactly the size
                // the read needs, and the pack alignment (four bytes by default) is what an RGBA8
                // row always satisfies.
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.read_pixels(
                    0,
                    0,
                    width,
                    height,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(bytes.as_mut_slice())),
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            })
            .map_err(context_lost)?;
        Ok(bytes)
    }

    /// A texture with known pixels, so a test can exercise a sample without a Wayland
    /// client.
    #[cfg(test)]
    pub(crate) fn texture(&mut self, size: (i32, i32), pixels: &[u8]) -> Result<Texture, RenderError> {
        let (width, height) = size;
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: the name belongs to this context and is stored in the returned texture;
                // the slice is `width * height * 4` bytes, which the caller guarantees.
                let name = gl.create_texture().map_err(|error| {
                    log::debug!(area = "render", %error, "no test texture");
                    RenderError::Unavailable
                })?;
                gl.bind_texture(glow::TEXTURE_2D, Some(name));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width,
                    height,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(pixels)),
                );
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
                gl.bind_texture(glow::TEXTURE_2D, None);
                Ok(Texture { handle: name, size })
            })
            .map_err(context_lost)?
    }
}

/// Attaches `texture` to the scratch framebuffer, making the framebuffer first when there is none yet, and answers
/// it bound as both the read and the draw target.
///
/// # Safety
///
/// `gl` is the current context and `texture` is one of its textures.
unsafe fn scratch_over(
    gl: &glow::Context,
    scratch: &mut Option<glow::Framebuffer>,
    texture: Texture,
) -> Result<glow::Framebuffer, RenderError> {
    let framebuffer = match *scratch {
        Some(framebuffer) => framebuffer,
        None => {
            // SAFETY: the name belongs to the current context, which frees it with the context.
            let created = unsafe { gl.create_framebuffer() }.map_err(|error| {
                log::debug!(area = "render", %error, "no scratch framebuffer");
                RenderError::Unavailable
            })?;
            *scratch = Some(created);
            created
        }
    };
    // SAFETY: both names belong to the current context, as the caller promises.
    let status = unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture.handle),
            0,
        );
        gl.check_framebuffer_status(glow::FRAMEBUFFER)
    };
    if status != glow::FRAMEBUFFER_COMPLETE {
        log::debug!(area = "render", "a client's texture cannot be a framebuffer attachment");
        // SAFETY: as above.
        unsafe {
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, None, 0);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        return Err(RenderError::Unavailable);
    }
    Ok(framebuffer)
}

/// One GLSL stage, compiled.
fn compile(gl: &glow::Context, vertex: bool, source: &str) -> Result<glow::Shader, CompileFailure> {
    let stage = if vertex { Stage::Vertex } else { Stage::Fragment };
    unsafe {
        // SAFETY: `source` is a complete GLSL stage; the name created here is deleted by the
        // caller once it is linked or its sibling fails, so no shader object outlives the link.
        let shader = gl.create_shader(if vertex {
            glow::VERTEX_SHADER
        } else {
            glow::FRAGMENT_SHADER
        });
        let shader = match shader {
            Ok(shader) => shader,
            Err(log) => return Err(CompileFailure { stage, log }),
        };
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if gl.get_shader_compile_status(shader) {
            return Ok(shader);
        }
        let log = gl.get_shader_info_log(shader);
        gl.delete_shader(shader);
        Err(CompileFailure { stage, log })
    }
}

/// The one render failure for a lost context, logged where it happened.
fn context_lost(error: GlesError) -> RenderError {
    log::debug!(area = "render", ?error, "the GLES context is not current");
    RenderError::Failed
}

/// A lost context while compiling, which is a link failure with nothing to point at.
fn context_compile_lost(error: GlesError) -> CompileFailure {
    log::debug!(area = "render", ?error, "the GLES context is not current");
    CompileFailure {
        stage: Stage::Link,
        log: "the GLES context is not current".to_owned(),
    }
}

/// A program: the linked GLSL and its uniform buffer.
pub(crate) struct Program {
    program: glow::Program,
    /// The uniform block's buffer.
    buffer: glow::Buffer,
    /// The geometry buffer this program's draws upload their vertices to.
    geometry: glow::Buffer,
}

impl std::fmt::Debug for Program {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Program")
            .field("bytes", &BLOCK_BYTES)
            .finish_non_exhaustive()
    }
}

impl Program {
    /// Packs the uniforms of the next draw. Every pass shares one fixed block, so it cannot fail.
    pub(crate) fn pack(&self, uniforms: &Uniforms, out: &mut Vec<u8>) {
        uniforms.pack(out);
    }
}

/// A texture a pass can sample.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Texture {
    handle: glow::Texture,
    size: (i32, i32),
}

impl Texture {
    /// The texture a client buffer was imported into, if Smithay made one.
    pub(crate) fn from_smithay(texture: &GlesTexture) -> Option<Self> {
        let handle = NonZeroU32::new(texture.tex_id())?;
        Some(Self {
            handle: glow::NativeTexture(handle),
            size: texture.size().into(),
        })
    }

    /// The texture's size in pixels.
    pub(crate) fn size(self) -> (i32, i32) {
        self.size
    }
}

/// An offscreen target: a colour texture, a depth or stencil buffer and the framebuffer that draws into them.
pub(crate) struct Framebuffer {
    framebuffer: glow::Framebuffer,
    texture: Texture,
    /// The depth buffer of an output's target, or the stencil buffer of a View's.
    renderbuffer: glow::Renderbuffer,
}

impl Framebuffer {
    /// The target's texture, for a pass that samples what it holds.
    pub(crate) fn texture(&self) -> Texture {
        self.texture
    }

    /// The target's size in pixels.
    pub(crate) fn size(&self) -> Size {
        Size::new(self.texture.size.0 as u32, self.texture.size.1 as u32)
    }
}
