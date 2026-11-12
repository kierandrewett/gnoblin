//! A View on the GPU path: what the composer describes, and that the GLES renderer draws it as the CPU rig does.
//!
//! The composer has two ways to draw a View. On the CPU path ([`Composer::set_gpu_views`] off) it rasterises the source
//! subtree itself and hands the renderer a picture. On the GPU path it hands the renderer the subtree as a
//! [`ViewScene`] and a layer that samples the texture the renderer draws from it. The CPU path is the reference: the
//! cases below build one world twice, draw the View both ways and compare the pictures pixel for pixel, within
//! [`TOLERANCE`] on every channel as the effect goldens do.
//!
//! A case with no perspective compares every pixel. A rotated or projected node has edges where the two rasterisers
//! may disagree by one pixel (the CPU tests a pixel centre against the node's bounds and samples the nearest texel; a
//! GPU interpolates and filters), so those cases allow a small share of the picture to differ and state the share.
//!
//! The GLES half of each case needs a surfaceless EGL context, which llvmpipe provides in CI: a machine without one
//! prints the reason and skips, as the renderer's own tests do. The absolute draw times are not what these tests are
//! about, so none is measured here: see `crates/gnoblin-render-software/tests/view_cost.rs`.

use gnoblin_render::{NodeSpec, Scene};
use gnoblin_render_gles::Gles;
use gnoblin_types::Vec3;

use super::*;

/// The most a channel of a pixel may differ between the two renderers.
const TOLERANCE: u8 = 3;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREY: [u8; 4] = [128, 128, 128, 255];

/// The View of every case.
const VIEW: NodeId = NodeId::new(100);
/// The root of its source subtree.
const SOURCE: NodeId = NodeId::new(101);

/// A composer with an 800 by 600 output, drawn once so the output's first frame is not a case's subject.
fn composer() -> (Composer, OutputId) {
    let mut composer = Composer::new();
    let output = OutputId::new(0);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    composer.set_reduced_motion(true);
    composer.begin_frame();
    composer.end_frame();
    (composer, output)
}

/// A paint list of one `size` rectangle of `base` with an `accent` rectangle in its top left quarter, so a picture
/// that is flipped, mirrored or shifted differs from the original.
fn picture(size: (f32, f32), base: [u8; 4], accent: [u8; 4]) -> Vec<u8> {
    use gnoblin_paint::{PaintList, PaintOp, Rgba8};
    let rect = |x: f32, y: f32, width: f32, height: f32, [r, g, b, a]: [u8; 4]| PaintOp::Rect {
        x,
        y,
        width,
        height,
        radius: 0.0,
        fill: Rgba8::new(r, g, b, a),
        stroke: Rgba8::TRANSPARENT,
        stroke_width: 0.0,
    };
    gnoblin_paint::encode(&PaintList {
        ops: vec![
            rect(0.0, 0.0, size.0, size.1, base),
            rect(0.0, 0.0, size.0 / 2.0, size.1 / 2.0, accent),
        ],
    })
}

/// Adds a Canvas node `id` at `at` under `parent`, holding a picture of `base` and `accent`.
fn canvas(composer: &mut Composer, id: u64, parent: u64, at: Rect, base: [u8; 4], accent: [u8; 4]) {
    composer.with_scene(|scene| {
        let mut spec = NodeSpec::new(at);
        spec.parent = Some(NodeId::new(parent));
        scene.insert(NodeId::new(id), spec).expect("a canvas node");
        scene
            .set_class(NodeId::new(id), Some("canvas".to_owned()))
            .expect("the canvas's class");
        let content = picture((at.width as f32, at.height as f32), base, accent);
        scene
            .set_value(NodeId::new(id), "content", TreeValue::Bytes(content))
            .expect("the canvas's content");
    });
}

/// Adds a plain node `id` at `at` under `parent`.
fn plain(composer: &mut Composer, id: u64, parent: u64, at: Rect) {
    composer.with_scene(|scene| {
        let mut spec = NodeSpec::new(at);
        spec.parent = Some(NodeId::new(parent));
        scene.insert(NodeId::new(id), spec).expect("a plain node");
    });
}

/// Edits the visual transform of node `id`.
fn transform(composer: &mut Composer, id: u64, edit: impl FnOnce(&mut VisualTransform)) {
    composer.with_scene(|scene| {
        let mut transform = *scene.transform(NodeId::new(id)).expect("the node's transform");
        edit(&mut transform);
        scene
            .set_transform(NodeId::new(id), transform)
            .expect("the transform is set");
    });
}

/// Adds the source root (a plain node of `size`) and the View of it, `size` big at (10, 20) on the output.
fn view_of_source(composer: &mut Composer, size: Size) {
    composer.with_scene(|scene: &mut Scene| {
        scene
            .insert(SOURCE, NodeSpec::new(Rect::new(0, 0, size.width, size.height)))
            .expect("the source root");
        scene
            .insert(VIEW, NodeSpec::new(Rect::new(10, 20, size.width, size.height)))
            .expect("the view");
        scene
            .set_class(VIEW, Some("view".to_owned()))
            .expect("the view's class");
        scene.set_source(VIEW, Some(SOURCE)).expect("the view's source");
    });
}

/// Gives the View a camera node `projection` at `position`, with `numbers` on top of the defaults.
fn camera(composer: &mut Composer, projection: &str, position: Vec3, numbers: &[(&str, f64)]) {
    let camera = NodeId::new(102);
    composer.with_scene(|scene| {
        scene
            .insert(camera, NodeSpec::new(Rect::new(0, 0, 0, 0)))
            .expect("a camera node");
        scene
            .set_class(camera, Some("camera".to_owned()))
            .expect("the camera's class");
        scene
            .set_text(camera, "projection", projection.to_owned())
            .expect("the camera's projection");
        for (key, value) in numbers {
            scene.set_number(camera, key, *value).expect("a camera number");
        }
        let mut pose = *scene.transform(camera).expect("the camera's transform");
        pose.base.offset = position;
        scene.set_transform(camera, pose).expect("the camera's pose");
        scene.set_camera(VIEW, Some(camera)).expect("the view's camera");
    });
}

/// Draws a frame for `output`, which is when the composer draws its Views.
fn draw(composer: &mut Composer, output: OutputId) -> OutputFrame {
    composer.damage_output(output);
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
}

/// The same world on the CPU path and on the GPU path, each drawn once.
fn both(build: impl Fn(&mut Composer)) -> ((Composer, OutputId), (Composer, OutputId)) {
    let [cpu, gpu] = [false, true].map(|gpu| {
        let (mut composer, output) = composer();
        composer.set_gpu_views(gpu);
        build(&mut composer);
        draw(&mut composer, output);
        (composer, output)
    });
    (cpu, gpu)
}

/// The picture the CPU path drew for the View, the one a capture reads.
fn cpu_picture(composer: &Composer) -> Arc<Image> {
    composer
        .view_pictures()
        .into_iter()
        .find(|(view, _)| *view == VIEW)
        .expect("the CPU path drew the View")
        .1
}

/// The scene the GPU path made for the View.
fn scene_of(composer: &Composer) -> Arc<ViewScene> {
    composer
        .view_scenes()
        .into_iter()
        .find(|(view, _)| *view == VIEW)
        .expect("the GPU path made a scene for the View")
        .1
}

/// A renderer, or a printed reason when this machine cannot make a GLES context.
fn gles() -> Option<Gles> {
    match Gles::new(Size::new(8, 8)) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            println!("skipping the GLES half of the case: {error} (no surfaceless EGL context on this machine)");
            None
        }
    }
}

/// How many pixels of two pictures differ by more than [`TOLERANCE`] on some channel.
fn differing(cpu: &Image, gpu: &Image) -> usize {
    assert_eq!(cpu.size(), gpu.size(), "the two pictures are the same size");
    let size = cpu.size();
    (0..size.height as i32)
        .flat_map(|y| (0..size.width as i32).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let (a, b) = (
                cpu.colour_at(*x, *y).expect("a CPU pixel"),
                gpu.colour_at(*x, *y).expect("a GPU pixel"),
            );
            a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > TOLERANCE)
        })
        .count()
}

/// Builds the world, draws the View both ways and asserts that at most `share` of the picture's pixels differ.
fn assert_parity(name: &str, share: f64, build: impl Fn(&mut Composer)) {
    let (cpu, gpu) = both(build);
    let Some(mut gles) = gles() else {
        return;
    };
    let expected = cpu_picture(&cpu.0);
    let drawn = gles
        .view_pixels(&scene_of(&gpu.0), false)
        .expect("the GLES renderer draws the scene");
    let total = expected.size().width as usize * expected.size().height as usize;
    let wrong = differing(&expected, &drawn);
    let driver = gles.driver();
    assert!(
        wrong as f64 <= share * total as f64,
        "{name}: {wrong} of {total} pixels differ by more than {TOLERANCE} (allowed {share}) on {driver}"
    );
}

const FLAT: Size = Size::new(80, 60);

#[test]
fn canvases_stacked_in_a_flat_view_match_the_cpu_rig_exactly() {
    assert_parity("two canvases", 0.0, |composer| {
        view_of_source(composer, FLAT);
        canvas(composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
        canvas(composer, 2, 101, Rect::new(30, 20, 40, 30), GREEN, BLUE);
    });
}

#[test]
fn a_nodes_opacity_blends_the_same_way() {
    assert_parity("opacity", 0.0, |composer| {
        view_of_source(composer, FLAT);
        canvas(composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
        canvas(composer, 2, 101, Rect::new(30, 20, 40, 30), GREEN, BLUE);
        transform(composer, 2, |transform| transform.opacity = 0.5);
        // The opacity of a parent multiplies into its children.
        plain(composer, 3, 101, Rect::new(0, 0, 80, 60));
        canvas(composer, 4, 3, Rect::new(50, 0, 30, 20), BLUE, RED);
        transform(composer, 3, |transform| transform.opacity = 0.25);
    });
}

#[test]
fn a_clipping_ancestor_keeps_its_children_to_its_rectangle() {
    assert_parity("clip", 0.0, |composer| {
        view_of_source(composer, FLAT);
        plain(composer, 1, 101, Rect::new(15, 10, 40, 30));
        composer.with_scene(|scene| scene.set_clip(NodeId::new(1), true).expect("the clip"));
        // The child is bigger than its clipping parent on every side.
        canvas(composer, 2, 1, Rect::new(-10, -10, 80, 60), RED, BLUE);
        // And a nested clip cuts the first further.
        plain(composer, 3, 1, Rect::new(20, 0, 30, 20));
        composer.with_scene(|scene| scene.set_clip(NodeId::new(3), true).expect("the nested clip"));
        canvas(composer, 4, 3, Rect::new(-5, -5, 60, 40), GREEN, BLUE);
    });
}

#[test]
fn a_child_that_paints_behind_its_parent_is_drawn_first() {
    assert_parity("behind", 0.0, |composer| {
        view_of_source(composer, FLAT);
        canvas(composer, 1, 101, Rect::new(10, 10, 40, 30), RED, BLUE);
        canvas(composer, 2, 1, Rect::new(-5, -5, 30, 20), GREEN, BLUE);
        composer.with_scene(|scene| scene.set_behind(NodeId::new(2), true).expect("behind"));
        // A sibling after it is above both.
        canvas(composer, 3, 101, Rect::new(35, 25, 30, 20), BLUE, GREEN);
    });
}

#[test]
fn an_invisible_node_and_a_node_of_no_opacity_take_their_subtrees_with_them() {
    assert_parity("hidden", 0.0, |composer| {
        view_of_source(composer, FLAT);
        plain(composer, 1, 101, Rect::new(0, 0, 80, 60));
        canvas(composer, 2, 1, Rect::new(0, 0, 40, 30), RED, BLUE);
        composer.with_scene(|scene| scene.set_visible(NodeId::new(1), false).expect("hidden"));
        plain(composer, 3, 101, Rect::new(0, 0, 80, 60));
        canvas(composer, 4, 3, Rect::new(40, 30, 40, 30), GREEN, BLUE);
        transform(composer, 3, |transform| transform.opacity = 0.0);
        canvas(composer, 5, 101, Rect::new(40, 0, 20, 20), BLUE, GREEN);
    });
}

#[test]
fn a_window_in_the_source_is_drawn_from_its_buffer() {
    assert_parity("window", 0.0, |composer| {
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 40, 30));
        let size = Size::new(40, 30);
        composer.set_window_content(
            window,
            Some(Content {
                src: RectF::new(0.0, 0.0, 40.0, 30.0),
                size,
                transform: Transform::Normal,
            }),
        );
        let pixels = [RED, GREEN, BLUE, GREY]
            .iter()
            .cycle()
            .take(40 * 30)
            .flatten()
            .copied()
            .collect();
        composer.set_window_source(
            window,
            Some(SurfaceSource::Snapshot(Arc::new(
                Image::new(size, 40 * 4, pixels).expect("an image of the window's size"),
            ))),
        );
        // The window's node exists from the next frame.
        composer.begin_frame();
        composer.end_frame();
        let node = composer.window_node(window).expect("the window's node");
        composer.with_scene(|scene| {
            scene
                .insert(VIEW, NodeSpec::new(Rect::new(10, 20, 40, 30)))
                .expect("the view");
            scene
                .set_class(VIEW, Some("view".to_owned()))
                .expect("the view's class");
            scene.set_source(VIEW, Some(node)).expect("the view's source");
        });
    });
}

#[test]
fn a_rotated_node_matches_the_cpu_rig_but_for_its_edges() {
    assert_parity("rotated about z", 0.03, |composer| {
        view_of_source(composer, FLAT);
        canvas(composer, 1, 101, Rect::new(20, 15, 40, 30), RED, BLUE);
        transform(composer, 1, |transform| transform.base.rotation_degrees.z = 20.0);
    });
}

#[test]
fn a_perspective_camera_matches_the_cpu_rig_but_for_edges() {
    assert_parity("perspective", 0.03, |composer| {
        view_of_source(composer, FLAT);
        canvas(composer, 1, 101, Rect::new(10, 10, 60, 40), RED, BLUE);
        transform(composer, 1, |transform| transform.base.rotation_degrees.y = 30.0);
        camera(
            composer,
            "perspective",
            Vec3 {
                x: 40.0,
                y: 30.0,
                z: 120.0,
            },
            &[],
        );
    });
}

#[test]
fn a_stereo_camera_matches_the_cpu_rig_but_for_edges() {
    assert_parity("stereo", 0.03, |composer| {
        view_of_source(composer, Size::new(80, 40));
        canvas(composer, 1, 101, Rect::new(10, 5, 60, 30), RED, BLUE);
        camera(
            composer,
            "stereo",
            Vec3 {
                x: 40.0,
                y: 20.0,
                z: 100.0,
            },
            &[],
        );
    });
}

#[test]
fn a_blank_rig_is_blank_on_both_paths() {
    assert_parity("blank", 0.0, |composer| {
        view_of_source(composer, FLAT);
        canvas(composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
        // A frustum that is not one.
        camera(
            composer,
            "perspective",
            Vec3 {
                x: 40.0,
                y: 30.0,
                z: 100.0,
            },
            &[
                ("fov_left", 10.0),
                ("fov_right", -10.0),
                ("fov_up", 10.0),
                ("fov_down", -10.0),
            ],
        );
    });
}

/// The world of the secure cases: the red canvas (node 1) is secure, the green one (node 2) is not.
fn secure_world(composer: &mut Composer) {
    view_of_source(composer, FLAT);
    canvas(composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
    canvas(composer, 2, 101, Rect::new(30, 20, 40, 30), GREEN, BLUE);
    composer.with_scene(|scene| {
        scene
            .set_value(NodeId::new(1), "secure", TreeValue::Bool(true))
            .expect("the secure value")
    });
}

#[test]
fn a_secure_node_is_masked_in_the_capture_variant_exactly_as_on_the_cpu() {
    let (cpu, gpu) = both(secure_world);
    let scene = scene_of(&gpu.0);
    assert!(
        scene.has_capture_variant(),
        "a secure node gives the scene a capture variant"
    );
    let Some(mut gles) = gles() else {
        return;
    };
    let expected = cpu_picture(&cpu.0);
    let capture = gles.view_pixels(&scene, true).expect("the capture variant is drawn");
    assert_eq!(
        differing(&expected, &capture),
        0,
        "the capture variant is the CPU's redacted picture"
    );
    assert_eq!(capture.colour_at(10, 10), Some(GREY), "the secure node is the mask");
    assert_eq!(capture.colour_at(60, 45), Some(GREEN), "the node beside it is drawn");
    let display = gles.view_pixels(&scene, false).expect("the display variant is drawn");
    assert_eq!(
        display.colour_at(10, 10),
        Some(RED_ACCENTED),
        "the display shows the node"
    );
}

/// The pixel of the red canvas (node 1) at (10, 10) of the picture: inside its blue accent, which covers the top left
/// quarter of the canvas.
const RED_ACCENTED: [u8; 4] = BLUE;

#[test]
fn omitting_a_secure_node_leaves_what_lies_beneath_it_in_both_variants_alike() {
    let build = |composer: &mut Composer| {
        composer.set_secure_treatment(SecureTreatment::Omit);
        secure_world(composer);
    };
    let (cpu, gpu) = both(build);
    let Some(mut gles) = gles() else {
        return;
    };
    let capture = gles.view_pixels(&scene_of(&gpu.0), true).expect("drawn");
    assert_eq!(differing(&cpu_picture(&cpu.0), &capture), 0);
    assert_eq!(
        capture.colour_at(10, 10),
        Some([0, 0, 0, 255]),
        "an omitted node shows the View's black"
    );
}

#[test]
fn a_gpu_view_is_a_scene_and_a_texture_layer_not_a_picture() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    view_of_source(&mut composer, FLAT);
    canvas(&mut composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
    let frame = draw(&mut composer, output);

    assert!(
        composer.view_pictures().is_empty(),
        "the composer drew no picture on the CPU"
    );
    assert!(composer.has_view_scene(VIEW));
    let layers: Vec<&Surface> = frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) => Some(surface),
            _ => None,
        })
        .filter(|surface| matches!(surface.source, SurfaceSource::View(_)))
        .collect();
    assert_eq!(layers.len(), 1, "one layer samples the View's texture");
    assert_eq!(layers[0].dst, Rect::new(10, 20, 80, 60), "where the View sits");
    assert!(
        matches!(&layers[0].source, SurfaceSource::View(texture) if texture.id == VIEW.get()),
        "and names the View"
    );
    assert_eq!(
        frame.view_scenes().len(),
        1,
        "the frame carries the scene the texture is drawn from"
    );
    assert_eq!(frame.live_views(), &[VIEW.get()]);
}

#[test]
fn a_gpu_view_off_the_output_is_alive_but_not_in_the_frame() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    view_of_source(&mut composer, FLAT);
    canvas(&mut composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
    composer.with_scene(|scene| {
        let mut transform = *scene.transform(VIEW).expect("the view's transform");
        transform.base.offset = Vec3 {
            x: 5000.0,
            y: 5000.0,
            z: 0.0,
        };
        scene.set_transform(VIEW, transform).expect("moved away");
    });
    let frame = draw(&mut composer, output);
    assert!(
        frame.view_scenes().is_empty(),
        "a View on no part of the output is not drawn for it"
    );
    assert_eq!(frame.live_views(), &[VIEW.get()], "but the renderer keeps its texture");
    assert_eq!(
        composer.view_scenes().len(),
        1,
        "and a consumer that reads it finds the scene"
    );
}

#[test]
fn a_scene_keeps_its_revision_until_the_view_is_drawn_again() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    view_of_source(&mut composer, FLAT);
    canvas(&mut composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
    draw(&mut composer, output);
    let first = scene_of(&composer);
    draw(&mut composer, output);
    assert!(
        Arc::ptr_eq(&first, &scene_of(&composer)),
        "a frame that changes nothing in the View keeps the scene, so the renderer draws nothing"
    );
    composer.with_scene(|scene| {
        scene
            .set_value(
                NodeId::new(1),
                "content",
                TreeValue::Bytes(picture((40.0, 30.0), GREEN, BLUE)),
            )
            .expect("the canvas's content")
    });
    draw(&mut composer, output);
    let second = scene_of(&composer);
    assert!(
        second.revision() > first.revision(),
        "a change in the source is a new revision"
    );
}

#[test]
fn a_gpu_view_obeys_its_fps_like_a_cpu_one() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    view_of_source(&mut composer, FLAT);
    canvas(&mut composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
    composer.with_scene(|scene| {
        scene
            .set_value(VIEW, "fps", TreeValue::Uint(10))
            .expect("the view's fps")
    });
    draw(&mut composer, output);
    let first = scene_of(&composer).revision();
    let repaint = |composer: &mut Composer, colour: [u8; 4]| {
        composer.with_scene(|scene| {
            scene
                .set_value(
                    NodeId::new(1),
                    "content",
                    TreeValue::Bytes(picture((40.0, 30.0), colour, BLUE)),
                )
                .expect("the canvas's content")
        });
    };
    repaint(&mut composer, GREEN);
    composer.advance_motion(20);
    draw(&mut composer, output);
    assert_eq!(
        scene_of(&composer).revision(),
        first,
        "a change inside the 100 ms slot waits"
    );
    composer.advance_motion(100);
    draw(&mut composer, output);
    assert!(
        scene_of(&composer).revision() > first,
        "and is drawn when the slot ends"
    );
}

#[test]
fn a_view_with_no_source_or_a_destroyed_one_has_no_scene() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    composer.with_scene(|scene| {
        scene
            .insert(VIEW, NodeSpec::new(Rect::new(10, 20, 80, 60)))
            .expect("a view");
        scene.set_class(VIEW, Some("view".to_owned())).expect("the class");
    });
    let frame = draw(&mut composer, output);
    assert!(!composer.has_view_scene(VIEW), "a View without a source draws nothing");
    assert!(frame.view_scenes().is_empty());
    assert!(frame.live_views().is_empty(), "and holds no texture");

    composer.with_scene(|scene| {
        scene
            .insert(NodeId::new(1), NodeSpec::new(Rect::new(0, 0, 80, 60)))
            .expect("a source node")
    });
    composer.with_scene(|scene| scene.set_source(VIEW, Some(NodeId::new(1))).expect("a source"));
    draw(&mut composer, output);
    assert!(composer.has_view_scene(VIEW));
    composer.with_scene(|scene| {
        scene.remove(NodeId::new(1)).expect("the source is destroyed");
    });
    let frame = draw(&mut composer, output);
    assert!(
        !composer.has_view_scene(VIEW),
        "a destroyed source leaves the View with no picture"
    );
    assert!(frame.live_views().is_empty(), "so the renderer gives its texture back");
}

#[test]
fn a_destroyed_view_leaves_the_live_list() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    view_of_source(&mut composer, FLAT);
    draw(&mut composer, output);
    assert_eq!(draw(&mut composer, output).live_views(), &[VIEW.get()]);
    composer.with_scene(|scene| {
        scene.remove(VIEW).expect("the View is destroyed");
    });
    assert!(draw(&mut composer, output).live_views().is_empty());
    assert!(composer.view_scenes().is_empty());
}

#[test]
fn a_view_of_no_size_or_too_big_a_size_draws_nothing_on_either_path() {
    for gpu in [false, true] {
        for size in [
            Size::new(0, 30),
            Size::new(30, 0),
            Size::new(gnoblin_render_frame::view_scene::MAX_SIDE + 1, 4),
            Size::new(
                gnoblin_render_frame::view_scene::MAX_SIDE,
                gnoblin_render_frame::view_scene::MAX_SIDE,
            ),
        ] {
            let (mut composer, output) = composer();
            composer.set_gpu_views(gpu);
            view_of_source(&mut composer, size);
            let frame = draw(&mut composer, output);
            assert!(
                composer.view_pictures().is_empty() && composer.view_scenes().is_empty(),
                "a View of {size:?} has no picture (gpu: {gpu})"
            );
            assert!(
                !frame
                    .layers()
                    .iter()
                    .any(|layer| matches!(layer, Layer::Surface(surface) if !matches!(surface.source, SurfaceSource::Buffer(_)))),
                "and no layer"
            );
            assert!(frame.view_scenes().is_empty(), "and no scene (gpu: {gpu})");
        }
    }
}

#[test]
fn a_view_at_the_largest_size_is_still_drawn() {
    let (mut composer, output) = composer();
    composer.set_gpu_views(true);
    let size = Size::new(gnoblin_render_frame::view_scene::MAX_SIDE, 4096);
    view_of_source(&mut composer, size);
    draw(&mut composer, output);
    assert!(composer.has_view_scene(VIEW), "a picture the cap allows is described");
}

#[test]
fn turning_the_gpu_path_on_or_off_draws_the_views_again_in_the_other_form() {
    let (mut composer, output) = composer();
    view_of_source(&mut composer, FLAT);
    canvas(&mut composer, 1, 101, Rect::new(2, 2, 40, 30), RED, BLUE);
    draw(&mut composer, output);
    assert_eq!(composer.view_pictures().len(), 1);
    composer.set_gpu_views(true);
    draw(&mut composer, output);
    assert!(composer.view_pictures().is_empty());
    assert!(composer.has_view_scene(VIEW));
    composer.set_gpu_views(false);
    draw(&mut composer, output);
    assert_eq!(composer.view_pictures().len(), 1);
    assert!(!composer.has_view_scene(VIEW));
}

/// What one redraw of a View costs on each path.
///
/// It is a measurement, not a check, so it is ignored by default:
///
/// ```text
/// cargo test --release -p gnoblin-backend --lib the_cost_of_one_view_redraw -- --ignored --nocapture
/// ```
///
/// Run it in release. The columns, for one View redraw:
///
/// - **CPU path, compositor thread**: the composer rasterises the source and builds the picture
///   (`Composer::begin_frame`, which is where a View is drawn). This is the cost the GPU path removes.
/// - **GPU path, compositor thread**: the composer builds the scene (`Composer::begin_frame` with the GPU path on):
///   the layers and the camera matrices, no pixel.
/// - **GPU path, GL draw**: `Gles::draw` of the frame that carries the scene, which draws the scene into the View's
///   texture and the frame that samples it. On llvmpipe this is a software GL driver, so the number is the time CPU
///   threads spend emulating a GPU: it is **not** a GPU number, and the absolute value should not be read as one.
///   What it shows is the work that moved off the compositor's own rasteriser loop and into the driver.
/// - **Readback**: `Gles::view_pixels` of a texture already drawn, which is the only CPU copy a View has, and is made
///   only for a View a consumer reads (a capture, a stream, the XR extension, a still).
///
/// The world is the one `crates/gnoblin-render-software/tests/view_cost.rs` measures: one canvas covering the View,
/// seen flat and through a stereo camera. A source canvas is painted once and kept; a redraw is forced by toggling the
/// source's opacity, which does not repaint the canvas.
#[test]
#[ignore = "a measurement, not a check: run it in release with --ignored --nocapture"]
fn the_cost_of_one_view_redraw_on_each_path() {
    use std::time::{Duration, Instant};

    const WARM_UP: usize = 3;
    const RUNS: usize = 15;
    const SIZES: [(u32, u32); 5] = [(320, 200), (640, 360), (1280, 720), (1920, 1080), (3840, 2160)];

    let median = |mut times: Vec<Duration>| {
        times.sort_unstable();
        times[times.len() / 2].as_secs_f64() * 1000.0
    };
    let world = |composer: &mut Composer, size: Size, stereo: bool| {
        view_of_source(composer, size);
        canvas(composer, 1, 101, Rect::new(0, 0, size.width, size.height), RED, BLUE);
        if stereo {
            camera(
                composer,
                "stereo",
                Vec3 {
                    x: f64::from(size.width) / 2.0,
                    y: f64::from(size.height) / 2.0,
                    z: f64::from(size.height),
                },
                &[
                    ("fov_left", -52.0),
                    ("fov_right", 42.0),
                    ("fov_up", 43.0),
                    ("fov_down", -47.0),
                    ("near", 1.0),
                    ("far", 100_000.0),
                ],
            );
        }
    };
    // Toggling the source's opacity marks the View stale without repainting the canvas.
    let dirty = |composer: &mut Composer, step: usize| {
        transform(composer, 101, |transform| {
            transform.opacity = if step % 2 == 0 { 1.0 } else { 0.99 }
        });
    };
    let Some(mut gles) = Gles::new(Size::new(800, 600)).ok() else {
        println!("no GLES context on this machine: the GPU columns cannot be measured");
        return;
    };
    println!("GLES driver: {} (software: {})", gles.driver(), gles.is_software());
    println!(
        "| View size | camera | CPU path, compositor thread | GPU path, compositor thread | GPU path, GL draw | readback |"
    );
    println!(
        "| --------- | ------ | --------------------------- | --------------------------- | ----------------- | -------- |"
    );
    for (width, height) in SIZES {
        let size = Size::new(width, height);
        for stereo in [false, true] {
            let mut cpu = Vec::new();
            let (mut composer, output) = composer();
            composer.set_gpu_views(false);
            world(&mut composer, size, stereo);
            draw(&mut composer, output);
            for step in 0..WARM_UP + RUNS {
                dirty(&mut composer, step);
                let started = Instant::now();
                composer.begin_frame();
                let elapsed = started.elapsed();
                composer.end_frame();
                if step >= WARM_UP {
                    cpu.push(elapsed);
                }
            }

            let (mut composer, output) = composer();
            composer.set_gpu_views(true);
            world(&mut composer, size, stereo);
            draw(&mut composer, output);
            let (mut build, mut gl, mut read) = (Vec::new(), Vec::new(), Vec::new());
            for step in 0..WARM_UP + RUNS {
                dirty(&mut composer, step);
                composer.damage_output(output);
                let started = Instant::now();
                composer.begin_frame();
                let built = started.elapsed();
                let mut frame = OutputFrame::new();
                composer.compose(output, &mut frame);
                composer.end_frame();
                let started = Instant::now();
                gles.draw(&frame, Scale::IDENTITY).expect("the frame is drawn");
                let drawn = started.elapsed();
                let scene = scene_of(&composer);
                let started = Instant::now();
                let image = gles.view_pixels(&scene, true).expect("the picture is read back");
                let read_back = started.elapsed();
                assert_eq!(image.size(), size);
                if step >= WARM_UP {
                    build.push(built);
                    gl.push(drawn);
                    read.push(read_back);
                }
            }
            println!(
                "| {width}x{height} | {} | {:.2} ms | {:.2} ms | {:.2} ms | {:.2} ms |",
                if stereo { "stereo" } else { "flat" },
                median(cpu),
                median(build),
                median(gl),
                median(read)
            );
        }
    }
}
