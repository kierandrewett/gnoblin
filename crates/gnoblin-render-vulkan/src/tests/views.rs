//! Views, 3D layers, the magnifier and colour matrix, and supersampling on the Vulkan renderer.
//!
//! The View cases are the GLES renderer's (`gnoblin-render-gles/src/subtree.rs`) and the renderer-level halves of the
//! backend's `view_gpu_tests.rs` and `view_secure_tests.rs`: a scene is drawn through a flat eye top-left first, clips nest,
//! eyes fill their own halves, a revision is drawn once, a destroyed View gives its images back, and a capture reads the
//! variant with the secure node masked while the picture a frame shows stays unredacted. The 3D cases are the GLES renderer's
//! depth cases. The view pass and the downsample are compared with the software renderer and the CPU average.
//!
//! The pixels are exact where a flat rectangle is drawn at whole pixels, and compared with a stated tolerance elsewhere.

use std::sync::Arc;

use gnoblin_render::Mat4;
use gnoblin_render_frame::frame::{ColourMatrix, Model, View};
use gnoblin_render_frame::view_scene::{ClipPlane, ViewEye, ViewOp, ViewScene, ViewTexture};
use gnoblin_render_software::{ViewCamera, ViewProjection, eye_pixel_matrix};
use gnoblin_types::Vec3;

use super::*;

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
        transform: Transform::Normal,
        opacity: 1.0,
        tint: Color::WHITE,
        corner_radius: 0.0,
        silhouette: None,
        effects: Effects::new(),
        model: Some(model(projection, at)),
        secure: false,
        content: None,
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
    frame.set_background(colour(10, 20, 30, 255));
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
        transform: Transform::Normal,
        opacity: 1.0,
        tint: Color::WHITE,
        corner_radius: 0.0,
        silhouette: None,
        effects: Effects::new(),
        model: None,
        secure: false,
        content: None,
    }));
    frame
}

#[test]
fn a_scene_is_drawn_through_a_flat_eye_top_left_first_and_in_painter_order() {
    let size = Size::new(32, 24);
    let Some(mut vulkan) = renderer(size) else { return };
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
    let image = vulkan.view_pixels(&scene, false).expect("the scene is drawn and read");
    assert_eq!(image.size(), size);
    assert_eq!(
        pixel(&image, 2, 2),
        RED,
        "a rectangle at the top is at the top of the picture"
    );
    assert_eq!(
        pixel(&image, 12, 6),
        GREEN,
        "the later layer is on top where they overlap"
    );
    assert_eq!(pixel(&image, 20, 10), GREEN);
    assert_eq!(pixel(&image, 28, 20), BLACK, "nothing drawn is opaque black");
    assert_eq!(
        pixel(&image, 2, 20),
        BLACK,
        "and the bottom of the picture is not the top"
    );
    clean(&vulkan);
}

#[test]
fn a_frame_that_shows_a_view_samples_its_picture_without_a_readback() {
    let size = Size::new(32, 24);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let scene = scene(7, 1, size, vec![fill(&projection, Rect::new(0, 0, 16, 12), BLUE)]);
    vulkan
        .draw(&showing(Some(&scene), 7, size), Scale::IDENTITY)
        .expect("the frame is drawn");
    let image = vulkan.screenshot().expect("the output's readback");
    assert_eq!(
        pixel(&image, 4, 4),
        BLUE,
        "the top-left quarter shows the View's rectangle"
    );
    assert_eq!(pixel(&image, 24, 16), BLACK, "the rest of the View is its black");
    assert_eq!(vulkan.subtree_draws(), 1, "the scene was drawn once");
    assert_eq!(vulkan.subtree_textures(), 1);
    clean(&vulkan);
}

#[test]
fn a_clip_keeps_what_is_drawn_inside_it_to_its_rectangle_and_clips_nest() {
    let size = Size::new(32, 24);
    let Some(mut vulkan) = renderer(size) else { return };
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
    let image = vulkan.view_pixels(&scene, false).expect("drawn");
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
    clean(&vulkan);
}

#[test]
fn two_eyes_draw_into_their_own_halves() {
    let size = Size::new(33, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    // The right eye is the wider half: the spare column of an odd width is its.
    let left = Size::new(16, 16);
    let right = Size::new(17, 16);
    let scene = ViewScene::new(
        1,
        1,
        size,
        vec![
            ViewEye {
                viewport: Rect::from_size(left),
                ops: vec![fill(&flat(left), Rect::new(0, 0, 40, 40), RED)],
            },
            ViewEye {
                viewport: Rect::new(16, 0, 17, 16),
                ops: vec![fill(&flat(right), Rect::new(0, 0, 40, 40), GREEN)],
            },
        ],
        None,
    );
    let image = vulkan.view_pixels(&scene, false).expect("drawn");
    assert_eq!(pixel(&image, 0, 0), RED);
    assert_eq!(pixel(&image, 15, 15), RED, "the left eye's last column is its own");
    assert_eq!(pixel(&image, 16, 0), GREEN, "the right eye starts at the next column");
    assert_eq!(pixel(&image, 32, 15), GREEN, "and takes the spare one");
    clean(&vulkan);
}

#[test]
fn a_scene_with_no_eye_is_opaque_black() {
    let size = Size::new(8, 8);
    let Some(mut vulkan) = renderer(size) else { return };
    let image = vulkan
        .view_pixels(&ViewScene::new(1, 1, size, Vec::new(), None), false)
        .expect("drawn");
    assert!(
        image.data().chunks(4).all(|texel| texel == BLACK),
        "a blank rig is the blank picture the CPU draws"
    );
    clean(&vulkan);
}

#[test]
fn an_unchanged_view_is_not_drawn_again() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let scene = scene(3, 5, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
    for _ in 0..3 {
        vulkan
            .draw(&showing(Some(&scene), 3, size), Scale::IDENTITY)
            .expect("drawn");
    }
    assert_eq!(
        vulkan.subtree_draws(),
        1,
        "three frames of one revision draw the scene once"
    );
    let submits = vulkan.stats().submits;
    vulkan.view_pixels(&scene, false).expect("a read of the same revision");
    assert_eq!(vulkan.subtree_draws(), 1, "and so does a readback of it");
    assert_eq!(
        vulkan.stats().submits,
        submits + 1,
        "the read is one copy and draws nothing"
    );
    clean(&vulkan);
}

#[test]
fn a_view_is_drawn_once_per_revision() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    for revision in 1..=4 {
        let scene = scene(3, revision, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
        vulkan
            .draw(&showing(Some(&scene), 3, size), Scale::IDENTITY)
            .expect("drawn");
        vulkan
            .draw(&showing(Some(&scene), 3, size), Scale::IDENTITY)
            .expect("drawn");
        assert_eq!(
            vulkan.subtree_draws(),
            revision,
            "two frames of revision {revision} draw it once"
        );
    }
    clean(&vulkan);
}

#[test]
fn a_new_revision_is_drawn_into_the_same_picture() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let first = scene(3, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
    let second = scene(3, 2, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), GREEN)]);
    assert_eq!(pixel(&vulkan.view_pixels(&first, false).expect("first"), 2, 2), RED);
    assert_eq!(pixel(&vulkan.view_pixels(&second, false).expect("second"), 2, 2), GREEN);
    assert_eq!(vulkan.subtree_draws(), 2);
    assert_eq!(vulkan.subtree_textures(), 1, "one View holds one picture");
    clean(&vulkan);
}

#[test]
fn a_view_that_changes_size_gets_a_picture_of_the_new_size() {
    let Some(mut vulkan) = renderer(Size::new(16, 16)) else {
        return;
    };
    let small = Size::new(8, 8);
    let large = Size::new(20, 12);
    let first = scene(3, 1, small, vec![fill(&flat(small), Rect::new(0, 0, 8, 8), RED)]);
    let second = scene(3, 2, large, vec![fill(&flat(large), Rect::new(0, 0, 20, 12), GREEN)]);
    assert_eq!(vulkan.view_pixels(&first, false).expect("first").size(), small);
    let image = vulkan.view_pixels(&second, false).expect("second");
    assert_eq!(
        image.size(),
        large,
        "the picture is the new size, not the old one resampled"
    );
    assert_eq!(pixel(&image, 19, 11), GREEN);
    assert_eq!(vulkan.subtree_textures(), 1, "the old picture was given back");
    clean(&vulkan);
}

#[test]
fn a_destroyed_view_gives_its_images_back() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let kept = scene(1, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
    let gone = scene(2, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), GREEN)]);
    for scene in [&kept, &gone] {
        vulkan.view_pixels(scene, false).expect("drawn");
    }
    assert_eq!(vulkan.subtree_textures(), 2);
    let destroyed = vulkan.stats().images_destroyed;
    // The next frame says only View 1 is alive; the frame shows neither.
    let mut frame = OutputFrame::new();
    frame.push_damage(Rect::from_size(size));
    frame.set_live_views(vec![1]);
    vulkan.draw(&frame, Scale::IDENTITY).expect("drawn");
    assert_eq!(vulkan.subtree_textures(), 1, "the destroyed View's picture is dropped");
    assert!(!vulkan.gpu().views.borrow().holds(2));
    assert!(vulkan.gpu().views.borrow().holds(1), "the live View keeps its own");
    // The images go once the frames that used them have finished: a capture waits for all of them.
    vulkan.capture(false).expect("a readback waits for the queue");
    vulkan.draw(&frame, Scale::IDENTITY).expect("drawn");
    // The frames' own one-pixel textures go too, so the count is at least the View's two.
    assert!(
        vulkan.stats().images_destroyed >= destroyed + 2,
        "the colour and the depth-stencil image of the destroyed View are destroyed ({} then {})",
        destroyed,
        vulkan.stats().images_destroyed
    );
    // A host that draws no frame here retires the same way.
    vulkan.retire_views(&[]);
    assert_eq!(vulkan.subtree_textures(), 0);
    clean(&vulkan);
}

/// The cover a locked output shows is a frame that names no View. Drawing it gives back every picture, the display one and
/// the capture variant alike, so nothing the desktop showed stays in GPU memory behind the cover, and a View read after the
/// unlock is drawn again from the scene it has then.
#[test]
fn a_frame_that_names_no_view_gives_back_the_display_and_the_capture_picture_alike() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let secure = ViewScene::new(
        1,
        1,
        size,
        vec![eye(size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)])],
        Some(vec![eye(size, vec![fill(&projection, Rect::new(0, 0, 8, 8), BLUE)])]),
    );
    vulkan
        .view_pixels(&secure, false)
        .expect("the display picture is drawn");
    vulkan.view_pixels(&secure, true).expect("the capture variant is drawn");
    assert_eq!(vulkan.subtree_textures(), 2, "a secure View holds both pictures");

    let mut cover = OutputFrame::new();
    cover.push_damage(Rect::from_size(size));
    vulkan.draw(&cover, Scale::IDENTITY).expect("the cover is drawn");
    assert_eq!(
        vulkan.subtree_textures(),
        0,
        "the cover names no View, so both pictures are gone"
    );

    // The unlock: the View is named again and read from a picture drawn afresh.
    let capture = vulkan.view_pixels(&secure, true).expect("drawn again");
    assert_eq!(
        pixel(&capture, 4, 4),
        BLUE,
        "the capture variant, not a picture left from before"
    );
    assert_eq!(vulkan.subtree_textures(), 1);
    clean(&vulkan);
}

#[test]
fn a_view_the_device_refuses_leaves_the_frame_alone() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    // No device makes an image this wide.
    let huge = Size::new(1_000_000, 2);
    let refused = scene(9, 1, huge, Vec::new());
    let mut frame = showing(Some(&refused), 9, size);
    solid(&mut frame, Rect::new(0, 0, 4, 4), colour(200, 100, 50, 255));
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("a View that cannot be drawn does not fail the frame");
    let image = vulkan.screenshot().expect("readback");
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
    assert_eq!(vulkan.subtree_textures(), 0, "no picture is kept for a refused View");
    assert!(
        vulkan.view_pixels(&refused, false).is_err(),
        "and a read of it is an error"
    );
    // A later revision of the View at a size the device takes is drawn.
    let accepted = scene(9, 2, size, vec![fill(&flat(size), Rect::new(0, 0, 16, 16), GREEN)]);
    assert_eq!(
        pixel(&vulkan.view_pixels(&accepted, false).expect("drawn"), 3, 3),
        GREEN
    );
    clean(&vulkan);
}

#[test]
fn a_layer_that_names_a_view_the_frame_holds_no_scene_for_draws_nothing() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    vulkan
        .draw(&showing(None, 4, size), Scale::IDENTITY)
        .expect("the frame is drawn");
    let image = vulkan.screenshot().expect("readback");
    assert_eq!(pixel(&image, 8, 8), [10, 20, 30, 255], "only the background is there");
    clean(&vulkan);
}

#[test]
fn a_capture_reads_the_scenes_redacted_eyes() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let shown = vec![eye(size, vec![fill(&projection, Rect::new(0, 0, 16, 16), RED)])];
    let hidden = vec![eye(
        size,
        vec![fill(&projection, Rect::new(0, 0, 16, 16), [128, 128, 128, 255])],
    )];
    let scene = ViewScene::new(1, 1, size, shown, Some(hidden));
    let display = vulkan.view_pixels(&scene, false).expect("display");
    let capture = vulkan.view_pixels(&scene, true).expect("capture");
    assert_eq!(pixel(&display, 8, 8), RED, "the display shows the node");
    assert_eq!(pixel(&capture, 8, 8), [128, 128, 128, 255], "a capture shows the mask");
    assert_eq!(vulkan.subtree_textures(), 2, "the variant is a picture of its own");
    // The picture a frame samples is still the unredacted one.
    vulkan
        .draw(&showing(Some(&scene), 1, size), Scale::IDENTITY)
        .expect("drawn");
    assert_eq!(pixel(&vulkan.screenshot().expect("readback"), 8, 8), RED);
    clean(&vulkan);
}

#[test]
fn omitting_a_secure_node_leaves_what_lies_beneath_it_in_both_variants_alike() {
    // The composer omits a node by leaving it out of the capture eyes: what was under it shows through, and a scene whose
    // variant equals its display draws the same picture twice.
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let under = fill(&projection, Rect::new(0, 0, 16, 16), GREEN);
    let secret = fill(&projection, Rect::new(4, 4, 8, 8), RED);
    let scene = ViewScene::new(
        1,
        1,
        size,
        vec![eye(size, vec![under.clone(), secret])],
        Some(vec![eye(size, vec![under])]),
    );
    let capture = vulkan.view_pixels(&scene, true).expect("capture");
    let display = vulkan.view_pixels(&scene, false).expect("display");
    assert_eq!(
        pixel(&capture, 8, 8),
        GREEN,
        "the node is not in the capture, so what is beneath shows"
    );
    assert_eq!(pixel(&display, 8, 8), RED);
    clean(&vulkan);
}

#[test]
fn two_outputs_on_one_device_share_one_picture_of_a_view() {
    let Some(first) = renderer(Size::new(16, 16)) else {
        return;
    };
    let Ok(mut second) = Vulkan::with_device(first.gpu(), Size::new(24, 24)) else {
        return;
    };
    let mut first = first;
    let projection = flat(Size::new(16, 16));
    let scene = scene(
        5,
        1,
        Size::new(16, 16),
        vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)],
    );
    first
        .draw(&showing(Some(&scene), 5, Size::new(16, 16)), Scale::IDENTITY)
        .expect("first");
    second
        .draw(&showing(Some(&scene), 5, Size::new(24, 24)), Scale::IDENTITY)
        .expect("second");
    assert_eq!(
        first.subtree_draws(),
        1,
        "the second output samples the picture the first drew"
    );
    assert_eq!(pixel(&first.screenshot().expect("a"), 2, 2), RED);
    assert_eq!(
        pixel(&second.screenshot().expect("b"), 2, 2),
        RED,
        "stretched over the larger output, still the corner"
    );
    clean(&first);
}

#[test]
fn a_view_inside_a_view_is_sampled_where_the_inner_one_is_drawn() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let inner = scene(1, 1, size, vec![fill(&projection, Rect::new(0, 0, 16, 16), GREEN)]);
    vulkan.view_pixels(&inner, false).expect("the inner view");
    let picture = ViewOp::Draw(Layer::Surface(Surface {
        source: SurfaceSource::View(ViewTexture { id: 1 }),
        src: RectF {
            x: 0.0,
            y: 0.0,
            width: 16.0,
            height: 16.0,
        },
        dst: Rect::new(0, 0, 8, 8),
        transform: Transform::Normal,
        opacity: 1.0,
        tint: Color::WHITE,
        corner_radius: 0.0,
        silhouette: None,
        effects: Effects::new(),
        model: Some(model(&projection, Rect::new(4, 4, 8, 8))),
        secure: false,
        content: None,
    }));
    let outer = scene(2, 1, size, vec![picture]);
    let image = vulkan.view_pixels(&outer, false).expect("the outer view");
    assert_eq!(pixel(&image, 8, 8), GREEN, "the outer picture shows the inner one");
    assert_eq!(pixel(&image, 1, 1), BLACK);
    clean(&vulkan);
}

// ---- Layers placed in three dimensions -----------------------------------------------------------------------------

/// A rectangle drawn in three dimensions: `matrix` maps its own plane, origin at its top-left corner, to the output before
/// the perspective divide.
fn spatial_solid(rect: Rect, colour: Color, matrix: Mat4, opaque: bool) -> Solid {
    let mut solid = Solid::new(rect, colour);
    solid.model = Some(Model { matrix, opaque });
    solid
}

/// A frame of `layers` painted over `background`, damaging the whole output.
fn layered_frame(background: Color, size: Size, layers: Vec<Solid>) -> OutputFrame {
    let mut frame = frame(background, size);
    for solid in layers {
        frame.push_layer(Layer::Solid(solid));
    }
    frame
}

/// The plane `z = slope * x + offset` over a rectangle's own coordinates.
fn tilted_plane(slope: f64, offset: f64) -> Mat4 {
    Mat4 {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [slope, 0.0, 0.0, offset],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

#[test]
fn a_layer_with_a_model_is_drawn_and_no_unsupported_3d_is_reported() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let tilted = spatial_solid(Rect::new(0, 0, 16, 16), colour(255, 0, 0, 255), Mat4::IDENTITY, true);
    vulkan
        .draw(&layered_frame(Color::BLACK, SIZE, vec![tilted]), Scale::IDENTITY)
        .expect("the frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(pixel(&image, 4, 4), RED, "the placed layer is drawn through its model");
    assert_eq!(pixel(&image, 30, 30), BLACK);
    assert!(
        !vulkan.warned_models,
        "no 3D warning is raised: the device has a depth pass"
    );
    clean(&vulkan);
}

#[test]
fn an_intersecting_pair_is_depth_tested_per_pixel() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let red = || spatial_solid(Rect::new(0, 0, 16, 16), colour(255, 0, 0, 255), Mat4::IDENTITY, true);
    let blue = || {
        spatial_solid(
            Rect::new(0, 0, 16, 16),
            colour(0, 0, 255, 255),
            tilted_plane(0.1, -0.8),
            true,
        )
    };
    for layers in [vec![red(), blue()], vec![blue(), red()]] {
        vulkan
            .draw(&layered_frame(Color::BLACK, size, layers), Scale::IDENTITY)
            .expect("drawn");
        let image = vulkan.screenshot().expect("a readback");
        assert_eq!(
            pixel(&image, 3, 4),
            BLUE,
            "left of the crossing blue is nearer, whatever the submission order"
        );
        assert_eq!(pixel(&image, 12, 4), RED, "right of it red is nearer");
    }
    clean(&vulkan);
}

#[test]
fn transparent_layers_blend_in_depth_order_as_the_flat_path_does() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let red = colour(255, 0, 0, 128);
    let blue = colour(0, 0, 255, 128);
    // Painter's order puts blue over red; the placed pair puts red behind blue and must blend to the same pixel, whichever
    // order it is submitted in.
    let flat_pair = layered_frame(
        Color::BLACK,
        size,
        vec![
            Solid::new(Rect::new(0, 0, 16, 16), red),
            Solid::new(Rect::new(0, 0, 16, 16), blue),
        ],
    );
    vulkan.draw(&flat_pair, Scale::IDENTITY).expect("the flat pair");
    let expected = pixel(&vulkan.screenshot().expect("a readback"), 8, 8);
    let far = || spatial_solid(Rect::new(0, 0, 16, 16), red, Mat4::translation(0.0, 0.0, 0.5), false);
    let near = || spatial_solid(Rect::new(0, 0, 16, 16), blue, Mat4::translation(0.0, 0.0, -0.5), false);
    for layers in [vec![near(), far()], vec![far(), near()]] {
        vulkan
            .draw(&layered_frame(Color::BLACK, size, layers), Scale::IDENTITY)
            .expect("the placed pair");
        let got = pixel(&vulkan.screenshot().expect("a readback"), 8, 8);
        assert!(
            got.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= BLEND),
            "placed {got:?}, flat path {expected:?}"
        );
    }
    clean(&vulkan);
}

#[test]
fn a_rotated_quad_under_perspective_lands_on_its_projected_corners() {
    let size = Size::new(64, 64);
    let Some(mut vulkan) = renderer(size) else { return };
    let (centre, distance, angle) = (32.0_f64, 20.0_f64, 30.0_f64);
    // A 32 by 32 quad centred on the output, turned about the vertical axis through its centre.
    let world = Mat4::translation(centre, centre, 0.0)
        .mul(&Mat4::rotation_y(angle))
        .mul(&Mat4::translation(-16.0, -16.0, 0.0));
    let matrix = Mat4::perspective(centre, centre, distance, 1.0, 100.0).mul(&world);
    let quad = spatial_solid(Rect::new(16, 16, 32, 32), colour(255, 0, 0, 255), matrix, true);
    vulkan
        .draw(&layered_frame(Color::BLACK, size, vec![quad]), Scale::IDENTITY)
        .expect("drawn");
    let image = vulkan.screenshot().expect("a readback");

    // A point at offset (a, b) from the centre is at (a cos, b, -a sin); the eye at `distance` projects (X, Y, Z) to
    // ((d X - c Z) / (d - Z), (d Y - c Z) / (d - Z)).
    let (sin, cos) = angle.to_radians().sin_cos();
    let corners: Vec<(f64, f64)> = [(-16.0, -16.0), (16.0, -16.0), (16.0, 16.0), (-16.0, 16.0)]
        .iter()
        .map(|&(a, b)| {
            let (x, y, z) = (centre + a * cos, centre + b, -a * sin);
            let w = distance - z;
            ((distance * x - centre * z) / w, (distance * y - centre * z) / w)
        })
        .collect();
    let edge = |i: usize| (corners[i], corners[(i + 1) % 4]);
    let area: f64 = (0..4)
        .map(|i| {
            let ((ax, ay), (bx, by)) = edge(i);
            ax * by - bx * ay
        })
        .sum();
    let orientation = area.signum();
    let (mut inside, mut outside) = (0, 0);
    for py in 0..64 {
        for px in 0..64 {
            let p = (f64::from(px) + 0.5, f64::from(py) + 0.5);
            // The signed distance to the nearest edge, positive inside. Pixels within a pixel of an edge are skipped:
            // rasterisation rounds there.
            let margin = (0..4)
                .map(|i| {
                    let ((ax, ay), (bx, by)) = edge(i);
                    let (ex, ey) = (bx - ax, by - ay);
                    orientation * (ex * (p.1 - ay) - ey * (p.0 - ax)) / (ex * ex + ey * ey).sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            if margin > 1.0 {
                assert_eq!(pixel(&image, px, py), RED, "inside the projected quad at {px},{py}");
                inside += 1;
            } else if margin < -1.0 {
                assert_eq!(pixel(&image, px, py), BLACK, "outside the projected quad at {px},{py}");
                outside += 1;
            }
        }
    }
    assert!(
        inside > 100 && outside > 100,
        "the probe covers both sides ({inside} in, {outside} out)"
    );
    clean(&vulkan);
}

#[test]
fn a_placed_layer_is_drawn_only_inside_the_damage_and_over_the_flat_layers() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let mut whole = layered_frame(
        Color::BLACK,
        size,
        vec![spatial_solid(
            Rect::new(0, 0, 16, 16),
            colour(255, 0, 0, 255),
            Mat4::IDENTITY,
            true,
        )],
    );
    vulkan.draw(&whole, Scale::IDENTITY).expect("the whole frame");
    // A flat layer drawn later is painter-ordered below every placed layer, and damage limits the repaint.
    whole.clear();
    whole.set_background(Color::BLACK);
    whole.push_damage(Rect::new(0, 0, 8, 16));
    whole.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, 16, 16),
        colour(0, 255, 0, 255),
    )));
    whole.push_layer(Layer::Solid(spatial_solid(
        Rect::new(0, 0, 16, 16),
        colour(0, 0, 255, 255),
        Mat4::IDENTITY,
        true,
    )));
    vulkan.draw(&whole, Scale::IDENTITY).expect("the damaged frame");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        pixel(&image, 4, 4),
        BLUE,
        "inside the damage the placed layer is over the flat one"
    );
    assert_eq!(pixel(&image, 12, 4), RED, "outside it the earlier frame stays");
    clean(&vulkan);
}

#[test]
fn a_placed_pipeline_is_made_when_first_drawn_with_and_once() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let none = || vulkan.gpu().extras.as_ref().map_or(0, DepthExtras::made);
    assert_eq!(none(), 0, "a device that has drawn nothing has made none");
    let placed = || spatial_solid(Rect::new(0, 0, 16, 16), colour(255, 0, 0, 255), Mat4::IDENTITY, true);
    vulkan
        .draw(&layered_frame(Color::BLACK, size, vec![placed()]), Scale::IDENTITY)
        .expect("drawn");
    let made = vulkan.gpu().extras.as_ref().map_or(0, DepthExtras::made);
    assert_eq!(made, 1, "an opaque solid makes the one pipeline it draws with");
    vulkan
        .draw(&layered_frame(Color::BLACK, size, vec![placed()]), Scale::IDENTITY)
        .expect("drawn again");
    assert_eq!(
        vulkan.gpu().extras.as_ref().map_or(0, DepthExtras::made),
        made,
        "and the next frame makes no more"
    );
    clean(&vulkan);
}

// ---- The magnifier and the colour matrix ---------------------------------------------------------------------------

/// A frame whose picture has a coloured quarter at each corner, so a magnified or flipped result differs from the plain one.
fn quarters(size: Size) -> OutputFrame {
    let mut frame = frame(Color::BLACK, size);
    let (w, h) = (size.width / 2, size.height / 2);
    solid(&mut frame, Rect::new(0, 0, w, h), colour(255, 0, 0, 255));
    solid(&mut frame, Rect::new(w as i32, 0, w, h), colour(0, 255, 0, 255));
    solid(&mut frame, Rect::new(0, h as i32, w, h), colour(0, 0, 255, 255));
    solid(
        &mut frame,
        Rect::new(w as i32, h as i32, w, h),
        colour(255, 255, 0, 255),
    );
    frame
}

#[test]
fn a_magnified_view_matches_the_software_renderer_pixel_for_pixel() {
    let size = Size::new(64, 48);
    let Some(mut vulkan) = renderer(size) else { return };
    let mut software = software(size);
    for (zoom, x, y) in [(2.0, 16.0, 12.0), (2.0, 48.0, 36.0), (3.0, 32.0, 24.0)] {
        let mut frame = quarters(size);
        frame.set_view(View::new(zoom, x, y));
        vulkan.draw(&frame, Scale::IDENTITY).expect("the Vulkan frame");
        software.draw(&frame, Scale::IDENTITY).expect("the software frame");
        let got = vulkan.screenshot().expect("a Vulkan readback");
        let expected = software.screenshot().expect("a software readback");
        assert_close(&format!("zoom {zoom} at {x},{y}"), &expected, &got, 0);
    }
    clean(&vulkan);
}

#[test]
fn a_colour_matrix_matches_the_software_renderer() {
    let size = Size::new(64, 48);
    let Some(mut vulkan) = renderer(size) else { return };
    let mut software = software(size);
    let invert = ColourMatrix::new([
        [-1.0, 0.0, 0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
    ])
    .expect("a finite matrix");
    let mut frame = quarters(size);
    solid(&mut frame, Rect::new(20, 14, 24, 20), colour(100, 150, 200, 128));
    frame.set_colour_matrix(Some(invert));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the Vulkan frame");
    software.draw(&frame, Scale::IDENTITY).expect("the software frame");
    assert_close(
        "an inverting matrix",
        &software.screenshot().expect("a software readback"),
        &vulkan.screenshot().expect("a Vulkan readback"),
        BLEND + 1,
    );
    clean(&vulkan);
}

#[test]
fn an_identity_view_and_matrix_draw_exactly_as_no_view_does() {
    let size = Size::new(64, 48);
    let Some(mut vulkan) = renderer(size) else { return };
    vulkan.draw(&quarters(size), Scale::IDENTITY).expect("plain");
    let plain = vulkan.screenshot().expect("a readback");
    let submits = vulkan.stats().submits;
    let mut frame = quarters(size);
    frame.set_view(View::new(1.0, 7.0, 9.0));
    frame.set_colour_matrix(Some(ColourMatrix::IDENTITY));
    vulkan.draw(&frame, Scale::IDENTITY).expect("identity");
    let identity = vulkan.screenshot().expect("a readback");
    assert_eq!(plain.data(), identity.data());
    assert!(
        !vulkan.filtered,
        "an idle view does not make the next frame repaint the whole output"
    );
    assert_eq!(
        vulkan.stats().submits,
        submits + 2,
        "one frame and one readback: no copy and no view pass is submitted"
    );
    clean(&vulkan);
}

#[test]
fn turning_the_view_off_repaints_the_whole_output_unfiltered() {
    let size = Size::new(64, 48);
    let Some(mut vulkan) = renderer(size) else { return };
    let mut frame = quarters(size);
    frame.set_view(View::new(2.0, 16.0, 12.0));
    vulkan.draw(&frame, Scale::IDENTITY).expect("magnified");
    assert!(vulkan.filtered);
    // A frame that damages only a corner, without the view: the rest of the output was magnified and must be painted again.
    let mut next = quarters(size);
    next.clear();
    next.set_background(Color::BLACK);
    next.push_damage(Rect::new(0, 0, 4, 4));
    solid(&mut next, Rect::new(0, 0, 32, 24), colour(255, 0, 0, 255));
    solid(&mut next, Rect::new(32, 0, 32, 24), colour(0, 255, 0, 255));
    solid(&mut next, Rect::new(0, 24, 32, 24), colour(0, 0, 255, 255));
    solid(&mut next, Rect::new(32, 24, 32, 24), colour(255, 255, 0, 255));
    vulkan.draw(&next, Scale::IDENTITY).expect("unfiltered");
    assert!(!vulkan.filtered);
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        pixel(&image, 60, 4),
        GREEN,
        "a pixel outside the damage is the unfiltered picture again"
    );
    assert_eq!(pixel(&image, 60, 44), [255, 255, 0, 255]);
    clean(&vulkan);
}

// ---- Supersampling ---------------------------------------------------------------------------------------------------

#[test]
fn a_supersampled_target_is_averaged_down_to_the_mode_as_the_cpu_does() {
    let mode = Size::new(32, 24);
    let virtual_size = Size::new(64, 48);
    let Some(mut vulkan) = renderer(virtual_size) else {
        return;
    };
    let mut frame = frame(Color::BLACK, virtual_size);
    // A checkerboard of one virtual pixel: every mode pixel averages two of each colour.
    for y in 0..virtual_size.height as i32 {
        for x in 0..virtual_size.width as i32 {
            if (x + y) % 2 == 0 {
                solid(&mut frame, Rect::new(x, y, 1, 1), colour(255, 255, 255, 255));
            }
        }
    }
    solid(&mut frame, Rect::new(0, 0, 8, 8), colour(255, 0, 0, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the virtual frame");
    let averaged = vulkan.downsampled(mode).expect("the average");
    assert_eq!(averaged.size(), mode);
    let reference = vulkan.capture(true).expect("the virtual picture").downsample(mode);
    assert_close("a 2x box average", &reference, &averaged, 0);
    assert_eq!(
        pixel(&averaged, 8, 8),
        [128, 128, 128, 255],
        "a checkerboard averages to half white"
    );
    assert_eq!(pixel(&averaged, 1, 1), RED, "a solid block stays itself");
    clean(&vulkan);
}

#[test]
fn a_ratio_that_is_not_whole_is_sampled_bilinearly_as_the_cpu_does() {
    let mode = Size::new(32, 24);
    let virtual_size = Size::new(48, 36);
    let Some(mut vulkan) = renderer(virtual_size) else {
        return;
    };
    let mut frame = frame(Color::BLACK, virtual_size);
    solid(&mut frame, Rect::new(0, 0, 24, 18), colour(255, 0, 0, 255));
    solid(&mut frame, Rect::new(24, 18, 24, 18), colour(0, 0, 255, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the virtual frame");
    let averaged = vulkan.downsampled(mode).expect("the average");
    let reference = vulkan.capture(true).expect("the virtual picture").downsample(mode);
    assert_close("a 1.5x bilinear sample", &reference, &averaged, 2);
    assert_eq!(pixel(&averaged, 2, 2), RED);
    assert_eq!(pixel(&averaged, 29, 21), BLUE);
    clean(&vulkan);
}

#[test]
fn the_averaged_picture_is_top_first_and_the_target_still_draws_afterwards() {
    let virtual_size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(virtual_size) else {
        return;
    };
    let mut frame = frame(Color::BLACK, virtual_size);
    solid(&mut frame, Rect::new(0, 0, 16, 4), colour(255, 0, 0, 255));
    vulkan.draw(&frame, Scale::IDENTITY).expect("the frame");
    let averaged = vulkan.downsampled(Size::new(8, 8)).expect("the average");
    assert_eq!(
        pixel(&averaged, 4, 0),
        RED,
        "the top row of the target is the top row of the average"
    );
    assert_eq!(pixel(&averaged, 4, 7), BLACK);
    // The target is back in the layout a frame draws into, and the average is made again for a new size.
    let mut again = frame.clone();
    solid(&mut again, Rect::new(0, 12, 16, 4), colour(0, 255, 0, 255));
    vulkan.draw(&again, Scale::IDENTITY).expect("a later frame");
    let second = vulkan.downsampled(Size::new(4, 4)).expect("a smaller average");
    assert_eq!(second.size(), Size::new(4, 4));
    assert_eq!(pixel(&second, 1, 3), GREEN);
    assert!(matches!(
        vulkan.downsampled(Size::new(0, 4)),
        Err(RenderError::Degenerate { width: 0, height: 4 })
    ));
    clean(&vulkan);
}

#[test]
fn a_frame_that_fails_before_it_is_submitted_does_not_leave_a_view_believed_drawn() {
    let size = Size::new(16, 16);
    let Some(mut vulkan) = renderer(size) else { return };
    let projection = flat(size);
    let scene = scene(3, 1, size, vec![fill(&projection, Rect::new(0, 0, 8, 8), RED)]);
    vulkan
        .gpu()
        .context
        .faults
        .submit
        .set(Some(vk::Result::ERROR_OUT_OF_HOST_MEMORY));
    assert!(
        vulkan.draw(&showing(Some(&scene), 3, size), Scale::IDENTITY).is_err(),
        "the submission is refused, so the frame fails"
    );
    assert_eq!(
        vulkan.subtree_textures(),
        0,
        "the picture the frame meant to draw is not kept as drawn"
    );
    vulkan
        .draw(&showing(Some(&scene), 3, size), Scale::IDENTITY)
        .expect("the next frame");
    assert_eq!(
        pixel(&vulkan.screenshot().expect("a readback"), 2, 2),
        RED,
        "and it draws the View"
    );
    assert_eq!(
        vulkan.subtree_draws(),
        2,
        "the same revision was drawn again, because the first draw never ran"
    );
    clean(&vulkan);
}
