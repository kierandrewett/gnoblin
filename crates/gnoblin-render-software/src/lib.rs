//! The software renderer: Smithay's Pixman renderer drawing into an offscreen
//! buffer.
//!
//! D12 ships on GLES with Pixman for software rendering, and this crate is the Pixman
//! half: it needs no GPU, no EGL and no session, so it runs in CI and it is what the
//! integration tests and the parity harness read pixels from. The GLES and Vulkan
//! renderers draw the same [`OutputFrame`] without changing it.
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
//! - **Corner radius and effect passes are not drawn here.** Pixman has no shader, so
//!   a rounded corner, a shadow, a backdrop blur and a user shader are the GLES
//!   renderer's work (D12: GLES now, Pixman for software). This path draws the
//!   rectangle and leaves the properties it cannot honour alone instead of faking
//!   them, which is what makes the two renderers comparable on the properties both
//!   implement (placement, opacity, tint, damage).
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
use smithay::backend::renderer::pixman::{PixmanRenderer, PixmanTarget, PixmanTexture};
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, Frame as _, ImportMem, ImportMemWl, Offscreen, Renderer, Texture, TextureFilter,
};
use smithay::reexports::pixman::Image as PixmanImage;
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Size as PhysicalSize, Transform};

use gnoblin_types::{Color, Scale, Size};

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{ColourMatrix, Layer, OutputFrame, Overlay, Surface, SurfaceSource, View};
use gnoblin_render_frame::geometry::physical_rect;
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::image::READBACK_FORMAT;

gnoblin_log::logger!(log = "render");

/// The colour format of the offscreen buffer.
///
/// ARGB8888 is what Pixman composites fastest and what an `wl_shm` client hands
/// over, so a client buffer needs no conversion.
const TARGET_FORMAT: Fourcc = Fourcc::Argb8888;

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
    /// The cursor's texture for the frame being drawn, if it has one.
    cursor: Option<PixmanTexture>,
    /// The pixels under the cursor before it was drawn: packed rows of its visible rectangle.
    cursor_under: Vec<u8>,
    /// The cursor's visible rectangle in the last frame, or `None` when it had none.
    cursor_under_rect: Option<Rectangle<i32, Physical>>,
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
            log::warn!(area = "render", ?error, "the pixman renderer could not be created");
            RenderError::Unavailable
        })?;
        let buffer = create_buffer(&mut renderer, physical)?;
        Ok(Self {
            renderer,
            buffer,
            physical,
            textures: Vec::new(),
            cursor: None,
            cursor_under: Vec::new(),
            cursor_under_rect: None,
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
        result?;
        // The magnifier and the colour matrix act on the whole buffer, so they run
        // after the layers are painted. An identity view or matrix changes nothing, so
        // it is dropped here: running it would still round-trip every pixel through a
        // straight colour and could move its values by one.
        let view = frame.view().filter(|view| view.zoom != 1.0);
        let matrix = frame.colour_matrix().filter(|matrix| *matrix != ColourMatrix::IDENTITY);
        if view.is_some() || matrix.is_some() {
            self.final_pass(view, matrix)?;
        }
        Ok(())
    }

    /// Copies what a surface layer shows, as the renderer samples it: the raw pixels of its
    /// client buffer, before the layer's tint, opacity and transform.
    ///
    /// The copy is what a closing window keeps after its client has let go of the buffer, so
    /// it is taken from the same import the draw uses. A [`SurfaceSource::Snapshot`] is
    /// already such a copy and is returned as it is.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the buffer cannot be imported or read back, for example
    /// when the client has destroyed it. The caller then has nothing to fade and removes the
    /// window at once.
    pub fn snapshot(&mut self, surface: &Surface) -> Result<Image, RenderError> {
        let buffer = match &surface.source {
            SurfaceSource::Snapshot(image) => return Ok(image.as_ref().clone()),
            SurfaceSource::Buffer(buffer) => buffer,
        };
        let texture = self.renderer.import_shm_buffer(buffer, None, &[]).map_err(failed)?;
        read_texture(&mut self.renderer, &texture)
    }

    /// Reads the buffer back as an [`Image`], without the cursor.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman cannot copy the buffer out.
    pub fn screenshot(&mut self) -> Result<Image, RenderError> {
        self.capture(false)
    }

    /// Reads the buffer back as an [`Image`], with the cursor composed in when `with_cursor`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman cannot copy the buffer out.
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        self.readback(with_cursor)
    }

    /// The buffer as drawn, overlay and cursor included. Exposed only to this crate's own
    /// tests and, with the `test-support` feature, to the Vulkan parity tests that compare a
    /// composed frame. Normal builds cannot reach it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn composed(&mut self) -> Result<Image, RenderError> {
        self.readback(true)
    }

    /// Reads the buffer back, with the cursor's pixels put back when `!with_cursor`.
    fn readback(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        let Self {
            renderer,
            buffer,
            physical,
            cursor_under,
            cursor_under_rect,
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
        let mut bytes = bytes.to_vec();
        // The cursor was drawn last, so its pixels go back first.
        if let (false, Some(rect)) = (with_cursor, *cursor_under_rect) {
            paste_under(&mut bytes, stride, rect, cursor_under);
        }
        Image::new(Size::new(mapping.width(), mapping.height()), stride, bytes).ok_or(RenderError::Failed)
    }

    /// Applies the magnifier view and the colour matrix to the whole buffer.
    ///
    /// Pixman cannot resample a buffer in place, so the buffer is read back with the
    /// cursor, transformed on the CPU, and uploaded over itself. The upload goes to a
    /// cleared target with an unblended copy, so nothing of the previous contents
    /// survives under transparent pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman cannot read, upload or draw the buffer.
    fn final_pass(&mut self, view: Option<View>, matrix: Option<ColourMatrix>) -> Result<(), RenderError> {
        let image = self.readback(true)?;
        let size = image.size();
        let packed = transform_pixels(&image, view, matrix);
        let Self {
            renderer,
            buffer,
            physical,
            ..
        } = self;
        let output = PhysicalSize::from((physical.width as i32, physical.height as i32));
        let source_size = PhysicalSize::from((size.width as i32, size.height as i32));
        let texture = renderer
            .import_memory(&packed, READBACK_FORMAT, source_size, false)
            .map_err(failed)?;
        let full: Rectangle<i32, Physical> = Rectangle::from_size(output);
        let source: Rectangle<f64, BufferCoords> = Rectangle::new(
            (0.0, 0.0).into(),
            (f64::from(size.width), f64::from(size.height)).into(),
        );
        let mut target = renderer.bind(buffer).map_err(failed)?;
        let mut pixman_frame = renderer
            .render(&mut target, output, Transform::Normal)
            .map_err(failed)?;
        pixman_frame
            .clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full])
            .map_err(failed)?;
        pixman_frame
            .render_texture_from_to(&texture, source, full, &[full], &[], Transform::Normal, 1.0)
            .map_err(failed)?;
        Ok(())
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
            cursor,
            cursor_under,
            cursor_under_rect,
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
                Layer::Overlay(overlay) => {
                    // The overlay is a bitmap, drawn in a pass of its own with nearest-neighbour
                    // scaling. Pixman cannot change the filter while a frame is open, so the frame
                    // closes here and reopens once the overlay is down.
                    let _sync = pixman_frame.finish().map_err(failed)?;
                    let texture = textures.get(index).and_then(Option::as_ref);
                    draw_overlay_layer(renderer, &mut target, output, overlay, texture, scale, damage, clipped)?;
                    pixman_frame = renderer
                        .render(&mut target, output, Transform::Normal)
                        .map_err(failed)?;
                    continue;
                }
            };
            layer_damage(damage, dst, clipped);
            if clipped.is_empty() {
                continue;
            }
            match layer {
                Layer::Solid(solid) => pixman_frame
                    .draw_solid(dst, clipped, colour(tinted(solid.colour, solid.tint)))
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
                Layer::Overlay(_) => continue,
            }
        }
        // Pixman signals nothing: its sync point is already signalled, and the
        // offscreen buffer is read on the same thread. The value is dropped on
        // purpose rather than waited on.
        let _sync = pixman_frame.finish().map_err(failed)?;
        *cursor_under_rect = None;
        if let (Some(cursor), Some(texture)) = (frame.cursor(), cursor.as_ref()) {
            // The cursor is drawn above the overlay, with the same nearest-neighbour scaling.
            // Its pixels are kept first, so a capture without the cursor can put them back.
            let bounds = Rectangle::new((0, 0).into(), (physical.width as i32, physical.height as i32).into());
            if let Some(dst) = physical_rect(cursor.dst(), scale).intersection(bounds) {
                capture_under(renderer, &target, dst, damage, cursor_under)?;
                *cursor_under_rect = Some(dst);
            }
            renderer.upscale_filter(TextureFilter::Nearest).map_err(failed)?;
            let drawn = draw_overlay(renderer, &mut target, output, cursor, texture, scale, damage, clipped);
            renderer.upscale_filter(TextureFilter::Linear).map_err(failed)?;
            drawn?;
        }
        Ok(())
    }

    /// Imports every surface layer's texture, in layer order.
    fn prepare(&mut self, frame: &OutputFrame) {
        let Self {
            renderer,
            textures,
            cursor: cursor_texture,
            scratch,
            ..
        } = self;
        textures.clear();
        for layer in frame.layers() {
            match layer {
                Layer::Solid(_) => textures.push(None),
                Layer::Overlay(overlay) => textures.push(import_bitmap(renderer, overlay.image())),
                Layer::Surface(surface) => textures.push(import(renderer, surface, scratch)),
            }
        }
        *cursor_texture = frame
            .cursor()
            .and_then(|cursor| import_bitmap(renderer, cursor.image()));
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

/// Draws the overlay layer at its place in the list with nearest-neighbour scaling. A capture
/// keeps the overlay; a secure one is masked by the backend, not here.
#[allow(clippy::too_many_arguments)]
fn draw_overlay_layer(
    renderer: &mut PixmanRenderer,
    target: &mut PixmanTarget<'_>,
    output: PhysicalSize<i32, Physical>,
    overlay: &Overlay,
    texture: Option<&PixmanTexture>,
    scale: Scale,
    damage: &[Rectangle<i32, Physical>],
    clipped: &mut Vec<Rectangle<i32, Physical>>,
) -> Result<(), RenderError> {
    let Some(texture) = texture else {
        return Ok(());
    };
    // The overlay is a bitmap, so it scales with nearest neighbour; the default is restored
    // whether or not the draw succeeded.
    renderer.upscale_filter(TextureFilter::Nearest).map_err(failed)?;
    let drawn = draw_overlay(renderer, target, output, overlay, texture, scale, damage, clipped);
    renderer.upscale_filter(TextureFilter::Linear).map_err(failed)?;
    drawn
}

/// Draws the overlay over what the layers painted, scaled to its destination.
///
/// The caller sets the renderer's upscale filter, which pixman uses for the scaling.
#[allow(clippy::too_many_arguments)]
fn draw_overlay(
    renderer: &mut PixmanRenderer,
    target: &mut PixmanTarget<'_>,
    output: PhysicalSize<i32, Physical>,
    overlay: &Overlay,
    texture: &PixmanTexture,
    scale: Scale,
    damage: &[Rectangle<i32, Physical>],
    clipped: &mut Vec<Rectangle<i32, Physical>>,
) -> Result<(), RenderError> {
    let dst = physical_rect(overlay.dst(), scale);
    layer_damage(damage, dst, clipped);
    if clipped.is_empty() {
        return Ok(());
    }
    let size = overlay.image().size();
    let src = Rectangle::new(
        (0.0, 0.0).into(),
        (f64::from(size.width), f64::from(size.height)).into(),
    );
    let mut pixman_frame = renderer.render(target, output, Transform::Normal).map_err(failed)?;
    pixman_frame
        .render_texture_from_to(texture, src, dst, clipped, &[], Transform::Normal, 1.0)
        .map_err(failed)?;
    let _sync = pixman_frame.finish().map_err(failed)?;
    Ok(())
}

/// Keeps the pixels under `dst`, read from the damaged parts of it, which the layers have just painted.
fn capture_under(
    renderer: &mut PixmanRenderer,
    target: &PixmanTarget<'_>,
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    under: &mut Vec<u8>,
) -> Result<(), RenderError> {
    let width = dst.size.w as usize * 4;
    under.resize(width * dst.size.h as usize, 0);
    for rect in damage {
        let Some(rect) = rect.intersection(dst) else {
            continue;
        };
        let row = rect.size.w as usize * 4;
        let x = (rect.loc.x - dst.loc.x) as usize * 4;
        let y = (rect.loc.y - dst.loc.y) as usize;
        let buffer: Rectangle<i32, BufferCoords> =
            Rectangle::new((rect.loc.x, rect.loc.y).into(), (rect.size.w, rect.size.h).into());
        let mapping = renderer
            .copy_framebuffer(target, buffer, READBACK_FORMAT)
            .map_err(failed)?;
        let height = mapping.height() as usize;
        if height == 0 {
            return Err(RenderError::Failed);
        }
        let bytes = renderer.map_texture(&mapping).map_err(failed)?;
        let stride = bytes.len() / height;
        for (line, source) in bytes.chunks(stride).enumerate() {
            let start = (y + line) * width + x;
            under[start..start + row].copy_from_slice(&source[..row]);
        }
    }
    Ok(())
}

/// Writes the captured pixels of the overlay's rectangle `rect` back over `bytes`,
/// a readback of the whole buffer with the given row `stride`.
fn paste_under(bytes: &mut [u8], stride: usize, rect: Rectangle<i32, Physical>, under: &[u8]) {
    let row = rect.size.w as usize * 4;
    if row == 0 {
        return;
    }
    let x = rect.loc.x as usize * 4;
    for (line, source) in under.chunks_exact(row).enumerate() {
        let start = (rect.loc.y as usize + line) * stride + x;
        bytes[start..start + row].copy_from_slice(source);
    }
}

/// Produces the magnified, recoloured pixels as a packed RGBA buffer.
///
/// The view is nearest-neighbour: output pixel `p` shows the source pixel at
/// `centre + (p - centre) / zoom`, clamped to the image. The colour matrix acts on
/// straight (unpremultiplied) colour and is then premultiplied again. Fully
/// transparent pixels are passed through, since they have no colour to change.
fn transform_pixels(image: &Image, view: Option<View>, matrix: Option<ColourMatrix>) -> Vec<u8> {
    let size = image.size();
    let (width, height) = (size.width as usize, size.height as usize);
    let stride = image.stride();
    let data = image.data();
    let mut out = vec![0_u8; width * height * 4];
    for y in 0..height {
        let src_y = match view {
            Some(view) => sample_index(y, view.centre_y, view.zoom, height),
            None => y,
        };
        for x in 0..width {
            let src_x = match view {
                Some(view) => sample_index(x, view.centre_x, view.zoom, width),
                None => x,
            };
            let at = src_y * stride + src_x * 4;
            let Some(pixel) = data.get(at..at + 4) else {
                continue;
            };
            let pixel = [pixel[0], pixel[1], pixel[2], pixel[3]];
            let pixel = match matrix {
                Some(matrix) => tint_pixel(pixel, &matrix),
                None => pixel,
            };
            let dst = (y * width + x) * 4;
            out[dst..dst + 4].copy_from_slice(&pixel);
        }
    }
    out
}

/// The source index shown at output position `position`, for a view magnified by `zoom`
/// about `centre`, clamped to `0..length`.
fn sample_index(position: usize, centre: f32, zoom: f32, length: usize) -> usize {
    // Sample at the pixel centre so that zoom 1 maps every pixel to itself.
    let at = position as f32 + 0.5;
    let source = centre + (at - centre) / zoom;
    let last = length.saturating_sub(1) as f32;
    source.floor().clamp(0.0, last) as usize
}

/// Applies a colour matrix to one premultiplied RGBA pixel, returning premultiplied RGBA.
///
/// Each output channel is the matrix row dotted with the straight `(r, g, b, a)`
/// input, plus its offset, clamped to `0..=1`.
///
/// Rule: a pixel with zero alpha is returned unchanged, whatever the matrix's offsets. It
/// has no straight colour to transform, and an offset would otherwise make empty space
/// visible.
fn tint_pixel(pixel: [u8; 4], matrix: &ColourMatrix) -> [u8; 4] {
    if pixel[3] == 0 {
        return pixel;
    }
    let alpha = f32::from(pixel[3]) / 255.0;
    let straight = [
        (f32::from(pixel[0]) / 255.0 / alpha).min(1.0),
        (f32::from(pixel[1]) / 255.0 / alpha).min(1.0),
        (f32::from(pixel[2]) / 255.0 / alpha).min(1.0),
        alpha,
    ];
    let mut out = [0.0_f32; 4];
    for (row, channel) in matrix.rows.iter().zip(out.iter_mut()) {
        let value = row[0] * straight[0] + row[1] * straight[1] + row[2] * straight[2] + row[3] * straight[3] + row[4];
        *channel = value.clamp(0.0, 1.0);
    }
    let new_alpha = out[3];
    [
        (out[0] * new_alpha * 255.0).round() as u8,
        (out[1] * new_alpha * 255.0).round() as u8,
        (out[2] * new_alpha * 255.0).round() as u8,
        (new_alpha * 255.0).round() as u8,
    ]
}

/// Creates an offscreen buffer.
fn create_buffer(renderer: &mut PixmanRenderer, physical: Size) -> Result<PixmanImage<'static, 'static>, RenderError> {
    renderer
        .create_buffer(
            TARGET_FORMAT,
            PhysicalSize::from((physical.width as i32, physical.height as i32)),
        )
        .map_err(|error| {
            log::warn!(
                area = "render",
                ?error,
                width = physical.width,
                height = physical.height,
                "no offscreen buffer"
            );
            RenderError::Unavailable
        })
}

/// Reads a whole texture back as an [`Image`] of its own size, top left first.
///
/// The bytes are the texture's own and are not un-premultiplied: pixman copies them raw, so a
/// client buffer's pixels come back premultiplied, which is the convention
/// [`SurfaceSource::Snapshot`] documents. The mapping's stride is pixman's, so it is measured
/// from the slice rather than assumed.
fn read_texture(renderer: &mut PixmanRenderer, texture: &PixmanTexture) -> Result<Image, RenderError> {
    let region = Rectangle::from_size(texture.size());
    let mapping = renderer
        .copy_texture(texture, region, READBACK_FORMAT)
        .map_err(failed)?;
    let width = mapping.width();
    let height = mapping.height();
    let bytes = renderer.map_texture(&mapping).map_err(failed)?;
    if height == 0 {
        return Err(RenderError::Failed);
    }
    let stride = bytes.len() / height as usize;
    Image::new(Size::new(width, height), stride, bytes.to_vec()).ok_or(RenderError::Failed)
}

/// Imports a layer's pixels, applying its tint.
///
/// A client buffer is imported through Smithay's `wl_shm` mapping, a snapshot from its own
/// bytes. An import failure is logged and skipped: the client may have destroyed the buffer
/// between the commit and this frame, and one window's missing content must not stop the
/// output from being drawn.
fn import(renderer: &mut PixmanRenderer, surface: &Surface, scratch: &mut Vec<u8>) -> Option<PixmanTexture> {
    let texture = match &surface.source {
        SurfaceSource::Buffer(buffer) => renderer
            .import_shm_buffer(buffer, None, &[])
            .map_err(|error| {
                log::warn!(
                    area = "render",
                    ?error,
                    "a client buffer could not be imported; the layer is skipped"
                );
            })
            .ok()?,
        SurfaceSource::Snapshot(image) => import_bitmap(renderer, image)?,
    };
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
            log::warn!(
                area = "render",
                ?error,
                "a tinted layer could not be made; the layer is skipped"
            );
        })
        .ok()
}

/// Imports premultiplied pixels held in an [`Image`]: the overlay, the cursor and a snapshot.
///
/// An import failure is logged and the bitmap is skipped for this frame, the same way a
/// client buffer is.
///
/// `import_memory` takes rows packed at `width * 4` bytes, so an image whose stride is padded
/// is repacked first; reading its bytes as they are would put every row after the first at the
/// wrong offset.
fn import_bitmap(renderer: &mut PixmanRenderer, image: &Image) -> Option<PixmanTexture> {
    let size = image.size();
    let row = size.width as usize * 4;
    let packed: std::borrow::Cow<'_, [u8]> = if image.stride() == row {
        std::borrow::Cow::Borrowed(image.data())
    } else {
        std::borrow::Cow::Owned(
            image
                .data()
                .chunks(image.stride())
                .take(size.height as usize)
                .flat_map(|line| &line[..row])
                .copied()
                .collect(),
        )
    };
    renderer
        .import_memory(
            &packed,
            READBACK_FORMAT,
            PhysicalSize::from((size.width as i32, size.height as i32)),
            false,
        )
        .map_err(|error| {
            log::warn!(
                area = "render",
                ?error,
                "a bitmap could not be imported; it is not drawn"
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

/// A straight colour multiplied by a layer's tint.
///
/// The tint is the same property a surface has, so a solid is tinted the same way: each
/// channel scaled, alpha scaled too, in straight space before the colour is premultiplied.
/// The GPU path's solid shader does exactly this, which is what makes the two comparable on
/// the property.
fn tinted(colour: Color, tint: Color) -> Color {
    Color::new(
        colour.r.clamp(0.0, 1.0) * tint.r.clamp(0.0, 1.0),
        colour.g.clamp(0.0, 1.0) * tint.g.clamp(0.0, 1.0),
        colour.b.clamp(0.0, 1.0) * tint.b.clamp(0.0, 1.0),
        colour.a.clamp(0.0, 1.0) * tint.a.clamp(0.0, 1.0),
    )
}

/// The one render failure for a pixman error, logged where it happens.
fn failed(error: impl std::fmt::Debug) -> RenderError {
    log::warn!(area = "render", ?error, "the software renderer refused an operation");
    RenderError::Failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render::Effects;
    use gnoblin_render_frame::frame::Solid;
    use gnoblin_types::Rect;

    fn frame_with(background: Color, rect: Rect, colour: Color) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.set_background(background);
        frame.push_damage(rect);
        frame.push_layer(Layer::Solid(Solid::new(rect, colour)));
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

    /// A surface layer drawing a snapshot, at `dst`, with no tint, opacity or transform.
    fn snapshot_layer(image: Image, dst: Rect) -> Layer {
        Layer::Surface(Surface {
            source: SurfaceSource::Snapshot(std::sync::Arc::new(image)),
            src: gnoblin_types::RectF::new(0.0, 0.0, f64::from(dst.width), f64::from(dst.height)),
            dst,
            transform: smithay::utils::Transform::Normal,
            opacity: 1.0,
            tint: Color::WHITE,
            corner_radius: 0.0,
            effects: Effects::new(),
            model: None,
            secure: false,
        })
    }

    /// A 2 by 2 image of four distinct opaque colours, stride padded to 12 bytes so a
    /// wrong stride would move the colours.
    fn quad_image() -> Image {
        let mut data = vec![0u8; 12 * 2];
        for (index, colour) in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [9, 9, 9, 255]]
            .iter()
            .enumerate()
        {
            let (x, y) = (index % 2, index / 2);
            data[y * 12 + x * 4..y * 12 + x * 4 + 4].copy_from_slice(colour);
        }
        Image::new(Size::new(2, 2), 12, data).expect("a valid image")
    }

    #[test]
    fn a_snapshot_layer_draws_its_pixels_unchanged() {
        // Rule: a closing window draws the picture it kept, byte for byte, so the snapshot
        // is what the window last showed rather than a re-rendered approximation.
        let image = quad_image();
        let mut frame = frame_with(Color::BLACK, Rect::new(0, 0, 2, 2), Color::TRANSPARENT);
        frame.push_layer(snapshot_layer(image.clone(), Rect::new(0, 0, 2, 2)));
        let mut software = Software::new(Size::new(4, 4)).expect("a software renderer");
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let drawn = software.screenshot().expect("a readback");
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(drawn.colour_at(x, y), image.colour_at(x, y), "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_snapshot_of_a_snapshot_is_the_same_image() {
        // Rule: snapshotting a layer that is already a snapshot copies it rather than
        // failing, so a window can close twice (fade out, then re-close) without losing its picture.
        let image = quad_image();
        let surface = Surface {
            source: SurfaceSource::Snapshot(std::sync::Arc::new(image.clone())),
            src: gnoblin_types::RectF::new(0.0, 0.0, 2.0, 2.0),
            dst: Rect::new(0, 0, 2, 2),
            transform: smithay::utils::Transform::Normal,
            opacity: 1.0,
            tint: Color::WHITE,
            corner_radius: 0.0,
            effects: Effects::new(),
            model: None,
            secure: false,
        };
        let mut software = Software::new(Size::new(2, 2)).expect("a software renderer");
        assert_eq!(software.snapshot(&surface).expect("a snapshot"), image);
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
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 16, 16),
            Color::from_rgba8(0, 0, 255, 255),
        )));
        let white = Image::new(Size::new(4, 4), 16, [255, 255, 255, 255].repeat(16)).expect("a 4x4 image");
        frame.set_cursor_image(Overlay::new(Rect::new(2, 2, 4, 4), &white));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.capture(true).expect("a readback");
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
    fn a_capture_without_the_cursor_leaves_it_out() {
        let mut software = Software::new(Size::new(16, 16)).expect("a software renderer");
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 16, 16));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 16, 16),
            Color::from_rgba8(0, 0, 255, 255),
        )));
        let white = Image::new(Size::new(4, 4), 16, [255, 255, 255, 255].repeat(16)).expect("a 4x4 image");
        frame.set_cursor_image(Overlay::new(Rect::new(2, 2, 4, 4), &white));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let without = software.capture(false).expect("a readback");
        assert_eq!(
            without.colour_at(3, 3),
            Some([0, 0, 255, 255]),
            "the cursor is left out"
        );
        assert_eq!(
            without.colour_at(10, 10),
            Some([0, 0, 255, 255]),
            "the layer below is kept"
        );
        let with = software.capture(true).expect("a readback");
        assert_eq!(with.colour_at(3, 3), Some([255, 255, 255, 255]), "the cursor is in");
    }

    #[test]
    fn a_translucent_layer_blends_with_the_background() {
        let mut software = Software::new(Size::new(8, 8)).expect("a software renderer");
        let mut frame = OutputFrame::new();
        frame.set_background(Color::from_rgba8(0, 0, 0, 255));
        frame.push_damage(Rect::new(0, 0, 8, 8));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 8, 8),
            Color::from_rgba8(255, 255, 255, 128),
        )));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        let [red, green, blue, alpha] = image.colour_at(4, 4).expect("a pixel");
        assert!(
            red.abs_diff(128) <= 2 && green.abs_diff(128) <= 2 && blue.abs_diff(128) <= 2,
            "half of white over black is mid grey, within a rounding step: {red} {green} {blue}"
        );
        assert_eq!(alpha, 255, "the opaque background keeps the frame opaque");
    }

    #[test]
    fn an_overlay_blends_over_the_layers_and_clearing_removes_it() {
        let mut software = Software::new(Size::new(6, 6)).expect("a software renderer");
        let translucent_red = Image::new(Size::new(2, 2), 8, [255, 0, 0, 128].repeat(4)).expect("a 2x2 image");
        let overlay = Overlay::new(Rect::new(2, 2, 2, 2), &translucent_red).expect("an overlay");
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 6, 6));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 6, 6),
            Color::from_rgba8(0, 0, 255, 255),
        )));
        frame.push_layer(Layer::Overlay(overlay));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.composed().expect("a readback");
        // Half red over opaque blue: red 128 from the premultiplied overlay, blue 255 * (255 - 128) / 255.
        assert_eq!(
            image.colour_at(2, 2),
            Some([128, 0, 127, 255]),
            "the overlay blends over the blue layer at its top-left pixel"
        );
        assert_eq!(
            image.colour_at(3, 3),
            Some([128, 0, 127, 255]),
            "and at its bottom-right pixel"
        );
        assert_eq!(
            image.colour_at(1, 1),
            Some([0, 0, 255, 255]),
            "the blue layer is unchanged just outside the overlay"
        );
        assert_eq!(
            image.colour_at(4, 4),
            Some([0, 0, 255, 255]),
            "and just past its far edge"
        );

        frame.clear();
        frame.push_damage(Rect::new(0, 0, 6, 6));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 6, 6),
            Color::from_rgba8(0, 0, 255, 255),
        )));
        software.draw(&frame, Scale::IDENTITY).expect("the cleared frame");
        let image = software.composed().expect("a readback");
        assert_eq!(
            image.colour_at(2, 2),
            Some([0, 0, 255, 255]),
            "clearing the frame removes the overlay"
        );
    }

    #[test]
    fn a_capture_includes_the_overlay_blended_over_the_layers() {
        let mut software = Software::new(Size::new(6, 6)).expect("a software renderer");
        let translucent_red = Image::new(Size::new(2, 2), 8, [255, 0, 0, 128].repeat(4)).expect("a 2x2 image");
        let overlay = Overlay::new(Rect::new(2, 2, 2, 2), &translucent_red).expect("an overlay");
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 6, 6));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 6, 6),
            Color::from_rgba8(0, 0, 255, 255),
        )));
        frame.push_layer(Layer::Overlay(overlay));
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        assert_eq!(
            image.colour_at(2, 2),
            Some([128, 0, 127, 255]),
            "a capture shows the overlay blended over the blue layer, not the layer alone"
        );
        assert_eq!(
            image.colour_at(3, 3),
            Some([128, 0, 127, 255]),
            "and at the overlay's far corner"
        );
        assert_eq!(
            image.colour_at(1, 1),
            Some([0, 0, 255, 255]),
            "and the layer outside the overlay is unchanged"
        );
    }

    /// A frame of solid one-pixel layers, one per pixel of a `width` by `height` output,
    /// each coloured by `colour(x, y)`, damaged over the whole output.
    fn pattern_frame(width: i32, height: i32, colour: impl Fn(i32, i32) -> Color) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.set_background(Color::TRANSPARENT);
        frame.push_damage(Rect::new(0, 0, width as u32, height as u32));
        for y in 0..height {
            for x in 0..width {
                frame.push_layer(Layer::Solid(Solid::new(Rect::new(x, y, 1, 1), colour(x, y))));
            }
        }
        frame
    }

    #[test]
    fn an_identity_view_and_matrix_leave_the_buffer_bit_identical() {
        // A translucent pattern, so a round trip through straight colour would show.
        let colour = |x: i32, y: i32| Color::from_rgba8((x * 60) as u8, (y * 60) as u8, 90, 128);
        let plain = pattern_frame(4, 4, colour);
        let mut identity = pattern_frame(4, 4, colour);
        identity.set_view(View::new(1.0, 2.0, 3.0));
        identity.set_colour_matrix(Some(ColourMatrix::IDENTITY));
        let mut software = Software::new(Size::new(4, 4)).expect("a software renderer");
        software.draw(&plain, Scale::IDENTITY).expect("drawn");
        let expected = software.screenshot().expect("a readback");
        software.draw(&identity, Scale::IDENTITY).expect("drawn");
        let actual = software.screenshot().expect("a readback");
        assert_eq!(actual.data(), expected.data(), "the identity path changes no byte");
    }

    #[test]
    fn a_two_times_magnifier_about_the_centre_is_nearest_neighbour() {
        // Output pixel x samples source floor(2 + (x + 0.5 - 2) / 2): 1, 1, 2, 2.
        let colour = |x: i32, y: i32| Color::from_rgba8((x * 60) as u8, (y * 60) as u8, 200, 255);
        let mut frame = pattern_frame(4, 4, colour);
        frame.set_view(View::new(2.0, 2.0, 2.0));
        let mut software = Software::new(Size::new(4, 4)).expect("a software renderer");
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        let source = [1, 1, 2, 2];
        for y in 0..4 {
            for x in 0..4 {
                let want = [
                    (source[x as usize] * 60) as u8,
                    (source[y as usize] * 60) as u8,
                    200,
                    255,
                ];
                assert_eq!(image.colour_at(x, y), Some(want), "output pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn an_invert_matrix_keeps_alpha_and_inverts_straight_colour() {
        // Straight grey 64 at alpha 128. The expected result is computed from the buffer's
        // own premultiplied value, so pixman's rounding is not guessed at.
        let mut frame = pattern_frame(1, 1, |_, _| Color::from_rgba8(64, 64, 64, 128));
        let plain = {
            let mut software = Software::new(Size::new(1, 1)).expect("a software renderer");
            software.draw(&frame, Scale::IDENTITY).expect("drawn");
            software
                .screenshot()
                .expect("a readback")
                .colour_at(0, 0)
                .expect("a pixel")
        };
        let invert = ColourMatrix::new([
            [-1.0, 0.0, 0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ])
        .expect("a finite matrix");
        frame.set_colour_matrix(Some(invert));
        let mut software = Software::new(Size::new(1, 1)).expect("a software renderer");
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let got = software
            .screenshot()
            .expect("a readback")
            .colour_at(0, 0)
            .expect("a pixel");
        assert_eq!(got[3], plain[3], "alpha is kept");
        let alpha = f32::from(plain[3]) / 255.0;
        let straight = f32::from(plain[0]) / 255.0 / alpha;
        let want = ((1.0 - straight) * alpha * 255.0).round() as i32;
        for (channel, &value) in got.iter().enumerate().take(3) {
            assert!(
                (i32::from(value) - want).abs() <= 1,
                "channel {channel}: got {value} want about {want}"
            );
        }
    }

    #[test]
    fn a_transparent_pixel_stays_transparent_under_an_offset() {
        // Rule: a pixel with zero alpha is left alone by the colour matrix, so an offset
        // row cannot make empty space visible.
        let mut frame = pattern_frame(2, 2, |_, _| Color::TRANSPARENT);
        frame.set_colour_matrix(ColourMatrix::new([
            [1.0, 0.0, 0.0, 0.0, 0.5],
            [0.0, 1.0, 0.0, 0.0, 0.5],
            [0.0, 0.0, 1.0, 0.0, 0.5],
            [0.0, 0.0, 0.0, 1.0, 0.5],
        ]));
        let mut software = Software::new(Size::new(2, 2)).expect("a software renderer");
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let image = software.screenshot().expect("a readback");
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(image.colour_at(x, y), Some([0, 0, 0, 0]), "pixel ({x}, {y})");
            }
        }
    }
}
