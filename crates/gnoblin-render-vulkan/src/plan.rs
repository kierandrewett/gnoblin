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
//!   overlay and the cursor follow, each as its own range of the draw list. The cursor's
//!   pixels are saved before it is drawn, so a capture can leave it out (see `Vulkan::capture`).
//! - **What is skipped**: a layer with a [`Model`](gnoblin_render_frame::frame::Model) belongs to
//!   the depth pass, which this renderer does not have, so it is counted and left out; a
//!   surface whose texture could not be made is left out; a layer that misses the rectangle
//!   is not drawn into it.
//!
//! # The uniform block and the quad
//!
//! Each draw's 96-byte block is packed by the same code the GLES renderer uses
//! ([`Uniforms::pack`]), so the two cannot disagree about an offset. The block sits at a
//! 256-byte stride, which is the largest alignment Vulkan allows a dynamic uniform offset
//! to demand. The shared vertex stage takes one instance of 22 floats per quad: the
//! rectangle and the viewport, three columns of an optional projection and its parameters.
//! A flat quad leaves the projection empty and its flag clear.

use gnoblin_render::{Effects, Pass, ShaderKey};
use gnoblin_types::{Color, Rect, RectF, Scale, Size};
use smithay::utils::{Physical, Rectangle};

use super::pipeline::{INSTANCE_FLOATS, UNIFORM_BYTES};
use gnoblin_render_frame::frame::{Layer, OutputFrame, Overlay, Surface};
use gnoblin_render_frame::geometry::physical_rect;
use gnoblin_render_frame::shader::uniforms::{Uniforms, UserBlock};

/// The distance between two draws' uniform blocks, in bytes.
pub(super) const UNIFORM_STRIDE: usize = 256;

/// Which pass a draw uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PassKind {
    /// A rectangle of one colour.
    Solid,
    /// A texture.
    Surface,
    /// A shell's shader pass, by the key of its source.
    User(ShaderKey),
}

/// Where a draw's texture comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextureSlot {
    /// The client buffer of the layer at this index of the frame (a solid layer with a shader pass has a pixel of its colour).
    Layer(usize),
    /// The portal overlay.
    Overlay,
    /// The cursor image.
    Cursor,
    /// What a step of the layer's shader chain drew: the layer at this index, and which of its two scratch images.
    Chain(usize, usize),
    /// The backdrop of the layer at this index: what is behind it, with secure layers black.
    Backdrop(usize),
}

impl TextureSlot {
    /// Whether the texture is sampled with the nearest texel.
    ///
    /// The overlay and the cursor are bitmaps and scale without smoothing, as they do in the
    /// other renderers, and a chain image is read one texel to a pixel; a client's surface and a
    /// backdrop are filtered.
    pub(super) fn nearest(self) -> bool {
        matches!(self, Self::Overlay | Self::Cursor | Self::Chain(..))
    }
}

/// One quad to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedDraw {
    /// Which fragment stage the draw runs.
    pub kind: PassKind,
    /// The texture the draw samples, or `None` for a pass that samples none.
    pub slot: Option<TextureSlot>,
    /// The layer whose backdrop the draw binds at set 2, for a shader pass that reads it.
    pub backdrop: Option<usize>,
    /// The damage rectangle the draw is clipped to, in the target's pixels.
    pub scissor: Rect,
    /// The draw's block, as an offset into [`FramePlan::uniforms`].
    pub uniform_offset: u32,
    /// The draw's shader block, as an offset into [`FramePlan::user`]; zero for a pass that has none.
    pub user_offset: u32,
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

/// One shader pass of a layer: the key of its source, the numbers it reads and whether it reads the backdrop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShaderUse {
    pub key: ShaderKey,
    pub params: [f32; 8],
    pub backdrop: bool,
}

/// The shader passes a frame draws: for each layer of the frame the passes whose pipeline exists and whose images were made,
/// in effect order.
///
/// A layer with none is drawn as its kind says. The plan reads a layer's passes only when the layer still carries a shader
/// effect itself, which is how the black stand-in of a secure layer (it carries no effects) keeps none in a backdrop.
#[derive(Clone, Debug, Default)]
pub(super) struct Shaders {
    pub layers: Vec<Vec<ShaderUse>>,
    /// The time in seconds a pass reads as `t`.
    pub time: f32,
}

impl Shaders {
    /// The passes drawn for the layer at `index`, none when it has none.
    pub(super) fn of(&self, index: usize) -> &[ShaderUse] {
        self.layers.get(index).map_or(&[], Vec::as_slice)
    }
}

/// Whether an effect chain has a shader pass.
pub(super) fn has_shader(effects: &Effects) -> bool {
    effects.passes().iter().any(|pass| matches!(pass, Pass::Shader(_)))
}

/// Everything the renderer records for a frame.
#[derive(Clone, Debug, Default)]
pub(super) struct FramePlan {
    /// The damage in the target's pixels: clipped to the output and disjoint.
    pub damage: Vec<Rect>,
    /// Every draw, layers first, then the overlay, then the cursor.
    pub draws: Vec<PlannedDraw>,
    /// The draws before this index are layers.
    pub layers_end: usize,
    /// The draws from `layers_end` to this index are the overlay, the rest the cursor.
    pub overlay_end: usize,
    /// The cursor's rectangle on the output, when it is drawn at all.
    pub cursor_visible: Option<Rect>,
    /// The parts of the cursor's rectangle this frame repaints, to be saved first.
    pub cursor_under: Vec<Rect>,
    /// The draws' uniform blocks, [`UNIFORM_STRIDE`] bytes apart.
    pub uniforms: Vec<u8>,
    /// The shader passes' own blocks, [`UNIFORM_STRIDE`] bytes apart.
    pub user: Vec<u8>,
    /// The quads, [`INSTANCE_FLOATS`] floats each.
    pub instances: Vec<f32>,
    /// Layers left out because they carry a 3D model.
    pub skipped_models: usize,
}

impl FramePlan {
    /// Moves the draws' offsets to where this plan's buffers will sit after those of `batch`, and appends them to it.
    ///
    /// Every plan of a frame (the frame itself, the backdrops and the chain images) is recorded into one command buffer, which
    /// reads one uniform buffer, one shader-block buffer and one instance buffer, so the plans' buffers are joined.
    pub(super) fn append_to(mut self, batch: &mut DrawBatch) -> Self {
        let (uniforms, user, instances) = (
            batch.uniforms.len(),
            batch.user.len(),
            batch.instances.len() / INSTANCE_FLOATS,
        );
        for draw in &mut self.draws {
            draw.uniform_offset += uniforms as u32;
            if matches!(draw.kind, PassKind::User(_)) {
                draw.user_offset += user as u32;
            }
            draw.instance += instances as u32;
        }
        batch.uniforms.extend_from_slice(&self.uniforms);
        batch.user.extend_from_slice(&self.user);
        batch.instances.extend_from_slice(&self.instances);
        self.uniforms.clear();
        self.user.clear();
        self.instances.clear();
        self
    }
}

/// The buffers every plan of one frame is joined into: see [`FramePlan::append_to`].
#[derive(Clone, Debug, Default)]
pub(super) struct DrawBatch {
    pub uniforms: Vec<u8>,
    pub user: Vec<u8>,
    pub instances: Vec<f32>,
}

/// What a plan draws into: a part of the output, shown at the target's origin.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlanTarget {
    /// The output's pixel that is the target's top-left.
    pub origin: (i32, i32),
    /// The target's size in pixels.
    pub size: Size,
}

impl PlanTarget {
    /// The whole output.
    pub(super) fn whole(physical: Size) -> Self {
        Self {
            origin: (0, 0),
            size: physical,
        }
    }
}

/// Plans `frame` for an output of `physical` pixels.
pub(super) fn plan(
    frame: &OutputFrame,
    scale: Scale,
    physical: Size,
    sizes: &TextureSizes,
    shaders: &Shaders,
) -> FramePlan {
    plan_into(frame, scale, physical, PlanTarget::whole(physical), sizes, shaders)
}

/// Plans `frame` into a target that shows the part of the output at `target`: a backdrop, which is the layers below one
/// layer drawn over that layer's rectangle.
pub(super) fn plan_into(
    frame: &OutputFrame,
    scale: Scale,
    physical: Size,
    target: PlanTarget,
    sizes: &TextureSizes,
    shaders: &Shaders,
) -> FramePlan {
    let bounds = Rect::new(0, 0, physical.width, physical.height);
    let damage = damage_of(frame, scale, physical);
    let mut built = FramePlan::default();
    let mut draws = Draws::new(target);
    let radius_scale = scale.as_f64() as f32;
    for damage in damage.clone() {
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
                    let passes = shader_passes(shaders, index, &solid.effects, sizes);
                    let mut uniforms = if passes.is_empty() {
                        Uniforms::solid(quad, solid.colour, solid.tint, 1.0)
                    } else {
                        // The shader reads the solid's colour from a pixel of it, so the draw is a textured one.
                        Uniforms::surface(quad, [0.0, 0.0, 1.0, 1.0], solid.tint, 1.0, 0.0)
                    };
                    uniforms.corner = solid.corner_radius * radius_scale;
                    if let Some(shape) = solid.silhouette.filter(|_| solid.corner_radius > 0.0) {
                        uniforms.round_to(to_rect(physical_rect(shape, scale)));
                    }
                    if passes.is_empty() {
                        draws.push(&mut built, PassKind::Solid, None, damage, quad, &uniforms);
                    } else {
                        draws.push_shader(&mut built, shaders, index, damage, quad, &uniforms);
                    }
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
                    let mut uniforms = Uniforms::surface(
                        quad,
                        texture_mapping(surface, *size),
                        surface.tint,
                        surface.opacity,
                        surface.corner_radius * radius_scale,
                    );
                    if let Some(shape) = surface.silhouette.filter(|_| surface.corner_radius > 0.0) {
                        uniforms.round_to(to_rect(physical_rect(shape, scale)));
                    }
                    let passes = shader_passes(shaders, index, &surface.effects, sizes);
                    if passes.is_empty() {
                        draws.push(
                            &mut built,
                            PassKind::Surface,
                            Some(TextureSlot::Layer(index)),
                            damage,
                            quad,
                            &uniforms,
                        );
                    } else {
                        draws.push_shader(&mut built, shaders, index, damage, quad, &uniforms);
                    }
                }
                Layer::Overlay(_) => continue,
            }
        }
    }
    built.layers_end = built.draws.len();
    let overlay = overlay_of(frame)
        .zip(sizes.overlay)
        .map(|(overlay, _)| to_rect(physical_rect(overlay.dst(), scale)));
    if let Some(quad) = overlay {
        bitmap(&mut built, &mut draws, &damage, TextureSlot::Overlay, quad);
    }
    built.overlay_end = built.draws.len();
    let cursor = frame
        .cursor()
        .zip(sizes.cursor)
        .map(|(cursor, _)| to_rect(physical_rect(cursor.dst(), scale)));
    built.cursor_visible = cursor.and_then(|quad| quad.intersection(bounds));
    if let Some(quad) = cursor {
        bitmap(&mut built, &mut draws, &damage, TextureSlot::Cursor, quad);
    }
    built.damage = damage.iter().map(|rect| draws.shift(*rect)).collect();
    built
}

/// The shader passes a layer draws: those of `shaders`, when the layer still has a shader effect and its texture exists.
fn shader_passes<'a>(shaders: &'a Shaders, index: usize, effects: &Effects, sizes: &TextureSizes) -> &'a [ShaderUse] {
    if has_shader(effects) && sizes.layers.get(index).is_some_and(Option::is_some) {
        shaders.of(index)
    } else {
        &[]
    }
}

/// One step of a layer's shader chain: a scratch image the pass draws the layer into.
#[derive(Clone, Debug)]
pub(super) struct ChainStep {
    /// Which of the layer's two scratch images the step draws into.
    pub into: usize,
    /// The step's draw, over the whole image.
    pub plan: FramePlan,
}

/// The scratch images of a layer that has more than one shader pass, drawn in order before the frames that use the layer.
///
/// Pass `k` reads the layer's own pixels (`k = 0`) or what pass `k - 1` drew, and draws the whole layer rectangle into an
/// image of its size, with the layer's tint and opacity left out and its corners square: those are the last pass's. The last
/// pass is the layer's draw in the frame, which reads the last image. Two images alternate, so a pass never reads what it draws.
pub(super) fn chain(
    layer: &Layer,
    index: usize,
    scale: Scale,
    sizes: &TextureSizes,
    shaders: &Shaders,
) -> Vec<ChainStep> {
    let passes = shaders.of(index);
    let (dst, src) = match layer {
        Layer::Solid(solid) => (solid.dst, [0.0, 0.0, 1.0, 1.0]),
        Layer::Surface(surface) => {
            let Some(Some(size)) = sizes.layers.get(index) else {
                return Vec::new();
            };
            (surface.dst, texture_mapping(surface, *size))
        }
        Layer::Overlay(_) => return Vec::new(),
    };
    let quad = to_rect(physical_rect(dst, scale));
    let image = PlanTarget {
        origin: (quad.x, quad.y),
        size: Size::new(quad.width, quad.height),
    };
    let mut steps = Vec::new();
    for (step, pass) in passes.iter().enumerate().take(passes.len().saturating_sub(1)) {
        let mut built = FramePlan::default();
        let mut draws = Draws::new(image);
        let slot = if step == 0 {
            TextureSlot::Layer(index)
        } else {
            TextureSlot::Chain(index, (step - 1) % 2)
        };
        let uniforms = Uniforms::surface(
            quad,
            if step == 0 { src } else { [0.0, 0.0, 1.0, 1.0] },
            Color::WHITE,
            1.0,
            0.0,
        );
        draws.push_user(&mut built, pass, shaders.time, slot, index, quad, quad, &uniforms);
        built.damage = vec![draws.shift(quad)];
        built.layers_end = built.draws.len();
        built.overlay_end = built.draws.len();
        steps.push(ChainStep {
            into: step % 2,
            plan: built,
        });
    }
    steps
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

/// The frame's overlay, which is its last layer when it has one. The plan draws it after the
/// layers and before the cursor.
pub(super) fn overlay_of(frame: &OutputFrame) -> Option<&Overlay> {
    frame.layers().iter().find_map(|layer| match layer {
        Layer::Overlay(overlay) => Some(overlay),
        _ => None,
    })
}

/// Plans a bitmap drawn whole at `quad` over every damage rectangle it touches, and notes
/// which parts of it are repainted.
fn bitmap(built: &mut FramePlan, draws: &mut Draws, damage: &[Rect], slot: TextureSlot, quad: Rect) {
    if quad.is_empty() {
        return;
    }
    let uniforms = Uniforms::surface(quad, [0.0, 0.0, 1.0, 1.0], Color::WHITE, 1.0, 0.0);
    for damage in damage {
        let Some(part) = damage.intersection(quad) else {
            continue;
        };
        if matches!(slot, TextureSlot::Cursor) {
            built.cursor_under.push(part);
        }
        draws.push(built, PassKind::Surface, Some(slot), *damage, quad, &uniforms);
    }
}

/// The buffers the draws are packed into.
struct Draws {
    target: PlanTarget,
    viewport: [f32; 2],
    block: Vec<u8>,
}

impl Draws {
    fn new(target: PlanTarget) -> Self {
        Self {
            target,
            viewport: [target.size.width as f32, target.size.height as f32],
            block: Vec::with_capacity(UNIFORM_BYTES),
        }
    }

    /// A rectangle of the output in the target's own pixels.
    fn shift(&self, rect: Rect) -> Rect {
        Rect::new(
            rect.x - self.target.origin.0,
            rect.y - self.target.origin.1,
            rect.width,
            rect.height,
        )
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
        self.append(built, kind, slot, None, None, scissor, quad, uniforms);
    }

    /// Appends a layer's last shader pass, which reads the layer itself, or the last image of its chain.
    fn push_shader(
        &mut self,
        built: &mut FramePlan,
        shaders: &Shaders,
        index: usize,
        scissor: Rect,
        quad: Rect,
        uniforms: &Uniforms,
    ) {
        let passes = shaders.of(index);
        let Some(last) = passes.last() else { return };
        let mut uniforms = *uniforms;
        let slot = if passes.len() == 1 {
            TextureSlot::Layer(index)
        } else {
            // The chain's images are the size of the quad and are read whole.
            uniforms.src = [0.0, 0.0, 1.0, 1.0];
            TextureSlot::Chain(index, (passes.len() - 2) % 2)
        };
        self.push_user(built, last, shaders.time, slot, index, scissor, quad, &uniforms);
    }

    /// Appends one draw of a shader pass.
    #[allow(clippy::too_many_arguments)]
    fn push_user(
        &mut self,
        built: &mut FramePlan,
        pass: &ShaderUse,
        time: f32,
        slot: TextureSlot,
        index: usize,
        scissor: Rect,
        quad: Rect,
        uniforms: &Uniforms,
    ) {
        let mut user = Vec::new();
        UserBlock::new(pass.params, time).pack(&mut user);
        self.append(
            built,
            PassKind::User(pass.key),
            Some(slot),
            pass.backdrop.then_some(index),
            Some(&user),
            scissor,
            quad,
            uniforms,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        built: &mut FramePlan,
        kind: PassKind,
        slot: Option<TextureSlot>,
        backdrop: Option<usize>,
        user: Option<&[u8]>,
        scissor: Rect,
        quad: Rect,
        uniforms: &Uniforms,
    ) {
        let (scissor, quad) = (self.shift(scissor), self.shift(quad));
        uniforms.pack(&mut self.block);
        let offset = built.draws.len() * UNIFORM_STRIDE;
        built.uniforms.resize(offset + UNIFORM_STRIDE, 0);
        built.uniforms[offset..offset + self.block.len()].copy_from_slice(&self.block);
        // A draw with no block of its own points the second binding at the first one, which is always inside the buffer.
        let user_offset = match user {
            Some(block) => {
                let at = built.user.len();
                built.user.resize(at + UNIFORM_STRIDE, 0);
                built.user[at..at + block.len()].copy_from_slice(block);
                at
            }
            None => 0,
        };
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
            backdrop,
            scissor,
            uniform_offset: offset as u32,
            user_offset: user_offset as u32,
            instance: instance as u32,
        });
    }
}

/// A logical rectangle of the frame as physical pixels of the output, by the rule every renderer shares.
pub(super) fn quad_of(rect: Rect, scale: Scale) -> Rect {
    to_rect(physical_rect(rect, scale))
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
        let planned = plan(
            &frame,
            Scale::IDENTITY,
            Size::new(32, 8),
            &TextureSizes::default(),
            &Shaders::default(),
        );
        assert_eq!(planned.draws.len(), 1);
        assert_eq!(planned.draws[0].scissor, Rect::new(0, 0, 8, 8));
        assert_eq!(planned.layers_end, 1);
        assert_eq!(planned.instances.len(), INSTANCE_FLOATS);
        assert_eq!(planned.uniforms.len(), UNIFORM_STRIDE);
    }

    #[test]
    fn damage_outside_the_output_is_dropped() {
        let frame = solid_frame(&[Rect::new(100, 100, 8, 8)], &[Rect::new(100, 100, 8, 8)]);
        let planned = plan(
            &frame,
            Scale::IDENTITY,
            Size::new(32, 32),
            &TextureSizes::default(),
            &Shaders::default(),
        );
        assert!(planned.damage.is_empty() && planned.draws.is_empty());
    }

    #[test]
    fn a_scale_places_the_quad_in_physical_pixels() {
        let frame = solid_frame(&[Rect::new(0, 0, 8, 8)], &[Rect::new(1, 1, 2, 2)]);
        let planned = plan(
            &frame,
            Scale::integral(2),
            Size::new(16, 16),
            &TextureSizes::default(),
            &Shaders::default(),
        );
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
        let planned = plan(
            &frame,
            Scale::IDENTITY,
            Size::new(8, 8),
            &TextureSizes::default(),
            &Shaders::default(),
        );
        assert_eq!(planned.skipped_models, 1);
        assert!(planned.draws.is_empty());
    }

    /// A frame whose one layer is a solid with `count` shader passes, and the shaders and sizes that make them drawable.
    fn shader_frame(count: usize, backdrop: bool) -> (OutputFrame, TextureSizes, Shaders) {
        let mut frame = solid_frame(&[Rect::new(0, 0, 32, 32)], &[]);
        let mut solid = Solid::new(Rect::new(8, 4, 16, 12), Color::from_rgba8(255, 0, 0, 255));
        let mut uses = Vec::new();
        for index in 0..count {
            let source = format!("fn effect() {{}} // {index}");
            let pass = gnoblin_render::ShaderPass::new(&source, &[index as f32], backdrop);
            uses.push(ShaderUse {
                key: pass.key(),
                params: pass.params_padded(),
                backdrop,
            });
            solid.effects.push(Pass::Shader(pass)).expect("a pass");
        }
        frame.push_layer(Layer::Solid(solid));
        let sizes = TextureSizes {
            layers: vec![Some((1, 1))],
            ..TextureSizes::default()
        };
        let shaders = Shaders {
            layers: vec![uses],
            time: 2.5,
        };
        (frame, sizes, shaders)
    }

    #[test]
    fn a_layer_with_one_shader_pass_is_drawn_by_it_over_its_own_pixels() {
        let (frame, sizes, shaders) = shader_frame(1, true);
        let planned = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
        assert_eq!(planned.draws.len(), 1);
        let draw = planned.draws[0];
        assert_eq!(draw.kind, PassKind::User(shaders.layers[0][0].key));
        assert_eq!(draw.slot, Some(TextureSlot::Layer(0)), "the layer's own pixels");
        assert_eq!(draw.backdrop, Some(0), "its backdrop is bound");
        assert_eq!(planned.user.len(), UNIFORM_STRIDE, "the pass has a block of its own");
        let time = f32::from_ne_bytes(planned.user[32..36].try_into().expect("four bytes"));
        assert_eq!(time, 2.5, "the block carries the time after the eight params");
        let first = f32::from_ne_bytes(planned.user[0..4].try_into().expect("four bytes"));
        assert_eq!(first, 0.0);
    }

    #[test]
    fn a_shader_that_does_not_read_the_backdrop_binds_none() {
        let (frame, sizes, shaders) = shader_frame(1, false);
        let planned = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
        assert_eq!(planned.draws[0].backdrop, None);
    }

    #[test]
    fn a_solid_whose_pixel_was_not_made_is_drawn_without_its_shader() {
        let (frame, _, shaders) = shader_frame(1, false);
        let sizes = TextureSizes {
            layers: vec![None],
            ..TextureSizes::default()
        };
        let planned = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
        assert_eq!(planned.draws[0].kind, PassKind::Solid);
    }

    #[test]
    fn a_layer_whose_shader_effect_is_gone_ignores_the_passes_of_the_frame() {
        // The black stand-in of a secure layer in a backdrop has no effects, whatever the frame it came from drew.
        let (_, sizes, shaders) = shader_frame(1, false);
        let mut masked = solid_frame(&[Rect::new(0, 0, 32, 32)], &[]);
        masked.push_layer(Layer::Solid(Solid::new(Rect::new(8, 4, 16, 12), Color::BLACK)));
        let planned = plan(&masked, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
        assert_eq!(planned.draws[0].kind, PassKind::Solid);
    }

    #[test]
    fn several_passes_chain_through_two_alternating_images_and_the_last_reads_the_last_image() {
        for (count, last_image) in [(2, 0), (3, 1), (4, 0)] {
            let (frame, sizes, shaders) = shader_frame(count, false);
            let planned = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
            assert_eq!(
                planned.draws[0].slot,
                Some(TextureSlot::Chain(0, last_image)),
                "{count} passes"
            );
            assert_eq!(planned.draws[0].kind, PassKind::User(shaders.layers[0][count - 1].key));
            let steps = chain(&frame.layers()[0], 0, Scale::IDENTITY, &sizes, &shaders);
            assert_eq!(steps.len(), count - 1, "every pass but the last is a step");
            let into: Vec<usize> = steps.iter().map(|step| step.into).collect();
            let expected: Vec<usize> = (0..count - 1).map(|step| step % 2).collect();
            assert_eq!(into, expected, "the steps alternate");
            for (index, step) in steps.iter().enumerate() {
                let draw = step.plan.draws[0];
                assert_eq!(draw.kind, PassKind::User(shaders.layers[0][index].key));
                let from = if index == 0 {
                    TextureSlot::Layer(0)
                } else {
                    TextureSlot::Chain(0, (index - 1) % 2)
                };
                assert_eq!(draw.slot, Some(from), "a step reads the layer, then the step before");
                assert_eq!(
                    step.plan.damage,
                    [Rect::new(0, 0, 16, 12)],
                    "and draws all of the image"
                );
                assert_eq!(&step.plan.instances[..6], &[0.0, 0.0, 16.0, 12.0, 16.0, 12.0]);
            }
        }
    }

    #[test]
    fn a_target_that_shows_part_of_the_output_shifts_what_it_draws() {
        let frame = solid_frame(&[Rect::new(10, 10, 8, 8)], &[Rect::new(8, 8, 16, 16)]);
        let target = PlanTarget {
            origin: (10, 10),
            size: Size::new(8, 8),
        };
        let planned = plan_into(
            &frame,
            Scale::IDENTITY,
            Size::new(64, 64),
            target,
            &TextureSizes::default(),
            &Shaders::default(),
        );
        assert_eq!(planned.damage, [Rect::new(0, 0, 8, 8)]);
        assert_eq!(planned.draws[0].scissor, Rect::new(0, 0, 8, 8));
        assert_eq!(
            &planned.instances[..6],
            &[-2.0, -2.0, 16.0, 16.0, 8.0, 8.0],
            "against the target's own size"
        );
    }

    #[test]
    fn the_plans_of_a_frame_are_joined_with_their_offsets_moved() {
        let (frame, sizes, shaders) = shader_frame(1, false);
        let first = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
        let second = plan(&frame, Scale::IDENTITY, Size::new(32, 32), &sizes, &shaders);
        let mut batch = DrawBatch::default();
        let first = first.append_to(&mut batch);
        let second = second.append_to(&mut batch);
        assert_eq!(first.draws[0].uniform_offset, 0);
        assert_eq!(second.draws[0].uniform_offset as usize, UNIFORM_STRIDE);
        assert_eq!(second.draws[0].user_offset as usize, UNIFORM_STRIDE);
        assert_eq!(second.draws[0].instance, 1);
        assert_eq!(batch.uniforms.len(), 2 * UNIFORM_STRIDE);
        assert_eq!(batch.user.len(), 2 * UNIFORM_STRIDE);
        assert_eq!(batch.instances.len(), 2 * INSTANCE_FLOATS);
    }
}
