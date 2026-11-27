//! The blur of a layer, planned: the images a layer's glass, group and picture are drawn through.
//!
//! A frame has no pass that reads the target it is drawing, so a blur does not read it: it draws what the blur reads
//! into an image of its own, from the layers below, which is what a shell's `backdrop(uv)` does too. Everything here
//! is pure: it turns the frame into jobs (a plan drawn into an image), and the recorder draws them before the frame
//! does. A partial redraw therefore blurs the same pixels a full one does, whatever the target held before.
//!
//! # A layer with a backdrop blur
//!
//! 1. The layers below, over the layer's visible rectangle grown by the blur's reach and clipped to the output, are
//!    drawn into an image of that region.
//! 2. Each blur is the separable Gaussian of `blur.wgsl`: nine taps `radius / 3` pixels apart, a horizontal sweep and a
//!    vertical one, `passes` times. A `downscale` below one first reduces the region to `ceil(size * downscale)` by
//!    halvings through bilinear taps, sweeps there at the radius scaled by it, and reads the small image back up with
//!    linear filtering.
//! 3. The grade, the tint and the grain belong to the last write, at full resolution. The grain is counted from the
//!    layer's own top-left corner. That write goes into the layer's *group*, an image of the part of the layer that is on
//!    the output, which then also holds the background, the inset shadows and the layer's own pixels. The frame composes
//!    the group over itself at the layer's opacity, clipped to its corners.
//!
//! Several blurs on one layer compound: each reads what the one before wrote.
//!
//! # A layer with a filter that blurs
//!
//! Everything the layer paints (its shadows, background, inset shadows, pixels and borders) is drawn into a *picture* of
//! its painted rectangle grown by the blur's reach, blurred by one pass of the same sweeps, and composed over the frame
//! with the layer's tint, opacity and the filter's grade. The picture is not clipped to the corners, so it spills past
//! the edge. A layer that also has glass draws the glass first, outside the picture, so the filter does not blur it.
//!
//! # Which layers
//!
//! Only a layer whose result reaches something drawn this frame is planned: a blurred panel nobody damaged costs nothing.
//! A layer is needed when its group or picture meets the damage, or the region another blur or a shader reads.

use std::collections::BTreeSet;

use gnoblin_render::{BlurParams, FilterParams, MAX_BLUR_PASSES, MIN_BLUR_DOWNSCALE, Pass};
use gnoblin_render_frame::frame::{Layer, OutputFrame};
use gnoblin_render_frame::shader::uniforms::Uniforms;
use gnoblin_types::{Color, Rect, Scale, Size};

use super::plan::{
    FramePlan, PassKind, Pieces, PlanTarget, QuadDraw, Shaders, TextureSizes, TextureSlot, around, damage_of,
    image_plan, layer_shape, plan_into_damage, plan_pieces,
};

/// The largest blur radius and picture reach the planner takes, in physical pixels: the limit the GLES renderer clamps to.
const MAX_RADIUS: f32 = 4096.0;

/// A layer's glass: the group image of the part of the layer that is on the output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Glass {
    /// The image, by id.
    pub image: u32,
    /// Where it lies on the output.
    pub rect: Rect,
}

/// A layer's picture: what a filter that blurs blurs and grades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlurPicture {
    pub image: u32,
    /// Where it lies on the output.
    pub extent: Rect,
    /// The filter, whose grade the composition applies.
    pub filter: FilterParams,
}

/// What a layer with a blur draws from.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum BlurState {
    /// The images could not be made (an image past the device's limit): the layer is drawn without its blur.
    Unavailable,
    /// Nothing the layer's blur makes reaches anything drawn this frame. Its shadows and borders still draw.
    Skipped { picture: bool },
    /// The layer's images are drawn by earlier passes of the frame.
    Drawn {
        glass: Option<Glass>,
        picture: Option<BlurPicture>,
    },
}

impl BlurState {
    /// Whether the layer's pieces are in a picture and not drawn on the frame.
    pub(super) fn has_picture(&self) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Skipped { picture } => *picture,
            Self::Drawn { picture, .. } => picture.is_some(),
        }
    }
}

/// One pass drawn into an image before the frame.
#[derive(Clone, Debug)]
pub(super) struct BlurJob {
    /// The layer the pass is for, which orders it among the passes of the frame.
    pub index: usize,
    /// The image it draws into.
    pub image: u32,
    pub size: Size,
    /// The colour the image starts as where the plan draws, when it is not clear: the frame's background.
    pub background: Option<Color>,
    pub plan: FramePlan,
}

/// The jobs of a frame and the images they need.
#[derive(Clone, Debug, Default)]
pub(super) struct BlurPlan {
    /// Every image the jobs and the frame use, by id and size. An id may be drawn into again once nothing reads it.
    pub images: Vec<(u32, Size)>,
    /// The passes, bottom layer first.
    pub jobs: Vec<BlurJob>,
}

/// The images of a plan, and which of them may be drawn into again.
#[derive(Default)]
struct Images {
    all: Vec<(u32, Size)>,
    free: Vec<(u32, Size)>,
}

impl Images {
    /// An image of `size`, one that was released if there is one.
    fn temporary(&mut self, size: Size) -> u32 {
        if let Some(at) = self.free.iter().position(|(_, held)| *held == size) {
            return self.free.swap_remove(at).0;
        }
        self.kept(size)
    }

    /// An image nothing releases: the frame reads it.
    fn kept(&mut self, size: Size) -> u32 {
        let id = self.all.len() as u32;
        self.all.push((id, size));
        id
    }

    fn release(&mut self, id: u32) {
        if let Some(entry) = self.all.iter().find(|(held, _)| *held == id) {
            self.free.push(*entry);
        }
    }
}

/// What the planner reads of a frame.
pub(super) struct Inputs<'a> {
    pub frame: &'a OutputFrame,
    pub scale: Scale,
    pub physical: Size,
    pub sizes: &'a TextureSizes,
    /// The largest edge of an image the device makes.
    pub limit: u32,
    /// Regions of the output that something other than the frame reads, in physical pixels: what a shader's backdrop covers.
    pub reads: &'a [Rect],
}

/// A layer with a blur, as the planner measures it.
struct MeasuredLayer {
    index: usize,
    quad: Rect,
    /// The part of `quad` that is on the output.
    visible: Option<Rect>,
    blurs: Vec<BlurParams>,
    /// The filter, when it blurs.
    filter: Option<FilterParams>,
    painted: Rect,
    factor: f32,
}

/// The reach of one blur's taps past the pixel it writes, in physical pixels: each sweep spreads the taps by four steps.
fn reach_of(blur: &BlurParams, factor: f32) -> u32 {
    let radius = (blur.radius * factor).clamp(1.0, MAX_RADIUS);
    blur.passes.clamp(1, MAX_BLUR_PASSES) * ((radius / 3.0 * 4.0).ceil() as u32 + 1)
}

/// `rect` grown by `by` on every side.
fn grown(rect: Rect, by: u32) -> Rect {
    Rect::new(
        rect.x.saturating_sub(by as i32),
        rect.y.saturating_sub(by as i32),
        rect.width + 2 * by,
        rect.height + 2 * by,
    )
}

/// The size one halving of `size` reaches towards `goal`: half, rounded up, but not below the goal.
fn halved(size: u32, goal: u32) -> u32 {
    if size > goal { size.div_ceil(2).max(goal) } else { size }
}

/// `size * downscale`, rounded up, at least a pixel and at most the size itself.
fn scaled(size: u32, downscale: f32) -> u32 {
    ((size as f32 * downscale).ceil() as u32).clamp(1, size.max(1))
}

/// Where `rect` lies in an image that shows `region`, as the crop a draw maps onto it: origin and extent in texture units.
fn crop(rect: Rect, region: Rect) -> [f32; 4] {
    let (w, h) = (region.width as f32, region.height as f32);
    [
        (rect.x - region.x) as f32 / w,
        (rect.y - region.y) as f32 / h,
        rect.width as f32 / w,
        rect.height as f32 / h,
    ]
}

/// One sweep of the blur over a whole image of `size`, reading `from` a tap `step` pixels apart along one axis.
fn sweep(from: u32, size: Size, step: [f32; 2]) -> QuadDraw {
    let area = Rect::new(0, 0, size.width, size.height);
    let mut uniforms = Uniforms::surface(area, [0.0, 0.0, 1.0, 1.0], Color::TRANSPARENT, 1.0, 0.0);
    uniforms.params = [step[0] / size.width as f32, step[1] / size.height as f32, 0.0, 0.0];
    QuadDraw {
        kind: PassKind::Blur,
        slot: Some(TextureSlot::Image(from)),
        quad: area,
        uniforms,
    }
}

/// The planner's state: the jobs so far and the images they use.
struct Planner<'a> {
    inputs: &'a Inputs<'a>,
    images: Images,
    jobs: Vec<BlurJob>,
}

impl Planner<'_> {
    fn job(&mut self, index: usize, image: u32, size: Size, quads: &[QuadDraw]) {
        self.jobs.push(BlurJob {
            index,
            image,
            size,
            background: None,
            plan: image_plan(size, quads),
        });
    }

    /// The sweeps of one blur over `source`, an image of `region`'s size. Returns the image that holds the result and the
    /// tap step of the vertical sweep the last write still has to make (none when the blur ran on a reduced image). Every
    /// image it made but that one is released.
    fn sweeps(&mut self, index: usize, source: u32, size: Size, blur: &BlurParams, factor: f32) -> (u32, [f32; 2]) {
        let radius = (blur.radius * factor).clamp(1.0, MAX_RADIUS);
        let sweeps = blur.passes.clamp(1, MAX_BLUR_PASSES);
        let downscale = blur.downscale.clamp(MIN_BLUR_DOWNSCALE, 1.0);
        let step = (radius / 3.0).max(0.5);
        let small = Size::new(scaled(size.width, downscale), scaled(size.height, downscale));
        if small == size {
            let second = self.images.temporary(size);
            for sweep_index in 0..sweeps {
                self.job(index, second, size, &[sweep(source, size, [step, 0.0])]);
                if sweep_index + 1 < sweeps {
                    self.job(index, source, size, &[sweep(second, size, [0.0, step])]);
                }
            }
            self.images.release(source);
            return (second, [0.0, step / size.height as f32]);
        }
        // Halve down to the small size through bilinear taps: each draw averages two by two texels.
        let mut held = source;
        let mut at = size;
        while at != small {
            let next = Size::new(halved(at.width, small.width), halved(at.height, small.height));
            let image = self.images.temporary(next);
            let area = Rect::new(0, 0, next.width, next.height);
            let uniforms = Uniforms::surface(area, [0.0, 0.0, 1.0, 1.0], Color::WHITE, 1.0, 0.0);
            self.job(
                index,
                image,
                next,
                &[QuadDraw {
                    kind: PassKind::Surface,
                    slot: Some(TextureSlot::Image(held)),
                    quad: area,
                    uniforms,
                }],
            );
            self.images.release(held);
            held = image;
            at = next;
        }
        // The same tap step as at full resolution, in the reduced image's own pixels: the blur's width on the output does
        // not depend on `downscale`.
        let step = step * downscale;
        let second = self.images.temporary(small);
        for _ in 0..sweeps {
            self.job(index, second, small, &[sweep(held, small, [step, 0.0])]);
            self.job(index, held, small, &[sweep(second, small, [0.0, step])]);
        }
        self.images.release(second);
        (held, [0.0, 0.0])
    }

    /// The glass of a layer: its backdrop blurred and written into its group. Returns the group.
    fn glass(
        &mut self,
        shaders: &Shaders,
        layer: &MeasuredLayer,
        visible: Rect,
        region: Rect,
        pieces: Pieces,
    ) -> Glass {
        let inputs = self.inputs;
        let index = layer.index;
        let size = Size::new(region.width, region.height);
        // What the blur reads: the layers below, over the region.
        let backdrop = self.images.temporary(size);
        let mut below = OutputFrame::new();
        below.set_background(inputs.frame.background());
        for under in &inputs.frame.layers()[..index] {
            below.push_layer(under.clone());
        }
        let plan = plan_into_damage(
            &below,
            inputs.scale,
            inputs.physical,
            PlanTarget {
                origin: (region.x, region.y),
                size,
            },
            inputs.sizes,
            shaders,
            vec![region],
        );
        self.jobs.push(BlurJob {
            index,
            image: backdrop,
            size,
            background: Some(inputs.frame.background()),
            plan,
        });
        let mut source = backdrop;
        let last = layer.blurs.len() - 1;
        for (at, blur) in layer.blurs.iter().enumerate() {
            let (swept, vertical) = self.sweeps(index, source, size, blur, layer.factor);
            let tint = blur.tint;
            let grade = [blur.saturation, blur.brightness, blur.contrast];
            let resolve = |quad: Rect, noise: f32, vertical: [f32; 2]| {
                let mut uniforms = Uniforms::surface(quad, crop(quad, region), tint, 1.0, 0.0);
                uniforms.params = [vertical[0], vertical[1], 0.0, 0.0];
                uniforms.grade = [grade[0], grade[1], grade[2], noise];
                QuadDraw {
                    kind: PassKind::Blur,
                    slot: Some(TextureSlot::Image(swept)),
                    quad,
                    uniforms,
                }
            };
            if at == last {
                // The layer's own rectangle carries the grain, counted from its top-left corner.
                let group = self.images.kept(Size::new(visible.width, visible.height));
                let lead = [resolve(layer.quad, blur.noise, vertical)];
                let plan = plan_pieces(
                    &inputs.frame.layers()[index],
                    index,
                    inputs.scale,
                    inputs.sizes,
                    shaders,
                    visible,
                    pieces,
                    &lead,
                );
                self.jobs.push(BlurJob {
                    index,
                    image: group,
                    size: Size::new(visible.width, visible.height),
                    background: None,
                    plan,
                });
                self.images.release(swept);
                return Glass {
                    image: group,
                    rect: visible,
                };
            }
            // A later blur reads this one's result over the whole region. The margin around the layer has no grain; the
            // layer's own rectangle is written with it, counted from the layer's corner.
            let next = self.images.temporary(size);
            let origin = Rect::new(
                layer.quad.x - region.x,
                layer.quad.y - region.y,
                layer.quad.width,
                layer.quad.height,
            );
            let mut quads: Vec<QuadDraw> = Vec::new();
            let whole = Rect::new(0, 0, size.width, size.height);
            let inside = |quad: Rect| {
                let mut draw = resolve(
                    Rect::new(quad.x + region.x, quad.y + region.y, quad.width, quad.height),
                    0.0,
                    vertical,
                );
                draw.quad = quad;
                draw
            };
            if blur.noise > 0.0 {
                quads.extend(around(whole, origin).into_iter().map(inside));
                let mut grain = inside(origin);
                grain.uniforms.grade[3] = blur.noise;
                quads.push(grain);
            } else {
                quads.push(inside(whole));
            }
            self.job(index, next, size, &quads);
            self.images.release(swept);
            source = next;
        }
        unreachable!("a glass has at least one blur")
    }

    /// The picture of a layer: its pieces, blurred, in an image of `extent`. Returns the picture.
    fn picture(&mut self, shaders: &Shaders, layer: &MeasuredLayer, extent: Rect, filter: FilterParams) -> BlurPicture {
        let inputs = self.inputs;
        let index = layer.index;
        let size = Size::new(extent.width, extent.height);
        let image = self.images.kept(size);
        let plan = plan_pieces(
            &inputs.frame.layers()[index],
            index,
            inputs.scale,
            inputs.sizes,
            shaders,
            extent,
            Pieces::Picture,
            &[],
        );
        self.jobs.push(BlurJob {
            index,
            image,
            size,
            background: None,
            plan,
        });
        let radius = (filter.blur * layer.factor).clamp(0.0, MAX_RADIUS);
        let step = (radius / 3.0).max(0.5);
        let second = self.images.temporary(size);
        self.job(index, second, size, &[sweep(image, size, [step, 0.0])]);
        self.job(index, image, size, &[sweep(second, size, [0.0, step])]);
        self.images.release(second);
        BlurPicture { image, extent, filter }
    }
}

/// The layers of the frame with a blur, measured.
fn measure(inputs: &Inputs<'_>) -> Vec<MeasuredLayer> {
    let output = Rect::new(0, 0, inputs.physical.width, inputs.physical.height);
    let mut found = Vec::new();
    for (index, layer) in inputs.frame.layers().iter().enumerate() {
        let effects = match layer {
            Layer::Solid(solid) => &solid.effects,
            Layer::Surface(surface) => &surface.effects,
            Layer::Overlay(_) => continue,
        };
        let blurs: Vec<BlurParams> = effects
            .passes()
            .iter()
            .filter_map(|pass| match pass {
                Pass::Blur(blur) if blur.radius > 0.0 => Some(*blur),
                _ => None,
            })
            .collect();
        let filter = effects.passes().iter().find_map(|pass| match pass {
            Pass::Filter(filter) if filter.blur > 0.0 => Some(*filter),
            _ => None,
        });
        if blurs.is_empty() && filter.is_none() {
            continue;
        }
        let Some(shape) = layer_shape(layer, index, inputs.scale, inputs.sizes) else {
            continue;
        };
        found.push(MeasuredLayer {
            index,
            quad: shape.quad,
            visible: shape.quad.intersection(output),
            blurs,
            filter,
            painted: shape.painted,
            factor: shape.factor,
        });
    }
    found
}

/// The region a layer's backdrop blur reads: its visible rectangle grown by every blur's reach, clipped to the output.
fn region_of(layer: &MeasuredLayer, visible: Rect, output: Rect) -> Rect {
    let reach: u32 = layer.blurs.iter().map(|blur| reach_of(blur, layer.factor)).sum();
    grown(visible, reach).intersection(output).unwrap_or(visible)
}

/// The rectangle of a layer's picture: what it paints grown by the blur's reach, kept to what can reach the output.
fn extent_of(layer: &MeasuredLayer, filter: &FilterParams, output: Rect) -> Option<Rect> {
    let radius = (filter.blur * layer.factor).clamp(0.0, MAX_RADIUS);
    let reach = if radius > 0.0 {
        (radius / 3.0 * 4.0).ceil() as u32 + 1
    } else {
        0
    };
    // Only what can reach the output matters: the blur spills `reach` pixels in.
    let seen = grown(output, reach);
    let painted = layer.painted.intersection(seen)?;
    Some(grown(painted, reach))
}

/// Plans the blurs of `inputs.frame` and records in `shaders` what each layer with a blur draws from.
pub(super) fn plan(inputs: &Inputs<'_>, shaders: &mut Shaders) -> BlurPlan {
    let layers = measure(inputs);
    if layers.is_empty() {
        return BlurPlan::default();
    }
    let output = Rect::new(0, 0, inputs.physical.width, inputs.physical.height);
    // The rectangles of each layer's result, and whether it can be made at all.
    struct Wanted {
        glass: Option<(Rect, Rect)>,
        picture: Option<(Rect, FilterParams)>,
    }
    let wanted: Vec<Wanted> = layers
        .iter()
        .map(|layer| {
            let glass = layer
                .visible
                .filter(|_| !layer.blurs.is_empty())
                .map(|visible| (visible, region_of(layer, visible, output)))
                .filter(|(_, region)| region.width <= inputs.limit && region.height <= inputs.limit);
            let picture = layer
                .filter
                .and_then(|filter| extent_of(layer, &filter, output).map(|extent| (extent, filter)))
                .filter(|(extent, _)| extent.width <= inputs.limit && extent.height <= inputs.limit);
            Wanted { glass, picture }
        })
        .collect();
    // From the top: a layer is needed when its result meets the damage or what another needed layer or a shader reads.
    let mut readers = damage_of(inputs.frame, inputs.scale, inputs.physical);
    readers.extend_from_slice(inputs.reads);
    let mut needed: BTreeSet<usize> = BTreeSet::new();
    for (layer, want) in layers.iter().zip(&wanted).rev() {
        let meets = |rect: Rect| readers.iter().any(|reader| reader.intersects(rect));
        let used = want.glass.is_some_and(|(visible, _)| meets(visible))
            || want.picture.is_some_and(|(extent, _)| meets(extent));
        if used {
            needed.insert(layer.index);
            if let Some((_, region)) = want.glass {
                readers.push(region);
            }
        }
    }
    let mut planner = Planner {
        inputs,
        images: Images::default(),
        jobs: Vec::new(),
    };
    for (layer, want) in layers.iter().zip(&wanted) {
        let made = want.glass.is_some() || want.picture.is_some();
        let state = if !needed.contains(&layer.index) {
            if made || layer.visible.is_none() {
                // What the blur makes meets nothing drawn this frame: only the shadows and borders around it can.
                BlurState::Skipped {
                    picture: want.picture.is_some(),
                }
            } else {
                // The images would be past what the device makes, so the layer is drawn without its blur.
                BlurState::Unavailable
            }
        } else {
            // A picture that cannot be made leaves the filter to its grade, drawn piece by piece, which the layer's normal
            // draws do; so does a glass that cannot be.
            let picture = want
                .picture
                .map(|(extent, filter)| planner.picture(shaders, layer, extent, filter));
            let glass = want.glass.map(|(visible, region)| {
                let pieces = if picture.is_some() {
                    Pieces::Glass
                } else {
                    Pieces::Group
                };
                planner.glass(shaders, layer, visible, region, pieces)
            });
            BlurState::Drawn { glass, picture }
        };
        shaders.blurs.insert(layer.index, state);
    }
    BlurPlan {
        images: planner.images.all,
        jobs: planner.jobs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render_frame::frame::Solid;

    const SIZE: Size = Size::new(64, 64);

    fn glass(dst: Rect, radius: f32) -> Layer {
        let mut layer = Solid::new(dst, Color::TRANSPARENT);
        layer
            .effects
            .push(Pass::Blur(BlurParams {
                radius,
                passes: 2,
                downscale: 1.0,
                ..BlurParams::default()
            }))
            .expect("a blur");
        Layer::Solid(layer)
    }

    fn frame(damage: Rect, layers: Vec<Layer>) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.push_damage(damage);
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 64, 64), Color::WHITE)));
        for layer in layers {
            frame.push_layer(layer);
        }
        frame
    }

    fn planned(frame: &OutputFrame, reads: &[Rect], limit: u32) -> (BlurPlan, Shaders) {
        let sizes = TextureSizes {
            layers: vec![None; frame.layers().len()],
            ..TextureSizes::default()
        };
        let mut shaders = Shaders::default();
        let blurs = plan(
            &Inputs {
                frame,
                scale: Scale::IDENTITY,
                physical: SIZE,
                sizes: &sizes,
                limit,
                reads,
            },
            &mut shaders,
        );
        (blurs, shaders)
    }

    #[test]
    fn a_blurred_layer_the_damage_meets_is_drawn_through_a_backdrop_sweeps_and_a_group() {
        let frame = frame(Rect::new(0, 0, 64, 64), vec![glass(Rect::new(24, 24, 16, 16), 6.0)]);
        let (blurs, shaders) = planned(&frame, &[], 1000);
        let Some(BlurState::Drawn {
            glass: Some(glass),
            picture: None,
        }) = shaders.blurs.get(&1)
        else {
            panic!("the layer has a glass: {:?}", shaders.blurs);
        };
        assert_eq!(
            glass.rect,
            Rect::new(24, 24, 16, 16),
            "the group is the layer's visible part"
        );
        // A radius of 6 spreads the taps 2 pixels, so the reach of a sweep is 4 * 2 + 1 = 9 (rounded up from 8.33) and two
        // sweeps reach 18 each way.
        let first = &blurs.jobs[0];
        assert_eq!(
            first.size,
            Size::new(52, 52),
            "the backdrop is the layer grown by its reach, kept to the output"
        );
        assert!(first.background.is_some(), "and starts as the frame's background");
        // The backdrop, a horizontal and a vertical sweep, a horizontal sweep, and the group.
        assert_eq!(blurs.jobs.len(), 5);
        assert!(
            blurs.jobs.iter().all(|job| job.index == 1),
            "every pass is for the layer"
        );
    }

    #[test]
    fn sweeps_use_the_images_the_backdrop_and_the_last_sweep_left() {
        let frame = frame(Rect::new(0, 0, 64, 64), vec![glass(Rect::new(8, 8, 48, 48), 6.0)]);
        let (blurs, _) = planned(&frame, &[], 1000);
        // The backdrop and one image to sweep into are enough for any number of sweeps; the group is the third.
        assert_eq!(blurs.images.len(), 3, "{:?}", blurs.images);
    }

    #[test]
    fn a_blurred_layer_nothing_reads_is_not_planned() {
        let layer = glass(Rect::new(40, 40, 20, 20), 6.0);
        let frame = frame(Rect::new(0, 0, 8, 8), vec![layer]);
        let (blurs, shaders) = planned(&frame, &[], 1000);
        assert!(blurs.jobs.is_empty() && blurs.images.is_empty());
        assert_eq!(shaders.blurs.get(&1), Some(&BlurState::Skipped { picture: false }));
        // What a shader's backdrop covers is a reader.
        let (blurs, shaders) = planned(&frame, &[Rect::new(30, 30, 20, 20)], 1000);
        assert!(!blurs.jobs.is_empty());
        assert!(matches!(shaders.blurs.get(&1), Some(BlurState::Drawn { .. })));
    }

    #[test]
    fn a_blur_under_the_one_the_damage_meets_is_drawn_when_the_upper_reads_it() {
        let lower = glass(Rect::new(0, 0, 30, 64), 4.0);
        let upper = glass(Rect::new(34, 0, 30, 64), 6.0);
        // The damage is on the upper layer alone, but the upper layer's backdrop reaches the lower one.
        let frame = frame(Rect::new(50, 0, 10, 64), vec![lower, upper]);
        let (_, shaders) = planned(&frame, &[], 1000);
        assert!(
            matches!(shaders.blurs.get(&2), Some(BlurState::Drawn { .. })),
            "the upper layer is drawn"
        );
        assert!(
            matches!(shaders.blurs.get(&1), Some(BlurState::Drawn { .. })),
            "so is the lower, whose glass is in the upper's backdrop: {:?}",
            shaders.blurs
        );
        // Far apart, the lower one is not read.
        let upper = glass(Rect::new(60, 0, 4, 64), 1.0);
        let frame = self::frame(
            Rect::new(60, 0, 4, 64),
            vec![glass(Rect::new(0, 0, 10, 64), 1.0), upper],
        );
        let (_, shaders) = planned(&frame, &[], 1000);
        assert_eq!(shaders.blurs.get(&1), Some(&BlurState::Skipped { picture: false }));
    }

    #[test]
    fn a_layer_cut_by_the_output_has_a_group_of_its_visible_part_and_grain_from_its_corner() {
        let mut layer = Solid::new(Rect::new(-10, 20, 30, 20), Color::TRANSPARENT);
        layer
            .effects
            .push(Pass::Blur(BlurParams {
                noise: 0.5,
                ..BlurParams::default()
            }))
            .expect("a blur");
        let frame = frame(Rect::new(0, 0, 64, 64), vec![Layer::Solid(layer)]);
        let (blurs, shaders) = planned(&frame, &[], 1000);
        let Some(BlurState::Drawn { glass: Some(glass), .. }) = shaders.blurs.get(&1) else {
            panic!("a glass");
        };
        assert_eq!(glass.rect, Rect::new(0, 20, 20, 20), "the part on the output");
        let group = blurs.jobs.last().expect("the group");
        assert_eq!(group.size, Size::new(20, 20));
        // The write is drawn over the layer's own rectangle, which starts off the output, so the grain counts from there.
        let first = group.plan.instances[..4].to_vec();
        assert_eq!(
            first,
            [-10.0, 0.0, 30.0, 20.0],
            "the quad is the layer's, shifted to the image's origin"
        );
    }

    #[test]
    fn a_picture_too_large_for_the_device_is_left_to_the_grade_alone() {
        let mut layer = Solid::new(Rect::new(8, 8, 48, 48), Color::WHITE);
        layer
            .effects
            .push(Pass::Filter(FilterParams {
                blur: 20.0,
                ..FilterParams::default()
            }))
            .expect("a filter");
        let frame = frame(Rect::new(0, 0, 64, 64), vec![Layer::Solid(layer)]);
        let (_, shaders) = planned(&frame, &[], 1000);
        assert!(matches!(
            shaders.blurs.get(&1),
            Some(BlurState::Drawn { picture: Some(_), .. })
        ));
        let (blurs, shaders) = planned(&frame, &[], 64);
        assert!(blurs.jobs.is_empty());
        assert_eq!(
            shaders.blurs.get(&1),
            Some(&BlurState::Unavailable),
            "drawn as the layer's kind says"
        );
    }

    #[test]
    fn a_picture_holds_what_the_layer_paints_grown_by_the_reach() {
        let mut layer = Solid::new(Rect::new(20, 20, 16, 16), Color::WHITE);
        layer
            .effects
            .push(Pass::Filter(FilterParams {
                blur: 6.0,
                ..FilterParams::default()
            }))
            .expect("a filter");
        let frame = frame(Rect::new(0, 0, 64, 64), vec![Layer::Solid(layer)]);
        let (blurs, shaders) = planned(&frame, &[], 1000);
        let Some(BlurState::Drawn {
            picture: Some(picture),
            glass: None,
        }) = shaders.blurs.get(&1)
        else {
            panic!("a picture");
        };
        // A radius of 6 is taps 2 pixels apart, a reach of 4 * 2 + 1 = 9 (rounded up from 8.33).
        assert_eq!(picture.extent, Rect::new(11, 11, 34, 34));
        assert_eq!(blurs.jobs.len(), 3, "the pieces, a horizontal sweep and a vertical one");
    }
}
