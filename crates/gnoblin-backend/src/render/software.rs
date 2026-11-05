//! The software renderer: Smithay's Pixman renderer drawing into an offscreen
//! buffer.
//!
//! D12 ships on GLES with Pixman for software rendering, and this slice is the
//! Pixman half: it needs no GPU, no EGL and no session, so it runs in CI and it is
//! what the integration tests and the parity harness read pixels from. GLES slots in
//! behind [`Target`](crate::Target) without changing an [`OutputFrame`].
//!
//! # Why Pixman and not our own rasteriser
//!
//! Client pixels live in shared memory the client can write at any time, and
//! reading it safely means going through Smithay, which is where the protocol's
//! mapping rules and the `SIGPIPE` guard for a shrunk pool live. [`PixmanRenderer`]
//! is Smithay's answer for exactly that: `import_shm_buffer` takes a `wl_buffer`
//! and gives a texture, `render_texture_from_to` blends it with the alpha the
//! compositor asked for, and `copy_framebuffer` reads the result back. This module
//! therefore only decides *what* is drawn where, which is the compositor's job.
//!
//! # What the 2D subset draws
//!
//! - **Placement** is a translation, and a scale when the destination is bigger or
//!   smaller than the crop: a [`OutputFrame`] gives a rectangle and this renderer fits
//!   the source into it. Rotation is refused before a layer is described, because
//!   pixman's destination transform would rotate the whole target.
//! - **Opacity** is the alpha a layer is blended with, so it is exact.
//! - **Tint** is a per-channel multiply of the layer's own colour. Pixman's mask
//!   path has no multiply operator, so a tinted layer is copied out of the client
//!   buffer, multiplied and imported back. White, the identity and the common case,
//!   costs nothing: the client's texture is drawn directly.
//! - **Damage** is the only region repainted. Every draw is clipped to the
//!   intersection of the frame's damage and the layer, so an unchanged pixel is
//!   never touched and a move cannot leave stale pixels behind.
//!
//! # Failure modes
//!
//! 1. **Idle iterations cost a full repaint.** [`Software::draw`] draws only the
//!    damage and returns at once for an empty frame: `an_empty_frame_draws_nothing`.
//! 2. **A stale rectangle survives a move.** Only damaged pixels are painted, so
//!    the old and new rectangles the scene reported are what changes:
//!    `damage_limits_what_is_repainted`.
//! 3. **A client buffer is read after the client destroyed it.** The buffer is
//!    imported per frame and an import failure skips that layer instead of ending
//!    the frame: `frame::tests::a_degenerate_layer_draws_nothing` covers the shape,
//!    and `tests/render.rs::a_client_that_destroys_its_buffer_does_not_stop_the_frame`
//!    covers the destroyed buffer.
//! 4. **A readback is upside down or the wrong colour order.** The image comes out
//!    top-left first in RGBA8: `a_drawn_rectangle_is_where_a_probe_finds_it`.
//! 5. **A resize keeps the old buffer.** [`Software::resize`] replaces the buffer
//!    and the composer asks for full damage, so the next frame repaints everything:
//!    `a_resize_replaces_the_buffer`.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::pixman::{PixmanRenderer, PixmanTexture};
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, Frame as _, ImportMem, ImportMemWl, Offscreen, Renderer, Texture,
};
use smithay::reexports::pixman::Image as PixmanImage;
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Size as PhysicalSize, Transform};
use tracing::warn;

use gnoblin_render::Color;
use gnoblin_types::{Rect, Scale, Size};

use crate::render::RenderError;
use crate::render::frame::{Layer, OutputFrame, Surface};
use crate::render::image::Image;

/// The colour format of the offscreen buffer.
///
/// ARGB8888 is what Pixman composites fastest and what an `wl_shm` client hands
/// over, so a client buffer needs no conversion.
const TARGET_FORMAT: Fourcc = Fourcc::Argb8888;

/// The format whose *bytes* are red, green, blue, alpha — what [`Image`] promises.
///
/// A DRM fourcc names the 32-bit word, so the byte order in memory is the reverse
/// of its spelling on a little-endian machine: `ABGR8888` stores red first here and
/// `RGBA8888` does on a big-endian one. Choosing per endianness means the readback
/// needs no channel swap, which is the difference between a copy and a copy plus a
/// pass over every pixel.
#[cfg(target_endian = "little")]
const READBACK_FORMAT: Fourcc = Fourcc::Abgr8888;
#[cfg(target_endian = "big")]
const READBACK_FORMAT: Fourcc = Fourcc::Rgba8888;

/// A Pixman renderer and the offscreen buffer it draws into.
pub struct Software {
    renderer: PixmanRenderer,
    /// The offscreen buffer, replaced by [`Software::resize`].
    buffer: PixmanImage<'static, 'static>,
    /// The buffer's size in physical pixels.
    physical: Size,
    /// The client textures of the frame being drawn, one entry per layer in the
    /// same order, reused across frames so a steady frame allocates nothing.
    textures: Vec<Option<PixmanTexture>>,
    /// The bytes of a tinted layer, reused for the same reason.
    scratch: Vec<u8>,
    /// The frame's damage and the current layer's share of it, in physical pixels,
    /// both reused for the same reason.
    damage: Vec<Rectangle<i32, Physical>>,
    clipped: Vec<Rectangle<i32, Physical>>,
}

impl Software {
    /// A renderer with a buffer of `physical` pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] for a size with no pixels on an axis, and
    /// [`RenderError::Unavailable`] when pixman cannot create the buffer at all.
    pub fn new(physical: Size) -> Result<Self, RenderError> {
        if physical.is_empty() {
            return Err(RenderError::Degenerate {
                width: physical.width,
                height: physical.height,
            });
        }
        let mut renderer = PixmanRenderer::new().map_err(|error| {
            warn!(?error, "[render] the pixman renderer could not be created");
            RenderError::Unavailable
        })?;
        let buffer = create_buffer(&mut renderer, physical)?;
        Ok(Self {
            renderer,
            buffer,
            physical,
            textures: Vec::new(),
            scratch: Vec::new(),
            damage: Vec::new(),
            clipped: Vec::new(),
        })
    }

    /// The buffer's size in physical pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.physical
    }

    /// Replaces the buffer with one of the given size.
    ///
    /// A new buffer holds no pixels, so the caller must damage the whole output
    /// afterwards; the composer does that from the scene's own resize path.
    ///
    /// # Errors
    ///
    /// The same two errors as [`Software::new`]. The old buffer is kept on failure,
    /// so a failed resize does not lose the frame that is on screen.
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
        self.buffer = create_buffer(&mut self.renderer, physical)?;
        self.physical = physical;
        Ok(())
    }

    /// Draws `frame`, repainting only what it damaged.
    ///
    /// `scale` is the output's compositing scale: a frame's rectangles are logical
    /// and the buffer is physical.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman refuses an operation. A layer whose
    /// texture cannot be imported is not an error: a client may destroy its buffer
    /// at any time, and the rest of the frame still has to appear.
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
                damage.push(clipped);
            }
        }
        if damage.is_empty() {
            // Everything the frame wanted to redraw is outside the output.
            self.damage = damage;
            return Ok(());
        }

        // Textures first: importing needs the renderer mutably, and the frame
        // borrows it for the whole of the draw.
        self.prepare(frame);
        let mut clipped = std::mem::take(&mut self.clipped);
        let result = self.paint(frame, scale, &damage, &mut clipped);
        self.clipped = clipped;
        self.damage = damage;
        result
    }

    /// Reads the buffer back as an [`Image`].
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman cannot copy the buffer out.
    pub fn screenshot(&mut self) -> Result<Image, RenderError> {
        let Self {
            renderer,
            buffer,
            physical,
            ..
        } = self;
        let size = PhysicalSize::from((physical.width as i32, physical.height as i32));
        let target = renderer.bind(buffer).map_err(failed)?;
        let mapping = renderer
            .copy_framebuffer(&target, Rectangle::from_size(size), READBACK_FORMAT)
            .map_err(failed)?;
        let height = mapping.height() as usize;
        if height == 0 {
            return Err(RenderError::Failed);
        }
        let bytes = renderer.map_texture(&mapping).map_err(failed)?;
        // The mapping's stride is pixman's own and is not the packed width, so it
        // is measured from the slice rather than assumed.
        let stride = bytes.len() / height;
        Image::new(Size::new(mapping.width(), mapping.height()), stride, bytes.to_vec()).ok_or(RenderError::Failed)
    }

    /// Paints the layers into the bound buffer.
    fn paint(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        damage: &[Rectangle<i32, Physical>],
        clipped: &mut Vec<Rectangle<i32, Physical>>,
    ) -> Result<(), RenderError> {
        let Self {
            renderer,
            buffer,
            physical,
            textures,
            ..
        } = self;
        let output = PhysicalSize::from((physical.width as i32, physical.height as i32));
        let mut target = renderer.bind(buffer).map_err(failed)?;
        let mut pixman_frame = renderer
            .render(&mut target, output, Transform::Normal)
            .map_err(failed)?;
        pixman_frame.clear(colour(frame.background()), damage).map_err(failed)?;
        for (index, layer) in frame.layers().iter().enumerate() {
            let dst = match layer {
                Layer::Surface(surface) => physical_rect(surface.dst, scale),
                Layer::Solid(solid) => physical_rect(solid.dst, scale),
            };
            layer_damage(damage, dst, clipped);
            if clipped.is_empty() {
                continue;
            }
            match layer {
                Layer::Solid(solid) => pixman_frame
                    .draw_solid(dst, clipped, colour(solid.colour))
                    .map_err(failed)?,
                Layer::Surface(surface) => {
                    let Some(Some(texture)) = textures.get(index) else {
                        continue;
                    };
                    let alpha = surface.opacity.clamp(0.0, 1.0) * surface.tint.a.clamp(0.0, 1.0);
                    pixman_frame
                        .render_texture_from_to(texture, source(surface), dst, clipped, &[], surface.transform, alpha)
                        .map_err(failed)?;
                }
            }
        }
        if let Some(cursor) = frame.cursor() {
            let dst = physical_rect(cursor.dst, scale);
            layer_damage(damage, dst, clipped);
            if !clipped.is_empty() {
                pixman_frame
                    .draw_solid(dst, clipped, colour(cursor.colour))
                    .map_err(failed)?;
            }
        }
        // Pixman signals nothing: its sync point is already signalled, and the
        // offscreen buffer is read on the same thread. The value is dropped on
        // purpose rather than waited on.
        let _sync = pixman_frame.finish().map_err(failed)?;
        Ok(())
    }

    /// Imports every surface layer's texture, in layer order.
    fn prepare(&mut self, frame: &OutputFrame) {
        let Self {
            renderer,
            textures,
            scratch,
            ..
        } = self;
        textures.clear();
        for layer in frame.layers() {
            match layer {
                Layer::Solid(_) => textures.push(None),
                Layer::Surface(surface) => textures.push(import(renderer, surface, scratch)),
            }
        }
    }
}

/// The part of the frame's damage that a layer at `dst` covers, in `dst`'s own
/// coordinates.
///
/// Pixman's draw calls offset their damage argument by the destination origin, so a
/// caller passes the damage relative to the layer, not to the output.
fn layer_damage(
    damage: &[Rectangle<i32, Physical>],
    dst: Rectangle<i32, Physical>,
    clipped: &mut Vec<Rectangle<i32, Physical>>,
) {
    clipped.clear();
    for rect in damage {
        if let Some(part) = rect.intersection(dst) {
            clipped.push(Rectangle::new(part.loc - dst.loc, part.size));
        }
    }
}

/// Creates an offscreen buffer.
fn create_buffer(renderer: &mut PixmanRenderer, physical: Size) -> Result<PixmanImage<'static, 'static>, RenderError> {
    renderer
        .create_buffer(
            TARGET_FORMAT,
            PhysicalSize::from((physical.width as i32, physical.height as i32)),
        )
        .map_err(|error| {
            warn!(
                ?error,
                width = physical.width,
                height = physical.height,
                "[render] no offscreen buffer"
            );
            RenderError::Unavailable
        })
}

/// Imports a layer's client buffer, applying its tint.
///
/// An import failure is logged and skipped: the client may have destroyed the
/// buffer between the commit and this frame, and one window's missing content must
/// not stop the output from being drawn.
fn import(renderer: &mut PixmanRenderer, surface: &Surface, scratch: &mut Vec<u8>) -> Option<PixmanTexture> {
    let texture = renderer
        .import_shm_buffer(&surface.buffer, None, &[])
        .map_err(|error| {
            warn!(
                ?error,
                "[render] a client buffer could not be imported; the layer is skipped"
            );
        })
        .ok()?;
    if surface.tint == Color::WHITE {
        return Some(texture);
    }
    let region = Rectangle::from_size(texture.size());
    let mapping = renderer.copy_texture(&texture, region, READBACK_FORMAT).ok()?;
    let bytes = renderer.map_texture(&mapping).ok()?;
    scratch.clear();
    scratch.extend_from_slice(bytes);
    multiply(scratch, surface.tint);
    renderer
        .import_memory(
            scratch,
            READBACK_FORMAT,
            PhysicalSize::from((mapping.width() as i32, mapping.height() as i32)),
            false,
        )
        .map_err(|error| {
            warn!(
                ?error,
                "[render] a tinted layer could not be made; the layer is skipped"
            );
        })
        .ok()
}

/// Multiplies every pixel's colour channels by `tint`.
///
/// The bytes come from pixman, which stores premultiplied values: multiplying the
/// premultiplied channels by a factor at most one keeps them premultiplied, which
/// is why the alpha channel is left alone.
fn multiply(bytes: &mut [u8], tint: Color) {
    let red = tint.r.clamp(0.0, 1.0);
    let green = tint.g.clamp(0.0, 1.0);
    let blue = tint.b.clamp(0.0, 1.0);
    let (pixels, remainder) = bytes.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty(), "a pixman readback is whole RGBA pixels");
    for pixel in pixels {
        pixel[0] = (f32::from(pixel[0]) * red).round() as u8;
        pixel[1] = (f32::from(pixel[1]) * green).round() as u8;
        pixel[2] = (f32::from(pixel[2]) * blue).round() as u8;
    }
}

/// The source rectangle of a layer, in buffer coordinates.
fn source(surface: &Surface) -> Rectangle<f64, BufferCoords> {
    Rectangle::new(
        (surface.src.x, surface.src.y).into(),
        (surface.src.width, surface.src.height).into(),
    )
}

/// A logical rectangle in physical pixels.
///
/// Both edges are converted and rounded rather than the origin and the extent, so
/// two rectangles that share an edge still share it after scaling: rounding the
/// extent on its own would leave a one-pixel gap that nothing ever repaints.
fn physical_rect(rect: Rect, scale: Scale) -> Rectangle<i32, Physical> {
    if scale.is_identity() {
        return Rectangle::new((rect.x, rect.y).into(), (rect.width as i32, rect.height as i32).into());
    }
    let ratio = scale.as_f64();
    let left = (f64::from(rect.x) * ratio).round() as i32;
    let top = (f64::from(rect.y) * ratio).round() as i32;
    let right = (rect.right() as f64 * ratio).round() as i32;
    let bottom = (rect.bottom() as f64 * ratio).round() as i32;
    Rectangle::new(
        (left, top).into(),
        ((right - left).max(0), (bottom - top).max(0)).into(),
    )
}

/// A scene colour as the renderer's colour type, premultiplied.
///
/// Pixman's solid fill takes premultiplied components: an unpremultiplied colour
/// with half alpha would be composited as if it were opaque on top of a half-bright
/// destination, which is the classic double-brightness blend bug. Multiplying here
/// keeps the compositor's colours straight and the renderer's premultiplied, which
/// is the conversion every GPU path does in its shader.
fn colour(colour: Color) -> Color32F {
    let alpha = colour.a.clamp(0.0, 1.0);
    Color32F::new(
        colour.r.clamp(0.0, 1.0) * alpha,
        colour.g.clamp(0.0, 1.0) * alpha,
        colour.b.clamp(0.0, 1.0) * alpha,
        alpha,
    )
}

/// The one render failure for a pixman error, logged where it happens.
fn failed(error: impl std::fmt::Debug) -> RenderError {
    warn!(?error, "[render] the software renderer refused an operation");
    RenderError::Failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Solid;

    fn frame_with(background: Color, rect: Rect, colour: Color) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.set_background(background);
        frame.push_damage(rect);
        frame.push_layer(Layer::Solid(Solid { dst: rect, colour }));
        frame
    }

    #[test]
    fn a_drawn_rectangle_is_where_a_probe_finds_it() {
        let mut software = Software::new(Size::new(64, 48)).expect("a software renderer");
        let mut frame = frame_with(Color::BLACK, Rect::new(0, 0, 8, 8), Color::from_rgba8(255, 0, 0, 255));
        frame.push_damage(Rect::new(0, 0, 64, 48));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        assert_eq!(image.size(), Size::new(64, 48));
        assert_eq!(
            image.colour_at(1, 1),
            Some([255, 0, 0, 255]),
            "the top-left corner is the drawn rectangle, so the image is not upside down"
        );
        assert_eq!(
            image.colour_at(63, 47),
            Some([0, 0, 0, 255]),
            "the background elsewhere"
        );
    }

    #[test]
    fn damage_limits_what_is_repainted() {
        let mut software = Software::new(Size::new(16, 16)).expect("a software renderer");
        let whole = Rect::new(0, 0, 16, 16);
        software
            .draw(
                &frame_with(
                    Color::from_rgba8(255, 0, 0, 255),
                    whole,
                    Color::from_rgba8(0, 0, 255, 255),
                ),
                Scale::IDENTITY,
            )
            .expect("the first frame");
        // A second frame repaints only the left half: the right half keeps the
        // pixels of the first frame, which is what "damage only" means.
        let mut frame = OutputFrame::new();
        frame.set_background(Color::from_rgba8(0, 255, 0, 255));
        frame.push_damage(Rect::new(0, 0, 8, 16));
        software.draw(&frame, Scale::IDENTITY).expect("the second frame");
        let image = software.screenshot().expect("a readback");
        assert_eq!(
            image.colour_at(2, 8),
            Some([0, 255, 0, 255]),
            "the damaged half was repainted"
        );
        assert_eq!(
            image.colour_at(12, 8),
            Some([0, 0, 255, 255]),
            "the undamaged half kept the first frame's blue layer"
        );
    }

    #[test]
    fn an_empty_frame_draws_nothing() {
        let mut software = Software::new(Size::new(8, 8)).expect("a software renderer");
        let mut frame = OutputFrame::new();
        frame.set_background(Color::from_rgba8(1, 2, 3, 255));
        software.draw(&frame, Scale::IDENTITY).expect("nothing drawn");
        let image = software.screenshot().expect("a readback");
        assert_eq!(
            image.colour_at(0, 0),
            Some([0, 0, 0, 0]),
            "an undamaged frame paints no pixel, not even the background"
        );
    }

    #[test]
    fn a_resize_replaces_the_buffer() {
        let mut software = Software::new(Size::new(8, 8)).expect("a software renderer");
        let frame = frame_with(Color::BLACK, Rect::new(0, 0, 8, 8), Color::from_rgba8(255, 0, 0, 255));
        software.draw(&frame, Scale::IDENTITY).expect("the first frame");
        software.resize(Size::new(32, 16)).expect("a bigger buffer");
        assert_eq!(software.size(), Size::new(32, 16));
        let image = software.screenshot().expect("a readback");
        assert_eq!(image.size(), Size::new(32, 16), "the readback follows the buffer");
        assert_eq!(
            image.colour_at(0, 0),
            Some([0, 0, 0, 0]),
            "a replaced buffer holds no pixels, so the composer must damage everything"
        );
    }

    #[test]
    fn a_scale_converts_the_frame_to_physical_pixels() {
        let mut software = Software::new(Size::new(32, 32)).expect("a software renderer at scale 2");
        let mut frame = frame_with(Color::BLACK, Rect::new(0, 0, 4, 4), Color::from_rgba8(255, 0, 0, 255));
        frame.push_damage(Rect::new(0, 0, 16, 16));
        software.draw(&frame, Scale::integral(2)).expect("drawn");
        let image = software.screenshot().expect("a readback");
        assert_eq!(
            image.colour_at(7, 7),
            Some([255, 0, 0, 255]),
            "the layer covers 8 physical pixels"
        );
        assert_eq!(
            image.colour_at(8, 8),
            Some([0, 0, 0, 255]),
            "and stops at the scaled edge"
        );
    }

    #[test]
    fn a_cursor_is_drawn_above_every_layer() {
        let mut software = Software::new(Size::new(16, 16)).expect("a software renderer");
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 16, 16));
        frame.push_layer(Layer::Solid(Solid {
            dst: Rect::new(0, 0, 16, 16),
            colour: Color::from_rgba8(0, 0, 255, 255),
        }));
        frame.set_cursor(Some(Solid {
            dst: Rect::new(2, 2, 4, 4),
            colour: Color::from_rgba8(255, 255, 255, 255),
        }));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        assert_eq!(
            image.colour_at(3, 3),
            Some([255, 255, 255, 255]),
            "the cursor is on top"
        );
        assert_eq!(
            image.colour_at(10, 10),
            Some([0, 0, 255, 255]),
            "the layer below shows around it"
        );
    }

    #[test]
    fn a_translucent_layer_blends_with_the_background() {
        let mut software = Software::new(Size::new(8, 8)).expect("a software renderer");
        let mut frame = OutputFrame::new();
        frame.set_background(Color::from_rgba8(0, 0, 0, 255));
        frame.push_damage(Rect::new(0, 0, 8, 8));
        frame.push_layer(Layer::Solid(Solid {
            dst: Rect::new(0, 0, 8, 8),
            colour: Color::from_rgba8(255, 255, 255, 128),
        }));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        let [red, green, blue, alpha] = image.colour_at(4, 4).expect("a pixel");
        assert!(
            red.abs_diff(128) <= 2 && green.abs_diff(128) <= 2 && blue.abs_diff(128) <= 2,
            "half of white over black is mid grey, within a rounding step: {red} {green} {blue}"
        );
        assert_eq!(alpha, 255, "the opaque background keeps the frame opaque");
    }
}
