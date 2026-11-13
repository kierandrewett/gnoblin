//! Secure content in a View: whatever shows or reads a View's picture redacts a secure node the same way.
//!
//! A node is *secure* when its own `secure` value is true, when it is a window the shell or a rule marked, or when any
//! ancestor is: everything below a secure node is secure with it. Four things read a View's picture, and each must
//! decide that from the one predicate ([`Composer::secure_nodes`]):
//!
//! 1. **a capture of the View**: the CPU rig's redacted picture, or the GPU scene's capture variant read back. It is
//!    what `View:capture`, `ext-image-copy-capture`, `gnoblin_view_source_v1` (so the XR extension), a PipeWire stream
//!    and the GPU-buffer copy all read;
//! 2. **the View on the screen**: it shows the real pixels, as the screen shows a secure window;
//! 3. **a capture of the output that shows the View**: the whole View is a secure layer, so the output capture
//!    replaces it ([`OutputFrame::redacted`]) and no consumer reaches the pixels through the View;
//! 4. **the window's own layers**, fades included: a closing window keeps the mark it had.
//!
//! Each case runs on the CPU path and on the GPU path (a [`ViewScene`] the GLES renderer draws, read back for a
//! capture), with the window's pixels in shared memory (a snapshot, the stand-in a composer test can make) and in a
//! DMA-BUF (`dmabuf_source`, a descriptor on `/dev/null`: the shape of a GPU buffer, which is all the composer reads).
//! A case that needs the GLES renderer prints the reason and skips its GPU half on a machine with no surfaceless EGL
//! context; the structural assertions on the scene (which source each capture layer samples) still run there.

use gnoblin_render_software::Software;
use smithay::backend::allocator::dmabuf::Dmabuf;

use super::super::surface_tree_tests::dmabuf_source;
use super::*;

const WINDOW: WindowId = WindowId::new(1);
/// A node that is not a window and holds things.
const ANCESTOR: NodeId = NodeId::new(50);
/// The side of the window, and of the View of it.
const SIDE: u32 = 40;
const BLACK: [u8; 4] = [0, 0, 0, 255];
/// Where the window sits on the output; the View of it is drawn elsewhere.
const WINDOW_AT: Rect = Rect::new(300, 300, SIDE, SIDE);

/// What holds a window's pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Buffer {
    /// Shared memory: red.
    Shm,
    /// A GPU buffer: the reader of the CPU path returns green for it, and a GPU renderer would import it.
    Dmabuf,
}

const BUFFERS: [Buffer; 2] = [Buffer::Shm, Buffer::Dmabuf];

fn solid(colour: [u8; 4]) -> Image {
    let size = Size::new(SIDE, SIDE);
    Image::new(size, SIDE as usize * 4, colour.repeat((SIDE * SIDE) as usize)).expect("an image")
}

/// Maps the window with `buffer`, draws once so it has a node, and answers the node.
fn map_window(composer: &mut Composer, buffer: Buffer) -> NodeId {
    composer.insert_window(WINDOW, WINDOW_AT);
    composer.set_window_content(
        WINDOW,
        Some(Content {
            src: RectF::new(0.0, 0.0, f64::from(SIDE), f64::from(SIDE)),
            size: Size::new(SIDE, SIDE),
            transform: Transform::Normal,
        }),
    );
    let source = match buffer {
        Buffer::Shm => SurfaceSource::Snapshot(Arc::new(solid(RED))),
        Buffer::Dmabuf => dmabuf_source(Size::new(SIDE, SIDE)),
    };
    composer.set_window_source(WINDOW, Some(source));
    composer.begin_frame();
    composer.end_frame();
    composer.window_node(WINDOW).expect("the window's node")
}

/// A View of `source` at (10, 20) on the output.
fn view_of(composer: &mut Composer, source: NodeId) {
    view_node(composer, VIEW, source, None);
}

/// The View `view` of `source`, under `parent` or at the root, at (10, 20).
fn view_node(composer: &mut Composer, view: NodeId, source: NodeId, parent: Option<NodeId>) {
    composer.with_scene(|scene| {
        let mut spec = NodeSpec::new(Rect::new(10, 20, SIDE, SIDE));
        spec.parent = parent;
        scene.insert(view, spec).expect("the view");
        scene
            .set_class(view, Some("view".to_owned()))
            .expect("the view's class");
        scene.set_source(view, Some(source)).expect("the view's source");
    });
}

/// A plain node `ANCESTOR` at the origin, secure when `secure`.
fn ancestor(composer: &mut Composer, secure: bool) {
    composer.with_scene(|scene| {
        scene
            .insert(ANCESTOR, NodeSpec::new(Rect::new(0, 0, 400, 400)))
            .expect("the ancestor");
    });
    if secure {
        set_secure(composer, ANCESTOR, true);
    }
}

fn set_secure(composer: &mut Composer, node: NodeId, secure: bool) {
    composer.with_scene(|scene| {
        scene
            .set_value(node, "secure", TreeValue::Bool(secure))
            .expect("the secure value")
    });
    composer.follow_shell_secure();
}

fn reparent(composer: &mut Composer, node: NodeId, parent: NodeId) {
    composer.with_scene(|scene| scene.reparent(node, Some(parent)).expect("the node is moved"));
}

/// Draws a frame for `output` where a GPU buffer is read as green, which is when the composer draws its Views.
fn draw_reading(composer: &mut Composer, output: OutputId) -> OutputFrame {
    composer.damage_output(output);
    let mut reader = |_: &Dmabuf| Some(solid(GREEN));
    composer.begin_frame_reading(&mut reader);
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
}

/// A composer on the path under test, with the treatment a capture draws in a secure node's place, and the renderer
/// that reads its GPU Views back (`None` on the CPU path, and on a machine with no surfaceless EGL context).
///
/// Each composer gets a renderer of its own: a renderer keeps a View's texture for the `(id, revision)` it drew, and
/// two composers count revisions each from one.
fn session(gpu: bool, treatment: SecureTreatment) -> (Composer, OutputId, Option<Gles>) {
    let (mut composer, output) = composer();
    composer.set_gpu_views(gpu);
    composer.set_secure_treatment(treatment);
    (composer, output, gpu.then(gles).flatten())
}

/// What a capture of the View reads: the CPU path's capture picture, or the GPU scene's capture variant read back.
/// `None` when the GPU path has no renderer to read it on.
fn captured(composer: &Composer, gles: &mut Option<Gles>) -> Option<Image> {
    captured_view(composer, VIEW, gles)
}

/// [`captured`] for the View `view`.
fn captured_view(composer: &Composer, view: NodeId, gles: &mut Option<Gles>) -> Option<Image> {
    if let Some(scene) = scene_view(composer, view) {
        return gles.as_mut()?.view_pixels(&scene, true).ok();
    }
    composer
        .view_pictures()
        .into_iter()
        .find(|(node, _)| *node == view)
        .map(|(_, picture)| Image::clone(&picture))
}

/// The scene the GPU path made for `view`.
fn scene_view(composer: &Composer, view: NodeId) -> Option<Arc<ViewScene>> {
    composer
        .view_scenes()
        .into_iter()
        .find_map(|(node, scene)| (node == view).then_some(scene))
}

/// What the View shows on the screen: the CPU picture, or the GPU display variant read back.
fn displayed(composer: &Composer, gles: &mut Option<Gles>) -> Option<Image> {
    if composer.has_view_scene(VIEW) {
        return gles.as_mut()?.view_pixels(&scene_of(composer), false).ok();
    }
    composer.views.get(&VIEW)?.image.as_deref().cloned()
}

fn pixels(image: &Image) -> Vec<[u8; 4]> {
    let size = image.size();
    (0..size.height as i32)
        .flat_map(|y| (0..size.width as i32).map(move |x| (x, y)))
        .map(|(x, y)| image.colour_at(x, y).expect("a pixel"))
        .collect()
}

/// Whether every pixel of the picture is `colour`.
fn all_are(image: &Image, colour: [u8; 4]) -> bool {
    pixels(image).into_iter().all(|pixel| pixel == colour)
}

/// What a capture draws where a secure node is.
fn redaction(treatment: SecureTreatment) -> [u8; 4] {
    match treatment {
        SecureTreatment::Mask => GREY,
        SecureTreatment::Omit => BLACK,
    }
}

/// The source each draw of the eyes samples, in order.
fn sources(eyes: &[ViewEye]) -> Vec<&SurfaceSource> {
    eyes.iter()
        .flat_map(|eye| eye.ops.iter())
        .filter_map(|op| match op {
            ViewOp::Draw(Layer::Surface(surface)) => Some(&surface.source),
            _ => None,
        })
        .collect()
}

/// Whether a source is the one-pixel mask a capture draws in a secure node's place.
fn is_mask(source: &SurfaceSource) -> bool {
    matches!(source, SurfaceSource::Snapshot(image) if image.size() == Size::new(1, 1))
}

/// Asserts what a GPU scene's two variants sample for a window of `buffer`: the display variant its real buffer, and
/// the capture variant, when `secure`, nothing of it but the mask (or nothing at all under `Omit`).
fn assert_scene_sources(scene: &ViewScene, buffer: Buffer, secure: bool, treatment: SecureTreatment, label: &str) {
    let real = |source: &&SurfaceSource| match buffer {
        Buffer::Shm => matches!(source, SurfaceSource::Snapshot(image) if image.size() == Size::new(SIDE, SIDE)),
        Buffer::Dmabuf => matches!(source, SurfaceSource::Dmabuf(_)),
    };
    let display = sources(scene.eyes(false));
    assert!(
        display.iter().any(real),
        "{label}: the display variant samples the window's own buffer"
    );
    assert_eq!(
        scene.has_capture_variant(),
        secure,
        "{label}: a capture variant exists when a node is secure"
    );
    let capture = sources(scene.eyes(true));
    if secure {
        assert!(
            !capture.iter().any(real),
            "{label}: the capture variant never names the secure window's buffer"
        );
        let expected = usize::from(treatment == SecureTreatment::Mask);
        assert_eq!(
            capture.iter().filter(|source| is_mask(source)).count(),
            expected,
            "{label}: the capture variant holds the mask (or nothing under omit) in the window's place"
        );
        assert_eq!(capture.len(), expected, "{label}: and nothing else");
    } else {
        assert!(
            capture.iter().any(real),
            "{label}: with nothing secure a capture reads the real buffer"
        );
    }
}

/// How a window is made secure.
#[derive(Clone, Copy, Debug)]
enum Marking {
    /// The mark a window rule or the compositor sets on the window.
    Window,
    /// The `secure` value the shell writes on the window's node.
    NodeValue,
}

fn mark(composer: &mut Composer, node: NodeId, marking: Marking) {
    match marking {
        Marking::Window => {
            composer.set_window_secure(WINDOW, true);
        }
        Marking::NodeValue => set_secure(composer, node, true),
    }
}

/// A secure window is redacted in a View's capture as it is in the capture of an output, on both paths, whether its
/// pixels are in shared memory or a GPU buffer, however it was marked, under either treatment. The View itself, on
/// the screen, still shows it.
#[test]
fn a_secure_window_is_redacted_in_a_views_capture_whatever_holds_its_pixels() {
    for gpu in [false, true] {
        for buffer in BUFFERS {
            for marking in [Marking::Window, Marking::NodeValue] {
                for treatment in [SecureTreatment::Mask, SecureTreatment::Omit] {
                    let label = format!("gpu: {gpu}, {buffer:?}, {marking:?}, {treatment:?}");
                    let (mut composer, output, mut gles) = session(gpu, treatment);
                    let node = map_window(&mut composer, buffer);
                    view_of(&mut composer, node);
                    mark(&mut composer, node, marking);
                    draw_reading(&mut composer, output);

                    if gpu {
                        assert_scene_sources(&scene_of(&composer), buffer, true, treatment, &label);
                    }
                    let Some(capture) = captured(&composer, &mut gles) else {
                        assert!(gpu && gles.is_none(), "{label}: a capture has a picture");
                        continue;
                    };
                    assert!(
                        all_are(&capture, redaction(treatment)),
                        "{label}: every pixel of a capture is the redaction, and none is the window's"
                    );
                    // The screen is what the user sees: the window is there.
                    if let Some(display) = displayed(&composer, &mut gles)
                        && !(gpu && buffer == Buffer::Dmabuf)
                    {
                        let expected = if buffer == Buffer::Shm { RED } else { GREEN };
                        assert!(
                            all_are(&display, expected),
                            "{label}: the View shows the window on the screen"
                        );
                    }
                }
            }
        }
    }
}

/// With nothing secure, a capture of the View is what the View shows (the control of the case above), and a GPU
/// buffer's real pixels are in it. The CPU path reads them back through the backend; a backend that cannot read them
/// masks the window, so nothing is ever shown that was not read.
#[test]
fn without_the_mark_a_capture_reads_the_real_pixels_of_a_gpu_buffer_and_an_unreadable_one_is_masked() {
    for buffer in BUFFERS {
        let (mut composer, output, mut gles) = session(false, SecureTreatment::Mask);
        let node = map_window(&mut composer, buffer);
        view_of(&mut composer, node);
        draw_reading(&mut composer, output);
        let capture = captured(&composer, &mut gles).expect("the CPU path drew a picture");
        let expected = if buffer == Buffer::Shm { RED } else { GREEN };
        assert!(all_are(&capture, expected), "{buffer:?}: the real pixels are captured");
    }

    // No reader: the CPU path cannot read a GPU buffer and fails closed, drawing the neutral grey.
    let (mut composer, output, mut gles) = session(false, SecureTreatment::Mask);
    let node = map_window(&mut composer, Buffer::Dmabuf);
    view_of(&mut composer, node);
    composer.damage_output(output);
    composer.begin_frame();
    composer.end_frame();
    let capture = captured(&composer, &mut gles).expect("a picture");
    assert!(all_are(&capture, GREY), "a GPU buffer nothing can read is masked");

    // The GPU path samples the buffer itself in both variants when nothing is secure.
    for buffer in BUFFERS {
        let (mut composer, output, _) = session(true, SecureTreatment::Mask);
        let node = map_window(&mut composer, buffer);
        view_of(&mut composer, node);
        draw_reading(&mut composer, output);
        assert_scene_sources(
            &scene_of(&composer),
            buffer,
            false,
            SecureTreatment::Mask,
            &format!("{buffer:?}"),
        );
    }
}

/// The same GPU buffer is redacted when it carries the mark, and shows when the mark is cleared: the choice follows the
/// mark, not the buffer, and a redraw after the mark never reuses the picture drawn before it.
#[test]
fn the_same_gpu_buffer_is_redacted_with_the_mark_and_shown_without_it() {
    for gpu in [false, true] {
        let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
        let node = map_window(&mut composer, Buffer::Dmabuf);
        view_of(&mut composer, node);
        draw_reading(&mut composer, output);
        let first = scene_for(&composer, gpu).map(|scene| scene.revision());
        composer.set_window_secure(WINDOW, true);
        draw_reading(&mut composer, output);
        if gpu {
            let scene = scene_of(&composer);
            assert!(
                scene.revision() > first.expect("a scene"),
                "the mark draws the View again"
            );
            assert_scene_sources(&scene, Buffer::Dmabuf, true, SecureTreatment::Mask, "marked");
        }
        if let Some(capture) = captured(&composer, &mut gles) {
            assert!(all_are(&capture, GREY), "gpu: {gpu}: marked, the buffer is the mask");
        }
        composer.set_window_secure(WINDOW, false);
        draw_reading(&mut composer, output);
        if gpu {
            assert_scene_sources(
                &scene_of(&composer),
                Buffer::Dmabuf,
                false,
                SecureTreatment::Mask,
                "cleared",
            );
        } else {
            let capture = captured(&composer, &mut gles).expect("a picture");
            assert!(all_are(&capture, GREEN), "cleared, the buffer's real pixels are back");
        }
    }
}

fn scene_for(composer: &Composer, gpu: bool) -> Option<Arc<ViewScene>> {
    gpu.then(|| scene_of(composer))
}

/// The View of `SOURCE` that holds the red canvas (node 1), `SIDE` square.
fn canvas_world(composer: &mut Composer) {
    view_of_source(composer, Size::new(SIDE, SIDE));
    canvas(composer, 1, 101, Rect::new(0, 0, SIDE, SIDE), RED, BLUE);
}

/// What the View sits on in a case of a secure node *above* the node the View is of.
#[derive(Clone, Copy, Debug)]
enum Above {
    /// The View is of a plain node (`SOURCE`) that is a child of the secure ancestor.
    Source,
    /// The View is of the canvas itself, which is a descendant of the secure ancestor.
    Canvas,
    /// The View is of a window's node, which is a child of the secure ancestor.
    Window(Buffer),
}

/// Builds a View whose source has a secure ancestor that is outside the source's own subtree.
fn below_a_secure_ancestor(composer: &mut Composer, above: Above) {
    match above {
        Above::Source => {
            canvas_world(composer);
            ancestor(composer, true);
            reparent(composer, SOURCE, ANCESTOR);
        }
        Above::Canvas => {
            canvas_world(composer);
            composer.with_scene(|scene| {
                scene
                    .set_source(VIEW, Some(NodeId::new(1)))
                    .expect("the canvas is the source")
            });
            ancestor(composer, true);
            reparent(composer, SOURCE, ANCESTOR);
        }
        Above::Window(buffer) => {
            let node = map_window(composer, buffer);
            view_of(composer, node);
            ancestor(composer, true);
            reparent(composer, node, ANCESTOR);
        }
    }
}

/// Everything below a secure node is secure with it, whether the node is inside the View's source or above it. A View
/// of a node that lies below a secure ancestor is of secure content, so its capture is redacted on both paths and the
/// View itself is a secure layer on the output.
#[test]
fn a_source_below_a_secure_ancestor_is_redacted_in_the_views_capture_and_the_view_is_a_secure_layer() {
    for gpu in [false, true] {
        for above in [
            Above::Source,
            Above::Canvas,
            Above::Window(Buffer::Shm),
            Above::Window(Buffer::Dmabuf),
        ] {
            for treatment in [SecureTreatment::Mask, SecureTreatment::Omit] {
                let label = format!("gpu: {gpu}, {above:?}, {treatment:?}");
                let (mut composer, output, mut gles) = session(gpu, treatment);
                below_a_secure_ancestor(&mut composer, above);
                let frame = draw_reading(&mut composer, output);

                if gpu {
                    assert!(
                        scene_of(&composer).has_capture_variant(),
                        "{label}: a source below a secure ancestor has a capture variant"
                    );
                }
                if let Some(capture) = captured(&composer, &mut gles) {
                    assert!(
                        all_are(&capture, redaction(treatment)),
                        "{label}: a capture of a View of secure content holds none of it"
                    );
                } else {
                    assert!(gpu && gles.is_none(), "{label}: a capture has a picture");
                }
                assert!(
                    view_layers(&frame).iter().all(|layer| layer.secure),
                    "{label}: the View is a secure layer, so a capture of the output replaces it"
                );
            }
        }
    }
}

/// The layers of `frame` that show the View: its snapshot on the CPU path, its texture on the GPU path.
fn view_layers(frame: &OutputFrame) -> Vec<&Surface> {
    let layers: Vec<&Surface> = frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) if surface.dst == Rect::new(10, 20, SIDE, SIDE) => Some(surface),
            _ => None,
        })
        .collect();
    assert_eq!(layers.len(), 1, "the frame draws the View once");
    layers
}

/// A View that is itself below a secure node is secure: its picture is what that node's subtree shows.
#[test]
fn a_view_below_a_secure_node_is_a_secure_layer_though_its_source_is_not() {
    for gpu in [false, true] {
        let (mut composer, output, _) = session(gpu, SecureTreatment::Mask);
        canvas_world(&mut composer);
        ancestor(&mut composer, true);
        reparent(&mut composer, VIEW, ANCESTOR);
        let frame = draw_reading(&mut composer, output);
        assert!(
            view_layers(&frame).iter().all(|layer| layer.secure),
            "gpu: {gpu}: a View under a secure node is a secure layer"
        );
        // And one that is not below a secure node is not.
        let (mut composer, output, _) = session(gpu, SecureTreatment::Mask);
        canvas_world(&mut composer);
        ancestor(&mut composer, false);
        reparent(&mut composer, VIEW, ANCESTOR);
        let frame = draw_reading(&mut composer, output);
        assert!(
            view_layers(&frame).iter().all(|layer| !layer.secure),
            "gpu: {gpu}: the control"
        );
    }
}

/// A window below a secure node is secure on the screen's capture as it is in a View's.
#[test]
fn a_window_below_a_secure_node_is_a_secure_layer() {
    for buffer in BUFFERS {
        let (mut composer, output, _) = session(false, SecureTreatment::Mask);
        let node = map_window(&mut composer, buffer);
        ancestor(&mut composer, true);
        reparent(&mut composer, node, ANCESTOR);
        let frame = draw_reading(&mut composer, output);
        let window: Vec<&Surface> = frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(surface) if surface.dst == WINDOW_AT => Some(surface),
                _ => None,
            })
            .collect();
        assert_eq!(window.len(), 1, "{buffer:?}: the window is drawn once");
        assert!(
            window[0].secure,
            "{buffer:?}: a window below a secure node is a secure layer"
        );
    }
}

/// A source that moves under a secure node is redacted in the next picture, one that moves out is shown again, and
/// the mark of a node that is only a sibling or a parent reaches neither.
#[test]
fn moving_a_source_under_a_secure_node_redacts_it_at_once_and_a_sibling_is_not_secure() {
    for gpu in [false, true] {
        let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
        canvas_world(&mut composer);
        ancestor(&mut composer, true);
        draw_reading(&mut composer, output);
        let open = captured(&composer, &mut gles);
        if let Some(open) = &open {
            assert_eq!(
                open.colour_at(30, 30),
                Some(RED),
                "gpu: {gpu}: the control shows the canvas"
            );
        }
        reparent(&mut composer, SOURCE, ANCESTOR);
        draw_reading(&mut composer, output);
        if let Some(moved) = captured(&composer, &mut gles) {
            assert!(
                all_are(&moved, GREY),
                "gpu: {gpu}: below the secure node the canvas is the mask"
            );
        }
        composer.with_scene(|scene| {
            scene
                .insert(NodeId::new(NODE_OUTSIDE), NodeSpec::new(Rect::new(0, 0, 400, 400)))
                .expect("a node outside the secure one")
        });
        reparent(&mut composer, SOURCE, NodeId::new(NODE_OUTSIDE));
        draw_reading(&mut composer, output);
        if let Some(shown) = captured(&composer, &mut gles) {
            assert_eq!(
                shown.colour_at(30, 30),
                Some(RED),
                "gpu: {gpu}: out from under it, the canvas shows"
            );
        }
    }
}

/// A plain node outside everything secure, to move a source to.
const NODE_OUTSIDE: u64 = 60;

/// A second source root (node 70) holding a small green canvas (node 2), to switch a View to.
fn second_source(composer: &mut Composer) {
    composer.with_scene(|scene| {
        scene
            .insert(NodeId::new(70), NodeSpec::new(Rect::new(0, 0, SIDE, SIDE)))
            .expect("the second source")
    });
    canvas(composer, 2, 70, Rect::new(0, 0, 10, 10), GREEN, GREEN);
}

/// A View never shows pixels of a source it no longer has: the texture or picture of the old source is not reused
/// where the new one draws nothing, and the secure status of each source is its own.
#[test]
fn a_view_keeps_no_pixels_of_a_source_it_had_and_each_source_is_redacted_on_its_own_status() {
    for gpu in [false, true] {
        let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
        view_of_source(&mut composer, Size::new(SIDE, SIDE));
        canvas(&mut composer, 1, 101, Rect::new(0, 0, SIDE, 30), RED, BLUE);
        second_source(&mut composer);
        draw_reading(&mut composer, output);
        let at = |composer: &Composer, gles: &mut Option<Gles>, x, y| {
            captured(composer, gles).and_then(|image| image.colour_at(x, y))
        };
        let label = |step: &str| format!("gpu: {gpu}: {step}");
        if gles.is_some() || !gpu {
            assert_eq!(
                at(&composer, &mut gles, 30, 20),
                Some(RED),
                "{}",
                label("the first source")
            );
        }

        composer.with_scene(|scene| scene.set_source(VIEW, Some(NodeId::new(70))).expect("a new source"));
        draw_reading(&mut composer, output);
        if gles.is_some() || !gpu {
            assert_eq!(
                at(&composer, &mut gles, 5, 5),
                Some(GREEN),
                "{}",
                label("the second source")
            );
            assert_eq!(
                at(&composer, &mut gles, 30, 20),
                Some(BLACK),
                "{}",
                label("where only the first source drew, nothing is left of it")
            );
        }

        set_secure(&mut composer, NodeId::new(2), true);
        draw_reading(&mut composer, output);
        if gles.is_some() || !gpu {
            let capture = captured(&composer, &mut gles).expect("a picture");
            assert_eq!(
                capture.colour_at(5, 5),
                Some(GREY),
                "{}",
                label("the secure second source is the mask")
            );
            assert!(
                !pixels(&capture).contains(&GREEN) && !pixels(&capture).contains(&RED),
                "{}",
                label("and no pixel of either source is in the capture")
            );
        }

        composer.with_scene(|scene| scene.set_source(VIEW, Some(SOURCE)).expect("back to the first"));
        draw_reading(&mut composer, output);
        if gles.is_some() || !gpu {
            assert_eq!(
                at(&composer, &mut gles, 30, 20),
                Some(RED),
                "{}",
                label("the first source is not secure because the second was")
            );
            assert!(
                !pixels(&captured(&composer, &mut gles).expect("a picture")).contains(&GREY),
                "{}",
                label("no mask left")
            );
        }
    }
}

/// One renderer reads two Views of the same size whose sources differ in secure status, in turn and then with the
/// statuses swapped: neither read carries a pixel of the other, so no buffer or texture is reused between them.
#[test]
fn the_readback_of_one_view_never_carries_the_pixels_of_a_view_of_another_secure_status() {
    let other = NodeId::new(200);
    for gpu in [false, true] {
        let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
        view_of_source(&mut composer, Size::new(SIDE, SIDE));
        canvas(&mut composer, 1, 101, Rect::new(0, 0, SIDE, SIDE), RED, RED);
        second_source(&mut composer);
        canvas(&mut composer, 3, 70, Rect::new(10, 10, 30, 30), GREEN, GREEN);
        view_node(&mut composer, other, NodeId::new(70), None);
        set_secure(&mut composer, NodeId::new(1), true);
        draw_reading(&mut composer, output);
        if gpu && gles.is_none() {
            continue;
        }
        let read = |composer: &Composer, gles: &mut Option<Gles>, view| {
            captured_view(composer, view, gles).expect("a picture")
        };
        for round in 0..3 {
            let first = read(&composer, &mut gles, VIEW);
            assert!(
                all_are(&first, GREY),
                "gpu: {gpu}, round {round}: the secure View is the mask only"
            );
            let second = read(&composer, &mut gles, other);
            assert!(
                !pixels(&second).contains(&GREY) && !pixels(&second).contains(&RED),
                "gpu: {gpu}, round {round}: the plain View holds nothing of the secure one"
            );
            assert_eq!(
                second.colour_at(15, 15),
                Some(GREEN),
                "gpu: {gpu}, round {round}: it shows its own source"
            );
        }
        // The statuses swap: the first source is shown and the second is redacted.
        set_secure(&mut composer, NodeId::new(1), false);
        set_secure(&mut composer, NodeId::new(70), true);
        draw_reading(&mut composer, output);
        let first = read(&composer, &mut gles, VIEW);
        assert!(
            all_are(&first, RED),
            "gpu: {gpu}: the View of the first source, no longer secure, shows it whole and holds no mask"
        );
        let second = read(&composer, &mut gles, other);
        assert!(
            pixels(&second).contains(&GREY) && !pixels(&second).contains(&GREEN),
            "gpu: {gpu}: the View of the second source, now secure, draws the mask where its canvases are and no green"
        );
        set_secure(&mut composer, NodeId::new(70), false);
        draw_reading(&mut composer, output);
        let second = read(&composer, &mut gles, other);
        assert_eq!(
            second.colour_at(15, 15),
            Some(GREEN),
            "gpu: {gpu}: and shows again when it is not"
        );
        assert!(!pixels(&second).contains(&GREY), "gpu: {gpu}: leaving no mask behind");
    }
}

/// A node that is secure redacts itself and what is below it, and nothing beside it or above it.
#[test]
fn a_secure_node_redacts_itself_and_what_is_below_it_and_nothing_beside_it() {
    for gpu in [false, true] {
        let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
        view_of_source(&mut composer, Size::new(SIDE, SIDE));
        plain(&mut composer, 4, 101, Rect::new(0, 0, 20, SIDE));
        canvas(&mut composer, 1, 4, Rect::new(0, 0, 20, SIDE), RED, RED);
        canvas(&mut composer, 2, 101, Rect::new(20, 0, 20, SIDE), GREEN, GREEN);
        set_secure(&mut composer, NodeId::new(4), true);
        draw_reading(&mut composer, output);
        let Some(capture) = captured(&composer, &mut gles) else {
            assert!(gpu && gles.is_none());
            continue;
        };
        assert_eq!(
            capture.colour_at(5, 5),
            Some(GREY),
            "gpu: {gpu}: the child of the secure node is the mask"
        );
        assert_eq!(
            capture.colour_at(30, 5),
            Some(GREEN),
            "gpu: {gpu}: its sibling is as drawn"
        );
        assert!(!pixels(&capture).contains(&RED), "gpu: {gpu}: none of the secure child");
    }
}

/// A secure node in a stereo View is redacted in both eyes, and the two paths agree.
#[test]
fn a_secure_node_is_redacted_in_both_eyes_of_a_stereo_view_as_on_the_cpu() {
    let build = |composer: &mut Composer| {
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
        set_secure(composer, NodeId::new(1), true);
    };
    let (cpu, gpu) = both(build);
    let expected = cpu_picture(&cpu.0);
    let scene = scene_of(&gpu.0);
    assert!(
        scene.has_capture_variant(),
        "the secure node gives the stereo scene a capture variant"
    );
    assert_eq!(scene.eyes(true).len(), 2, "both eyes are in the capture variant");
    assert!(
        !sources(scene.eyes(true)).iter().any(|source| !is_mask(source)),
        "and neither eye samples anything but the mask"
    );
    let mut pictures = vec![("the CPU rig", Image::clone(&expected))];
    if let Some(mut gles) = gles() {
        let capture = gles.view_pixels(&scene, true).expect("the capture variant is drawn");
        let wrong = differing(&expected, &capture);
        assert!(
            wrong <= 80 * 40 * 3 / 100,
            "the two paths agree but for the edges: {wrong} pixels differ"
        );
        pictures.push(("the GLES renderer", capture));
    }
    for (name, picture) in pictures {
        let pixels = pixels(&picture);
        assert!(
            !pixels.contains(&RED) && !pixels.contains(&BLUE),
            "{name}: no pixel of the secure node in either eye"
        );
        let half = |x0: i32, x1: i32| {
            (0..40)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|(x, y)| picture.colour_at(*x, *y) == Some(GREY))
                .count()
        };
        assert!(
            half(0, 40) > 0 && half(40, 80) > 0,
            "{name}: the mask is drawn in each eye"
        );
    }
}

/// The head camera is moved by the XR extension, an observer, and a View through it may show a secure window. Where the
/// camera stands decides what is in each eye and never whether the secure node is masked: the capture variant is chosen
/// from the secure marks of the source, not from the camera, so no pose, near, far, behind the node or off to the side,
/// lets a pixel of it into a capture.
#[test]
fn a_secure_node_stays_redacted_wherever_the_head_camera_is_written_to_stand() {
    let poses = [
        (40.0, 20.0, 100.0),
        (0.0, 0.0, 1.0),
        (-500.0, 300.0, 40.0),
        (40.0, 20.0, -100.0),
        (1.0e6, -1.0e6, 50.0),
        (40.0, 20.0, 0.001),
    ];
    for (x, y, z) in poses {
        let build = |composer: &mut Composer| {
            view_of_source(composer, Size::new(80, 40));
            canvas(composer, 1, 101, Rect::new(10, 5, 60, 30), RED, BLUE);
            camera(composer, "stereo", Vec3 { x, y, z }, &[]);
            set_secure(composer, NodeId::new(1), true);
        };
        let (cpu, gpu) = both(build);
        let capture = cpu_picture(&cpu.0);
        let colours = pixels(&capture);
        assert!(
            !colours.contains(&RED) && !colours.contains(&BLUE),
            "a camera at ({x}, {y}, {z}) shows a pixel of the secure node"
        );
        let scene = scene_of(&gpu.0);
        assert!(
            scene.has_capture_variant(),
            "({x}, {y}, {z}): the capture variant exists"
        );
        assert!(
            !sources(scene.eyes(true)).iter().any(|source| !is_mask(source)),
            "({x}, {y}, {z}): neither eye of the capture samples anything but the mask"
        );
    }
}

/// A node that becomes secure after a consumer started reading is in the very next picture, though the View's `fps`
/// leaves it a second before another draw.
#[test]
fn a_node_that_becomes_secure_is_redacted_in_the_next_picture_though_the_view_is_paced() {
    for gpu in [false, true] {
        let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
        view_of_source(&mut composer, Size::new(SIDE, SIDE));
        canvas(&mut composer, 1, 101, Rect::new(0, 0, SIDE, SIDE), RED, RED);
        composer.with_scene(|scene| scene.set_value(VIEW, "fps", TreeValue::Uint(1)).expect("fps"));
        draw_reading(&mut composer, output);
        let first = scene_for(&composer, gpu).map(|scene| scene.revision());
        if let Some(open) = captured(&composer, &mut gles) {
            assert_eq!(open.colour_at(30, 30), Some(RED), "gpu: {gpu}: the first picture");
        }
        composer.advance_motion(20);
        set_secure(&mut composer, NodeId::new(1), true);
        draw_reading(&mut composer, output);
        if gpu {
            let scene = scene_of(&composer);
            assert!(
                scene.revision() > first.expect("a first scene"),
                "the mark is a new scene at once, not at the slot's end"
            );
            assert!(scene.has_capture_variant());
        }
        if let Some(masked) = captured(&composer, &mut gles) {
            assert!(
                all_are(&masked, GREY),
                "gpu: {gpu}: the very next picture holds the mask"
            );
        }
    }
}

/// A secure window's fade keeps the mark, drawn from the GPU buffer itself (a capture replaces the layer), and a View
/// of the window shows none of it: the buffer is dropped for the fade, so there are no pixels to carry.
#[test]
fn a_secure_window_that_shows_a_gpu_buffer_stays_secure_while_it_fades() {
    for buffer in BUFFERS {
        for closing in [true, false] {
            for gpu in [false, true] {
                let label = format!("{buffer:?}, closing: {closing}, gpu: {gpu}");
                let (mut composer, output, mut gles) = session(gpu, SecureTreatment::Mask);
                composer.set_reduced_motion(false);
                let node = map_window(&mut composer, buffer);
                view_of(&mut composer, node);
                composer.set_window_secure(WINDOW, true);
                composer.advance_motion(MOTION_MS);
                draw_reading(&mut composer, output);
                if closing {
                    composer.close_window(WINDOW);
                } else {
                    composer.unmap_window(WINDOW);
                }
                composer.advance_motion(MOTION_MS / 2);
                let frame = draw_reading(&mut composer, output);

                let window_layers: Vec<&Surface> = frame
                    .layers()
                    .iter()
                    .filter_map(|layer| match layer {
                        Layer::Surface(surface) if surface.dst.x >= 200 => Some(surface),
                        _ => None,
                    })
                    .collect();
                assert_eq!(window_layers.len(), 1, "{label}: the fading window is drawn");
                assert!(
                    matches!(&window_layers[0].source, SurfaceSource::Dmabuf(_)) == (buffer == Buffer::Dmabuf),
                    "{label}: from its own buffer"
                );
                assert!(window_layers[0].secure, "{label}: and the fade keeps the mark");
                let redacted = frame.redacted(SecureTreatment::Mask, Rect::new(0, 0, 800, 600));
                assert!(
                    redacted.layers().iter().all(|layer| match layer {
                        Layer::Surface(surface) => surface.dst.x < 200,
                        _ => true,
                    }),
                    "{label}: a capture of the output draws no pixel of the fading window"
                );

                if let Some(capture) = captured(&composer, &mut gles) {
                    assert!(
                        !pixels(&capture).contains(&RED) && !pixels(&capture).contains(&GREEN),
                        "{label}: a capture of a View of the fading window holds none of its pixels"
                    );
                }
            }
        }
    }
}

/// A capture of the output that shows a View of secure content replaces the View, and the window, so no consumer
/// reaches the pixels through the screen's capture either: nothing in the redacted frame samples the View or either
/// buffer, and what it draws where they are is the redaction.
#[test]
fn a_capture_of_the_output_replaces_a_view_of_secure_content_and_the_window_itself() {
    for gpu in [false, true] {
        for buffer in BUFFERS {
            for treatment in [SecureTreatment::Mask, SecureTreatment::Omit] {
                let label = format!("gpu: {gpu}, {buffer:?}, {treatment:?}");
                let (mut composer, output, _) = session(gpu, treatment);
                let node = map_window(&mut composer, buffer);
                view_of(&mut composer, node);
                composer.set_window_secure(WINDOW, true);
                let frame = draw_reading(&mut composer, output);
                assert!(frame.has_secure(), "{label}: the frame has secure layers");
                let redacted = frame.redacted(treatment, Rect::new(0, 0, 800, 600));
                assert!(
                    !redacted.layers().iter().any(|layer| matches!(
                        layer,
                        Layer::Surface(surface) if matches!(surface.source, SurfaceSource::View(_) | SurfaceSource::Dmabuf(_))
                            || matches!(&surface.source, SurfaceSource::Snapshot(image) if image.size() == Size::new(SIDE, SIDE))
                    )),
                    "{label}: nothing in a capture of the output samples the View or the window"
                );
                let mut software = Software::new(Size::new(800, 600)).expect("a software renderer");
                software
                    .draw(&redacted, Scale::IDENTITY)
                    .expect("the redacted frame draws");
                let image = software.screenshot().expect("a readback");
                for (x, y, what) in [(20, 30, "the View"), (320, 320, "the window")] {
                    assert_eq!(
                        image.colour_at(x, y),
                        Some(redaction(treatment)),
                        "{label}: a capture of the output shows the redaction where {what} is"
                    );
                }
            }
        }
    }
}
