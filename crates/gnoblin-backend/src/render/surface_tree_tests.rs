//! The composer's surface tree: the subsurfaces and popups of a window, drawn through the scene.
//!
//! Every pixel assertion here reads the software renderer's output for the frame the composer describes, so it is
//! the pixels an output shows and not a proxy for them. A surface's pixels are a [`SurfaceSource::Snapshot`] of a
//! flat colour, which the composer draws as it draws a client's buffer and which a test can make without a
//! Wayland client; `tests/surface_tree.rs` in `gnoblin-wayland` runs the same cases against a real client.

use gnoblin_render::{Actor, Pass, Shadow};
use gnoblin_render_software::Software;
use gnoblin_types::Scale;

use super::*;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BACKGROUND: [u8; 4] = [0, 0, 0, 255];

/// The window of every test: red, 100 by 100, with its top left corner at (100, 100).
const WINDOW: Rect = Rect::new(100, 100, 100, 100);

/// A flat colour of `size`, as a surface's pixels.
fn flat(colour: [u8; 4], size: Size) -> SurfaceSource {
    let pixels = colour.repeat(size.width as usize * size.height as usize);
    let image = Image::new(size, size.width as usize * 4, pixels).expect("an image of that size");
    SurfaceSource::Snapshot(Arc::new(image))
}

fn content_of(size: Size) -> Content {
    Content {
        src: RectF::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
        size,
        transform: Transform::Normal,
    }
}

/// A surface of a window's tree, drawn flat `colour`, `size` big, at `at` in its parent.
fn tree_surface(
    key: u64,
    parent: Option<u64>,
    role: TreeSurfaceRole,
    at: (i32, i32),
    size: (u32, u32),
    colour: [u8; 4],
) -> TreeSurface {
    let size = Size::new(size.0, size.1);
    TreeSurface {
        key,
        parent,
        role,
        behind: false,
        offset: Point::new(at.0, at.1),
        alpha: 1.0,
        content: content_of(size),
        source: flat(colour, size),
        damaged: false,
    }
}

/// A popup of the window itself.
fn popup(key: u64, at: (i32, i32), size: (u32, u32), colour: [u8; 4]) -> TreeSurface {
    tree_surface(key, None, TreeSurfaceRole::Popup, at, size, colour)
}

/// A subsurface of the window's own surface.
fn subsurface(key: u64, at: (i32, i32), size: (u32, u32), colour: [u8; 4]) -> TreeSurface {
    tree_surface(key, None, TreeSurfaceRole::Subsurface, at, size, colour)
}

/// A composer with an 800 by 600 output and the red window, drawn at once rather than faded in.
fn drawing() -> (Composer, OutputId, WindowId) {
    let mut composer = Composer::new();
    let output = OutputId::new(0);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    composer.set_reduced_motion(true);
    composer.begin_frame();
    composer.end_frame();
    let window = WindowId::new(1);
    map_window(&mut composer, window, WINDOW);
    (composer, output, window)
}

fn map_window(composer: &mut Composer, window: WindowId, rect: Rect) {
    let size = Size::new(rect.width, rect.height);
    composer.insert_window(window, rect);
    composer.set_window_content(window, Some(content_of(size)));
    composer.set_window_source(window, Some(flat(RED, size)));
}

/// The frame the composer describes for `output` now, with the whole output damaged so every layer is in it.
fn frame_of(composer: &mut Composer, output: OutputId) -> OutputFrame {
    composer.damage_output(output);
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
}

/// The pixels `frame` draws on an output of `size`.
fn pixels_of(frame: &OutputFrame, size: Size) -> Image {
    let mut software = Software::new(size).expect("a software renderer");
    software.draw(frame, Scale::IDENTITY).expect("the frame draws");
    software.screenshot().expect("a readback")
}

/// The pixels the first output shows now.
fn shown(composer: &mut Composer, output: OutputId) -> Image {
    let frame = frame_of(composer, output);
    pixels_of(&frame, Size::new(800, 600))
}

fn probe(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image
        .colour_at(x, y)
        .unwrap_or_else(|| panic!("({x}, {y}) is on the output"))
}

/// The damage of the next frame for `output`, or `None` when nothing changed.
fn damage(composer: &mut Composer, output: OutputId) -> Option<Vec<Rect>> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    let changed = composer.compose(output, &mut frame);
    composer.end_frame();
    changed.then(|| frame.damage().to_vec())
}

fn surface_layers(frame: &OutputFrame) -> Vec<&Surface> {
    frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) => Some(surface),
            _ => None,
        })
        .collect()
}

fn node_of(composer: &Composer, window: WindowId) -> NodeId {
    composer.window_node(window).expect("the window has a node")
}

/// The node of the surface `key` of `window`.
fn tree_node(composer: &Composer, window: WindowId, key: u64) -> NodeId {
    composer.windows[&window].tree_nodes[&key].node
}

#[test]
fn a_popup_past_the_window_edge_is_drawn_at_its_offset_above_the_window() {
    let (mut composer, output, window) = drawing();
    // The popup's corner is 10 pixels inside the window's bottom right corner, so it overlaps the window and hangs
    // past its edge: it spans (190, 190) to (230, 230).
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    let image = shown(&mut composer, output);

    assert_eq!(probe(&image, 150, 150), RED, "the window is drawn where it is");
    assert_eq!(
        probe(&image, 185, 185),
        RED,
        "the window shows up to the popup's corner"
    );
    assert_eq!(
        probe(&image, 195, 195),
        BLUE,
        "the popup is above the window where they overlap"
    );
    assert_eq!(
        probe(&image, 215, 215),
        BLUE,
        "the popup is drawn past the window's edge"
    );
    assert_eq!(probe(&image, 229, 229), BLUE, "the popup is drawn to its last pixel");
    assert_eq!(probe(&image, 231, 231), BACKGROUND, "nothing is drawn past the popup");
}

#[test]
fn a_popup_follows_its_window_when_the_window_moves() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    shown(&mut composer, output);
    composer.place_window(window, Rect::new(300, 200, 100, 100));
    let image = shown(&mut composer, output);

    assert_eq!(
        probe(&image, 400, 300),
        BLUE,
        "the popup moved with the window's corner"
    );
    assert_eq!(probe(&image, 215, 215), BACKGROUND, "nothing is left where it was");
}

#[test]
fn a_subsurface_below_the_parent_is_drawn_behind_it() {
    let (mut composer, output, window) = drawing();
    let mut below = subsurface(10, (80, 80), (40, 40), BLUE);
    below.behind = true;
    composer.set_window_surfaces(window, vec![below]);
    let image = shown(&mut composer, output);

    assert_eq!(
        probe(&image, 190, 190),
        RED,
        "the parent covers a subsurface placed below it"
    );
    assert_eq!(
        probe(&image, 210, 210),
        BLUE,
        "the subsurface shows where the parent does not reach"
    );
    assert_eq!(probe(&image, 150, 150), RED);
}

#[test]
fn a_subsurface_above_the_parent_is_drawn_over_it() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![subsurface(10, (80, 80), (40, 40), BLUE)]);
    let image = shown(&mut composer, output);

    assert_eq!(
        probe(&image, 190, 190),
        BLUE,
        "a subsurface placed above its parent covers it"
    );
    assert_eq!(probe(&image, 210, 210), BLUE);
}

#[test]
fn place_above_and_place_below_reorder_the_subsurfaces_and_damage_them() {
    let (mut composer, output, window) = drawing();
    // Two subsurfaces beside the window that overlap each other on (240..260, 120..140).
    let blue = subsurface(10, (120, 0), (40, 40), BLUE);
    let green = subsurface(11, (140, 20), (40, 40), GREEN);
    composer.set_window_surfaces(window, vec![blue.clone(), green.clone()]);
    let image = shown(&mut composer, output);
    assert_eq!(
        probe(&image, 250, 130),
        GREEN,
        "the later sibling is above the earlier one"
    );

    composer.set_window_surfaces(window, vec![green.clone(), blue.clone()]);
    let rects = damage(&mut composer, output).expect("a reorder that changes what shows damages the surfaces");
    assert!(
        rects.iter().any(|rect| rect.contains(Point::new(250, 130))),
        "the overlap is damaged: {rects:?}"
    );
    let image = shown(&mut composer, output);
    assert_eq!(probe(&image, 250, 130), BLUE, "place_above puts the first one on top");

    // A subsurface placed below its parent leaves the siblings in front, and the scene's order follows.
    let mut below = blue;
    below.behind = true;
    composer.set_window_surfaces(window, vec![below, green]);
    let node = node_of(&composer, window);
    let blue_node = tree_node(&composer, window, 10);
    let green_node = tree_node(&composer, window, 11);
    assert_eq!(
        composer.scene.children(node),
        Some(&[blue_node, green_node][..]),
        "a surface behind its parent is the first child"
    );
    assert_eq!(composer.scene.behind(blue_node), Some(true));
    assert_eq!(composer.scene.behind(green_node), Some(false));
}

#[test]
fn a_submenu_is_drawn_beside_its_menu() {
    let (mut composer, output, window) = drawing();
    let menu = popup(10, (90, 0), (60, 60), GREEN);
    // The submenu hangs from the menu, so its offset is the menu's, not the window's.
    let submenu = tree_surface(11, Some(10), TreeSurfaceRole::Popup, (50, 10), (40, 40), BLUE);
    composer.set_window_surfaces(window, vec![menu, submenu]);
    let image = shown(&mut composer, output);

    assert_eq!(probe(&image, 200, 105), GREEN, "the menu is drawn at its own offset");
    assert_eq!(
        probe(&image, 245, 115),
        BLUE,
        "the submenu is above the menu it hangs from"
    );
    assert_eq!(probe(&image, 275, 125), BLUE);
    assert_eq!(
        composer.scene.parent(tree_node(&composer, window, 11)),
        Some(tree_node(&composer, window, 10)),
        "the submenu's node is under its menu's"
    );
}

#[test]
fn a_surface_whose_parent_is_not_in_the_list_is_not_drawn() {
    let (mut composer, output, window) = drawing();
    let orphan = tree_surface(11, Some(10), TreeSurfaceRole::Popup, (0, 0), (40, 40), BLUE);
    composer.set_window_surfaces(window, vec![orphan]);
    let image = shown(&mut composer, output);

    assert_eq!(
        probe(&image, 105, 105),
        RED,
        "a surface with no parent is not drawn over the window"
    );
    assert!(composer.windows[&window].tree_nodes.is_empty());
}

#[test]
fn a_surface_that_changes_parent_is_made_again_under_the_new_one() {
    let (mut composer, _output, window) = drawing();
    let menu = popup(10, (90, 0), (60, 60), GREEN);
    let under_menu = tree_surface(11, Some(10), TreeSurfaceRole::Popup, (50, 10), (40, 40), BLUE);
    composer.set_window_surfaces(window, vec![menu.clone(), under_menu]);
    let first = tree_node(&composer, window, 11);

    composer.set_window_surfaces(window, vec![menu, popup(11, (150, 10), (40, 40), BLUE)]);
    let second = tree_node(&composer, window, 11);
    assert_ne!(first, second, "a surface moved to another parent is a new node");
    assert_eq!(composer.scene.parent(second), Some(node_of(&composer, window)));
    assert!(!composer.scene.contains(first), "the old node is gone");
}

#[test]
fn a_destroyed_popup_disappears_and_the_pixels_it_held_are_damaged() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    shown(&mut composer, output);
    let node = tree_node(&composer, window, 10);
    composer.take_tree_changes();

    composer.set_window_surfaces(window, Vec::new());
    let rects = damage(&mut composer, output).expect("removing a popup damages what it held");
    assert!(
        rects.iter().any(|rect| rect.contains(Point::new(215, 215))),
        "the popup's rectangle is damaged: {rects:?}"
    );
    let image = shown(&mut composer, output);
    assert_eq!(probe(&image, 215, 215), BACKGROUND, "the popup is gone from the output");
    assert_eq!(probe(&image, 150, 150), RED, "the window stays");
    assert!(!composer.scene.contains(node));
    assert!(
        composer.take_tree_changes().iter().any(|change| matches!(
            change,
            TreeChange::Destroyed { node: destroyed, .. } if *destroyed == node
        )),
        "the tree is told the popup's node is gone"
    );
}

#[test]
fn only_a_surface_that_committed_is_damaged() {
    let (mut composer, output, window) = drawing();
    // Beside the window, so the window's own rectangle is not part of it.
    let mut beside = subsurface(10, (120, 0), (40, 40), BLUE);
    composer.set_window_surfaces(window, vec![beside.clone()]);
    damage(&mut composer, output);

    composer.set_window_surfaces(window, vec![beside.clone()]);
    assert_eq!(
        damage(&mut composer, output),
        None,
        "a tree that did not change and did not commit damages nothing"
    );

    beside.damaged = true;
    composer.set_window_surfaces(window, vec![beside.clone()]);
    assert_eq!(
        damage(&mut composer, output),
        Some(vec![Rect::new(220, 100, 40, 40)]),
        "a commit damages the surface's rectangle and no more"
    );

    beside.damaged = false;
    composer.set_window_surfaces(window, vec![beside]);
    assert_eq!(damage(&mut composer, output), None, "the commit is damaged once");
}

#[test]
fn a_moved_or_resized_surface_damages_where_it_was_and_where_it_is() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (120, 0), (40, 40), BLUE)]);
    damage(&mut composer, output);

    composer.set_window_surfaces(window, vec![popup(10, (120, 60), (50, 50), BLUE)]);
    let rects = damage(&mut composer, output).expect("a moved popup damages the output");
    for corner in [Point::new(221, 101), Point::new(221, 161), Point::new(269, 209)] {
        assert!(
            rects.iter().any(|rect| rect.contains(corner)),
            "{corner:?} is not damaged: {rects:?}"
        );
    }
}

#[test]
fn a_popup_of_a_secure_window_is_secure() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    composer.set_window_secure(window, true);
    let frame = frame_of(&mut composer, output);

    let layers = surface_layers(&frame);
    assert_eq!(layers.len(), 2, "the window and its popup");
    assert!(
        layers.iter().all(|layer| layer.secure),
        "the window's mark is on its popup too"
    );

    // What a capture draws in place of the frame hides both.
    let masked = frame.redacted(SecureTreatment::Mask, Rect::new(0, 0, 800, 600));
    let image = pixels_of(&masked, Size::new(800, 600));
    let in_window = probe(&image, 150, 150);
    let in_popup = probe(&image, 215, 215);
    assert_ne!(in_window, RED);
    assert_ne!(in_popup, BLUE, "the popup's pixels do not reach a capture");
    assert_eq!(in_window, in_popup, "both are replaced by the same mask");

    // A View's capture treats the whole tree as secure too.
    let node = node_of(&composer, window);
    let popup_node = tree_node(&composer, window, 10);
    let secure = composer.secure_nodes(composer.subtree(node));
    assert!(secure.contains(&node) && secure.contains(&popup_node));
}

#[test]
fn a_popup_of_a_window_that_is_not_secure_is_not() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    let frame = frame_of(&mut composer, output);
    assert!(!frame.has_secure());
}

#[test]
fn a_view_of_a_window_draws_its_popups() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    let node = node_of(&composer, window);
    let view = NodeId::new(1);
    composer.with_scene(|scene| {
        scene
            .insert(view, NodeSpec::new(Rect::new(0, 0, 300, 300)))
            .expect("a view node");
        scene
            .set_class(view, Some("view".to_owned()))
            .expect("the view's class");
        scene.set_source(view, Some(node)).expect("the view's source");
    });
    frame_of(&mut composer, output);

    let (_, picture) = composer
        .view_pictures()
        .into_iter()
        .find(|(candidate, _)| *candidate == view)
        .expect("the view was drawn");
    assert_eq!(probe(&picture, 50, 50), RED, "the window is in the view");
    assert_eq!(probe(&picture, 100, 100), BLUE, "the popup is over it");
    assert_eq!(
        probe(&picture, 125, 125),
        BLUE,
        "the popup is drawn past the window's edge"
    );
    assert_eq!(probe(&picture, 150, 150), BACKGROUND, "nothing else is");
}

#[test]
fn a_view_draws_a_subsurface_below_its_parent_behind_it() {
    let (mut composer, output, window) = drawing();
    let mut below = subsurface(10, (80, 80), (40, 40), BLUE);
    below.behind = true;
    composer.set_window_surfaces(window, vec![below]);
    let node = node_of(&composer, window);
    let view = NodeId::new(1);
    composer.with_scene(|scene| {
        scene
            .insert(view, NodeSpec::new(Rect::new(0, 0, 300, 300)))
            .expect("a view node");
        scene
            .set_class(view, Some("view".to_owned()))
            .expect("the view's class");
        scene.set_source(view, Some(node)).expect("the view's source");
    });
    frame_of(&mut composer, output);

    let (_, picture) = composer
        .view_pictures()
        .into_iter()
        .find(|(candidate, _)| *candidate == view)
        .expect("the view was drawn");
    assert_eq!(probe(&picture, 90, 90), RED, "the parent covers it in a view too");
    assert_eq!(probe(&picture, 110, 110), BLUE);
}

#[test]
fn a_tree_surface_takes_the_windows_opacity_and_tint_but_not_its_look() {
    let (mut composer, output, window) = drawing();
    let mut faded = popup(10, (90, 90), (40, 40), BLUE);
    faded.alpha = 0.5;
    composer.set_window_surfaces(window, vec![faded]);
    composer.set_window_opacity(window, 0.5);
    composer.set_window_corner_radius(window, 8.0);
    composer.set_window_tint(window, Color::new(1.0, 0.5, 0.25, 1.0));
    let node = node_of(&composer, window);
    let mut shadow = Effects::new();
    shadow
        .push(Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 4.0, 0.0),
            blur: 12.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
        }))
        .expect("a shadow");
    composer.with_scene(|scene| scene.set_effects(node, shadow).expect("the effects"));
    let frame = frame_of(&mut composer, output);

    let layers = surface_layers(&frame);
    assert_eq!(layers.len(), 2);
    let (window_layer, popup_layer) = (layers[0], layers[1]);
    assert_eq!(window_layer.opacity, 0.5);
    assert_eq!(
        popup_layer.opacity, 0.25,
        "the popup's factor times the window's opacity"
    );
    assert_eq!(
        popup_layer.tint, window_layer.tint,
        "the window's tint reaches its popup"
    );
    assert_eq!(window_layer.corner_radius, 8.0);
    assert_eq!(
        popup_layer.corner_radius, 0.0,
        "a menu is not rounded by its window's corners"
    );
    assert!(!window_layer.effects.is_empty());
    assert!(popup_layer.effects.is_empty(), "a shadow belongs to the toplevel alone");
}

#[test]
fn a_hidden_window_draws_no_popup_and_draws_it_again_when_shown() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    composer.set_window_visible(window, false);
    let image = shown(&mut composer, output);
    assert_eq!(
        probe(&image, 215, 215),
        BACKGROUND,
        "a hidden window's popup is hidden with it"
    );
    assert!(composer.windows[&window].tree_nodes.is_empty());

    composer.set_window_visible(window, true);
    let image = shown(&mut composer, output);
    assert_eq!(probe(&image, 215, 215), BLUE, "the tree is kept and drawn again");
}

#[test]
fn a_tree_given_before_the_window_has_a_node_is_built_with_it() {
    let mut composer = Composer::new();
    let output = OutputId::new(0);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    composer.set_reduced_motion(true);
    composer.begin_frame();
    composer.end_frame();
    let window = WindowId::new(1);
    composer.insert_window(window, WINDOW);
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    assert!(
        composer.windows[&window].tree_nodes.is_empty(),
        "no node yet, so no surface nodes"
    );

    composer.set_window_content(window, Some(content_of(Size::new(100, 100))));
    composer.set_window_source(window, Some(flat(RED, Size::new(100, 100))));
    let image = shown(&mut composer, output);
    assert_eq!(probe(&image, 215, 215), BLUE);
}

#[test]
fn a_closing_window_takes_its_popups_with_it() {
    let (mut composer, output, window) = drawing();
    composer.set_reduced_motion(false);
    composer.advance_motion(MOTION_MS);
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    let node = node_of(&composer, window);
    let popup_node = tree_node(&composer, window, 10);

    composer.close_window(window);
    assert!(composer.windows[&window].closing.is_some(), "the window fades out");
    assert!(
        !composer.scene.contains(popup_node),
        "its popup does not fade from a buffer that may be gone"
    );
    assert!(composer.scene.contains(node));
    let frame = frame_of(&mut composer, output);
    assert_eq!(
        surface_layers(&frame).len(),
        1,
        "only the window's own picture is drawn"
    );
}

#[test]
fn a_popup_hanging_onto_the_next_output_is_drawn_there() {
    let (mut composer, first, window) = drawing();
    let second = OutputId::new(1);
    composer.set_output(second, Rect::new(800, 0, 800, 600));
    composer.damage_output(second);
    // The window sits at the right edge of the first output; its popup hangs over onto the second.
    composer.place_window(window, Rect::new(700, 100, 100, 100));
    composer.set_window_surfaces(window, vec![popup(10, (90, 0), (40, 40), BLUE)]);

    let frame = frame_of(&mut composer, second);
    let layers = surface_layers(&frame);
    assert_eq!(layers.len(), 1, "the window is not on the second output, its popup is");
    assert_eq!(
        layers[0].dst,
        Rect::new(-10, 100, 40, 40),
        "placed in the second output's own coordinates"
    );
    let image = pixels_of(&frame, Size::new(800, 600));
    assert_eq!(probe(&image, 5, 110), BLUE);
    assert_eq!(probe(&image, 35, 110), BACKGROUND);

    let image = shown(&mut composer, first);
    assert_eq!(probe(&image, 795, 110), BLUE, "and over the edge of the first");
    assert_eq!(probe(&image, 750, 150), RED);
}

#[test]
fn a_window_on_a_second_output_is_drawn_in_its_own_coordinates() {
    let (mut composer, _first, window) = drawing();
    let second = OutputId::new(1);
    composer.set_output(second, Rect::new(800, 0, 800, 600));
    composer.place_window(window, Rect::new(900, 100, 100, 100));
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);

    let frame = frame_of(&mut composer, second);
    let image = pixels_of(&frame, Size::new(800, 600));
    assert_eq!(
        probe(&image, 150, 150),
        RED,
        "the window is at (100, 100) of the second output"
    );
    assert_eq!(probe(&image, 215, 215), BLUE);
}

#[test]
fn a_popup_is_published_to_the_tree_as_a_surface_under_its_window() {
    let (mut composer, _output, window) = drawing();
    composer.take_tree_changes();
    composer.set_window_surfaces(window, vec![popup(10, (90, 80), (40, 30), BLUE)]);
    let node = node_of(&composer, window);
    let popup_node = tree_node(&composer, window, 10);
    let changes = composer.take_tree_changes();

    assert!(
        changes.iter().any(|change| matches!(
            change,
            TreeChange::Created { node: created, class, .. } if *created == popup_node && class == "surface"
        )),
        "a popup is a `surface` node: {changes:?}"
    );
    assert!(
        changes.iter().any(|change| matches!(
            change,
            TreeChange::Parented { node: moved, parent, .. } if *moved == popup_node && *parent == node
        )),
        "under its window's node: {changes:?}"
    );
    for (key, value) in [
        ("role", TreeValue::Text("popup".to_owned())),
        ("mapped", TreeValue::Bool(true)),
        ("x", TreeValue::Double(90.0)),
        ("y", TreeValue::Double(80.0)),
        ("width", TreeValue::Double(40.0)),
        ("height", TreeValue::Double(30.0)),
    ] {
        assert!(
            changes.iter().any(|change| matches!(
                change,
                TreeChange::Set { node: set, key: name, value: written }
                    if *set == popup_node && name == key && *written == value
            )),
            "{key} is published as {value:?}: {changes:?}"
        );
    }
    assert_eq!(composer.scene.class(popup_node), Some("surface"));
    assert_eq!(composer.scene.text(popup_node, "role"), Some("popup"));
    assert_eq!(
        composer.scene.input(popup_node),
        Some(false),
        "the scene's pick passes through a popup"
    );

    composer.set_window_surfaces(window, vec![popup(10, (90, 80), (40, 30), BLUE)]);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged popup publishes nothing"
    );
}

#[test]
fn a_subsurface_is_published_with_its_own_role() {
    let (mut composer, _output, window) = drawing();
    composer.set_window_surfaces(window, vec![subsurface(10, (0, 0), (20, 20), BLUE)]);
    let node = tree_node(&composer, window, 10);
    assert_eq!(composer.scene.text(node, "role"), Some("subsurface"));
}

#[test]
fn only_windows_are_windows() {
    let (mut composer, output, window) = drawing();
    composer.set_window_surfaces(window, vec![popup(10, (90, 90), (40, 40), BLUE)]);
    assert_eq!(
        composer.window_stack(),
        vec![window],
        "a popup does not join the stacking order"
    );
    let popup_node = tree_node(&composer, window, 10);
    assert_eq!(composer.window_of_node(popup_node), None);
    assert_eq!(
        composer.window_at(PointF::new(215.0, 215.0), &|_| true),
        None,
        "the pick passes through a popup: the protocol engine finds it from its own lookup"
    );
    assert_eq!(
        composer
            .window_at(PointF::new(150.0, 150.0), &|_| true)
            .map(|(found, _)| found),
        Some(window)
    );
    shown(&mut composer, output);
}

#[test]
fn the_shells_own_children_of_a_window_keep_their_place_between_the_surfaces() {
    let (mut composer, _output, window) = drawing();
    let node = node_of(&composer, window);
    let titlebar = NodeId::new(5);
    composer.with_scene(|scene| {
        let mut spec = NodeSpec::new(Rect::new(0, 0, 100, 10));
        spec.parent = Some(node);
        scene
            .insert_as(Actor::Shell, titlebar, spec)
            .expect("a shell node under the window");
    });
    let mut below = subsurface(10, (0, 0), (20, 20), BLUE);
    below.behind = true;
    composer.set_window_surfaces(window, vec![below, popup(11, (90, 90), (40, 40), GREEN)]);

    let behind = tree_node(&composer, window, 10);
    let above = tree_node(&composer, window, 11);
    assert_eq!(
        composer.scene.children(node),
        Some(&[behind, titlebar, above][..]),
        "a surface behind the window, then the shell's node, then the surfaces above"
    );
}
