//! A View's subtree drawn into a texture, so a live View costs no CPU redraw.
//!
//! The CPU path of a View (`gnoblin_render_software::render_rig`) rasterises the source subtree per pixel on the
//! compositor's thread and uploads the result every time it changes. This module draws the same subtree on the GPU:
//! the composer describes it as a [`ViewScene`] (the layers of the source through the camera, per eye), the renderer
//! draws the scene into an offscreen texture it owns, and the layer that shows the View samples that texture. Nothing
//! is read back for the frame that shows it.
//!
//! # What the texture is
//!
//! One texture per View, the size of the View's picture, `RGBA8`, opaque black where nothing is drawn, with the stencil
//! buffer that nests the clipping ancestors. It is kept until the scene's `revision` changes, the size changes, or the
//! View is no longer alive on the frame's `live_views` list, and is then deleted: Views come and go for as long as the
//! session runs, so unlike the output's target a texture here is given back one by one
//! ([`GlContext::release`](crate::gl::GlContext::release)).
//!
//! A capture that must not show a secure node reads a second texture, drawn from the scene's capture variant. It is
//! made only when a capture asks ([`Gles::view_pixels`]) and only for a scene that has a variant, so a View without a
//! secure node holds one texture.
//!
//! # The only CPU copy
//!
//! [`Gles::view_pixels`] reads a texture back for a consumer that needs bytes: a capture, a PipeWire stream, the
//! XR extension (which reads a View through the capture protocol into a `wl_shm` buffer) and a still. It is the only
//! place the picture reaches the CPU. The composer asks for it only for a View a consumer is reading, and only once per
//! revision.
//!
//! # Failure modes
//!
//! 1. **A texture or draw fails** (a size past the driver's limit, a refused framebuffer, a lost context). The View is
//!    not drawn: its layer samples nothing and is skipped, the rest of the frame is drawn, and the failure is logged
//!    once per revision. The revision is not tried again, so a driver that refuses it is asked once, and a new revision
//!    asks again: `tests::a_view_the_driver_refuses_leaves_the_frame_alone`.
//! 2. **The View is destroyed.** Its id leaves the frame's `live_views` and the next frame drawn deletes its textures:
//!    `tests::a_destroyed_view_gives_its_texture_back`.
//! 3. **An unchanged View is drawn again.** The revision and size are compared first, so a frame that changes
//!    something else draws no View: `tests::an_unchanged_view_is_not_drawn_again`.
//! 4. **A secure node is in a capture.** The capture reads the scene's capture variant, never the display texture:
//!    `tests::a_capture_reads_the_scenes_redacted_eyes`.

use std::collections::HashMap;

use gnoblin_render::Mat4;
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{Layer, Model, OutputFrame, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shader::uniforms::Uniforms;
use gnoblin_render_frame::view_scene::{ClipPlane, ViewEye, ViewOp, ViewScene};
use gnoblin_types::{Color, Rect, Scale, Size};

use crate::gl::{Depth, Framebuffer, QuadPlacement, Stencil, Texture};
use crate::{Gles, Painter, flip_rows, log};

/// The deepest chain of clipping ancestors a View draws: the stencil buffer is eight bits. A chain past it draws the
/// deeper nodes unclipped by the extra ancestors, which is as far as the stencil can count.
const MAX_CLIP_DEPTH: u8 = 254;

/// What the renderer holds of one View's picture.
enum Slot {
    /// The texture, drawn at `revision`.
    Ready { target: Framebuffer, revision: u64 },
    /// The driver refused `revision` at `size`; it is not asked again until the scene changes.
    Refused { revision: u64, size: Size },
}

/// The textures of the Views this renderer has drawn, keyed by the View's id and whether the texture is the
/// capture variant.
#[derive(Default)]
pub(crate) struct Subtrees {
    slots: HashMap<(u64, bool), Slot>,
    /// How many scenes were drawn into a texture, which a test reads to prove an unchanged View costs nothing.
    drawn: u64,
}

impl Gles {
    /// How many View scenes this renderer has drawn into a texture.
    #[must_use]
    pub fn subtree_draws(&self) -> u64 {
        self.subtrees.drawn
    }

    /// How many View textures this renderer holds now.
    #[must_use]
    pub fn subtree_textures(&self) -> usize {
        self.subtrees
            .slots
            .values()
            .filter(|slot| matches!(slot, Slot::Ready { .. }))
            .count()
    }

    /// Draws the scenes of `frame` into their textures, and drops the textures of the Views the frame says are gone.
    ///
    /// A failure is logged and leaves the View without a picture for this frame: it never fails the frame.
    pub(crate) fn prepare_subtrees(&mut self, frame: &OutputFrame) {
        let alive: Vec<u64> = frame
            .live_views()
            .iter()
            .copied()
            .chain(frame.view_scenes().iter().map(|scene| scene.id()))
            .collect();
        self.retire_views(&alive);
        for scene in frame.view_scenes() {
            // `ensure` has logged the reason; the frame goes on without this View's picture.
            let _ = self.ensure_subtree(scene, false);
        }
    }

    /// Deletes the textures of every View not in `alive`, so a View that is gone gives its texture back.
    pub fn retire_views(&mut self, alive: &[u64]) {
        let gone: Vec<(u64, bool)> = self
            .subtrees
            .slots
            .keys()
            .copied()
            .filter(|(id, _)| !alive.contains(id))
            .collect();
        for key in gone {
            if let Some(Slot::Ready { target, .. }) = self.subtrees.slots.remove(&key) {
                self.device.release(target);
            }
        }
    }

    /// [`Gles::view_pixels`] for a host that draws no frame on this renderer: `live` names every View alive on the
    /// GPU path, and the textures of the others are deleted first, so a View that is gone gives its texture back
    /// whether or not a frame is ever drawn here.
    ///
    /// # Errors
    ///
    /// As [`Gles::view_pixels`].
    pub fn read_view(&mut self, scene: &ViewScene, capture: bool, live: &[u64]) -> Result<Image, RenderError> {
        let alive: Vec<u64> = live.iter().copied().chain([scene.id()]).collect();
        self.retire_views(&alive);
        self.view_pixels(scene, capture)
    }

    /// The texture of a View's display picture, when the renderer holds one.
    pub(crate) fn subtree_texture(&self, id: u64) -> Option<Texture> {
        match self.subtrees.slots.get(&(id, false))? {
            Slot::Ready { target, .. } => Some(target.texture()),
            Slot::Refused { .. } => None,
        }
    }

    /// Reads a View's picture back as an [`Image`]: the capture variant when `capture` is set and the scene has one,
    /// the display picture otherwise. The scene is drawn first when the renderer does not hold its current revision.
    ///
    /// The image is opaque, top-left first, in the byte order every readback has, so it compares with the picture the
    /// CPU path draws.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the texture or the read, and [`RenderError::Degenerate`] for a
    /// scene with no pixels.
    pub fn view_pixels(&mut self, scene: &ViewScene, capture: bool) -> Result<Image, RenderError> {
        let capture = capture && scene.has_capture_variant();
        self.ensure_subtree(scene, capture)?;
        let Some(Slot::Ready { target, .. }) = self.subtrees.slots.get(&(scene.id(), capture)) else {
            return Err(RenderError::Failed);
        };
        let size = target.size();
        let mut bytes = self.device.read(target)?;
        let stride = size.width as usize * 4;
        let expected = stride.saturating_mul(size.height as usize);
        if bytes.len() < expected {
            return Err(RenderError::Failed);
        }
        bytes.truncate(expected);
        // `glReadPixels` starts at the bottom row; an `Image` starts at the top.
        flip_rows(&mut bytes, stride);
        Image::new(size, stride, bytes).ok_or(RenderError::Failed)
    }

    /// Makes sure the texture of `scene` (its capture variant when `capture`) holds the scene's current revision.
    fn ensure_subtree(&mut self, scene: &ViewScene, capture: bool) -> Result<(), RenderError> {
        let key = (scene.id(), capture);
        match self.subtrees.slots.get(&key) {
            Some(Slot::Ready { target, revision })
                if *revision == scene.revision() && target.size() == scene.size() =>
            {
                return Ok(());
            }
            Some(Slot::Refused { revision, size }) if *revision == scene.revision() && *size == scene.size() => {
                return Err(RenderError::Failed);
            }
            _ => {}
        }
        // A texture of the wrong size is replaced, not resized: its pixels are of another picture.
        let kept = match self.subtrees.slots.remove(&key) {
            Some(Slot::Ready { target, revision }) if target.size() == scene.size() => Some((target, revision)),
            Some(Slot::Ready { target, .. }) => {
                self.device.release(target);
                None
            }
            _ => None,
        };
        let drawn = match kept {
            Some((target, _)) => Ok(target),
            None => self.device.subtree_target(scene.size()),
        }
        .and_then(|target| match self.draw_subtree(scene, capture, &target) {
            Ok(()) => Ok(target),
            Err(error) => {
                self.device.release(target);
                Err(error)
            }
        });
        match drawn {
            Ok(target) => {
                self.subtrees.drawn = self.subtrees.drawn.saturating_add(1);
                self.subtrees.slots.insert(
                    key,
                    Slot::Ready {
                        target,
                        revision: scene.revision(),
                    },
                );
                Ok(())
            }
            Err(error) => {
                log::warn!(
                    area = "render",
                    view = scene.id(),
                    width = scene.size().width,
                    height = scene.size().height,
                    ?error,
                    "a View could not be drawn into a texture; it shows nothing until its next change"
                );
                self.subtrees.slots.insert(
                    key,
                    Slot::Refused {
                        revision: scene.revision(),
                        size: scene.size(),
                    },
                );
                Err(error)
            }
        }
    }

    /// Draws every eye of `scene` into `target`, in the order the scene lists them.
    fn draw_subtree(&mut self, scene: &ViewScene, capture: bool, target: &Framebuffer) -> Result<(), RenderError> {
        let eyes = scene.eyes(capture);
        // Textures first: importing needs the renderer mutably and the pass borrows it for the whole of the draw.
        let mut owned = Vec::new();
        let textures: Vec<Vec<Option<Texture>>> = eyes
            .iter()
            .map(|eye| {
                eye.ops
                    .iter()
                    .map(|op| match op {
                        ViewOp::Draw(Layer::Surface(surface)) => self.import(surface, &mut owned),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let Self {
            device,
            scratch,
            passes,
            ..
        } = self;
        device.begin(target)?;
        device.clear_subtree(target)?;
        let size = scene.size();
        let mut cursor_under = Vec::new();
        let mut painter = Painter {
            device,
            target,
            scratch,
            passes,
            physical: size,
            scale: Scale::IDENTITY,
            background: Color::BLACK,
            bytes: Vec::new(),
            clip: Rect::from_size(size),
            cursor_under: &mut cursor_under,
        };
        for (eye, textures) in eyes.iter().zip(&textures) {
            painter.eye(eye, textures)?;
        }
        painter.finish()
    }
}

impl Painter<'_> {
    /// Draws one eye: its viewport, then each of its steps in order.
    fn eye(&mut self, eye: &ViewEye, textures: &[Option<Texture>]) -> Result<(), RenderError> {
        if eye.viewport.is_empty() {
            return Ok(());
        }
        self.device.eye(eye.viewport);
        let screen = screen_matrix(eye.viewport.size());
        let mut depth: u8 = 0;
        for (op, texture) in eye.ops.iter().zip(textures) {
            match op {
                ViewOp::Draw(layer) => {
                    let Some(model) = layer_model(layer) else {
                        continue;
                    };
                    let stencil = if depth > 0 { Stencil::Test(depth) } else { Stencil::Off };
                    let placement = placement(&screen, model, layer_dst(layer), stencil);
                    self.layer(layer, *texture, placement)?;
                }
                ViewOp::PushClip(plane) if depth < MAX_CLIP_DEPTH => {
                    depth += 1;
                    self.clip_plane(&screen, plane, Stencil::Push(depth))?;
                }
                ViewOp::PopClip(plane) if depth > 0 => {
                    self.clip_plane(&screen, plane, Stencil::Pop(depth))?;
                    depth -= 1;
                }
                // A chain deeper than the stencil counts is left unclipped past its limit, and its pops with it.
                ViewOp::PushClip(_) | ViewOp::PopClip(_) => {}
            }
        }
        Ok(())
    }

    /// Writes (or erases) a clip's rectangle in the stencil, drawing no colour.
    fn clip_plane(&mut self, screen: &Mat4, plane: &ClipPlane, stencil: Stencil) -> Result<(), RenderError> {
        let quad = plane.dst;
        if quad.is_empty() {
            return Ok(());
        }
        let Self {
            device, passes, bytes, ..
        } = self;
        let uniforms = Uniforms::solid(quad, Color::BLACK, Color::WHITE, 1.0);
        passes.solid.pack(&uniforms, bytes);
        device.draw(
            &passes.solid,
            bytes,
            quad,
            None,
            placement(screen, &plane.model, quad, stencil),
        )
    }
}

/// The matrix from an eye's pixel space (top-left first, y down) to clip space.
pub(crate) fn screen_matrix(eye: Size) -> Mat4 {
    let (w, h) = (f64::from(eye.width), f64::from(eye.height));
    Mat4 {
        m: [
            [2.0 / w, 0.0, 0.0, -1.0],
            [0.0, 2.0 / h, 0.0, -1.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// How a quad of `dst`'s size, placed by `model`, lands in the eye: through `screen`, drawn in painter's order with no
/// depth test.
fn placement(screen: &Mat4, model: &Model, dst: Rect, stencil: Stencil) -> QuadPlacement {
    let matrix = screen
        .mul(&model.matrix)
        .mul(&Mat4::scale(f64::from(dst.width), f64::from(dst.height), 1.0));
    let column = |point: [f64; 4]| {
        let v = matrix.apply(point);
        [v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
    };
    QuadPlacement {
        stencil,
        ..QuadPlacement::projected(
            [
                column([1.0, 0.0, 0.0, 0.0]),
                column([0.0, 1.0, 0.0, 0.0]),
                column([0.0, 0.0, 0.0, 1.0]),
            ],
            Depth::Off,
        )
    }
}

/// The layer's model, `None` for a layer a View's scene should never hold (one placed flat).
fn layer_model(layer: &Layer) -> Option<&Model> {
    match layer {
        Layer::Surface(Surface { model, .. }) => model.as_ref(),
        Layer::Solid(solid) => solid.model.as_ref(),
        Layer::Overlay(_) => None,
    }
}

/// The layer's rectangle in its own plane.
fn layer_dst(layer: &Layer) -> Rect {
    match layer {
        Layer::Surface(surface) => surface.dst,
        Layer::Solid(solid) => solid.dst,
        Layer::Overlay(overlay) => overlay.dst(),
    }
}

/// The mapping of a source rectangle (the picture's pixels, top-left first) of a View's own texture, which GL stores
/// from the bottom row: the v axis is flipped by a negative height, as [`crate::flipped_mapping`] states it.
pub(crate) fn picture_mapping(src: [f64; 4], texture: Texture) -> [f32; 4] {
    let (width, height) = texture.size();
    let (width, height) = (f64::from(width), f64::from(height));
    [
        (src[0] / width) as f32,
        ((height - src[1]) / height) as f32,
        (src[2] / width) as f32,
        (-src[3] / height) as f32,
    ]
}

impl Gles {
    /// Imports a View layer's texture: the one this renderer drew from the scene the frame carries.
    pub(crate) fn view_source(&self, surface: &Surface) -> Option<Texture> {
        match &surface.source {
            SurfaceSource::View(view) => self.subtree_texture(view.id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gnoblin_render_frame::frame::{Layer, Model, OutputFrame, Surface, SurfaceSource};
    use gnoblin_render_frame::view_scene::{ClipPlane, ViewEye, ViewOp, ViewScene, ViewTexture};
    use gnoblin_render_software::{ViewCamera, ViewProjection, eye_pixel_matrix};
    use gnoblin_types::{Color, Rect, RectF, Scale, Size, Vec3};

    use super::*;
    use crate::tests::gles;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    /// The flat camera the composer gives a View without one: centred on the picture, 100 units in front of it.
    fn flat(size: Size) -> Mat4 {
        eye_pixel_matrix(
            &ViewCamera {
                position: Vec3 {
                    x: f64::from(size.width) / 2.0,
                    y: f64::from(size.height) / 2.0,
                    z: 100.0,
                },
                rotation_degrees: Vec3::ZERO,
                projection: ViewProjection::Orthographic,
                near: 0.1,
                far: 10_000.0,
            },
            size,
        )
        .expect("a flat camera projects")
    }

    /// The model of a rectangle at `at` in the view's world, seen through `projection`.
    fn model(projection: &Mat4, at: Rect) -> Model {
        Model {
            matrix: projection.mul(&Mat4::translation(f64::from(at.x), f64::from(at.y), 0.0)),
            opaque: false,
        }
    }

    /// A layer that fills `at` with one opaque colour: a one-pixel picture stretched over the rectangle.
    fn fill(projection: &Mat4, at: Rect, colour: [u8; 4]) -> ViewOp {
        let picture = Image::new(Size::new(1, 1), 4, colour.to_vec()).expect("a one-pixel picture");
        ViewOp::Draw(Layer::Surface(Surface {
            source: SurfaceSource::Snapshot(Arc::new(picture)),
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            dst: Rect::new(0, 0, at.width, at.height),
            transform: smithay::utils::Transform::Normal,
            opacity: 1.0,
            tint: Color::WHITE,
            corner_radius: 0.0,
            effects: gnoblin_render::Effects::new(),
            model: Some(model(projection, at)),
            secure: false,
        }))
    }

    fn clip(projection: &Mat4, at: Rect) -> ClipPlane {
        ClipPlane {
            dst: Rect::new(0, 0, at.width, at.height),
            model: model(projection, at),
        }
    }

    fn eye(size: Size, ops: Vec<ViewOp>) -> ViewEye {
        ViewEye {
            viewport: Rect::from_size(size),
            ops,
        }
    }

    fn scene(id: u64, revision: u64, size: Size, ops: Vec<ViewOp>) -> ViewScene {
        ViewScene::new(id, revision, size, vec![eye(size, ops)], None)
    }

    fn pixel(image: &Image, x: i32, y: i32) -> [u8; 4] {
        image.colour_at(x, y).expect("a pixel inside the picture")
    }

    /// An output frame that draws the picture of the View `id` over the whole of `size`, carrying `scene`.
    fn showing(scene: Option<&ViewScene>, id: u64, size: Size) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.set_background(Color::from_rgba8(10, 20, 30, 255));
        frame.push_damage(Rect::from_size(size));
        if let Some(scene) = scene {
            frame.push_view_scene(Arc::new(scene.clone()));
        }
        frame.set_live_views(vec![id]);
        frame.push_layer(Layer::Surface(Surface {
            source: SurfaceSource::View(ViewTexture { id }),
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: f64::from(size.width),
                height: f64::from(size.height),
            },
            dst: Rect::from_size(size),
            transform: smithay::utils::Transform::Normal,
            opacity: 1.0,
            tint: Color::WHITE,
            corner_radius: 0.0,
            effects: gnoblin_render::Effects::new(),
            model: None,
            secure: false,
        }));
        frame
    }

    #[test]
    fn a_scene_is_drawn_through_a_flat_eye_top_left_first_and_in_painter_order() {
        let size = Size::new(32, 24);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let scene = scene(
            1,
            1,
            size,
            vec![
                fill(&projection, Rect::new(0, 0, 16, 8), RED),
                fill(&projection, Rect::new(8, 4, 16, 8), GREEN),
            ],
        );
        let image = renderer
            .view_pixels(&scene, false)
            .expect("the scene is drawn and read");
        assert_eq!(image.size(), size);
        assert_eq!(
            pixel(&image, 2, 2),
            RED,
            "a rectangle at the top is at the top of the picture"
        );
        assert_eq!(
            pixel(&image, 12, 6),
            GREEN,
            "the later layer is on top where they overlap, as the CPU rig draws"
        );
        assert_eq!(pixel(&image, 20, 10), GREEN);
        assert_eq!(pixel(&image, 28, 20), BLACK, "nothing drawn is opaque black");
        assert_eq!(
            pixel(&image, 2, 20),
            BLACK,
            "and the bottom of the picture is not the top"
        );
    }

    #[test]
    fn a_frame_that_shows_a_view_samples_its_texture_without_a_readback() {
        let size = Size::new(32, 24);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let scene = scene(7, 1, size, vec![fill(&projection, Rect::new(0, 0, 16, 12), BLUE)]);
        renderer
            .draw(&showing(Some(&scene), 7, size), Scale::IDENTITY)
            .expect("the frame is drawn");
        let image = renderer.screenshot().expect("the output's readback");
        assert_eq!(
            pixel(&image, 4, 4),
            BLUE,
            "the top-left quarter shows the View's rectangle"
        );
        assert_eq!(pixel(&image, 24, 16), BLACK, "the rest of the View is its black");
        assert_eq!(renderer.subtree_draws(), 1, "the scene was drawn once");
        assert_eq!(renderer.subtree_textures(), 1);
    }

    #[test]
    fn a_clip_keeps_what_is_drawn_inside_it_to_its_rectangle_and_clips_nest() {
        let size = Size::new(32, 24);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let outer = Rect::new(4, 4, 24, 16);
        let inner = Rect::new(12, 0, 20, 24);
        let scene = scene(
            1,
            1,
            size,
            vec![
                ViewOp::PushClip(clip(&projection, outer)),
                fill(&projection, Rect::new(0, 0, 32, 24), RED),
                ViewOp::PushClip(clip(&projection, inner)),
                fill(&projection, Rect::new(0, 0, 32, 24), GREEN),
                ViewOp::PopClip(clip(&projection, inner)),
                ViewOp::PopClip(clip(&projection, outer)),
                fill(&projection, Rect::new(0, 22, 32, 2), BLUE),
            ],
        );
        let image = renderer.view_pixels(&scene, false).expect("drawn");
        assert_eq!(
            pixel(&image, 2, 2),
            BLACK,
            "outside the outer clip nothing of the child is drawn"
        );
        assert_eq!(
            pixel(&image, 6, 8),
            RED,
            "inside the outer clip and outside the inner one is the red"
        );
        assert_eq!(pixel(&image, 20, 8), GREEN, "inside both clips is the green");
        assert_eq!(
            pixel(&image, 20, 2),
            BLACK,
            "inside the inner clip but outside the outer one is neither"
        );
        assert_eq!(pixel(&image, 20, 21), BLACK, "the clip ends where the outer one does");
        assert_eq!(
            pixel(&image, 4, 23),
            BLUE,
            "a layer after the clips ended is not clipped"
        );
    }

    #[test]
    fn two_eyes_draw_into_their_own_halves() {
        let size = Size::new(33, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        // The right eye is the wider half: the spare column of an odd width is its.
        let left = Size::new(16, 16);
        let right = Size::new(17, 16);
        let left_view = flat(left);
        let right_view = flat(right);
        let scene = ViewScene::new(
            1,
            1,
            size,
            vec![
                ViewEye {
                    viewport: Rect::from_size(left),
                    ops: vec![fill(&left_view, Rect::new(0, 0, 40, 40), RED)],
                },
                ViewEye {
                    viewport: Rect::new(16, 0, 17, 16),
                    ops: vec![fill(&right_view, Rect::new(0, 0, 40, 40), GREEN)],
                },
            ],
            None,
        );
        let image = renderer.view_pixels(&scene, false).expect("drawn");
        assert_eq!(pixel(&image, 0, 0), RED);
        assert_eq!(pixel(&image, 15, 15), RED, "the left eye's last column is its own");
        assert_eq!(pixel(&image, 16, 0), GREEN, "the right eye starts at the next column");
        assert_eq!(pixel(&image, 32, 15), GREEN, "and takes the spare one");
    }

    #[test]
    fn a_scene_with_no_eye_is_opaque_black() {
        let size = Size::new(8, 8);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let scene = ViewScene::new(1, 1, size, Vec::new(), None);
        let image = renderer.view_pixels(&scene, false).expect("drawn");
        assert!(
            image.data().chunks(4).all(|texel| texel == BLACK),
            "a blank rig is the blank picture the CPU draws"
        );
    }

    #[test]
    fn an_unchanged_view_is_not_drawn_again() {
        let size = Size::new(16, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let scene = scene(3, 5, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
        for _ in 0..3 {
            renderer
                .draw(&showing(Some(&scene), 3, size), Scale::IDENTITY)
                .expect("drawn");
        }
        assert_eq!(
            renderer.subtree_draws(),
            1,
            "three frames of one revision draw the scene once"
        );
        renderer
            .view_pixels(&scene, false)
            .expect("a read of the same revision");
        assert_eq!(renderer.subtree_draws(), 1, "and so does a readback of it");
    }

    #[test]
    fn a_new_revision_is_drawn_into_the_same_texture() {
        let size = Size::new(16, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let first = scene(3, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
        let second = scene(3, 2, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), GREEN)]);
        assert_eq!(pixel(&renderer.view_pixels(&first, false).expect("first"), 2, 2), RED);
        assert_eq!(
            pixel(&renderer.view_pixels(&second, false).expect("second"), 2, 2),
            GREEN
        );
        assert_eq!(renderer.subtree_draws(), 2);
        assert_eq!(renderer.subtree_textures(), 1, "one View holds one texture");
    }

    #[test]
    fn a_view_that_changes_size_gets_a_texture_of_the_new_size() {
        let Some(mut renderer) = gles(Size::new(16, 16)) else {
            return;
        };
        let small = Size::new(8, 8);
        let large = Size::new(20, 12);
        let first = scene(3, 1, small, vec![fill(&flat(small), Rect::new(0, 0, 8, 8), RED)]);
        let second = scene(3, 2, large, vec![fill(&flat(large), Rect::new(0, 0, 20, 12), GREEN)]);
        assert_eq!(renderer.view_pixels(&first, false).expect("first").size(), small);
        let image = renderer.view_pixels(&second, false).expect("second");
        assert_eq!(
            image.size(),
            large,
            "the picture is the new size, not the old one resampled"
        );
        assert_eq!(pixel(&image, 19, 11), GREEN);
        assert_eq!(renderer.subtree_textures(), 1, "the old texture was given back");
    }

    #[test]
    fn a_destroyed_view_gives_its_texture_back() {
        let size = Size::new(16, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let kept = scene(1, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
        let gone = scene(2, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), GREEN)]);
        for scene in [&kept, &gone] {
            renderer.view_pixels(scene, false).expect("drawn");
        }
        assert_eq!(renderer.subtree_textures(), 2);
        // The next frame says only View 1 is alive; the frame shows neither.
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::from_size(size));
        frame.set_live_views(vec![1]);
        renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
        assert_eq!(
            renderer.subtree_textures(),
            1,
            "the destroyed View's texture is deleted"
        );
        assert!(renderer.subtree_texture(2).is_none());
        assert!(renderer.subtree_texture(1).is_some(), "the live View keeps its own");
        // A host that draws no frame here retires the same way.
        renderer.retire_views(&[]);
        assert_eq!(renderer.subtree_textures(), 0);
    }

    #[test]
    fn a_view_the_driver_refuses_leaves_the_frame_alone() {
        let size = Size::new(16, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        // No driver makes a texture this wide, so the framebuffer is never complete.
        let huge = Size::new(1_000_000, 2);
        let refused = scene(9, 1, huge, Vec::new());
        let mut frame = showing(Some(&refused), 9, size);
        frame.push_layer(Layer::Solid(gnoblin_render_frame::frame::Solid::new(
            Rect::new(0, 0, 4, 4),
            Color::from_rgba8(200, 100, 50, 255),
        )));
        renderer
            .draw(&frame, Scale::IDENTITY)
            .expect("a View that cannot be drawn does not fail the frame");
        let image = renderer.screenshot().expect("readback");
        assert_eq!(
            pixel(&image, 2, 2),
            [200, 100, 50, 255],
            "the rest of the frame is drawn"
        );
        assert_eq!(
            pixel(&image, 10, 10),
            [10, 20, 30, 255],
            "the refused View's layer draws nothing: the background shows, not a flat colour"
        );
        assert_eq!(renderer.subtree_textures(), 0, "no texture is kept for a refused View");
        assert!(
            renderer.view_pixels(&refused, false).is_err(),
            "and a read of it is an error"
        );
        // A later revision of the View at a size the driver takes is drawn.
        let projection = flat(size);
        let accepted = scene(9, 2, size, vec![fill(&projection, Rect::new(0, 0, 16, 16), GREEN)]);
        assert_eq!(
            pixel(&renderer.view_pixels(&accepted, false).expect("drawn"), 3, 3),
            GREEN
        );
    }

    #[test]
    fn a_layer_that_names_a_view_the_frame_holds_no_scene_for_draws_nothing() {
        let size = Size::new(16, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        renderer
            .draw(&showing(None, 4, size), Scale::IDENTITY)
            .expect("the frame is drawn");
        let image = renderer.screenshot().expect("readback");
        assert_eq!(pixel(&image, 8, 8), [10, 20, 30, 255], "only the background is there");
    }

    #[test]
    fn a_capture_reads_the_scenes_redacted_eyes() {
        let size = Size::new(16, 16);
        let Some(mut renderer) = gles(size) else {
            return;
        };
        let projection = flat(size);
        let shown = vec![eye(size, vec![fill(&projection, Rect::new(0, 0, 16, 16), RED)])];
        let hidden = vec![eye(
            size,
            vec![fill(&projection, Rect::new(0, 0, 16, 16), [128, 128, 128, 255])],
        )];
        let scene = ViewScene::new(1, 1, size, shown, Some(hidden));
        let display = renderer.view_pixels(&scene, false).expect("display");
        let capture = renderer.view_pixels(&scene, true).expect("capture");
        assert_eq!(pixel(&display, 8, 8), RED, "the display shows the node");
        assert_eq!(pixel(&capture, 8, 8), [128, 128, 128, 255], "a capture shows the mask");
        assert_eq!(renderer.subtree_textures(), 2, "the variant is a texture of its own");
        // The display texture a frame samples is still the unredacted one.
        renderer
            .draw(&showing(Some(&scene), 1, size), Scale::IDENTITY)
            .expect("drawn");
        assert_eq!(pixel(&renderer.screenshot().expect("readback"), 8, 8), RED);
    }
}
