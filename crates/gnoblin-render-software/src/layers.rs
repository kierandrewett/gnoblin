//! Painting one layer: its shadows, its backdrop blur, its own pixels, its rounded corners and its borders.
//!
//! Pixman composites rectangles. Everything softer than a rectangle is computed in memory instead, on a
//! [`Patch`](crate::raster::Patch): a rectangle of the output read back from the buffer, changed by [`raster`] and
//! [`blur`], and written back over the pixels the frame damaged. The order is the GPU renderers' (`gles`'s
//! `draw_parts`):
//!
//! 1. The drop shadows, under everything the layer draws. A shadow takes the layer's opacity, as CSS does, and is not part
//!    of the layer's group: it is faded by the opacity on its own.
//! 2. The backdrop blur, which replaces the pixels behind the layer with their blur and grade.
//! 3. The layer's own pixels, which Pixman blends with the layer's tint and opacity.
//! 4. The layer's corners and, for a layer with a blur, its opacity: the layer is drawn square into the frame and
//!    [`raster::fade`] puts the pixels outside its curve back as they were, with an anti-aliased edge. A layer with a
//!    blur is a group: its blur and its pixels are drawn at full strength and the finished group is faded over what
//!    was behind it, so the blur fades with the layer rather than showing through it.
//! 5. The borders, over the layer's own pixels, faded with the layer.
//!
//! A layer with none of those draws nothing but its own pixels, and costs what it always did.
//!
//! # The filter
//!
//! A `filter` grades and may blur everything the layer paints, and is drawn as the GLES renderer draws it
//! (`gles/effects.rs`, "The filter"):
//!
//! - A grade alone grades each piece as it is drawn: a solid colour (a shadow, a background, a border, a solid layer's
//!   fill) by [`grade::apply`], and a surface's pixels as they are imported (`lib.rs`), on straight colour.
//! - A blur draws the pieces (shadows, background, inset shadows, the layer's own pixels, borders) with this same code
//!   into a transparent picture of their own, which is the layer's painted extent grown by the blur's reach; blurs the
//!   picture with one pair of sweeps; grades it as a whole; and composes it over the frame at the layer's tint and
//!   opacity. The picture is not clipped to the layer's corners, so the blur spills past the edge, and a layer that also
//!   has a backdrop blur draws that glass first, outside the picture. The picture is made whatever the damage; only the
//!   composition is limited to it.

use std::sync::Once;

use smithay::backend::renderer::pixman::{PixmanRenderer, PixmanTarget, PixmanTexture};
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Frame, ImportMem, Renderer, Texture};
use smithay::utils::{Physical, Rectangle, Size as PhysicalSize, Transform};

use gnoblin_render::{BlurParams, Border, Effects, FilterParams, MAX_BLUR_PASSES, Pass, Shadow};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::Layer;
use gnoblin_render_frame::geometry::physical_rect;
use gnoblin_render_frame::grade::{self, Grade};
use gnoblin_render_frame::image::READBACK_FORMAT;
use gnoblin_types::{Color, Scale};

use crate::blur::Blur;
use crate::raster::{self, Area, Cast, MAX_RADIUS, Patch, Shape, Stroke};
use crate::{colour, failed, layer_damage, log, source, tinted};

/// The output buffer and what a layer needs to draw into it.
pub(crate) struct Canvas<'a, 'b> {
    pub(crate) renderer: &'a mut PixmanRenderer,
    pub(crate) target: &'a mut PixmanTarget<'b>,
    pub(crate) output: PhysicalSize<i32, Physical>,
    /// The frame's damage in physical pixels: the only pixels a frame changes.
    pub(crate) damage: &'a [Area],
    pub(crate) scale: Scale,
}

impl Canvas<'_, '_> {
    fn bounds(&self) -> Area {
        Rectangle::from_size(self.output)
    }

    /// The part of `wanted` the frame redraws, as one rectangle: the smallest that holds every damaged pixel of it.
    fn touched(&self, wanted: Area) -> Option<Area> {
        let wanted = wanted.intersection(self.bounds())?;
        self.damage
            .iter()
            .filter_map(|damaged| damaged.intersection(wanted))
            .reduce(Rectangle::merge)
    }

    /// Reads `rect`, which must lie inside the output, back from the buffer.
    fn read(&mut self, rect: Area) -> Result<Patch, RenderError> {
        let region: Rectangle<i32, smithay::utils::Buffer> =
            Rectangle::new((rect.loc.x, rect.loc.y).into(), (rect.size.w, rect.size.h).into());
        let mapping = self
            .renderer
            .copy_framebuffer(self.target, region, READBACK_FORMAT)
            .map_err(failed)?;
        let height = mapping.height() as usize;
        if height == 0 {
            return Err(RenderError::Failed);
        }
        let bytes = self.renderer.map_texture(&mapping).map_err(failed)?;
        // The mapping's stride is pixman's own, so it is measured from the slice rather than assumed.
        let stride = bytes.len() / height;
        let row = rect.size.w as usize * 4;
        let mut packed = Vec::with_capacity(row * height);
        for line in bytes.chunks_exact(stride) {
            packed.extend_from_slice(&line[..row]);
        }
        Ok(Patch { rect, bytes: packed })
    }

    /// Writes the pixels of `patch` that lie in `limit` over the buffer, inside the frame's damage: the pixels outside
    /// it were not redrawn, so they are not touched.
    ///
    /// The damaged pixels are cleared and the patch is blended over them, which is a copy: the patch holds premultiplied
    /// pixels, as the buffer does.
    fn write(&mut self, patch: &Patch, limit: Area) -> Result<(), RenderError> {
        let Some(limit) = patch.rect.intersection(limit) else {
            return Ok(());
        };
        let texture = self
            .renderer
            .import_memory(
                &patch.bytes,
                READBACK_FORMAT,
                PhysicalSize::from((patch.rect.size.w, patch.rect.size.h)),
                false,
            )
            .map_err(failed)?;
        let mut frame = self
            .renderer
            .render(self.target, self.output, Transform::Normal)
            .map_err(failed)?;
        for part in self.damage.iter().filter_map(|damaged| damaged.intersection(limit)) {
            frame
                .clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[part])
                .map_err(failed)?;
            let from = Rectangle::new(
                (
                    f64::from(part.loc.x - patch.rect.loc.x),
                    f64::from(part.loc.y - patch.rect.loc.y),
                )
                    .into(),
                (f64::from(part.size.w), f64::from(part.size.h)).into(),
            );
            frame
                .render_texture_from_to(
                    &texture,
                    from,
                    part,
                    &[Rectangle::from_size(part.size)],
                    &[],
                    Transform::Normal,
                    1.0,
                )
                .map_err(failed)?;
        }
        let _sync = frame.finish().map_err(failed)?;
        Ok(())
    }
}

/// The corner radius of a layer in physical pixels, 0 for square corners. The radius is logical, so it is scaled the
/// way GLES scales it: `corner * scale`.
fn corner_radius(logical: f32, scale: Scale) -> f32 {
    (f64::from(logical.max(0.0)) * scale.as_f64()) as f32
}

/// The four squares at the corners of `outline` that a corner of `radius` can change, each one pixel more than the
/// radius on a side and no more than half the outline, so they never overlap.
fn corner_squares(outline: Area, radius: f32) -> [Area; 4] {
    let (width, height) = (outline.size.w, outline.size.h);
    let reach = radius.min(width as f32 / 2.0).min(height as f32 / 2.0).ceil() as i32 + 1;
    let (left, right) = (reach.min((width + 1) / 2), reach.min(width / 2));
    let (top, bottom) = (reach.min((height + 1) / 2), reach.min(height / 2));
    let (x, y) = (outline.loc.x, outline.loc.y);
    let square = |x, y, w, h| Rectangle::new((x, y).into(), (w, h).into());
    [
        square(x, y, left, top),
        square(x + width - right, y, right, top),
        square(x, y + height - bottom, left, bottom),
        square(x + width - right, y + height - bottom, right, bottom),
    ]
}

/// What `paint` draws a layer from, in physical pixels: the layer's own rectangle, the outline its corners and borders
/// follow, and what its effects, tint and opacity say.
///
/// A filter's picture is drawn by `paint_parts` again, on a canvas of its own, from parts that are the layer's moved
/// into the picture's coordinates, so the pieces of a filtered layer are drawn by the code that draws any layer.
#[derive(Clone, Copy)]
struct Parts<'a> {
    dst: Area,
    outline: Area,
    radius: f32,
    effects: &'a Effects,
    tint: Color,
    opacity: f32,
    /// Whether the layer is on the flat plane: a layer placed in depth is drawn without its effects.
    flat: bool,
}

/// What a layer's `filter` does to how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Treatment {
    /// The grade each piece of the layer is drawn with: its own pixels, its solid colour, its shadows, its background and
    /// its borders. Identity when the filter grades a picture as a whole, and for a layer with no filter.
    pub(crate) grade: Grade,
    /// Whether the filter blurs, which draws the pieces into a picture of their own, blurs that and grades the result.
    pub(crate) picture: bool,
}

impl Treatment {
    const NONE: Self = Self {
        grade: Grade::IDENTITY,
        picture: false,
    };
}

/// The layer's filter: the one `Pass::Filter` of its effects, or the filter that changes nothing.
fn filter_of(effects: &Effects) -> FilterParams {
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
fn grade_of(filter: &FilterParams) -> Grade {
    Grade {
        saturate: filter.saturate,
        brightness: filter.brightness,
        contrast: filter.contrast,
        noise: 0.0,
    }
}

/// How a layer's filter treats its pieces: a grade alone grades each piece as it is drawn, a blur draws a picture.
pub(crate) fn treatment(effects: &Effects, flat: bool) -> Treatment {
    if !flat {
        return Treatment::NONE;
    }
    let filter = filter_of(effects);
    if filter.blur > 0.0 {
        Treatment {
            grade: Grade::IDENTITY,
            picture: true,
        }
    } else {
        Treatment {
            grade: grade_of(&filter),
            picture: false,
        }
    }
}

/// The largest picture a filter's blur draws, on a side: the texture limit the GLES renderer holds a filtered layer to.
const MAX_PICTURE: i32 = 16_384;

/// Draws one layer into the canvas.
///
/// `texture` is the layer's imported pixels, which a solid layer has none of. `clipped` is scratch for the damage a
/// draw is clipped to, reused across layers.
pub(crate) fn paint(
    canvas: &mut Canvas<'_, '_>,
    layer: &Layer,
    texture: Option<&PixmanTexture>,
    clipped: &mut Vec<Area>,
) -> Result<(), RenderError> {
    let scale = canvas.scale;
    let (dst, silhouette, effects, tint, opacity, flat, logical_radius) = match layer {
        Layer::Surface(surface) => (
            physical_rect(surface.dst, scale),
            surface.silhouette,
            &surface.effects,
            surface.tint,
            surface.opacity,
            surface.model.is_none(),
            surface.corner_radius,
        ),
        Layer::Solid(solid) => (
            physical_rect(solid.dst, scale),
            solid.silhouette,
            &solid.effects,
            solid.tint,
            1.0,
            solid.model.is_none(),
            solid.corner_radius,
        ),
        Layer::Overlay(_) => return Ok(()),
    };
    // The rectangle the corner radius rounds, a shadow that asks for the frame follows and a border runs along: the
    // layer's silhouette when it is one part of a larger frame, else its own rectangle.
    let outline = silhouette.map_or(dst, |area| physical_rect(area, scale));
    let parts = Parts {
        dst,
        outline,
        radius: corner_radius(logical_radius, scale).min(MAX_RADIUS),
        effects,
        tint,
        opacity: opacity.clamp(0.0, 1.0),
        flat,
    };
    paint_parts(canvas, &parts, layer, texture, clipped)
}

fn paint_parts(
    canvas: &mut Canvas<'_, '_>,
    parts: &Parts<'_>,
    layer: &Layer,
    texture: Option<&PixmanTexture>,
    clipped: &mut Vec<Area>,
) -> Result<(), RenderError> {
    let Parts {
        dst,
        outline,
        radius,
        effects,
        tint,
        opacity,
        flat,
    } = *parts;
    let shape = Shape::new(outline, radius);

    // A layer placed in depth is drawn flat here, without the effects: they are not drawn on a node in 3D.
    let blurs = |pass: &&Pass| matches!(pass, Pass::Blur(held) if held.radius > 0.0);
    let blurred = flat && effects.passes().iter().any(|pass| blurs(&pass));
    let Treatment { grade, picture } = treatment(effects, flat);
    if flat {
        warn_unsupported(effects);
        // A filter that blurs draws the shadows into its picture, with the rest of the layer.
        if !picture {
            shadows(canvas, effects, (dst, outline), radius, opacity, grade)?;
        }
    }
    // A layer with a backdrop blur is a group: it is drawn at full strength and faded as one.
    let group = blurred && opacity < 1.0;

    // A filter's picture draws the layer's own pixels, so on the frame there is only the glass to draw.
    if (!picture || blurred)
        && let Some(need) = canvas.touched(dst)
    {
        // What the layer's square drawing must be faded back to: the whole of it for a group, else the corners.
        let fixes: Vec<Area> = if group {
            vec![need]
        } else if radius > 0.0 {
            corner_squares(outline, radius)
                .into_iter()
                .filter_map(|square| canvas.touched(square.intersection(dst)?))
                .collect()
        } else {
            Vec::new()
        };
        let mut before = Vec::with_capacity(fixes.len());
        for rect in &fixes {
            before.push(canvas.read(*rect)?);
        }

        if blurred {
            for pass in effects.passes() {
                if let Pass::Blur(held) = pass
                    && held.radius > 0.0
                {
                    blur_backdrop(canvas, held, need, dst)?;
                }
            }
        }

        if !picture {
            // What of the layer's opacity each part of it is drawn with: none of it for a group, which fades as a whole.
            let alpha = if group { 1.0 } else { opacity };
            if flat {
                under_content(canvas, effects, (dst, need), radius, (tint, grade), alpha)?;
            }

            layer_damage(canvas.damage, dst, clipped);
            if !clipped.is_empty() {
                draw_own_pixels(canvas, layer, texture, (dst, clipped), alpha, (tint, grade))?;
            }
        }

        for (rect, before) in fixes.iter().zip(&before) {
            let mut drawn = canvas.read(*rect)?;
            raster::fade(&mut drawn, before, shape, if group { opacity } else { 1.0 });
            canvas.write(&drawn, *rect)?;
        }
    }
    if flat {
        if picture {
            filter_picture(canvas, parts, layer, texture, &filter_of(effects))?;
        } else {
            borders(canvas, effects, outline, radius, (tint, grade), opacity)?;
        }
    }
    Ok(())
}

/// Draws a layer whose filter blurs: its pieces into a picture of their own, the blur over that picture, then the
/// picture over the frame at the layer's tint and opacity, graded as one.
///
/// The picture holds what the layer paints (shadows, background, inset shadows, its own pixels, borders) at full strength
/// and is not clipped to the layer's corners, so the blur spills past the edge. Its extent is what the pieces cover grown
/// by the blur's reach. It does not depend on the frame's damage; only what the composition writes does.
fn filter_picture(
    canvas: &mut Canvas<'_, '_>,
    parts: &Parts<'_>,
    layer: &Layer,
    texture: Option<&PixmanTexture>,
    filter: &FilterParams,
) -> Result<(), RenderError> {
    let scale = canvas.scale;
    let radius = (filter.blur * scale.as_f64() as f32).clamp(0.0, MAX_RADIUS);
    let reach = (radius / 3.0 * 4.0).ceil() as i32 + 1;
    // What the pieces cover: the layer, its outline, every outer shadow and every border.
    let mut painted = parts.dst.merge(parts.outline);
    for pass in parts.effects.passes() {
        match pass {
            Pass::Shadow(held) if !held.inset && held.opacity > 0.0 && held.color.a > 0.0 => {
                let base = if held.frame { parts.outline } else { parts.dst };
                painted = painted.merge(Cast::outer(held, base, parts.radius, 1.0, scale).quad);
            }
            Pass::Border(held) if held.width > 0.0 && held.color.a > 0.0 => {
                painted = painted.merge(Stroke::new(held, parts.outline, parts.radius, Color::WHITE, 1.0, scale).quad);
            }
            _ => {}
        }
    }
    // Only what can reach the output matters: the picture's blur spills `reach` pixels in.
    let Some(painted) = painted.intersection(raster::grown(canvas.bounds(), reach)) else {
        return Ok(());
    };
    let extent = raster::grown(painted, reach);
    let Some(need) = canvas.touched(extent) else {
        return Ok(());
    };
    if extent.size.w > MAX_PICTURE || extent.size.h > MAX_PICTURE {
        log::warn!(
            area = "render",
            "a filtered layer is larger than the picture limit; it is drawn without its filter"
        );
        return Ok(());
    }
    let mut picture = draw_picture(canvas, parts, layer, texture, extent)?;
    Blur::sweep_picture(&mut picture, radius);
    picture.rect = extent;
    let mut frame = canvas.read(need)?;
    raster::compose(&mut frame, &picture, grade_of(filter), parts.tint, parts.opacity);
    canvas.write(&frame, need)
}

/// The pieces of a filtered layer, drawn over a transparent picture of `extent` pixels and read back, with the layer's
/// own tint, opacity and grade left to the composition.
fn draw_picture(
    canvas: &mut Canvas<'_, '_>,
    parts: &Parts<'_>,
    layer: &Layer,
    texture: Option<&PixmanTexture>,
    extent: Area,
) -> Result<Patch, RenderError> {
    let size = gnoblin_types::Size::new(extent.size.w as u32, extent.size.h as u32);
    let mut buffer = crate::create_buffer(canvas.renderer, size)?;
    let mut target = canvas.renderer.bind(&mut buffer).map_err(failed)?;
    let whole: Area = Rectangle::from_size(extent.size);
    {
        let mut frame = canvas
            .renderer
            .render(&mut target, extent.size, Transform::Normal)
            .map_err(failed)?;
        frame
            .clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[whole])
            .map_err(failed)?;
        let _sync = frame.finish().map_err(failed)?;
    }
    let moved = |rect: Area| Rectangle::new((rect.loc.x - extent.loc.x, rect.loc.y - extent.loc.y).into(), rect.size);
    // The pieces the picture holds: what the layer paints. The filter itself and the glass are not part of it.
    let mut effects = Effects::new();
    for pass in parts.effects.passes() {
        if matches!(pass, Pass::Shadow(_) | Pass::Background { .. } | Pass::Border(_)) {
            effects.push(pass.clone()).map_err(failed)?;
        }
    }
    let inner = Parts {
        dst: moved(parts.dst),
        outline: moved(parts.outline),
        radius: parts.radius,
        effects: &effects,
        tint: Color::WHITE,
        opacity: 1.0,
        flat: true,
    };
    let damage = [whole];
    let mut sub = Canvas {
        renderer: &mut *canvas.renderer,
        target: &mut target,
        output: extent.size,
        damage: &damage,
        scale: canvas.scale,
    };
    paint_parts(&mut sub, &inner, layer, texture, &mut Vec::new())?;
    sub.read(whole)
}

/// Draws the layer's own pixels with Pixman: a solid's colour or a surface's texture.
///
/// The layer is drawn square, whatever its corner radius: the corners are put back afterwards. `place` is the layer's
/// rectangle and the part of the frame's damage inside it. `opacity` is the alpha it is drawn with, which is 1 for a layer
/// whose group is faded as a whole. A solid is graded by `grade` and multiplied by `tint`; a surface's texture is imported
/// already graded and tinted, so only the tint's alpha is applied here.
fn draw_own_pixels(
    canvas: &mut Canvas<'_, '_>,
    layer: &Layer,
    texture: Option<&PixmanTexture>,
    place: (Area, &[Area]),
    opacity: f32,
    look: (Color, Grade),
) -> Result<(), RenderError> {
    let (dst, clipped) = place;
    let (tint, grade) = look;
    let mut frame = canvas
        .renderer
        .render(canvas.target, canvas.output, Transform::Normal)
        .map_err(failed)?;
    match layer {
        Layer::Solid(solid) => {
            let fill = tinted(grade::apply(grade, solid.colour), tint);
            // A layer that is only its effects (a border's ring) has nothing of its own to fill.
            if fill.a > 0.0 {
                frame.draw_solid(dst, clipped, colour(fill)).map_err(failed)?;
            }
        }
        Layer::Surface(surface) => {
            if let Some(texture) = texture {
                let alpha = opacity * tint.a.clamp(0.0, 1.0);
                frame
                    .render_texture_from_to(texture, source(surface), dst, clipped, &[], surface.transform, alpha)
                    .map_err(failed)?;
            }
        }
        Layer::Overlay(_) => {}
    }
    let _sync = frame.finish().map_err(failed)?;
    Ok(())
}

/// Draws what lies between the backdrop and the layer's own pixels: its `background`, then its inset shadows, in one read
/// of the output and one write.
///
/// `place` is the layer's rectangle and the part of it the frame redraws. The fill is square and the layer's corners are
/// put back afterwards, so a rounded layer's background has the same anti-aliased corner as its pixels. `look` is the
/// layer's tint and the grade each colour is drawn with, and both multiply an inset shadow's colour as they do on the
/// GPU, whose inset pass takes no tint of its own.
fn under_content(
    canvas: &mut Canvas<'_, '_>,
    effects: &Effects,
    place: (Area, Area),
    radius: f32,
    look: (Color, Grade),
    alpha: f32,
) -> Result<(), RenderError> {
    let (dst, need) = place;
    let (tint, grade) = look;
    let passes = effects.passes();
    let drawn = |pass: &&Pass| match pass {
        Pass::Background { color } => color.a > 0.0 && tint.a > 0.0,
        Pass::Shadow(held) => held.inset && held.opacity > 0.0 && held.color.a > 0.0,
        _ => false,
    };
    if !passes.iter().any(|pass| drawn(&pass)) {
        return Ok(());
    }
    let mut patch = canvas.read(need)?;
    for pass in passes.iter().filter(drawn) {
        match pass {
            Pass::Background { color } => {
                let color = grade::apply(grade, *color);
                let colour = Color::new(color.r * tint.r, color.g * tint.g, color.b * tint.b, color.a * tint.a);
                raster::fill(&mut patch, dst, colour, colour.a * alpha);
            }
            Pass::Shadow(held) => {
                let color = grade::apply(grade, held.color);
                let held = Shadow {
                    color: Color::new(color.r * tint.r, color.g * tint.g, color.b * tint.b, color.a * tint.a),
                    ..*held
                };
                Cast::inset(&held, dst, radius, alpha, canvas.scale).draw(&mut patch);
            }
            _ => {}
        }
    }
    canvas.write(&patch, need)
}

/// Draws the layer's drop shadows behind it, in one read of the output and one write. `place` is the layer's rectangle and
/// the rectangle a shadow that asks for the frame follows; `opacity` is the layer's, which each shadow takes; `grade` is
/// the filter's, which each shadow's colour is drawn with.
fn shadows(
    canvas: &mut Canvas<'_, '_>,
    effects: &Effects,
    place: (Area, Area),
    radius: f32,
    opacity: f32,
    grade: Grade,
) -> Result<(), RenderError> {
    let (dst, outline) = place;
    let casts: Vec<Cast> = effects
        .passes()
        .iter()
        .filter_map(|pass| match pass {
            Pass::Shadow(held) if !held.inset && held.opacity > 0.0 && held.color.a > 0.0 => {
                let graded = Shadow {
                    color: grade::apply(grade, held.color),
                    ..*held
                };
                Some(Cast::outer(
                    &graded,
                    if held.frame { outline } else { dst },
                    radius,
                    opacity,
                    canvas.scale,
                ))
            }
            _ => None,
        })
        .collect();
    let Some(quad) = casts.iter().map(|cast| cast.quad).reduce(Rectangle::merge) else {
        return Ok(());
    };
    let Some(area) = canvas.touched(quad) else {
        return Ok(());
    };
    let mut patch = canvas.read(area)?;
    for cast in &casts {
        cast.draw(&mut patch);
    }
    canvas.write(&patch, area)
}

/// Replaces the pixels behind the layer with their blur: the sweeps, then the grade, the tint and the grain.
///
/// At full resolution the blur reads the layer's damaged part grown by the reach of the kernel, so the pixels at the
/// layer's edge are blurred with what lies beyond it, and writes only the layer's own damaged pixels. A blur with a
/// `downscale` is a blur of the whole region the GLES renderer copies (the layer grown by every sweep's reach, clipped
/// to the output), reduced and swept as that picture, and so reads all of it whatever the damage.
fn blur_backdrop(canvas: &mut Canvas<'_, '_>, held: &BlurParams, need: Area, dst: Area) -> Result<(), RenderError> {
    let radius = (held.radius * canvas.scale.as_f64() as f32).clamp(1.0, MAX_RADIUS);
    let mut blur = Blur::new(
        radius,
        held.passes.clamp(1, MAX_BLUR_PASSES),
        held.downscale,
        Grade {
            saturate: held.saturation,
            brightness: held.brightness,
            contrast: held.contrast,
            noise: held.noise,
        },
        held.tint,
    );
    let bounds = canvas.bounds();
    let whole = dst
        .intersection(bounds)
        .and_then(|rect| blur.region(rect, bounds))
        .filter(|region| blur.reduces((region.size.w, region.size.h)));
    let region = match whole {
        Some(region) => region,
        None => {
            blur = blur.at_full_resolution();
            let Some(region) = raster::grown(need, blur.margin()).intersection(bounds) else {
                return Ok(());
            };
            region
        }
    };
    let mut patch = canvas.read(region)?;
    blur.run(&mut patch, need, (dst.loc.x, dst.loc.y));
    canvas.write(&patch, need)
}

/// Draws the layer's borders over its pixels, a strip of the output at a time. `look` is the layer's tint and the grade
/// each border's colour is drawn with.
fn borders(
    canvas: &mut Canvas<'_, '_>,
    effects: &Effects,
    outline: Area,
    radius: f32,
    look: (Color, Grade),
    opacity: f32,
) -> Result<(), RenderError> {
    let (tint, grade) = look;
    let strokes: Vec<Stroke> = effects
        .passes()
        .iter()
        .filter_map(|pass| match pass {
            Pass::Border(held) if held.width > 0.0 && held.color.a > 0.0 => {
                let graded = Border {
                    color: grade::apply(grade, held.color),
                    ..*held
                };
                Some(Stroke::new(&graded, outline, radius, tint, opacity, canvas.scale))
            }
            _ => None,
        })
        .collect();
    let Some(quad) = strokes.iter().map(|stroke| stroke.quad).reduce(Rectangle::merge) else {
        return Ok(());
    };
    let reach = strokes.iter().map(Stroke::reach).max().unwrap_or(0);
    // A ring is thin: only the strips along the outline are read, drawn and written, not the window's whole area.
    for strip in raster::ring(quad, outline, reach) {
        let Some(area) = canvas.touched(strip) else {
            continue;
        };
        let mut patch = canvas.read(area)?;
        for stroke in &strokes {
            stroke.draw(&mut patch);
        }
        canvas.write(&patch, area)?;
    }
    Ok(())
}

/// Says, once, that a shader the shell wrote is not drawn.
///
/// A shader is WGSL, and drawing it needs the compiler and the GPU the GLES and Vulkan renderers have. The software
/// renderer has no WGSL interpreter, so a layer with a shader is drawn without it, and the node's `skipped` list names
/// `shader: unsupported-by-renderer`.
fn warn_unsupported(effects: &Effects) {
    static SHADER_REFUSED: Once = Once::new();
    if effects.passes().iter().any(|pass| matches!(pass, Pass::Shader(_))) {
        SHADER_REFUSED.call_once(|| {
            log::warn!(
                area = "render",
                "unsupported-effect: the software renderer has no WGSL interpreter, so a user shader is not drawn; \
                 the GLES and Vulkan renderers draw it"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Software;
    use gnoblin_render::{Border, Effects, Shadow};
    use gnoblin_render_frame::frame::{Model, OutputFrame, Solid, Surface, SurfaceSource};
    use gnoblin_render_frame::image::Image;
    use gnoblin_types::{Color, Rect, Size, Vec3};
    use std::sync::Arc;

    const BLUE: Color = Color::new(0.0, 0.0, 1.0, 1.0);
    const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);
    const GREEN: Color = Color::new(0.0, 1.0, 0.0, 1.0);
    const SIZE: Size = Size::new(96, 96);

    fn frame_of(background: Color, layers: Vec<Solid>) -> OutputFrame {
        damaged(background, layers, Rect::new(0, 0, SIZE.width, SIZE.height))
    }

    fn damaged(background: Color, layers: Vec<Solid>, damage: Rect) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.set_background(background);
        frame.push_damage(damage);
        for layer in layers {
            frame.push_layer(Layer::Solid(layer));
        }
        frame
    }

    fn picture(background: Color, layers: Vec<Solid>) -> Image {
        let mut software = Software::new(SIZE).expect("a software renderer");
        software
            .draw(&frame_of(background, layers), Scale::IDENTITY)
            .expect("drawn");
        software.screenshot().expect("a readback")
    }

    fn chain(passes: Vec<Pass>) -> Effects {
        let mut effects = Effects::new();
        for pass in passes {
            effects.push(pass).expect("a pass in range");
        }
        effects
    }

    fn stroke(width: f32, offset: f32, inset: bool, color: Color) -> Pass {
        Pass::Border(Border {
            width,
            offset,
            inset,
            color,
        })
    }

    /// The one place these tests build a shadow, so a field the pass gains is added once.
    fn soft_shadow(frame: bool) -> Pass {
        Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 10.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame,
            inset: false,
        })
    }

    fn glass(rect: Rect, blur: BlurParams) -> Solid {
        let mut glass = Solid::new(rect, Color::TRANSPARENT);
        glass.effects = chain(vec![Pass::Blur(blur)]);
        glass
    }

    fn luma(pixel: [u8; 4]) -> u32 {
        u32::from(pixel[0]) * 3 + u32::from(pixel[1]) * 6 + u32::from(pixel[2])
    }

    #[test]
    fn a_silhouette_cuts_each_layer_from_the_frames_corners() {
        let frame = Rect::new(8, 8, 48, 48);
        let layer = |rect: Rect, colour: Color| {
            let mut solid = Solid::new(rect, colour);
            solid.corner_radius = 10.0;
            solid.silhouette = Some(frame);
            solid
        };
        let image = picture(
            BLUE,
            vec![
                layer(Rect::new(8, 8, 48, 12), RED),
                layer(Rect::new(8, 20, 48, 36), GREEN),
            ],
        );
        assert_eq!(
            image.colour_at(9, 9),
            Some([0, 0, 255, 255]),
            "the frame's corner is cut"
        );
        assert_eq!(
            image.colour_at(9, 54),
            Some([0, 0, 255, 255]),
            "and the content's bottom corner"
        );
        assert_eq!(
            image.colour_at(9, 21),
            Some([0, 255, 0, 255]),
            "the join under the bar is not a corner of the frame"
        );
        assert_eq!(image.colour_at(32, 14), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(32, 40), Some([0, 255, 0, 255]));
    }

    #[test]
    fn a_rounded_corner_is_anti_aliased() {
        let mut solid = Solid::new(Rect::new(8, 8, 48, 48), RED);
        solid.corner_radius = 12.0;
        let image = picture(BLUE, vec![solid]);
        // Walking along the corner's diagonal the colour passes through a mix of the layer and the backdrop.
        let mixed = (8..20)
            .filter_map(|i| image.colour_at(i, i))
            .filter(|pixel| pixel[0] > 0 && pixel[0] < 255)
            .count();
        assert!(
            mixed >= 1,
            "a corner pixel is part layer, part background, not one or the other"
        );
        assert_eq!(
            image.colour_at(8, 8),
            Some([0, 0, 255, 255]),
            "the very corner is the backdrop"
        );
    }

    #[test]
    fn borders_are_drawn_as_rings_and_stack_in_list_order() {
        let mut layer = Solid::new(Rect::new(8, 8, 48, 48), Color::TRANSPARENT);
        layer.effects = chain(vec![stroke(2.0, 0.0, true, RED), stroke(2.0, 2.0, true, GREEN)]);
        let image = picture(BLUE, vec![layer]);
        assert_eq!(image.colour_at(8, 30), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(9, 30), Some([255, 0, 0, 255]));
        assert_eq!(
            image.colour_at(10, 30),
            Some([0, 255, 0, 255]),
            "the second ring sits inside the first"
        );
        assert_eq!(image.colour_at(11, 30), Some([0, 255, 0, 255]));
        assert_eq!(image.colour_at(12, 30), Some([0, 0, 255, 255]));
        assert_eq!(
            image.colour_at(7, 30),
            Some([0, 0, 255, 255]),
            "an inside border stays inside"
        );

        let mut outside = Solid::new(Rect::new(8, 8, 48, 48), Color::TRANSPARENT);
        outside.effects = chain(vec![stroke(2.0, 1.0, false, RED)]);
        let image = picture(Color::WHITE, vec![outside]);
        assert_eq!(image.colour_at(5, 30), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(6, 30), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(7, 30), Some([255, 255, 255, 255]));
        assert_eq!(image.colour_at(4, 30), Some([255, 255, 255, 255]));
    }

    #[test]
    fn a_border_follows_the_rounded_corner() {
        let mut fill = Solid::new(Rect::new(8, 8, 48, 48), GREEN);
        fill.corner_radius = 12.0;
        fill.effects = chain(vec![stroke(2.0, 0.0, true, RED)]);
        let image = picture(BLUE, vec![fill]);
        assert_eq!(image.colour_at(9, 9), Some([0, 0, 255, 255]), "the corner is cut");
        assert_eq!(image.colour_at(8, 32), Some([255, 0, 0, 255]), "the edge is the border");
        assert_eq!(
            image.colour_at(32, 32),
            Some([0, 255, 0, 255]),
            "the middle is the fill"
        );
        assert_eq!(
            image.colour_at(12, 12),
            Some([255, 0, 0, 255]),
            "the border bends round the corner"
        );
        assert_eq!(image.colour_at(14, 14), Some([0, 255, 0, 255]));
    }

    #[test]
    fn a_shadow_is_darkest_at_the_edge_and_never_darker_further_out() {
        let mut solid = Solid::new(Rect::new(32, 32, 32, 32), RED);
        solid.corner_radius = 8.0;
        solid.effects = chain(vec![soft_shadow(false)]);
        let image = picture(Color::WHITE, vec![solid]);
        let mut previous = 0;
        for x in (12..32).rev() {
            let brightness = luma(image.colour_at(x, 48).expect("a pixel"));
            assert!(
                brightness >= previous,
                "x = {x} is {brightness}, the pixel nearer was {previous}"
            );
            previous = brightness;
        }
        assert!(
            luma(image.colour_at(31, 48).expect("a pixel")) < luma(image.colour_at(22, 48).expect("a pixel")),
            "darker at the edge than ten pixels out"
        );
        assert_eq!(
            image.colour_at(8, 48),
            Some([255, 255, 255, 255]),
            "past the blur nothing is drawn"
        );
    }

    #[test]
    fn a_shadows_falloff_is_smooth() {
        let mut solid = Solid::new(Rect::new(48, 16, 32, 64), RED);
        solid.effects = chain(vec![Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 24.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: false,
        })]);
        let image = picture(Color::WHITE, vec![solid]);
        let levels: std::collections::BTreeSet<u8> = (24..48)
            .filter_map(|x| image.colour_at(x, 48))
            .map(|pixel| pixel[0])
            .collect();
        assert!(
            levels.len() >= 18,
            "24 pixels of blur are many levels, not a handful of steps: {levels:?}"
        );
    }

    /// A shadow's darkness at the pixel beside a 32 pixel layer drawn at `opacity`, as a count of levels below white.
    fn shadow_levels(opacity: f32, blurred: bool) -> u8 {
        let mut effects = vec![soft_shadow(false)];
        if blurred {
            effects.push(Pass::Blur(BlurParams::default()));
        }
        let surface = Surface {
            source: SurfaceSource::Snapshot(Arc::new(
                Image::new(Size::new(32, 32), 128, [200, 100, 50, 255].repeat(32 * 32)).expect("a packed image"),
            )),
            src: gnoblin_types::RectF {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            dst: Rect::new(32, 32, 32, 32),
            transform: Transform::Normal,
            opacity,
            tint: Color::WHITE,
            corner_radius: 0.0,
            silhouette: None,
            effects: chain(effects),
            model: None,
            secure: false,
            content: None,
        };
        let mut frame = frame_of(Color::WHITE, Vec::new());
        frame.push_layer(Layer::Surface(surface));
        let mut software = Software::new(SIZE).expect("a software renderer");
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        255 - software
            .screenshot()
            .expect("a readback")
            .colour_at(30, 48)
            .expect("a pixel")[0]
    }

    /// An outer shadow takes the layer's opacity, as CSS does, on the plain path and on the backdrop blur's.
    #[test]
    fn a_shadow_takes_the_layers_opacity() {
        for blurred in [false, true] {
            let (whole, half) = (shadow_levels(1.0, blurred), shadow_levels(0.5, blurred));
            assert!(
                whole > 20,
                "the shadow is drawn at full opacity (blurred: {blurred}): {whole}"
            );
            assert!(
                (i32::from(whole) - 2 * i32::from(half)).abs() <= 2,
                "half the opacity casts half the shadow (blurred: {blurred}): {whole} against {half}"
            );
        }
    }

    #[test]
    fn a_frame_shadow_covers_the_room_above_the_content() {
        let frame = Rect::new(16, 16, 64, 64);
        let body = |follows_frame: bool| {
            let mut solid = Solid::new(Rect::new(16, 40, 64, 40), RED);
            solid.silhouette = Some(frame);
            solid.effects = chain(vec![soft_shadow(follows_frame)]);
            solid
        };
        let with_frame = picture(Color::WHITE, vec![body(true)]);
        let content_only = picture(Color::WHITE, vec![body(false)]);
        assert!(
            luma(with_frame.colour_at(48, 28).expect("a pixel")) < 200,
            "the room above the content is shadowed"
        );
        assert_eq!(content_only.colour_at(48, 28), Some([255, 255, 255, 255]));
    }

    #[test]
    fn a_layer_placed_in_depth_is_drawn_without_its_border_and_shadow() {
        let mut solid = Solid::new(Rect::new(32, 32, 32, 32), RED);
        solid.model = Some(Model {
            matrix: gnoblin_render::Mat4::IDENTITY,
            opaque: true,
        });
        solid.effects = chain(vec![soft_shadow(false), stroke(2.0, 1.0, false, GREEN)]);
        let image = picture(Color::WHITE, vec![solid]);
        assert_eq!(
            image.colour_at(30, 48),
            Some([255, 255, 255, 255]),
            "no shadow beside it"
        );
        assert_eq!(
            image.colour_at(29, 48),
            Some([255, 255, 255, 255]),
            "no border beside it"
        );
    }

    /// A pale half and a dark half, so a blur across the edge has something to soften.
    fn split() -> Vec<Solid> {
        vec![
            Solid::new(Rect::new(0, 0, 48, 96), Color::from_rgba8(20, 20, 20, 255)),
            Solid::new(Rect::new(48, 0, 48, 96), Color::from_rgba8(230, 230, 230, 255)),
        ]
    }

    #[test]
    fn a_backdrop_blur_softens_what_is_behind_the_layer_and_only_there() {
        let mut layers = split();
        layers.push(glass(Rect::new(16, 16, 64, 64), BlurParams::default()));
        let image = picture(Color::BLACK, layers);
        let across: Vec<u8> = (40..56).map(|x| image.colour_at(x, 48).expect("a pixel")[0]).collect();
        assert!(
            across.windows(2).all(|pair| pair[0] <= pair[1]),
            "the edge is a smooth ramp: {across:?}"
        );
        assert!(
            across[7] > 40 && across[8] < 210,
            "and it is soft where it was a step: {across:?}"
        );
        assert_eq!(
            image.colour_at(8, 48),
            Some([20, 20, 20, 255]),
            "outside the layer the backdrop is as it was"
        );
        assert_eq!(image.colour_at(90, 48), Some([230, 230, 230, 255]), "on both sides");
    }

    #[test]
    fn a_blurs_grade_and_tint_are_applied() {
        let mut layers = vec![Solid::new(
            Rect::new(0, 0, 96, 96),
            Color::from_rgba8(200, 100, 50, 255),
        )];
        layers.push(glass(
            Rect::new(16, 16, 64, 64),
            BlurParams {
                radius: 4.0,
                passes: 1,
                brightness: 0.5,
                ..BlurParams::default()
            },
        ));
        let image = picture(Color::BLACK, layers);
        assert_eq!(image.colour_at(48, 48), Some([100, 50, 25, 255]), "half as bright");
        assert_eq!(image.colour_at(4, 4), Some([200, 100, 50, 255]), "outside the glass");

        let mut saturated = vec![Solid::new(
            Rect::new(0, 0, 96, 96),
            Color::from_rgba8(200, 100, 50, 255),
        )];
        saturated.push(glass(
            Rect::new(16, 16, 64, 64),
            BlurParams {
                radius: 4.0,
                passes: 1,
                saturation: 0.0,
                ..BlurParams::default()
            },
        ));
        let grey = picture(Color::BLACK, saturated).colour_at(48, 48).expect("a pixel");
        assert!(
            grey[0] == grey[1] && grey[1] == grey[2],
            "no saturation is grey: {grey:?}"
        );
    }

    #[test]
    fn a_blurs_corners_follow_the_radius() {
        let mut layers = split();
        let mut round = glass(Rect::new(16, 16, 64, 64), BlurParams::default());
        round.corner_radius = 16.0;
        layers.push(round);
        let image = picture(Color::BLACK, layers);
        assert_eq!(
            image.colour_at(16, 16),
            Some([20, 20, 20, 255]),
            "the square corner of the glass shows the unblurred backdrop"
        );
        assert_ne!(image.colour_at(48, 48), Some([20, 20, 20, 255]));
    }

    /// A window over glass fades as one group: at opacity 0.5 its opaque pixels are half the window and half what was
    /// behind it, and the blur does not show through them.
    #[test]
    fn a_translucent_layer_with_a_blur_fades_as_a_group() {
        let behind = Color::from_rgba8(240, 240, 240, 255);
        let mut frame = frame_of(Color::BLACK, vec![Solid::new(Rect::new(0, 0, 96, 96), behind)]);
        // A surface carries the opacity; a solid does not, so the group is a snapshot layer.
        let image =
            Arc::new(Image::new(Size::new(64, 64), 256, [0, 0, 200, 255].repeat(64 * 64)).expect("a packed image"));
        let surface = Surface {
            source: SurfaceSource::Snapshot(image),
            src: gnoblin_types::RectF {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            dst: Rect::new(16, 16, 64, 64),
            transform: Transform::Normal,
            opacity: 0.5,
            tint: Color::WHITE,
            corner_radius: 0.0,
            silhouette: None,
            effects: chain(vec![Pass::Blur(BlurParams::default())]),
            model: None,
            secure: false,
            content: None,
        };
        frame.push_layer(Layer::Surface(surface));
        let mut software = Software::new(SIZE).expect("a software renderer");
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        let drawn = software.screenshot().expect("a readback");
        // Half of (0, 0, 200) and half of the flat 240 grey behind it: the blur of a flat colour is itself.
        assert_eq!(drawn.colour_at(48, 48), Some([120, 120, 220, 255]));
        assert_eq!(drawn.colour_at(4, 4), Some([240, 240, 240, 255]));
    }

    /// A frame redrawn over a small damage is the frame drawn whole: the pixels the damage does not cover are not
    /// touched, and the ones it covers are not drawn on twice. (A backdrop blur reads what is already in the buffer around
    /// its damage, so the compositor damages the whole of a blurred layer, as it does for the GLES renderer.)
    #[test]
    fn a_redraw_of_a_small_damage_leaves_the_picture_as_it_was() {
        let mut layers = split();
        let mut window = Solid::new(Rect::new(24, 24, 48, 48), RED);
        window.corner_radius = 10.0;
        window.effects = chain(vec![soft_shadow(false), stroke(2.0, 0.0, true, GREEN)]);
        layers.push(window);
        let mut software = Software::new(SIZE).expect("a software renderer");
        let whole = frame_of(Color::BLACK, layers.clone());
        software.draw(&whole, Scale::IDENTITY).expect("drawn");
        let first = software.screenshot().expect("a readback");

        let again = damaged(Color::BLACK, layers, Rect::new(30, 30, 40, 40));
        software.draw(&again, Scale::IDENTITY).expect("drawn");
        let second = software.screenshot().expect("a readback");
        assert_eq!(first, second, "a repaint of part of the frame gives the same picture");
    }

    #[test]
    fn a_background_fills_under_the_content_and_over_a_blur() {
        let mut layers = split();
        let mut glass = glass(Rect::new(16, 16, 64, 64), BlurParams::default());
        glass.effects = chain(vec![
            Pass::Blur(BlurParams::default()),
            Pass::Background {
                color: Color::from_rgba8(255, 0, 0, 128),
            },
        ]);
        layers.push(glass);
        let image = picture(Color::BLACK, layers);
        let inside = image.colour_at(48, 48).expect("a pixel");
        assert!(
            inside[0] > 120 && inside[1] < 120,
            "a translucent red tints the blurred backdrop: {inside:?}"
        );
        assert_eq!(
            image.colour_at(8, 48),
            Some([20, 20, 20, 255]),
            "outside the layer nothing changes"
        );
    }

    #[test]
    fn a_background_follows_the_rounded_corner_and_is_under_the_content() {
        let mut fill = Solid::new(Rect::new(8, 8, 48, 48), Color::from_rgba8(0, 255, 0, 128));
        fill.corner_radius = 12.0;
        fill.effects = chain(vec![Pass::Background { color: Color::WHITE }]);
        let image = picture(BLUE, vec![fill]);
        assert_eq!(image.colour_at(9, 9), Some([0, 0, 255, 255]), "the corner is cut");
        let middle = image.colour_at(32, 32).expect("a pixel");
        // White, then half green over it: the content is over the background.
        assert!(
            middle[0].abs_diff(128) <= 1 && middle[1] == 255 && middle[2].abs_diff(128) <= 1,
            "white under half green: {middle:?}"
        );
    }

    #[test]
    fn an_inset_shadow_darkens_the_edge_inside_the_layer_only() {
        let mut solid = Solid::new(Rect::new(16, 16, 48, 48), Color::from_rgba8(255, 255, 255, 0));
        let mut inset = soft_shadow(false);
        if let Pass::Shadow(held) = &mut inset {
            held.inset = true;
        }
        solid.effects = chain(vec![inset]);
        let image = picture(Color::WHITE, vec![solid]);
        assert!(
            luma(image.colour_at(16, 40).expect("a pixel")) < 400,
            "the edge inside is shadowed"
        );
        assert_eq!(image.colour_at(40, 40), Some([255, 255, 255, 255]), "the middle is not");
        assert_eq!(
            image.colour_at(14, 40),
            Some([255, 255, 255, 255]),
            "and outside is not"
        );
    }

    #[test]
    fn a_filters_grade_is_drawn_on_the_layer() {
        use gnoblin_render::FilterParams;
        let mut solid = Solid::new(Rect::new(16, 16, 32, 32), RED);
        solid.effects = chain(vec![Pass::Filter(FilterParams {
            saturate: 0.0,
            ..FilterParams::default()
        })]);
        let image = picture(Color::WHITE, vec![solid]);
        assert_eq!(
            image.colour_at(24, 24),
            Some([54, 54, 54, 255]),
            "red with no saturation is its luma grey"
        );
    }

    #[test]
    fn a_shader_is_not_drawn_and_the_layer_still_is() {
        use gnoblin_render::ShaderPass;
        let mut solid = Solid::new(Rect::new(16, 16, 32, 32), RED);
        solid.effects = chain(vec![Pass::Shader(ShaderPass::new("fn effect() {}", &[], false))]);
        let image = picture(Color::WHITE, vec![solid]);
        assert_eq!(
            image.colour_at(24, 24),
            Some([255, 0, 0, 255]),
            "the layer is drawn as if it had no shader"
        );
    }

    #[test]
    fn the_corner_squares_never_overlap_and_stay_inside_the_outline() {
        for (w, h, radius) in [(48, 48, 12.0), (5, 5, 100.0), (1, 9, 4.0), (16, 16, 8.0), (7, 40, 20.0)] {
            let outline = Rectangle::<i32, Physical>::new((10, 20).into(), (w, h).into());
            let squares = corner_squares(outline, radius);
            for (index, square) in squares.iter().enumerate() {
                assert!(outline.contains_rect(*square), "{square:?} is inside {outline:?}");
                for other in &squares[index + 1..] {
                    assert!(
                        square.intersection(*other).is_none_or(|overlap| overlap.is_empty()),
                        "{square:?} and {other:?} overlap in a {w}x{h} outline"
                    );
                }
            }
        }
    }
}
