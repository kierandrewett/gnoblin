//! A frame turned into the list of draws the Vulkan renderer records.
//!
//! Planning is pure: it reads an [`OutputFrame`] and the sizes of the textures that could be
//! created, and produces the draws, their uniform blocks and their per-quad vertex data. It
//! calls no driver, so everything here is tested without a device.
//!
//! # What a plan decides
//!
//! - **Damage** is clipped to the output and made disjoint. A frame's damage rectangles may
//!   overlap, and drawing a translucent layer once per rectangle would blend it twice where
//!   they do. Pixman clips to the union and GLES repeats the background under every
//!   rectangle; disjoint rectangles give the union's result without a second code path.
//! - **Order** is the frame's: for every damage rectangle, the layers bottom first. The
//!   overlay and the cursor follow, each as its own range of the draw list, because the
//!   renderer saves the pixels under each before it draws it (see `Vulkan::capture`).
//! - **What is skipped**: a layer with a [`Model`](gnoblin_render_frame::frame::Model) belongs to
//!   the depth pass, which this renderer does not have, so it is counted and left out; a
//!   surface whose texture could not be made is left out; a layer that misses the rectangle
//!   is not drawn into it.
//!
//! # The uniform block and the quad
//!
//! Each draw's 96-byte block is packed by the same code the GLES renderer uses
//! ([`UniformLayout`]), so the two cannot disagree about an offset. The block sits at a
//! 256-byte stride, which is the largest alignment Vulkan allows a dynamic uniform offset
//! to demand. The shared vertex stage takes one instance of 22 floats per quad: the
//! rectangle and the viewport, three columns of an optional projection and its parameters.
//! A flat quad leaves the projection empty and its flag clear.

use gnoblin_types::{Color, Rect, RectF, Scale, Size};
use smithay::utils::{Physical, Rectangle};

use super::pipeline::{INSTANCE_FLOATS, UNIFORM_BYTES};
use gnoblin_render_frame::frame::{Layer, OutputFrame, Surface};
use gnoblin_render_frame::geometry::physical_rect;
use gnoblin_render_frame::shader::uniforms::{UniformLayout, Uniforms};

/// The distance between two draws' uniform blocks, in bytes.
pub(super) const UNIFORM_STRIDE: usize = 256;

/// Which pass a draw uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PassKind {
    /// A rectangle of one colour.
    Solid,
    /// A texture.
    Surface,
}

/// Where a draw's texture comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextureSlot {
    /// The client buffer of the layer at this index of the frame.
    Layer(usize),
    /// The portal overlay.
    Overlay,
    /// The cursor image.
    Cursor,
}

impl TextureSlot {
    /// Whether the texture is sampled with the nearest texel.
    ///
    /// The overlay and the cursor are bitmaps and scale without smoothing, as they do in the
    /// other renderers; a client's surface is filtered.
    pub(super) fn nearest(self) -> bool {
        !matches!(self, Self::Layer(_))
    }
}

/// One quad to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedDraw {
    /// Which fragment stage the draw runs.
    pub kind: PassKind,
    /// The texture the draw samples, or `None` for a pass that samples none.
    pub slot: Option<TextureSlot>,
    /// The damage rectangle the draw is clipped to, in physical pixels.
    pub scissor: Rect,
    /// The draw's block, as an offset into [`FramePlan::uniforms`].
    pub uniform_offset: u32,
    /// The draw's quad, as an index into [`FramePlan::instances`].
    pub instance: u32,
}

/// The sizes of the textures that exist for a frame, by slot.
#[derive(Clone, Debug, Default)]
pub(super) struct TextureSizes {
    /// Dimensions in pixels of each layer's texture, `None` for a solid or a surface with no texture.
    pub layers: Vec<Option<(u32, u32)>>,
    /// Dimensions in pixels of the overlay texture, if one exists this frame.
    pub overlay: Option<(u32, u32)>,
    /// Dimensions in pixels of the cursor texture, if one exists this frame.
    pub cursor: Option<(u32, u32)>,
}

/// Everything the renderer records for a frame.
#[derive(Clone, Debug, Default)]
pub(super) struct FramePlan {
    /// The damage in physical pixels: clipped to the output and disjoint.
    pub damage: Vec<Rect>,
    /// Every draw, layers first, then the overlay, then the cursor.
    pub draws: Vec<PlannedDraw>,
    /// The draws before this index are layers.
    pub layers_end: usize,
    /// The draws from `layers_end` to this index are the overlay, the rest the cursor.
    pub overlay_end: usize,
    /// The overlay's rectangle on the output, when it is drawn at all.
    pub overlay_visible: Option<Rect>,
    /// The cursor's rectangle on the output, when it is drawn at all.
    pub cursor_visible: Option<Rect>,
    /// The parts of the overlay's rectangle this frame repaints, to be saved first.
    pub overlay_under: Vec<Rect>,
    /// The parts of the cursor's rectangle this frame repaints, to be saved first.
    pub cursor_under: Vec<Rect>,
    /// The draws' uniform blocks, [`UNIFORM_STRIDE`] bytes apart.
    pub uniforms: Vec<u8>,
    /// The quads, [`INSTANCE_FLOATS`] floats each.
    pub instances: Vec<f32>,
    /// Layers left out because they carry a 3D model.
    pub skipped_models: usize,
}

/// Plans `frame` for an output of `physical` pixels.
pub(super) fn plan(frame: &OutputFrame, scale: Scale, physical: Size, sizes: &TextureSizes) -> FramePlan {
    let bounds = Rect::new(0, 0, physical.width, physical.height);
    let mut built = FramePlan {
        damage: damage_of(frame, scale, physical),
        ..FramePlan::default()
    };
    let mut draws = Draws::new(physical);
    let radius_scale = scale.as_f64() as f32;
    for damage in built.damage.clone() {
        for (index, layer) in frame.layers().iter().enumerate() {
            match layer {
                Layer::Solid(solid) => {
                    if solid.model.is_some() {
                        built.skipped_models += 1;
                        continue;
                    }
                    let quad = to_rect(physical_rect(solid.dst, scale));
                    if quad.is_empty() || !quad.intersects(damage) {
                        continue;
                    }
                    let mut uniforms = Uniforms::solid(quad, solid.colour, solid.tint, 1.0);
                    uniforms.corner = solid.corner_radius * radius_scale;
                    draws.push(&mut built, PassKind::Solid, None, damage, quad, &uniforms);
                }
                Layer::Surface(surface) => {
                    if surface.model.is_some() {
                        built.skipped_models += 1;
                        continue;
                    }
                    let Some(Some(size)) = sizes.layers.get(index) else {
                        continue;
                    };
                    let quad = to_rect(physical_rect(surface.dst, scale));
                    if quad.is_empty() || !quad.intersects(damage) {
                        continue;
                    }
                    let uniforms = Uniforms::surface(
                        quad,
                        texture_mapping(surface, *size),
                        surface.tint,
                        surface.opacity,
                        surface.corner_radius * radius_scale,
                    );
                    draws.push(
                        &mut built,
                        PassKind::Surface,
                        Some(TextureSlot::Layer(index)),
                        damage,
                        quad,
                        &uniforms,
                    );
                }
                Layer::Overlay(_) => continue,
            }
        }
    }
    built.layers_end = built.draws.len();
    let overlay = frame
        .overlay()
        .zip(sizes.overlay)
        .map(|(overlay, _)| to_rect(physical_rect(overlay.dst(), scale)));
    built.overlay_visible = overlay.and_then(|quad| quad.intersection(bounds));
    if let Some(quad) = overlay {
        bitmap(&mut built, &mut draws, TextureSlot::Overlay, quad);
    }
    built.overlay_end = built.draws.len();
    let cursor = frame
        .cursor()
        .zip(sizes.cursor)
        .map(|(cursor, _)| to_rect(physical_rect(cursor.dst(), scale)));
    built.cursor_visible = cursor.and_then(|quad| quad.intersection(bounds));
    if let Some(quad) = cursor {
        bitmap(&mut built, &mut draws, TextureSlot::Cursor, quad);
    }
    built
}

/// The frame's damage in physical pixels: clipped to the output and disjoint.
///
/// A frame whose damage is all outside the output has none, and draws nothing.
pub(super) fn damage_of(frame: &OutputFrame, scale: Scale, physical: Size) -> Vec<Rect> {
    let bounds = Rect::new(0, 0, physical.width, physical.height);
    let clipped: Vec<Rect> = frame
        .damage()
        .iter()
        .filter_map(|rect| to_rect(physical_rect(*rect, scale)).intersection(bounds))
        .collect();
    disjoint(&clipped)
}

/// Plans a bitmap drawn whole at `quad` over every damage rectangle it touches, and notes
/// which parts of it are repainted.
fn bitmap(built: &mut FramePlan, draws: &mut Draws, slot: TextureSlot, quad: Rect) {
    if quad.is_empty() {
        return;
    }
    let uniforms = Uniforms::surface(quad, [0.0, 0.0, 1.0, 1.0], Color::WHITE, 1.0, 0.0);
    for damage in built.damage.clone() {
        let Some(part) = damage.intersection(quad) else {
            continue;
        };
        match slot {
            TextureSlot::Cursor => built.cursor_under.push(part),
            _ => built.overlay_under.push(part),
        }
        draws.push(built, PassKind::Surface, Some(slot), damage, quad, &uniforms);
    }
}

/// The buffers the draws are packed into.
struct Draws {
    viewport: [f32; 2],
    block: Vec<u8>,
}

impl Draws {
    fn new(physical: Size) -> Self {
        Self {
            viewport: [physical.width as f32, physical.height as f32],
            block: Vec::with_capacity(UNIFORM_BYTES),
        }
    }

    /// Appends one draw: its uniform block, its quad and the record that names them.
    fn push(
        &mut self,
        built: &mut FramePlan,
        kind: PassKind,
        slot: Option<TextureSlot>,
        scissor: Rect,
        quad: Rect,
        uniforms: &Uniforms,
    ) {
        // The fixed layout has no user fields, so packing it cannot be refused.
        if UniformLayout::fixed().pack(uniforms, &[], &mut self.block).is_err() {
            return;
        }
        let offset = built.draws.len() * UNIFORM_STRIDE;
        built.uniforms.resize(offset + UNIFORM_STRIDE, 0);
        built.uniforms[offset..offset + self.block.len()].copy_from_slice(&self.block);
        let instance = built.draws.len();
        built.instances.extend_from_slice(&[
            quad.x as f32,
            quad.y as f32,
            quad.width as f32,
            quad.height as f32,
            self.viewport[0],
            self.viewport[1],
        ]);
        // The projection's columns and parameters stay zero for a flat quad.
        built.instances.extend([0.0; INSTANCE_FLOATS - 6]);
        built.draws.push(PlannedDraw {
            kind,
            slot,
            scissor,
            uniform_offset: offset as u32,
            instance: instance as u32,
        });
    }
}

/// A physical rectangle of the renderer's own type.
fn to_rect(rect: Rectangle<i32, Physical>) -> Rect {
    Rect::new(
        rect.loc.x,
        rect.loc.y,
        rect.size.w.max(0) as u32,
        rect.size.h.max(0) as u32,
    )
}

/// Where a surface's crop sits in its texture, as origin and scale in texture coordinates.
fn texture_mapping(surface: &Surface, (width, height): (u32, u32)) -> [f32; 4] {
    let crop: RectF = surface.src;
    [
        crop.x as f32 / width as f32,
        crop.y as f32 / height as f32,
        crop.width as f32 / width as f32,
        crop.height as f32 / height as f32,
    ]
}

/// The union of `rects` as rectangles that do not overlap.
///
/// Each rectangle keeps only the parts no earlier one covers, so the result paints exactly the
/// pixels the union does, once.
pub(super) fn disjoint(rects: &[Rect]) -> Vec<Rect> {
    let mut kept: Vec<Rect> = Vec::with_capacity(rects.len());
    for rect in rects {
        let mut pieces = vec![*rect];
        for existing in &kept {
            let mut next = Vec::with_capacity(pieces.len());
            for piece in pieces {
                subtract(piece, *existing, &mut next);
            }
            pieces = next;
        }
        kept.extend(pieces);
    }
    kept
}

/// Appends the parts of `from` that `taken` does not cover.
fn subtract(from: Rect, taken: Rect, out: &mut Vec<Rect>) {
    let Some(overlap) = from.intersection(taken) else {
        out.push(from);
        return;
    };
    let (left, top, right, bottom) = (i64::from(from.x), i64::from(from.y), from.right(), from.bottom());
    let (inner_left, inner_top) = (i64::from(overlap.x), i64::from(overlap.y));
    let (inner_right, inner_bottom) = (overlap.right(), overlap.bottom());
    for (x0, y0, x1, y1) in [
        (left, top, right, inner_top),
        (left, inner_bottom, right, bottom),
        (left, inner_top, inner_left, inner_bottom),
        (inner_right, inner_top, right, inner_bottom),
    ] {
        if x1 > x0 && y1 > y0 {
            out.push(Rect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render_frame::frame::Solid;

    fn area(rects: &[Rect]) -> u64 {
        rects.iter().map(|rect| rect.area()).sum()
    }

    #[test]
    fn overlapping_damage_becomes_disjoint_and_keeps_the_union() {
        let rects = [Rect::new(0, 0, 10, 10), Rect::new(5, 5, 10, 10), Rect::new(2, 2, 3, 3)];
        let kept = disjoint(&rects);
        // 100 + 100 - 25 overlap; the third rectangle is inside the first.
        assert_eq!(area(&kept), 175, "the union of the rectangles");
        for (index, first) in kept.iter().enumerate() {
            for second in &kept[index + 1..] {
                assert!(!first.intersects(*second), "{first:?} and {second:?} overlap");
            }
        }
    }

    #[test]
    fn disjoint_damage_is_left_alone() {
        let rects = [Rect::new(0, 0, 4, 4), Rect::new(8, 8, 4, 4)];
        assert_eq!(disjoint(&rects), rects);
    }

    fn solid_frame(damage: &[Rect], layers: &[Rect]) -> OutputFrame {
        let mut frame = OutputFrame::new();
        for rect in damage {
            frame.push_damage(*rect);
        }
        for rect in layers {
            frame.push_layer(Layer::Solid(Solid::new(*rect, Color::from_rgba8(255, 0, 0, 255))));
        }
        frame
    }

    #[test]
    fn a_layer_is_drawn_only_into_the_damage_it_touches() {
        let frame = solid_frame(
            &[Rect::new(0, 0, 8, 8), Rect::new(16, 0, 8, 8)],
            &[Rect::new(0, 0, 4, 4)],
        );
        let planned = plan(&frame, Scale::IDENTITY, Size::new(32, 8), &TextureSizes::default());
        assert_eq!(planned.draws.len(), 1);
        assert_eq!(planned.draws[0].scissor, Rect::new(0, 0, 8, 8));
        assert_eq!(planned.layers_end, 1);
        assert_eq!(planned.instances.len(), INSTANCE_FLOATS);
        assert_eq!(planned.uniforms.len(), UNIFORM_STRIDE);
    }

    #[test]
    fn damage_outside_the_output_is_dropped() {
        let frame = solid_frame(&[Rect::new(100, 100, 8, 8)], &[Rect::new(100, 100, 8, 8)]);
        let planned = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &TextureSizes::default());
        assert!(planned.damage.is_empty() && planned.draws.is_empty());
    }

    #[test]
    fn a_scale_places_the_quad_in_physical_pixels() {
        let frame = solid_frame(&[Rect::new(0, 0, 8, 8)], &[Rect::new(1, 1, 2, 2)]);
        let planned = plan(&frame, Scale::integral(2), Size::new(16, 16), &TextureSizes::default());
        assert_eq!(&planned.instances[..6], &[2.0, 2.0, 4.0, 4.0, 16.0, 16.0]);
    }

    #[test]
    fn a_layer_with_a_model_is_counted_and_left_out() {
        let mut frame = solid_frame(&[Rect::new(0, 0, 8, 8)], &[]);
        let mut solid = Solid::new(Rect::new(0, 0, 4, 4), Color::from_rgba8(0, 255, 0, 255));
        solid.model = Some(gnoblin_render_frame::frame::Model {
            matrix: gnoblin_render::Mat4::IDENTITY,
            opaque: true,
        });
        frame.push_layer(Layer::Solid(solid));
        let planned = plan(&frame, Scale::IDENTITY, Size::new(8, 8), &TextureSizes::default());
        assert_eq!(planned.skipped_models, 1);
        assert!(planned.draws.is_empty());
    }
}
