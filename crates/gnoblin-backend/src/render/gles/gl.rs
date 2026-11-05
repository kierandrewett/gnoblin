//! The crate's one OpenGL ES shim.
//!
//! D12 puts the GPU path on Smithay's GLES renderer, whose safe half is the EGL
//! context (see [`super::egl`]) and whose drawing half is `glow`, the safe-API wrapper
//! Smithay's `renderer_glow` feature hands out. `glow`'s calls are still `unsafe` —
//! each one takes the caller's word that the GL state it is about to change is not in
//! use elsewhere — so every one of them lives in this module, and the rest of the
//! renderer is ordinary Rust that names [`Device`] and never a `glow` type.
//!
//! That split is what the workspace's unsafe policy asks for: one FFI shim module,
//! opted out with `#[allow(unsafe_code)]`, a `// SAFETY:` comment per block, and no
//! `unsafe` anywhere else in the crate. Every block here is of one kind: the renderer
//! owns the context, all of its GL calls are made from one thread through this module,
//! and each pass that changes state sets what it needs and resets what it changed.
//!
//! # Objects are freed by the context, not one by one
//!
//! A [`Target`] and a [`Program`] own GL names but do not delete them: the context that
//! created them is destroyed with the renderer, and destroying a context frees every
//! object it made. Deleting them individually would need a current context at drop
//! time, which is exactly the state this module cannot promise on a failure path.
#![allow(unsafe_code)]

use std::num::NonZeroU32;

use glow::HasContext;
use smithay::backend::renderer::Texture as _;
use smithay::backend::renderer::gles::{GlesError, GlesTexture};
use smithay::backend::renderer::glow::GlowRenderer;
use tracing::debug;

use gnoblin_render::{Color, ResolvedUniform};
use gnoblin_types::{Rect, Size};

use crate::render::RenderError;
use crate::render::gles::ShaderError;
use crate::render::gles::program::{CompileFailure, Stage, UniformLayout, Uniforms};

/// The binding the uniform buffer is uploaded to.
const UNIFORM_BINDING: u32 = 0;

/// The texture unit every pass samples from.
const TEXTURE_UNIT: u32 = 0;

/// The floats one quad vertex carries: the unit position, the destination rectangle and
/// the target's size.
const VERTEX_FLOATS: usize = 8;

/// The GL context and the calls every pass is made of.
pub(crate) struct Device {
    /// The Smithay renderer that owns the EGL context and imports client buffers.
    renderer: GlowRenderer,
    /// The one-pixel white texture a pass binds when it has no layer texture.
    ///
    /// A solid layer carries no pixels, but a user shader still samples `gnoblin_tex`;
    /// white is the identity for a multiply, so such a shader's own colour is the only
    /// thing that shows.
    white: Texture,
    /// The size of the target last bound, which the vertex stage needs as an attribute.
    viewport: (i32, i32),
}

impl Device {
    /// The device on a fresh surfaceless GLES context.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when EGL offers no context on this machine.
    pub(crate) fn new() -> Result<Self, RenderError> {
        let mut renderer = super::egl::context()?;
        let white = renderer
            .with_context(|gl| unsafe {
                // SAFETY: the context is current (that is what `with_context` establishes)
                // and the one name created here is stored in `Device`, which the context
                // outlives.
                let name = gl.create_texture().map_err(|error| {
                    debug!(%error, "[render] no white texture for the GLES renderer");
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

    /// Links a program from GLSL sources and records its uniform layout.
    ///
    /// # Errors
    ///
    /// [`CompileFailure`] with the driver's log, which the caller turns into a line of the
    /// shader that was written.
    pub(crate) fn program(
        &mut self,
        vertex: &str,
        fragment: &str,
        layout: UniformLayout,
    ) -> Result<Program, CompileFailure> {
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
            debug!(
                blocks,
                "[render] a pass declares an unexpected number of uniform blocks"
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
            layout,
        })
    }

    /// A target of `size` physical pixels, holding no defined pixel.
    pub(crate) fn target(&mut self, size: Size) -> Result<Target, RenderError> {
        let (width, height) = (size.width as i32, size.height as i32);
        if width <= 0 || height <= 0 {
            return Err(RenderError::Degenerate {
                width: size.width,
                height: size.height,
            });
        }
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: every name created here belongs to this context and is stored in the
                // returned `Target`, which the context outlives. The texture is a colour
                // attachment; it is cleared once so a fresh target holds no defined pixel, the
                // state a fresh pixman buffer is in.
                let texture = gl.create_texture().map_err(|error| {
                    debug!(%error, "[render] no target texture");
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

                let framebuffer = gl.create_framebuffer().map_err(|error| {
                    debug!(%error, "[render] no target framebuffer");
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
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                if status != glow::FRAMEBUFFER_COMPLETE {
                    debug!(width, height, "[render] the target framebuffer is incomplete");
                    return Err(RenderError::Unavailable);
                }
                Ok(Target {
                    framebuffer,
                    texture: Texture {
                        handle: texture,
                        size: (width, height),
                    },
                })
            })
            .map_err(context_lost)?
    }

    /// Binds a target and sets the state a frame starts from.
    ///
    /// Setting the state here rather than per pass is what makes "a frame does not inherit
    /// the last frame's state" true: whatever a pass left behind, the next frame begins
    /// from this.
    pub(crate) fn begin(&mut self, target: &Target) -> Result<(), RenderError> {
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

    /// Ends the frame: sends the passes to the driver so a readback sees them.
    pub(crate) fn end(&mut self) -> Result<(), RenderError> {
        self.renderer
            .with_context(|gl| unsafe {
                // SAFETY: flushing sends the commands already issued and changes no state.
                gl.flush();
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
    pub(crate) fn clear(&mut self, target: &Target, colour: Color) {
        let _ = self.renderer.with_context(|gl| unsafe {
            // SAFETY: the target belongs to this context; the clear is unrestricted because
            // the scissor is disabled first.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(colour.r, colour.g, colour.b, colour.a);
            gl.clear(glow::COLOR_BUFFER_BIT);
        });
    }

    /// Copies `rect` of `from` into the same rectangle of `to`.
    pub(crate) fn copy(&mut self, from: &Target, to: &Target, rect: Rect) -> Result<(), RenderError> {
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
    ) -> Result<(), RenderError> {
        let (width, height) = self.viewport;
        let texture = texture.unwrap_or(self.white);
        // Two triangles, each vertex carrying the unit position, the destination and the
        // viewport. Per-vertex rather than a uniform so the fragment stage is the only stage
        // with a uniform block.
        let mut geometry: Vec<u8> = Vec::with_capacity(6 * VERTEX_FLOATS * 4);
        for (x, y) in [
            (0.0f32, 0.0f32),
            (1.0, 0.0),
            (0.0, 1.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (1.0, 1.0),
        ] {
            for value in [
                x,
                y,
                quad.x as f32,
                quad.y as f32,
                quad.width as f32,
                quad.height as f32,
                width as f32,
                height as f32,
            ] {
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
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
                gl.enable_vertex_attrib_array(1);
                gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 2 * 4);
                gl.enable_vertex_attrib_array(2);
                gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, 6 * 4);

                gl.bind_buffer(glow::UNIFORM_BUFFER, Some(program.buffer));
                gl.buffer_data_u8_slice(glow::UNIFORM_BUFFER, bytes, glow::DYNAMIC_DRAW);
                gl.bind_buffer_base(glow::UNIFORM_BUFFER, UNIFORM_BINDING, Some(program.buffer));

                gl.active_texture(glow::TEXTURE0 + TEXTURE_UNIT);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture.handle));
                gl.draw_arrays(glow::TRIANGLES, 0, 6);

                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                gl.bind_buffer(glow::UNIFORM_BUFFER, None);
                gl.disable_vertex_attrib_array(0);
                gl.disable_vertex_attrib_array(1);
                gl.disable_vertex_attrib_array(2);
            })
            .map_err(context_lost)
    }

    /// Reads a whole target back, bottom row first, as RGBA8.
    pub(crate) fn read(&mut self, target: &Target) -> Result<Vec<u8>, RenderError> {
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
                    debug!(%error, "[render] no test texture");
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
    debug!(?error, "[render] the GLES context is not current");
    RenderError::Failed
}

/// A lost context while compiling, which is a link failure with nothing to point at.
fn context_compile_lost(error: GlesError) -> CompileFailure {
    debug!(?error, "[render] the GLES context is not current");
    CompileFailure {
        stage: Stage::Link,
        log: "the GLES context is not current".to_owned(),
    }
}

/// A program: the linked GLSL, its uniform buffer and its layout.
pub(crate) struct Program {
    program: glow::Program,
    /// The uniform block's buffer.
    buffer: glow::Buffer,
    /// The geometry buffer this program's draws upload their vertices to.
    geometry: glow::Buffer,
    /// Where each uniform sits in the block.
    layout: UniformLayout,
}

impl std::fmt::Debug for Program {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Program")
            .field("bytes", &self.layout.bytes())
            .finish_non_exhaustive()
    }
}

impl Program {
    /// Packs the uniforms of a pass that declares no user field.
    ///
    /// It cannot fail: a program with no user fields packs only the fixed members, and a
    /// failed pack of those would be a bug in the layout rather than in a caller's value.
    pub(crate) fn pack_fixed(&self, uniforms: &Uniforms, out: &mut Vec<u8>) {
        if let Err(error) = self.layout.pack(uniforms, &[], out) {
            debug!(%error, "[render] an effect pass failed to pack its uniforms");
        }
    }

    /// Packs the uniforms of the next draw, checking each value against the declaration.
    ///
    /// # Errors
    ///
    /// [`ShaderError`] when a value's type is not the type the compiled shader declares for
    /// that name: uploading it would put four bytes where a vector belongs, which reads back
    /// as a value nobody wrote.
    pub(crate) fn pack(
        &self,
        uniforms: &Uniforms,
        values: &[ResolvedUniform],
        out: &mut Vec<u8>,
    ) -> Result<(), ShaderError> {
        self.layout.pack(uniforms, values, out)
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

/// An offscreen target: a texture and the framebuffer that draws into it.
pub(crate) struct Target {
    framebuffer: glow::Framebuffer,
    texture: Texture,
}

impl Target {
    /// The target's texture, for a pass that samples what it holds.
    pub(crate) fn texture(&self) -> Texture {
        self.texture
    }

    /// The target's size in pixels.
    pub(crate) fn size(&self) -> Size {
        Size::new(self.texture.size.0 as u32, self.texture.size.1 as u32)
    }
}
