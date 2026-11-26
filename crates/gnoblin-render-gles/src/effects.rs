//! The effects of one layer: outer and inset shadows, the backdrop blur, the background, the filter, the layer's own
//! pixels and the borders, drawn in the order `crates/gnoblin-render-vulkan/DESIGN.md` section 6.1 fixes.
//!
//! The order is by kind, whatever order the effects are listed in; within a kind the list order is the paint order.
//!
//! 1. Outer shadows, on the frame, at the layer's opacity as well as their own.
//! 2. Backdrop blur, into a group the layer's own paint joins.
//! 3. Backgrounds, then 4. inset shadows, then 5. the layer's pixels, in that group (or on the frame).
//! 6. Borders, on the frame: a border reaches outside the layer, past the group's corner clip.
//!
//! # The filter
//!
//! `Pass::Filter` grades everything the layer paints (shadows, background, inset shadows, content, borders) and may blur it.
//!
//! - A grade alone grades each piece as it is drawn. A solid colour is graded on the CPU by [`grade::apply`], exactly, and
//!   the layer's pixels in the surface pass by `gnoblin_grade`; a shell's shader pass grades its own result.
//! - A blur draws the pieces into a *picture* of
//!   their own at the layer's painted extent grown by the blur's reach, blurs the picture with the same sweeps as the
//!   backdrop blur, and composes it over the frame with the layer's tint and opacity, graded as one. It is not clipped by
//!   the corner radius, so it spills past the edge. A layer that also has a backdrop blur draws that glass first, outside
//!   the picture, so the filter does not blur the glass.
//!
//! # The backdrop blur's downscale and grain
//!
//! `downscale` d reduces the copied region to `ceil(size * d)` by successive halvings through a bilinear tap (each halving
//! averages 2x2 texels exactly), blurs that at radius `radius * d`, and scales it back up with linear filtering when it
//! is written into the group. The grade, tint and grain belong to that last write, at full resolution: grain made at the
//! small size would be stretched into blobs. At d = 1 the last vertical sweep is that write.
//!
//! Grain is counted from the layer's own top-left corner, so it moves with the layer. The write that carries it is made
//! over the layer's rectangle; the margin around it, which only a stacked blur reads, is written without grain.

use gnoblin_render::{
    BlurParams, Border, Effects, FilterParams, MAX_BLUR_PASSES, MIN_BLUR_DOWNSCALE, Pass, ShaderPass, Shadow,
};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::grade::{self, Grade};
use gnoblin_render_frame::shader::uniforms::Uniforms;
use gnoblin_types::{Color, Rect, Size};

use crate::gl::{Framebuffer, GlContext, QuadPlacement, Texture};
use crate::{MAX_RADIUS, Painter, Passes, Shaded, flipped_mapping, log};

/// A layer, in the form each of its effects needs it.
#[derive(Clone, Copy)]
pub(crate) struct EffectLayer<'a> {
    /// The layer's own physical rectangle.
    pub(crate) quad: Rect,
    /// The shape the corner radius rounds, a background and an inset shadow fill and a border runs along: the layer's
    /// silhouette when it is one part of a larger frame, else `quad`.
    pub(crate) shape: Rect,
    pub(crate) radius: f32,
    /// `Some(silhouette)` when the layer's own pixels are rounded to a frame larger than `quad`.
    pub(crate) rounded_to: Option<Rect>,
    /// The solid's straight colour; `None` for a layer of pixels.
    pub(crate) colour: Option<Color>,
    pub(crate) texture: Option<Texture>,
    pub(crate) src: [f32; 4],
    pub(crate) tint: Color,
    pub(crate) placement: QuadPlacement,
    /// The picture a shader on the layer reads with `backdrop(uv)`.
    pub(crate) backdrop: Option<Texture>,
    /// The shader passes the frame can draw.
    pub(crate) ready: &'a [&'a ShaderPass],
    pub(crate) effects: &'a Effects,
    /// The grade each piece is drawn with. Identity when the filter grades a picture instead.
    pub(crate) grade: Grade,
}

/// The layer's filter: the one `Pass::Filter` of its effects, or the filter that changes nothing.
pub(crate) fn filter_of(effects: &Effects) -> FilterParams {
    effects
        .passes()
        .iter()
        .find_map(|pass| match pass {
            Pass::Filter(filter) => Some(*filter),
            _ => None,
        })
        .unwrap_or(FilterParams::IDENTITY)
}

/// The grade a filter puts on colour.
pub(crate) fn grade_of(filter: &FilterParams) -> Grade {
    Grade {
        saturate: filter.saturate,
        brightness: filter.brightness,
        contrast: filter.contrast,
        noise: 0.0,
    }
}

fn shifted(rect: Rect, dx: i32, dy: i32) -> Rect {
    Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height)
}

/// The smallest rectangle holding both.
fn union(first: Rect, second: Rect) -> Rect {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = (first.x + first.width as i32).max(second.x + second.width as i32);
    let bottom = (first.y + first.height as i32).max(second.y + second.height as i32);
    Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
}

/// The quad of an outer shadow of `dst` and its blur in physical pixels: `dst` moved by the offset and grown by the
/// blur and the spread.
fn shadow_quad(dst: Rect, shadow: &Shadow, factor: f32) -> (Rect, f32) {
    let blur = (shadow.blur * factor).clamp(1.0, MAX_RADIUS);
    let inset = blur.round() as i32;
    // The shader insets the quad by `blur` on each side to find the shadow's shape, so growing the quad by `spread`
    // grows that shape by `spread` before it is blurred.
    let spread = (shadow.spread * factor).round() as i32;
    let offset_x = (shadow.offset.x as f32 * factor).round() as i32;
    let offset_y = (shadow.offset.y as f32 * factor).round() as i32;
    let grow = inset + spread;
    let quad = Rect::new(
        dst.x + offset_x - grow,
        dst.y + offset_y - grow,
        (dst.width as i32 + 2 * grow).max(0) as u32,
        (dst.height as i32 + 2 * grow).max(0) as u32,
    );
    (quad, blur)
}

/// A border's quad (the shape grown by how far the ring reaches, and one pixel more for the antialiased fringe), that
/// growth and the ring's band of signed distance from the shape's edge. `None` for a border that draws nothing.
fn border_geometry(shape: Rect, border: &Border, factor: f32) -> Option<(Rect, f32, (f32, f32))> {
    let width = border.width * factor;
    if width <= 0.0 || border.color.a <= 0.0 {
        return None;
    }
    let offset = border.offset * factor;
    let (near, far) = if border.inset {
        (-(offset + width), -offset)
    } else {
        (offset, offset + width)
    };
    let reach = if border.inset { -offset } else { offset + width };
    let grow = reach.max(0.0).ceil() as i32 + 1;
    let quad = Rect::new(
        shape.x - grow,
        shape.y - grow,
        shape.width + 2 * grow as u32,
        shape.height + 2 * grow as u32,
    );
    Some((quad, grow as f32, (near, far)))
}

/// One sweep of the Gaussian: `from` read along `step` physical pixels (a horizontal or a vertical step), written over
/// `area` of `into`, replacing what was there. The identity grade, no tint, no grain.
fn sweep(
    device: &mut GlContext,
    passes: &Passes,
    bytes: &mut Vec<u8>,
    from: &Framebuffer,
    into: &Framebuffer,
    area: Rect,
    step: [f32; 2],
) -> Result<(), RenderError> {
    let size = from.size();
    let mut uniforms = Uniforms::surface(area, flipped_mapping(area, size), Color::TRANSPARENT, 1.0, 0.0);
    uniforms.params = [step[0] / size.width as f32, step[1] / size.height as f32, 0.0, 0.0];
    device.begin(into)?;
    device.blend_replace()?;
    device.scissor(Some(area));
    passes.blur.pack(&uniforms, bytes);
    let drawn = device.draw(&passes.blur, bytes, area, Some(from.texture()), QuadPlacement::FLAT);
    device.blend_premultiplied()?;
    drawn?;
    device.end()
}

/// The last write of a backdrop blur: what is blurred, graded, tinted and given grain, written into the group.
struct Resolve<'b> {
    /// The blurred picture.
    source: Texture,
    /// Where the source's first texel sits in output pixels, and how many output pixels it spans.
    origin: (i32, i32),
    span: Size,
    /// One more tap along the source, in its texture units: the last vertical sweep, or none.
    step: [f32; 2],
    /// The rectangle the blur keeps.
    region: Rect,
    /// The layer's own rectangle, which carries the grain.
    dst: Rect,
    /// What the write may touch.
    write: Rect,
    blur: &'b BlurParams,
}

impl Resolve<'_> {
    fn run(
        &self,
        device: &mut GlContext,
        passes: &Passes,
        bytes: &mut Vec<u8>,
        into: &Framebuffer,
    ) -> Result<(), RenderError> {
        device.begin(into)?;
        device.blend_replace()?;
        device.scissor(Some(self.write));
        // The margin around the layer is only read by a stacked blur, so it has no grain; the layer's own rectangle is
        // written again, over it, with the grain counted from its top-left.
        let drawn = self.draw(device, passes, bytes, self.region, 0.0).and_then(|()| {
            if self.blur.noise > 0.0 {
                self.draw(device, passes, bytes, self.dst, self.blur.noise)
            } else {
                Ok(())
            }
        });
        device.blend_premultiplied()?;
        drawn
    }

    fn draw(
        &self,
        device: &mut GlContext,
        passes: &Passes,
        bytes: &mut Vec<u8>,
        quad: Rect,
        noise: f32,
    ) -> Result<(), RenderError> {
        let relative = Rect::new(quad.x - self.origin.0, quad.y - self.origin.1, quad.width, quad.height);
        let mut uniforms = Uniforms::surface(quad, flipped_mapping(relative, self.span), self.blur.tint, 1.0, 0.0);
        uniforms.params = [self.step[0], self.step[1], 0.0, 0.0];
        uniforms.grade = [self.blur.saturation, self.blur.brightness, self.blur.contrast, noise];
        passes.blur.pack(&uniforms, bytes);
        device.draw(&passes.blur, bytes, quad, Some(self.source), QuadPlacement::FLAT)
    }
}

/// The size one halving of `size` reaches towards `goal`: half, rounded up, but not below the goal.
fn halved(size: u32, goal: u32) -> u32 {
    if size > goal { size.div_ceil(2).max(goal) } else { size }
}

/// `size * downscale`, rounded up, at least a pixel and at most the size itself.
fn scaled(size: u32, downscale: f32) -> u32 {
    ((size as f32 * downscale).ceil() as u32).clamp(1, size.max(1))
}

impl Painter<'_> {
    /// Binds the canvas again, with the damage as the scissor, after a pass drew into a picture of its own.
    pub(crate) fn rebind_canvas(&mut self) -> Result<(), RenderError> {
        let Self {
            device,
            canvas,
            target,
            clip,
            ..
        } = self;
        device.begin(canvas.as_ref().unwrap_or(*target))?;
        device.scissor(Some(*clip));
        Ok(())
    }

    /// Draws the layer's outer shadows behind it, in list order, at the layer's `opacity` as well as their own: opacity
    /// fades the whole element, its shadows included.
    pub(crate) fn shadows(&mut self, paint: &EffectLayer<'_>, opacity: f32) -> Result<(), RenderError> {
        let factor = self.scale.as_f64() as f32;
        for pass in paint.effects.passes() {
            let Pass::Shadow(shadow) = pass else { continue };
            if shadow.inset || shadow.opacity <= 0.0 || shadow.color.a <= 0.0 {
                continue;
            }
            let (quad, blur) = shadow_quad(if shadow.frame { paint.shape } else { paint.quad }, shadow, factor);
            let mut uniforms = Uniforms::solid(
                quad,
                grade::apply(paint.grade, shadow.color),
                Color::WHITE,
                shadow.opacity * opacity,
            );
            uniforms.corner = paint.radius;
            uniforms.params = [blur, blur, blur, 0.0];
            let Self {
                device, passes, bytes, ..
            } = self;
            passes.shadow.pack(&uniforms, bytes);
            device.draw(&passes.shadow, bytes, quad, None, QuadPlacement::FLAT)?;
        }
        Ok(())
    }

    /// Fills the layer's shape with each background colour, in list order, below its content.
    pub(crate) fn backgrounds(&mut self, paint: &EffectLayer<'_>, opacity: f32) -> Result<(), RenderError> {
        for pass in paint.effects.passes() {
            let Pass::Background { color } = pass else { continue };
            if color.a <= 0.0 {
                continue;
            }
            let mut uniforms = Uniforms::solid(paint.shape, grade::apply(paint.grade, *color), paint.tint, opacity);
            uniforms.corner = paint.radius;
            let Self {
                device, passes, bytes, ..
            } = self;
            passes.solid.pack(&uniforms, bytes);
            device.draw(&passes.solid, bytes, paint.shape, None, QuadPlacement::FLAT)?;
        }
        Ok(())
    }

    /// Casts each inset shadow inwards from the edge of the layer's own rectangle, in list order, below its content. An
    /// inset shadow ignores `frame`: it follows the layer's own shape and never its silhouette.
    ///
    /// The layer's tint multiplies the colour here, on the CPU, because the inset pass carries no tint.
    pub(crate) fn inset_shadows(&mut self, paint: &EffectLayer<'_>, opacity: f32) -> Result<(), RenderError> {
        let factor = self.scale.as_f64() as f32;
        for pass in paint.effects.passes() {
            let Pass::Shadow(shadow) = pass else { continue };
            if !shadow.inset || shadow.opacity <= 0.0 || shadow.color.a <= 0.0 {
                continue;
            }
            let colour = grade::apply(paint.grade, shadow.color);
            let tint = paint.tint;
            let colour = Color::new(
                colour.r * tint.r,
                colour.g * tint.g,
                colour.b * tint.b,
                colour.a * tint.a,
            );
            let offset = [shadow.offset.x as f32 * factor, shadow.offset.y as f32 * factor];
            let blur = (shadow.blur * factor).clamp(1.0, MAX_RADIUS);
            let uniforms = Uniforms::inset_shadow(
                paint.quad,
                offset,
                blur,
                shadow.spread * factor,
                colour,
                shadow.opacity * opacity,
                paint.radius,
            );
            let Self {
                device, passes, bytes, ..
            } = self;
            passes.inset_shadow.pack(&uniforms, bytes);
            device.draw(&passes.inset_shadow, bytes, paint.quad, None, QuadPlacement::FLAT)?;
        }
        Ok(())
    }

    /// Draws the borders along the layer's shape, in list order, over everything else. `opacity` is the layer's own, so
    /// a faded window fades its border with it.
    pub(crate) fn borders(&mut self, paint: &EffectLayer<'_>, opacity: f32) -> Result<(), RenderError> {
        let factor = self.scale.as_f64() as f32;
        for pass in paint.effects.passes() {
            let Pass::Border(border) = pass else { continue };
            let Some((quad, grow, band)) = border_geometry(paint.shape, border, factor) else {
                continue;
            };
            let colour = grade::apply(paint.grade, border.color);
            let uniforms = Uniforms::border(quad, grow, band, colour, paint.tint, opacity, paint.radius);
            let Self {
                device, passes, bytes, ..
            } = self;
            passes.border.pack(&uniforms, bytes);
            device.draw(&passes.border, bytes, quad, None, QuadPlacement::FLAT)?;
        }
        Ok(())
    }

    /// Draws the layer's own pixels (or its solid colour, or its shader passes' result) at `opacity`. `grouped` says the
    /// layer is drawn into its blur group, which the last shader pass binds.
    pub(crate) fn content(&mut self, paint: &EffectLayer<'_>, opacity: f32, grouped: bool) -> Result<(), RenderError> {
        if paint.ready.is_empty() {
            let mut uniforms = match paint.colour {
                Some(colour) => {
                    let mut uniforms =
                        Uniforms::solid(paint.quad, grade::apply(paint.grade, colour), paint.tint, opacity);
                    uniforms.corner = paint.radius;
                    uniforms
                }
                None => {
                    let mut uniforms = Uniforms::surface(paint.quad, paint.src, paint.tint, opacity, paint.radius);
                    uniforms.grade = paint.grade.to_array();
                    uniforms
                }
            };
            if let Some(frame) = paint.rounded_to {
                uniforms.round_to(frame);
            }
            let Self {
                device, passes, bytes, ..
            } = self;
            let program = if paint.texture.is_some() {
                &passes.surface
            } else {
                &passes.solid
            };
            program.pack(&uniforms, bytes);
            device.draw(program, bytes, paint.quad, paint.texture, paint.placement)
        } else {
            self.draw_shaded(
                paint.ready,
                &Shaded {
                    quad: paint.quad,
                    colour: paint.colour,
                    texture: paint.texture,
                    src: paint.src,
                    tint: paint.tint,
                    opacity,
                    radius: paint.radius,
                    rounded_to: paint.rounded_to,
                    backdrop: paint.backdrop,
                    grade: paint.grade,
                    grouped,
                },
            )
        }
    }

    /// Blurs the pixels behind a layer into the layer's group, and reports whether it did.
    ///
    /// The first blur of a layer copies the frame's backdrop into the group. A later blur reads what the group already
    /// holds, so stacked blurs compound in order. The backdrop is what was drawn before the layer, not what it covers.
    ///
    /// One pass is a horizontal sweep of the separable Gaussian in [`gnoblin_render_frame::shader::sources::BLUR`] into
    /// the second scratch target, then a vertical sweep back. `blur.passes` such passes run in turn, so each sweep reads
    /// what the one before wrote. The grade (saturation, brightness, contrast, then the tint mixed over the result) and
    /// the grain belong to the last write only, so repeated passes do not compound them: see the module docs for how
    /// `blur.downscale` changes where that write happens.
    ///
    /// The copied region is the layer's rectangle grown by the kernel's reach, so the taps near the layer's edge read
    /// blurred pixels rather than the scratch target's clearing colour. Each pass spreads the taps by one reach, so the
    /// margin grows with the pass count; the group keeps that whole region, so that ring is the only price of it.
    pub(crate) fn blur(&mut self, dst: Rect, blur: BlurParams, grouped: bool) -> Result<bool, RenderError> {
        let radius = (blur.radius * self.scale.as_f64() as f32).clamp(1.0, MAX_RADIUS);
        let output = Rect::from_size(self.physical);
        let Some(rect) = dst.intersection(output) else {
            return Ok(false);
        };
        let sweeps = blur.passes.clamp(1, MAX_BLUR_PASSES);
        let reach = sweeps * ((radius / 3.0 * 4.0).ceil() as u32 + 1);
        let region = Rect::new(
            rect.x.saturating_sub(reach as i32),
            rect.y.saturating_sub(reach as i32),
            rect.width + 2 * reach,
            rect.height + 2 * reach,
        )
        .intersection(output)
        .unwrap_or(rect);
        self.ensure_scratch()?;
        let downscale = blur.downscale.clamp(MIN_BLUR_DOWNSCALE, 1.0);
        let small = Size::new(scaled(region.width, downscale), scaled(region.height, downscale));
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
            return Ok(false);
        };
        if !grouped {
            // The scratch target starts as the frame's background everywhere the copy does not reach, so a blur that
            // samples past the output repeats a colour the frame defined.
            device.clear(first, *background);
            device.copy(target, first, region)?;
        }
        let job = BackdropBlur {
            region,
            dst,
            // Only the last write is scissored to the damage, which the frame expects.
            write: region.intersection(*clip).unwrap_or(region),
            radius,
            sweeps,
            blur: &blur,
        };
        let done = if small.width == region.width && small.height == region.height {
            job.full(device, passes, bytes, first, second, *physical)
        } else {
            job.reduced(device, passes, bytes, first, *physical, small, downscale)
        };
        // Back to the damage scissor, which the rest of this rectangle's passes expect.
        device.scissor(Some(*clip));
        done?;
        Ok(true)
    }

    /// Draws a layer whose filter blurs as a picture: the pieces of the layer into a target
    /// of their own, the blur over that target, then the target over the frame at the layer's tint and opacity, graded.
    pub(crate) fn picture(
        &mut self,
        paint: &EffectLayer<'_>,
        filter: &FilterParams,
        opacity: f32,
    ) -> Result<(), RenderError> {
        let factor = self.scale.as_f64() as f32;
        let radius = (filter.blur * factor).clamp(0.0, MAX_RADIUS);
        let reach = if radius > 0.0 {
            (radius / 3.0 * 4.0).ceil() as i32 + 1
        } else {
            0
        };
        // What the pieces cover: the layer, its shape, every outer shadow and every border.
        let mut painted = union(paint.quad, paint.shape);
        for pass in paint.effects.passes() {
            match pass {
                Pass::Shadow(shadow) if !shadow.inset && shadow.opacity > 0.0 && shadow.color.a > 0.0 => {
                    let dst = if shadow.frame { paint.shape } else { paint.quad };
                    painted = union(painted, shadow_quad(dst, shadow, factor).0);
                }
                Pass::Border(border) => {
                    if let Some((quad, ..)) = border_geometry(paint.shape, border, factor) {
                        painted = union(painted, quad);
                    }
                }
                _ => {}
            }
        }
        // Only what can reach the output matters: the picture's blur spills `reach` pixels in.
        let output = Rect::from_size(self.physical);
        let seen = Rect::new(
            output.x - reach,
            output.y - reach,
            output.width + 2 * reach as u32,
            output.height + 2 * reach as u32,
        );
        let Some(painted) = painted.intersection(seen) else {
            return Ok(());
        };
        let extent = Rect::new(
            painted.x - reach,
            painted.y - reach,
            painted.width + 2 * reach as u32,
            painted.height + 2 * reach as u32,
        );
        if extent.intersection(self.clip).is_none() {
            return Ok(());
        }
        let limit = self.device.max_texture_size();
        if extent.width > limit || extent.height > limit {
            log::warn!(
                area = "render",
                "a filtered layer is larger than the driver's texture limit; it is drawn without its filter"
            );
            return Ok(());
        }
        let size = Size::new(extent.width, extent.height);
        let mut leased: Vec<Framebuffer> = Vec::with_capacity(2);
        let drawn = self.picture_into(&mut leased, paint, filter, opacity, extent, size, radius);
        for target in leased {
            self.device.give_back(target);
        }
        drawn
    }

    /// The body of [`Self::picture`]: `leased` takes the targets it makes, for the caller to give back on every path.
    #[allow(clippy::too_many_arguments)]
    fn picture_into(
        &mut self,
        leased: &mut Vec<Framebuffer>,
        paint: &EffectLayer<'_>,
        filter: &FilterParams,
        opacity: f32,
        extent: Rect,
        size: Size,
        radius: f32,
    ) -> Result<(), RenderError> {
        leased.push(self.device.lease(size)?);
        if radius > 0.0 {
            leased.push(self.device.lease(size)?);
        }
        // The pieces, in the picture's own coordinates, with the identity tint, opacity and grade: those belong to the
        // composition, which treats the picture as one.
        let (dx, dy) = (-extent.x, -extent.y);
        let local = EffectLayer {
            quad: shifted(paint.quad, dx, dy),
            shape: shifted(paint.shape, dx, dy),
            rounded_to: paint.rounded_to.map(|frame| shifted(frame, dx, dy)),
            tint: Color::WHITE,
            grade: Grade::IDENTITY,
            ..*paint
        };
        let saved = self.clip;
        let picture = leased.swap_remove(0);
        self.canvas = Some(picture);
        let painted = self.picture_pieces(&local, size);
        self.clip = saved;
        let Some(picture) = self.canvas.take() else {
            return Err(RenderError::Failed);
        };
        // `swap_remove` moved the second target to the front: put the picture first again.
        leased.insert(0, picture);
        painted?;
        let area = Rect::from_size(size);
        let Self {
            device,
            passes,
            bytes,
            target,
            clip,
            ..
        } = self;
        if let [picture, second] = &leased[..] {
            let step = (radius / 3.0).max(0.5);
            sweep(device, passes, bytes, picture, second, area, [step, 0.0])?;
            sweep(device, passes, bytes, second, picture, area, [0.0, step])?;
        }
        let picture = &leased[0];
        device.begin(target)?;
        device.scissor(Some(*clip));
        let mut uniforms = Uniforms::surface(extent, flipped_mapping(area, size), paint.tint, opacity, 0.0);
        uniforms.grade = grade_of(filter).to_array();
        passes.surface.pack(&uniforms, bytes);
        device.draw(
            &passes.surface,
            bytes,
            extent,
            Some(picture.texture()),
            QuadPlacement::FLAT,
        )
    }

    /// Draws the pieces of a layer into the picture held as the canvas.
    fn picture_pieces(&mut self, local: &EffectLayer<'_>, size: Size) -> Result<(), RenderError> {
        let area = Rect::from_size(size);
        let Self { device, canvas, .. } = self;
        let Some(canvas) = canvas.as_ref() else {
            return Err(RenderError::Failed);
        };
        device.begin(canvas)?;
        device.clear(canvas, Color::TRANSPARENT);
        device.scissor(Some(area));
        self.clip = area;
        self.shadows(local, 1.0)?;
        self.backgrounds(local, 1.0)?;
        self.inset_shadows(local, 1.0)?;
        self.content(local, 1.0, false)?;
        self.borders(local, 1.0)
    }
}

/// One backdrop blur: the region it keeps and how it is swept.
struct BackdropBlur<'b> {
    /// The layer's rectangle grown by the kernel's reach and clipped to the output.
    region: Rect,
    /// The layer's own rectangle.
    dst: Rect,
    /// What the last write may touch.
    write: Rect,
    /// The blur radius in physical pixels at full resolution.
    radius: f32,
    sweeps: u32,
    blur: &'b BlurParams,
}

impl BackdropBlur<'_> {
    /// At full resolution: horizontal and vertical sweeps in the scratch targets over the region, the last vertical sweep
    /// being the write that grades.
    fn full(
        &self,
        device: &mut GlContext,
        passes: &Passes,
        bytes: &mut Vec<u8>,
        first: &Framebuffer,
        second: &Framebuffer,
        physical: Size,
    ) -> Result<(), RenderError> {
        let step = (self.radius / 3.0).max(0.5);
        for sweep_index in 0..self.sweeps {
            sweep(device, passes, bytes, first, second, self.region, [step, 0.0])?;
            if sweep_index + 1 < self.sweeps {
                sweep(device, passes, bytes, second, first, self.region, [0.0, step])?;
            }
        }
        Resolve {
            source: second.texture(),
            origin: (0, 0),
            span: physical,
            step: [0.0, step / physical.height as f32],
            region: self.region,
            dst: self.dst,
            write: self.write,
            blur: self.blur,
        }
        .run(device, passes, bytes, first)
    }

    /// At a fraction of the resolution: the region is halved down to `small` through bilinear taps, blurred there at the
    /// radius scaled by `downscale`, and written back up into the group through the grade.
    #[allow(clippy::too_many_arguments)]
    fn reduced(
        &self,
        device: &mut GlContext,
        passes: &Passes,
        bytes: &mut Vec<u8>,
        first: &Framebuffer,
        physical: Size,
        small: Size,
        downscale: f32,
    ) -> Result<(), RenderError> {
        let mut leased: Vec<Framebuffer> = Vec::new();
        let done = self.reduce_and_blur(device, passes, bytes, first, physical, small, downscale, &mut leased);
        for target in leased {
            device.give_back(target);
        }
        done
    }

    #[allow(clippy::too_many_arguments)]
    fn reduce_and_blur(
        &self,
        device: &mut GlContext,
        passes: &Passes,
        bytes: &mut Vec<u8>,
        first: &Framebuffer,
        physical: Size,
        small: Size,
        downscale: f32,
        leased: &mut Vec<Framebuffer>,
    ) -> Result<(), RenderError> {
        let mut size = Size::new(self.region.width, self.region.height);
        let mut source = first.texture();
        let mut mapping = flipped_mapping(self.region, physical);
        while size != small {
            let next = Size::new(halved(size.width, small.width), halved(size.height, small.height));
            leased.push(device.lease(next)?);
            let Some(picture) = leased.last() else {
                return Err(RenderError::Failed);
            };
            let area = Rect::from_size(next);
            let uniforms = Uniforms::surface(area, mapping, Color::WHITE, 1.0, 0.0);
            device.begin(picture)?;
            device.blend_replace()?;
            passes.surface.pack(&uniforms, bytes);
            let drawn = device.draw(&passes.surface, bytes, area, Some(source), QuadPlacement::FLAT);
            device.blend_premultiplied()?;
            drawn?;
            source = picture.texture();
            mapping = flipped_mapping(area, next);
            size = next;
        }
        leased.push(device.lease(small)?);
        let (Some(second), Some(reduced)) = (leased.pop(), leased.last()) else {
            return Err(RenderError::Failed);
        };
        let area = Rect::from_size(small);
        // The same tap step as at full resolution, in the reduced picture's own pixels: the blur's width in the output
        // does not depend on `downscale`.
        let step = (self.radius / 3.0).max(0.5) * downscale;
        let swept = (0..self.sweeps).try_for_each(|_| {
            sweep(device, passes, bytes, reduced, &second, area, [step, 0.0])?;
            sweep(device, passes, bytes, &second, reduced, area, [0.0, step])
        });
        let written = swept.and_then(|()| {
            Resolve {
                source: reduced.texture(),
                origin: (self.region.x, self.region.y),
                span: Size::new(self.region.width, self.region.height),
                step: [0.0, 0.0],
                region: self.region,
                dst: self.dst,
                write: self.write,
                blur: self.blur,
            }
            .run(device, passes, bytes, first)
        });
        leased.push(second);
        written
    }
}
