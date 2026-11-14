//! A decorated window: its content, its titlebar canvas and its border ring share one rounded frame.
//!
//! The window is red, 100 by 100 at (100, 100), with a green 28 pixel titlebar canvas above it, a child of the
//! window's node. Every pixel assertion reads the software renderer's output for the frame the composer describes, so
//! it is the pixels an output shows; the layer assertions read the frame the composer hands every renderer.

use gnoblin_paint::{PaintList, PaintOp, Rgba8};
use gnoblin_render::{Border, Pass, Shadow, Vec3};
use gnoblin_render_software::Software;
use gnoblin_types::Scale;

use super::*;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BACKGROUND: [u8; 4] = [0, 0, 0, 255];

const WINDOW: Rect = Rect::new(100, 100, 100, 100);
/// The titlebar's height, which sits above the window's content.
const BAR: u32 = 28;
/// The frame: the content with the titlebar's room above it.
const FRAME: Rect = Rect::new(100, 72, 100, 128);

fn flat(colour: [u8; 4], size: Size) -> SurfaceSource {
    let pixels = colour.repeat(size.width as usize * size.height as usize);
    let image = Image::new(size, size.width as usize * 4, pixels).expect("an image of that size");
    SurfaceSource::Snapshot(Arc::new(image))
}

fn paint_list(width: f32, height: f32, rgb: [u8; 3]) -> Vec<u8> {
    gnoblin_paint::encode(&PaintList {
        ops: vec![PaintOp::Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
            radius: 0.0,
            fill: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            stroke: Rgba8::TRANSPARENT,
            stroke_width: 0.0,
        }],
    })
}

/// A composer with the red window and its green titlebar, drawn at once rather than faded in.
fn decorated() -> (Composer, OutputId, WindowId, NodeId) {
    let mut composer = Composer::new();
    let output = OutputId::new(0);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    composer.set_reduced_motion(true);
    composer.begin_frame();
    composer.end_frame();
    let window = WindowId::new(1);
    let size = Size::new(WINDOW.width, WINDOW.height);
    composer.insert_window(window, WINDOW);
    composer.set_window_content(
        window,
        Some(Content {
            src: RectF::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            size,
            transform: Transform::Normal,
        }),
    );
    composer.set_window_source(window, Some(flat(RED, size)));
    let node = composer.window_node(window).expect("the window has a node");
    composer.with_scene(|scene| {
        let mut bar = NodeSpec::new(Rect::new(0, -(BAR as i32), WINDOW.width, BAR));
        bar.parent = Some(node);
        scene.insert(NodeId::new(900), bar).expect("the titlebar node");
        scene
            .set_class(NodeId::new(900), Some("canvas".to_owned()))
            .expect("the titlebar's class");
        scene
            .set_value(
                NodeId::new(900),
                "content",
                TreeValue::Bytes(paint_list(WINDOW.width as f32, BAR as f32, [0, 255, 0])),
            )
            .expect("the titlebar's content");
    });
    (composer, output, window, node)
}

fn chain(passes: Vec<Pass>) -> Effects {
    let mut effects = Effects::new();
    for pass in passes {
        effects.push(pass).expect("a pass in range");
    }
    effects
}

fn border(width: f32, offset: f32, inset: bool) -> Pass {
    Pass::Border(Border {
        width,
        offset,
        inset,
        color: Color::WHITE,
    })
}

fn shadow(frame: bool) -> Pass {
    Pass::Shadow(Shadow {
        offset: Vec3::ZERO,
        blur: 8.0,
        spread: 0.0,
        opacity: 1.0,
        color: Color::BLACK,
        frame,
    })
}

fn frame_of(composer: &mut Composer, output: OutputId) -> OutputFrame {
    composer.damage_output(output);
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
}

fn pixels_of(frame: &OutputFrame) -> Image {
    let mut software = Software::new(Size::new(800, 600)).expect("a software renderer");
    software.draw(frame, Scale::IDENTITY).expect("the frame draws");
    software.screenshot().expect("a readback")
}

fn probe(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image
        .colour_at(x, y)
        .unwrap_or_else(|| panic!("({x}, {y}) is on the output"))
}

fn luma(pixel: [u8; 4]) -> u32 {
    u32::from(pixel[0]) * 3 + u32::from(pixel[1]) * 6 + u32::from(pixel[2])
}

#[test]
fn the_titlebar_and_the_content_of_a_rounded_window_are_cut_from_one_set_of_corners() {
    let (mut composer, output, window, _) = decorated();
    composer.set_window_corner_radius(window, 12.0);
    let frame = frame_of(&mut composer, output);

    let layers: Vec<&Surface> = frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) => Some(surface),
            _ => None,
        })
        .collect();
    assert_eq!(layers.len(), 2, "the window's content and its titlebar");
    for layer in &layers {
        assert_eq!(layer.corner_radius, 12.0, "the bar takes the window's radius");
        assert_eq!(layer.silhouette, Some(FRAME), "and both are rounded to the frame");
    }

    let image = pixels_of(&frame);
    assert_eq!(
        probe(&image, 101, 73),
        BACKGROUND,
        "the frame's top-left corner is cut, on the bar"
    );
    assert_eq!(probe(&image, 198, 73), BACKGROUND, "and its top-right");
    assert_eq!(probe(&image, 101, 198), BACKGROUND, "and the content's bottom-left");
    assert_eq!(probe(&image, 198, 198), BACKGROUND, "and its bottom-right");
    assert_eq!(
        probe(&image, 101, 101),
        RED,
        "the content's own top-left corner is not cut: it is in the frame's straight edge"
    );
    assert_eq!(probe(&image, 150, 80), GREEN, "the bar draws");
    assert_eq!(probe(&image, 150, 150), RED, "and so does the content");
}

#[test]
fn a_window_with_no_radius_is_not_cut_at_all() {
    let (mut composer, output, _, _) = decorated();
    let frame = frame_of(&mut composer, output);
    let image = pixels_of(&frame);
    assert_eq!(probe(&image, 101, 73), GREEN);
    assert_eq!(probe(&image, 101, 198), RED);
}

#[test]
fn a_border_is_a_ring_over_the_frame_drawn_after_the_titlebar() {
    let (mut composer, output, _, node) = decorated();
    composer.with_scene(|scene| {
        scene
            .set_effects(node, chain(vec![border(2.0, 0.0, true)]))
            .expect("effects")
    });
    let frame = frame_of(&mut composer, output);

    let kinds: Vec<&str> = frame
        .layers()
        .iter()
        .map(|layer| match layer {
            Layer::Surface(_) => "surface",
            Layer::Solid(_) => "solid",
            Layer::Overlay(_) => "overlay",
        })
        .collect();
    assert_eq!(
        kinds,
        ["surface", "surface", "solid"],
        "the content, the titlebar, then the ring over both"
    );
    let Some(Layer::Solid(ring)) = frame.layers().last() else {
        panic!("the last layer is the ring");
    };
    assert_eq!(ring.dst, FRAME, "the ring runs along the whole frame");
    assert!(ring.effects.has("border") && ring.effects.len() == 1);
    let Layer::Surface(content) = &frame.layers()[0] else {
        panic!("the content comes first");
    };
    assert!(
        !content.effects.has("border"),
        "the content's layer does not draw the border"
    );

    let image = pixels_of(&frame);
    assert_eq!(
        probe(&image, 150, 72),
        WHITE,
        "the border runs over the titlebar's top edge"
    );
    assert_eq!(probe(&image, 150, 73), WHITE);
    assert_eq!(probe(&image, 150, 74), GREEN, "inside the stroke the bar shows");
    assert_eq!(probe(&image, 100, 150), WHITE, "the left edge of the content");
    assert_eq!(probe(&image, 150, 199), WHITE, "the bottom edge");
    assert_eq!(probe(&image, 150, 197), RED);
}

#[test]
fn a_rounded_window_is_outlined_along_its_rounded_frame() {
    let (mut composer, output, window, node) = decorated();
    composer.set_window_corner_radius(window, 12.0);
    composer.with_scene(|scene| {
        scene
            .set_effects(node, chain(vec![border(2.0, 0.0, true)]))
            .expect("effects")
    });
    let frame = frame_of(&mut composer, output);
    let Some(Layer::Solid(ring)) = frame.layers().last() else {
        panic!("the last layer is the ring");
    };
    assert_eq!(ring.corner_radius, 12.0, "the ring is rounded like the frame");
    let image = pixels_of(&frame);
    assert_eq!(probe(&image, 101, 73), BACKGROUND, "the corner is cut, ring and all");
    assert_eq!(probe(&image, 150, 72), WHITE, "the top edge is stroked");
    assert_eq!(probe(&image, 100, 150), WHITE, "and the left");
}

#[test]
fn a_renderer_that_draws_no_border_gets_no_ring() {
    let (mut composer, output, _, node) = decorated();
    composer.set_effect_kinds(&["shadow"]);
    composer.with_scene(|scene| {
        scene
            .set_effects(node, chain(vec![border(2.0, 0.0, true)]))
            .expect("effects")
    });
    let frame = frame_of(&mut composer, output);
    assert!(
        frame.layers().iter().all(|layer| !matches!(layer, Layer::Solid(_))),
        "no ring is described for a renderer that would not draw it"
    );
    assert_eq!(
        composer.with_scene(|scene| scene.effects(node).map(|effects| effects.len())),
        Some(1),
        "the node keeps the list it was given"
    );
}

#[test]
fn a_frame_shadow_follows_the_decorated_frame_and_a_content_shadow_follows_the_content() {
    let (mut composer, output, _, node) = decorated();
    composer.set_background(Color::WHITE);
    composer.with_scene(|scene| scene.set_effects(node, chain(vec![shadow(true)])).expect("effects"));
    let framed = frame_of(&mut composer, output);
    let Layer::Surface(content) = &framed.layers()[0] else {
        panic!("the content comes first");
    };
    assert_eq!(
        content.silhouette,
        Some(FRAME),
        "a frame shadow needs the frame, rounded or not"
    );
    let image = pixels_of(&framed);
    assert!(
        luma(probe(&image, 150, 68)) < luma(WHITE),
        "the shadow reaches above the titlebar, four pixels from its top edge"
    );

    composer.with_scene(|scene| scene.set_effects(node, chain(vec![shadow(false)])).expect("effects"));
    let content_only = frame_of(&mut composer, output);
    let Layer::Surface(content) = &content_only.layers()[0] else {
        panic!("the content comes first");
    };
    assert_eq!(
        content.silhouette, None,
        "a content shadow with no radius needs no frame"
    );
    let image = pixels_of(&content_only);
    assert_eq!(
        probe(&image, 150, 68),
        WHITE,
        "a content shadow stops short of the bar's top edge"
    );
}

#[test]
fn a_border_damages_the_whole_frame_it_runs_along() {
    let (mut composer, output, _, node) = decorated();
    frame_of(&mut composer, output);
    composer.with_scene(|scene| {
        scene
            .set_effects(node, chain(vec![border(2.0, 1.0, false)]))
            .expect("effects")
    });
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(output, &mut frame), "the change is drawn");
    composer.end_frame();
    // The stroke lies 1 to 3 pixels outside the frame, titlebar included.
    let wanted = Rect::new(97, 69, 106, 134);
    assert!(
        frame.damage().iter().any(|damaged| {
            damaged.x <= wanted.x
                && damaged.y <= wanted.y
                && damaged.x + damaged.width as i32 >= wanted.x + wanted.width as i32
                && damaged.y + damaged.height as i32 >= wanted.y + wanted.height as i32
        }),
        "{:?} covers {wanted:?}",
        frame.damage()
    );
}
