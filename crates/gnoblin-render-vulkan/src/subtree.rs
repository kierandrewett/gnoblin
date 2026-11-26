#![allow(unsafe_code)]
//! A View's subtree drawn into an image, so a live View costs no CPU redraw.
//!
//! The composer describes a View as a [`ViewScene`]: the layers of its source through the camera, per eye. The renderer draws
//! the scene into an image it owns, and the layer that shows the View samples that image. Nothing is read back for the frame
//! that shows it. `gnoblin-render-gles`'s `subtree.rs` is the other renderer's half of this contract and the two are tested
//! against the same cases.
//!
//! # What the picture is
//!
//! One `RGBA8` colour image per View, the size of its picture, cleared to opaque black, and a depth-stencil image
//! ([`DepthTarget`]) whose stencil nests the clipping ancestors (`spatial.rs`). Both come from the allocator. The picture is
//! kept until the scene's revision changes, its size changes, or the View is no longer on the frame's `live_views`; it is then
//! given to the retire queue with the timeline value of the last frame that used it, so a frame still in flight never loses an
//! image it samples. The store is the device's: two outputs showing one View share one picture, and a revision is drawn once.
//!
//! A capture that must not show a secure node reads a second picture drawn from the scene's capture variant. It is made only
//! when a capture asks ([`Vulkan::view_pixels`]) and only for a scene that has a variant.
//!
//! # Where the draw goes
//!
//! A scene is drawn by the frame that first needs it: its draws join the frame's buffers like a backdrop's, and its render pass
//! is recorded before every pass that samples a View. A read ([`Vulkan::view_pixels`]) is a frame with no layers that draws
//! the scene, followed by a copy of the picture into a host buffer, which is the only place a picture reaches the CPU.
//!
//! # Failure modes
//!
//! 1. **An image or a draw is refused** (a size past the device's limit, no memory, no depth-stencil format). The View is not
//!    drawn: its layer samples nothing and is skipped, the rest of the frame is drawn, and the failure is logged once per
//!    revision. The revision is not tried again, and a new one asks again:
//!    `tests::a_view_the_device_refuses_leaves_the_frame_alone`.
//! 2. **The View is destroyed.** Its id leaves the frame's `live_views` and the next frame drawn gives its images back:
//!    `tests::a_destroyed_view_gives_its_images_back`.
//! 3. **An unchanged View is drawn again.** The revision and size are compared first: `tests::an_unchanged_view_is_not_drawn_again`.
//! 4. **A secure node is in a capture.** The capture reads the scene's capture variant, never the display picture:
//!    `tests::a_capture_reads_the_scenes_redacted_eyes`.
//! 5. **A frame fails after the store decided a picture was drawn.** The pictures that frame meant to draw are dropped, so the
//!    next frame draws them: see [`ViewStore::invalidate`].

use std::collections::HashMap;
use std::rc::Rc;

use ash::vk;
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{Layer, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shader::uniforms::Uniforms;
use gnoblin_render_frame::view_scene::{ViewEye, ViewOp, ViewScene, size_fits};
use gnoblin_types::{Color, Rect, Scale, Size};

use super::Vulkan;
use super::context::{TARGET_FORMAT, VkContext};
use super::frame::Resting;
use super::memory::VkImage;
use super::plan::{Draws, FramePlan, PassKind, PlanTarget, TextureSlot, quad_of, texture_mapping};
use super::spatial::{
    DepthExtras, DepthTarget, DrawVariant, MAX_CLIP_DEPTH, Placed, layer_dst, layer_model, place, screen_matrix,
};
use super::sync::RetireQueue;
use crate::log;
use gnoblin_render_frame::frame::OutputFrame;

/// A scene to have a picture of, and which of its two variants.
#[derive(Clone, Copy)]
pub(super) struct ViewRequest<'a> {
    pub scene: &'a ViewScene,
    /// Whether the picture is the capture variant, which is only ever true for a scene that has one.
    pub capture: bool,
}

/// What the store holds of one picture: its two images.
struct PictureImages {
    // The framebuffer is destroyed before the images it is over.
    depth: DepthTarget,
    colour: VkImage,
}

impl PictureImages {
    fn new(context: &Rc<VkContext>, extras: &DepthExtras, size: Size) -> Result<Self, vk::Result> {
        let colour = VkImage::new(
            context,
            size.width,
            size.height,
            TARGET_FORMAT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC,
            false,
        )?;
        let depth = DepthTarget::over(context, extras, &colour)?;
        Ok(Self { depth, colour })
    }

    fn size(&self) -> Size {
        Size::new(self.colour.width(), self.colour.height())
    }
}

enum ViewSlot {
    /// The picture, drawn at `revision`, last used by the frame that signals `used`.
    Ready {
        picture: Box<PictureImages>,
        revision: u64,
        used: u64,
    },
    /// The device refused `revision` at `size`; it is not asked again until the scene changes.
    Refused { revision: u64, size: Size },
}

/// What [`ViewStore::ensure`] decided for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnsureOutcome {
    /// The picture holds the scene's revision already.
    Current,
    /// The picture is to be drawn by the frame being recorded.
    Draw,
    /// The picture cannot be had.
    Refused,
}

/// The handles a frame draws a picture with.
#[derive(Clone, Copy)]
pub(super) struct PictureHandles {
    pub colour: vk::Image,
    pub framebuffer: vk::Framebuffer,
    pub size: Size,
}

/// The pictures of the Views a device has drawn, by View and whether the picture is the capture variant.
#[derive(Default)]
pub(super) struct ViewStore {
    slots: HashMap<(u64, bool), ViewSlot>,
    /// How many scenes were drawn, which a test reads to prove an unchanged View costs nothing.
    drawn: u64,
}

impl ViewStore {
    /// How many scenes this device has drawn into a picture.
    pub(super) fn drawn(&self) -> u64 {
        self.drawn
    }

    /// How many pictures are held.
    pub(super) fn len(&self) -> usize {
        self.slots
            .values()
            .filter(|slot| matches!(slot, ViewSlot::Ready { .. }))
            .count()
    }

    /// The nominal bytes of the held pictures, depth-stencil images included.
    pub(super) fn bytes(&self) -> u64 {
        self.slots
            .values()
            .filter_map(|slot| match slot {
                ViewSlot::Ready { picture, .. } => {
                    Some(picture.colour.bytes().saturating_add(picture.depth.depth.bytes()))
                }
                ViewSlot::Refused { .. } => None,
            })
            .fold(0, u64::saturating_add)
    }

    /// The view and size of a View's display picture, when the store holds one.
    pub(super) fn sampled(&self, id: u64) -> Option<(vk::ImageView, (u32, u32))> {
        match self.slots.get(&(id, false))? {
            ViewSlot::Ready { picture, .. } => {
                Some((picture.colour.view(), (picture.colour.width(), picture.colour.height())))
            }
            ViewSlot::Refused { .. } => None,
        }
    }

    /// Whether the store holds a picture of the View's display.
    #[cfg(test)]
    pub(super) fn holds(&self, id: u64) -> bool {
        self.sampled(id).is_some()
    }

    /// The handles to draw into a picture, or to read it.
    pub(super) fn handles(&self, key: (u64, bool)) -> Option<PictureHandles> {
        match self.slots.get(&key)? {
            ViewSlot::Ready { picture, .. } => Some(PictureHandles {
                colour: picture.colour.raw(),
                framebuffer: picture.depth.framebuffer,
                size: picture.size(),
            }),
            ViewSlot::Refused { .. } => None,
        }
    }

    /// Notes that the frame signalling `value` samples the display picture of `id`.
    pub(super) fn touch(&mut self, id: u64, value: u64) {
        if let Some(ViewSlot::Ready { used, .. }) = self.slots.get_mut(&(id, false)) {
            *used = (*used).max(value);
        }
    }

    /// Gives back the pictures of every View not in `alive`, display and capture alike.
    pub(super) fn retire_except(&mut self, alive: &[u64], retire: &mut RetireQueue) {
        let gone: Vec<(u64, bool)> = self
            .slots
            .keys()
            .copied()
            .filter(|(id, _)| !alive.contains(id))
            .collect();
        self.remove(&gone, retire);
    }

    /// Drops the pictures a frame that failed meant to draw, which would otherwise be taken for drawn.
    pub(super) fn invalidate(&mut self, keys: &[(u64, bool)], retire: &mut RetireQueue) {
        self.remove(keys, retire);
    }

    fn remove(&mut self, keys: &[(u64, bool)], retire: &mut RetireQueue) {
        for key in keys {
            if let Some(ViewSlot::Ready { picture, used, .. }) = self.slots.remove(key) {
                retire.push(used, picture);
            }
        }
    }

    /// Decides whether `request` needs a draw, and makes the images for it when it does.
    ///
    /// `value` is the timeline value of the frame that will draw it.
    pub(super) fn ensure(
        &mut self,
        request: ViewRequest<'_>,
        context: &Rc<VkContext>,
        extras: Option<&DepthExtras>,
        value: u64,
        retire: &mut RetireQueue,
    ) -> EnsureOutcome {
        let scene = request.scene;
        let key = (scene.id(), request.capture);
        match self.slots.get_mut(&key) {
            Some(ViewSlot::Ready {
                picture,
                revision,
                used,
            }) if *revision == scene.revision() && picture.size() == scene.size() => {
                *used = (*used).max(value);
                return EnsureOutcome::Current;
            }
            Some(ViewSlot::Refused { revision, size }) if *revision == scene.revision() && *size == scene.size() => {
                return EnsureOutcome::Refused;
            }
            _ => {}
        }
        // A picture of the wrong size is replaced, not resized: its pixels are of another picture.
        let kept = match self.slots.remove(&key) {
            Some(ViewSlot::Ready { picture, used, .. }) if picture.size() == scene.size() => Some((picture, used)),
            Some(ViewSlot::Ready { picture, used, .. }) => {
                retire.push(used, picture);
                None
            }
            _ => None,
        };
        let made = match (kept, extras) {
            (Some((picture, used)), _) => Ok((picture, used)),
            (None, Some(extras)) if size_fits(scene.size()) => {
                PictureImages::new(context, extras, scene.size()).map(|picture| (Box::new(picture), 0))
            }
            (None, Some(_)) => Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY),
            (None, None) => Err(vk::Result::ERROR_FORMAT_NOT_SUPPORTED),
        };
        match made {
            Ok((picture, used)) => {
                self.drawn = self.drawn.saturating_add(1);
                self.slots.insert(
                    key,
                    ViewSlot::Ready {
                        picture,
                        revision: scene.revision(),
                        used: used.max(value),
                    },
                );
                EnsureOutcome::Draw
            }
            Err(result) => {
                log::warn!(
                    area = "render",
                    view = scene.id(),
                    width = scene.size().width,
                    height = scene.size().height,
                    ?result,
                    "a View could not be drawn into an image; it shows nothing until its next change"
                );
                self.slots.insert(
                    key,
                    ViewSlot::Refused {
                        revision: scene.revision(),
                        size: scene.size(),
                    },
                );
                EnsureOutcome::Refused
            }
        }
    }
}

/// The surface layers of a scene that are drawn from a texture the cache makes, in the order the scene lists them, each with
/// its running number. A layer placed flat, one that is not a surface, and one that shows another View are left out: the
/// first is not drawn, the second has no texture of its own, and the third samples the store.
pub(super) fn scene_surfaces(eyes: &[ViewEye]) -> impl Iterator<Item = (usize, &Surface)> {
    eyes.iter()
        .flat_map(|eye| eye.ops.iter())
        .filter_map(|op| match op {
            ViewOp::Draw(Layer::Surface(surface))
                if surface.model.is_some() && !matches!(surface.source, SurfaceSource::View(_)) =>
            {
                Some(surface)
            }
            _ => None,
        })
        .enumerate()
}

/// The views a scene samples: the layers of it that show another View.
pub(super) fn scene_views(eyes: &[ViewEye]) -> impl Iterator<Item = u64> {
    eyes.iter().flat_map(|eye| eye.ops.iter()).filter_map(|op| match op {
        ViewOp::Draw(Layer::Surface(Surface {
            source: SurfaceSource::View(view),
            ..
        })) => Some(view.id),
        _ => None,
    })
}

/// What a scene's draws are made from: the sizes of the textures its layers have, and of the Views it samples.
pub(super) struct SceneTextures<'a> {
    /// The scene's number among the ones this frame draws, which names its texture slots.
    pub scene: usize,
    pub sizes: &'a [(TextureSlot, (u32, u32))],
    /// The size of the picture of each View the frame samples.
    pub views: &'a [(u64, (u32, u32))],
}

/// Plans the draws of a scene into a picture of `size`: each eye's layers in painter's order, and the stencil writes of its
/// clipping ancestors in between.
pub(super) fn plan_scene(eyes: &[ViewEye], size: Size, textures: &SceneTextures<'_>) -> FramePlan {
    let mut built = FramePlan::default();
    let mut draws = Draws::new(PlanTarget::whole(size));
    let picture = Rect::from_size(size);
    let mut surfaces = 0;
    for eye in eyes {
        let Some(scissor) = eye.viewport.intersection(picture) else {
            continue;
        };
        let screen = screen_matrix(eye.viewport.size());
        let placed = |model: &gnoblin_render_frame::frame::Model, dst: Rect, variant: DrawVariant| {
            let (columns, depth) = place(&screen, model, dst);
            Placed {
                columns,
                depth,
                variant,
                viewport: Some(eye.viewport),
            }
        };
        let mut depth: u8 = 0;
        for op in &eye.ops {
            match op {
                ViewOp::Draw(layer) => {
                    let Some(model) = layer_model(layer) else {
                        continue;
                    };
                    let dst = layer_dst(layer);
                    let quad = quad_of(dst, Scale::IDENTITY);
                    let placement = placed(model, dst, DrawVariant::inside(depth));
                    match layer {
                        Layer::Solid(solid) => {
                            let mut uniforms = Uniforms::solid(quad, solid.colour, solid.tint, 1.0);
                            uniforms.corner = solid.corner_radius;
                            draws.push_placed(&mut built, PassKind::Solid, None, scissor, quad, &uniforms, placement);
                        }
                        Layer::Surface(surface) => {
                            let (slot, size) = match &surface.source {
                                SurfaceSource::View(view) => {
                                    let Some((_, size)) = textures.views.iter().find(|(id, _)| *id == view.id) else {
                                        continue;
                                    };
                                    (TextureSlot::View(view.id), *size)
                                }
                                _ => {
                                    let slot = TextureSlot::Scene(textures.scene, surfaces);
                                    surfaces += 1;
                                    let Some((_, size)) = textures.sizes.iter().find(|(held, _)| *held == slot) else {
                                        continue;
                                    };
                                    (slot, *size)
                                }
                            };
                            let uniforms = Uniforms::surface(
                                quad,
                                texture_mapping(surface, size),
                                surface.tint,
                                surface.opacity,
                                surface.corner_radius,
                            );
                            draws.push_placed(
                                &mut built,
                                PassKind::Surface,
                                Some(slot),
                                scissor,
                                quad,
                                &uniforms,
                                placement,
                            );
                        }
                        Layer::Overlay(_) => {}
                    }
                }
                ViewOp::PushClip(plane) if depth < MAX_CLIP_DEPTH => {
                    depth += 1;
                    clip(
                        &mut draws,
                        &mut built,
                        plane.dst,
                        scissor,
                        placed(&plane.model, plane.dst, DrawVariant::push(depth)),
                    );
                }
                ViewOp::PopClip(plane) if depth > 0 => {
                    clip(
                        &mut draws,
                        &mut built,
                        plane.dst,
                        scissor,
                        placed(&plane.model, plane.dst, DrawVariant::pop(depth)),
                    );
                    depth -= 1;
                }
                // A chain deeper than the stencil counts is left unclipped past its limit, and its pops with it.
                ViewOp::PushClip(_) | ViewOp::PopClip(_) => {}
            }
        }
    }
    built.layers_end = built.draws.len();
    built.models_end = built.layers_end;
    built.overlay_end = built.layers_end;
    built
}

/// Writes (or erases) a clip's rectangle in the stencil, drawing no colour.
fn clip(draws: &mut Draws, built: &mut FramePlan, dst: Rect, scissor: Rect, placement: Placed) {
    let quad = quad_of(dst, Scale::IDENTITY);
    if quad.is_empty() {
        return;
    }
    let uniforms = Uniforms::solid(quad, Color::BLACK, Color::WHITE, 1.0);
    draws.push_placed(built, PassKind::Solid, None, scissor, quad, &uniforms, placement);
}

impl Vulkan {
    /// How many View scenes this device has drawn into a picture.
    #[must_use]
    pub fn subtree_draws(&self) -> u64 {
        self.device.views.borrow().drawn()
    }

    /// How many View pictures this device holds now.
    #[must_use]
    pub fn subtree_textures(&self) -> usize {
        self.device.views.borrow().len()
    }

    /// Gives back the pictures of every View not in `alive`. The images go when the frames that used them have finished.
    pub fn retire_views(&mut self, alive: &[u64]) {
        let mut retire = self.device.retire.borrow_mut();
        self.device.views.borrow_mut().retire_except(alive, &mut retire);
        retire.release(self.device.context.completed());
    }

    /// [`Vulkan::view_pixels`] for a host that draws no frame on this renderer: `live` names every View alive on the GPU
    /// path, and the pictures of the others are given back first, so a View that is gone gives its images back whether or
    /// not a frame is ever drawn here.
    ///
    /// # Errors
    ///
    /// As [`Vulkan::view_pixels`].
    pub fn read_view(&mut self, scene: &ViewScene, capture: bool, live: &[u64]) -> Result<Image, RenderError> {
        let alive: Vec<u64> = live.iter().copied().chain([scene.id()]).collect();
        self.retire_views(&alive);
        self.view_pixels(scene, capture)
    }

    /// Reads a View's picture back as an [`Image`]: the capture variant when `capture` is set and the scene has one, the
    /// display picture otherwise. The scene is drawn first when the device does not hold its current revision.
    ///
    /// The image is opaque, top-left first, in the byte order every readback has, so it compares with the picture the CPU
    /// path draws.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the device refuses the picture or the read, [`RenderError::Lost`] when it is gone.
    pub fn view_pixels(&mut self, scene: &ViewScene, capture: bool) -> Result<Image, RenderError> {
        if self.is_lost() {
            return Err(RenderError::Lost);
        }
        let capture = capture && scene.has_capture_variant();
        let key = (scene.id(), capture);
        let requests = [ViewRequest { scene, capture }];
        let painted = self.paint_with(&OutputFrame::new(), Scale::IDENTITY, &requests, true);
        self.collect();
        painted.map_err(|result| self.fail(result))?;
        let Some(handles) = self.device.views.borrow().handles(key) else {
            return Err(RenderError::Failed);
        };
        let size = handles.size;
        let read = self.read_back(handles.colour, size, Resting::sampled());
        let bytes = read.map_err(|result| self.fail(result))?;
        self.trim();
        Image::new(size, size.width as usize * 4, bytes).ok_or(RenderError::Failed)
    }
}
