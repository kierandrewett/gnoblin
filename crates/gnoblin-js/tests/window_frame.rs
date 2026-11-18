//! The window frame (`data/js/gnoblin-std/frame/`, design/window-frame.md): a surface round each window with outlets a script
//! fills, over the fake compositor of `tests/common`.
//!
//! Nodes are read from the fake compositor the way the compositor would hold them: an outlet is a canvas that is a
//! child of the window's node, placed in the window's own coordinates (the origin is the content's top left corner), so
//! `-10,-30,430,30` is a strip 30 high that starts 10 to the left of the content and 30 above it.

use gnoblin_js::{JsRole, JsRoleLinks};
use gnoblin_paint::{PaintList, PaintOp, decode};
use gnoblin_tree_client::tree_mirror::MirrorValue;

use std::sync::Arc;

mod common;
use common::*;

const LEFT: u64 = 272;

/// A frame with a different room on every side, so no side can be mistaken for another: 30 above, 20 right, 40 below and
/// 10 left of the content.
const FOUR: &str = "startDecorations({ frame(client, frame) {\n\
    if (!frame.server) return false;\n\
    frame.top.size = 30; frame.right.size = 20; frame.bottom.size = 40; frame.left.size = 10;\n\
} });";

/// A shell over a screen of 1000 by 800, one seat, one lock and `windows` windows (ids 3 on), whose `shell.ts` is `body`.
fn world(windows: u64, body: &str) -> Rig {
    let mut rig = rig_setup(LIBRARY_GLOBALS, body, move |comp| {
        comp.add_desktop(1000.0, 800.0);
        for n in 0..windows {
            comp.add_window(3 + n, &format!("win{}", n + 1), "foot");
        }
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// The content rectangle the compositor holds for window `node`: `(x, y, width, height)`.
fn content(rig: &Rig, node: u64) -> (f64, f64, f64, f64) {
    (
        rig.comp.number(node, "x"),
        rig.comp.number(node, "y"),
        rig.comp.number(node, "width"),
        rig.comp.number(node, "height"),
    )
}

/// The room the frame of the first window takes, as `top,right,bottom,left`.
fn inset(rig: &mut Rig) -> String {
    rig.eval(
        "(() => { const i = gnoblin.clients.at(0).frameInset; return `${i.top},${i.right},${i.bottom},${i.left}`; })()",
    )
}

fn decorate(rig: &mut Rig, node: u64, decoration: &str) {
    rig.comp.external_set(node, "decoration", text(decoration));
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

fn request(rig: &mut Rig, node: u64, name: &str) {
    rig.comp.request(node, name, 0.0, 0.0);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{name}: {diagnostics:?}");
}

fn exec(rig: &mut Rig, code: &str) {
    rig.eval(&format!("{code}; 'ok'"));
}

/// The paint list a canvas holds.
fn painted(rig: &Rig, node: u64) -> PaintList {
    decode(&rig.comp.content(node).expect("the node is painted")).expect("it decodes")
}

fn texts(list: &PaintList) -> Vec<String> {
    list.ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn icons(list: &PaintList) -> Vec<String> {
    list.ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Icon { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// A rectangle read from `x,y,width,height`.
fn parts(rect: &str) -> (f64, f64, f64, f64) {
    let numbers: Vec<f64> = rect.split(',').map(|part| part.parse().expect("a number")).collect();
    (numbers[0], numbers[1], numbers[2], numbers[3])
}

fn overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

// ---- insets on all four sides ---------------------------------------------------------------------------------------

#[test]
fn a_frame_takes_room_on_all_four_sides_and_its_outlets_lie_round_the_content() {
    let mut rig = world(0, FOUR);
    rig.comp.create_window(3, "term", "foot");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert_eq!(inset(&mut rig), "30,20,40,10", "the frame's room, side by side");
    assert_eq!(
        content(&rig, 3),
        (10.0, 30.0, 400.0, 300.0),
        "the window is placed as a frame at the corner: its content is what the room leaves, the size the client has"
    );
    // However they were made (top first here), the outlets stack left, right, top, bottom.
    let outlets: Vec<String> = rig.comp.children(3).iter().map(|node| rig.comp.rect(*node)).collect();
    assert_eq!(
        outlets,
        ["-10,0,10,300", "400,0,20,300", "-10,-30,430,30", "-10,300,430,40"],
        "left, right, top and bottom"
    );
    // The outlets and the content cover the frame exactly: no gap, no overlap.
    let rects: Vec<(f64, f64, f64, f64)> = outlets
        .iter()
        .map(|rect| parts(rect))
        .chain([(0.0, 0.0, 400.0, 300.0)])
        .collect();
    let area: f64 = rects.iter().map(|rect| rect.2 * rect.3).sum();
    assert_eq!(area, 430.0 * 370.0, "together they are the whole frame");
    for (at, a) in rects.iter().enumerate() {
        for b in &rects[at + 1..] {
            assert!(!overlap(*a, *b), "{a:?} and {b:?} do not overlap");
        }
    }
}

#[test]
fn a_fullscreen_window_has_no_room_taken_and_no_outlet_shown() {
    let mut rig = world(0, FOUR);
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    rig.comp.external_set(3, "fullscreen", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(inset(&mut rig), "0,0,0,0");
    for node in rig.comp.children(3) {
        assert_eq!(rig.comp.flag(node, "visible"), Some(false), "outlet {node} is hidden");
    }
    rig.comp.external_set(3, "fullscreen", MirrorValue::Bool(false));
    rig.settle();
    assert_eq!(inset(&mut rig), "30,20,40,10", "the frame comes back with the window");
}

#[test]
fn an_edge_can_give_its_room_back_while_the_window_is_maximised() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            frame.top.size = 30; frame.right.size = 20; frame.bottom.size = 40; frame.left.size = 10;\n\
            frame.left.hideWhen = (c) => c.maximised;\n\
        } });",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let floating = content(&rig, 3);
    assert_eq!(floating, (10.0, 30.0, 400.0, 300.0));
    request(&mut rig, 3, "maximize");
    assert_eq!(inset(&mut rig), "30,20,40,0", "the left edge is gone");
    assert_eq!(
        content(&rig, 3),
        (0.0, 30.0, 980.0, 730.0),
        "the work area less the room the other three edges keep"
    );
    request(&mut rig, 3, "unmaximize");
    assert_eq!(inset(&mut rig), "30,20,40,10");
    assert_eq!(
        content(&rig, 3),
        floating,
        "unmaximising returns the frame where it was"
    );
}

#[test]
fn a_window_that_gains_a_frame_keeps_the_frame_where_it_was_on_every_side() {
    let mut rig = world(0, FOUR);
    rig.comp.create_window_decorated(3, "term", "foot", "client");
    rig.settle();
    exec(
        &mut rig,
        "const c = gnoblin.clients.at(0); c.bounds = { x: 300, y: 200, width: 400, height: 300 }; c.floatingBounds = { ...c.bounds }",
    );
    rig.settle();
    assert_eq!(content(&rig, 3), (300.0, 200.0, 400.0, 300.0));
    decorate(&mut rig, 3, "server");
    assert_eq!(
        content(&rig, 3),
        (310.0, 230.0, 400.0, 300.0),
        "the frame stays at 300, 200: the content is 10 right and 30 below it"
    );
    decorate(&mut rig, 3, "client");
    assert_eq!(content(&rig, 3), (300.0, 200.0, 400.0, 300.0));
}

#[test]
fn a_frame_that_comes_to_a_window_at_the_bottom_of_the_area_is_kept_inside_it() {
    let mut rig = world(0, FOUR);
    rig.comp.create_window_decorated(3, "term", "foot", "client");
    rig.settle();
    exec(
        &mut rig,
        "gnoblin.clients.at(0).bounds = { x: 600, y: 500, width: 400, height: 300 }",
    );
    rig.settle();
    decorate(&mut rig, 3, "server");
    let (x, y, width, height) = content(&rig, 3);
    assert!(
        x - 10.0 >= 0.0 && x + width + 20.0 <= 1000.0,
        "the frame is inside across: {x} {width}"
    );
    assert!(y - 30.0 >= 0.0 && y + height + 40.0 <= 800.0, "and down: {y} {height}");
}

#[test]
fn a_snapped_and_a_tiled_window_are_placed_as_frames_with_the_room_on_every_side_taken_off() {
    let mut rig = world(0, FOUR);
    rig.comp.create_window_decorated(3, "term", "foot", "client");
    rig.settle();
    exec(
        &mut rig,
        "snapClient(gnoblin.clients.at(0), 'left', { edgeTiling: true, area: { x: 0, y: 0, width: 1000, height: 800 } })",
    );
    rig.settle();
    assert_eq!(content(&rig, 3), (0.0, 0.0, 500.0, 800.0));
    decorate(&mut rig, 3, "server");
    assert_eq!(
        content(&rig, 3),
        (10.0, 30.0, 470.0, 730.0),
        "the frame is still the whole left half: the content is what the four edges leave"
    );

    drop(rig);
    let mut rig = world(
        0,
        &format!("{FOUR} gnoblin.workspaces.add(new Workspace({{ id: '1', layout: new MasterStackLayout() }}));"),
    );
    rig.comp.create_window_decorated(3, "term", "foot", "client");
    rig.settle();
    let tile = content(&rig, 3);
    decorate(&mut rig, 3, "server");
    assert_eq!(
        content(&rig, 3),
        (tile.0 + 10.0, tile.1 + 30.0, tile.2 - 30.0, tile.3 - 70.0),
        "the tile is the frame, and the content is what is left of it"
    );
}

#[test]
fn a_script_that_changes_the_size_of_an_edge_moves_the_content_and_places_a_tiled_window_again() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) { frame.top.size = 30; globalThis.top = frame.top; } });\n\
         gnoblin.workspaces.add(new Workspace({ id: '1', layout: new MasterStackLayout() }));",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let tile = content(&rig, 3);
    assert_eq!(
        tile.1, 30.0,
        "the tile's top is the frame's: the content starts under the edge"
    );
    exec(&mut rig, "globalThis.top.size = 50");
    rig.settle();
    let now = content(&rig, 3);
    assert_eq!(now.1, 50.0, "the content moved down by what the edge gained");
    assert_eq!(now.3, tile.3 - 20.0, "and the tile is the frame again");
}

// ---- layers, and a window that draws its own frame --------------------------------------------------------------------

#[test]
fn a_window_that_draws_its_own_frame_gets_layers_but_no_edge_and_no_room() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            frame.underlay.bleed = 20;\n\
            frame.underlay.add(new Box({ x: 20, y: 20, width: 400, height: 300, fill: '#102030' }));\n\
            const badge = new Box({ x: 4, y: 4, width: 10, height: 10, fill: '#ff0000' });\n\
            frame.overlay.add(badge);\n\
            try { frame.top; } catch (error) { globalThis.refused = String(error.message); }\n\
            globalThis.kind = frame.kind;\n\
        } });",
    );
    rig.comp.create_window_decorated(3, "term", "foot", "client");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert_eq!(rig.eval("globalThis.kind"), "layers");
    assert!(
        rig.eval("globalThis.refused").contains("draws its own frame"),
        "an edge of a client-drawn window is refused: {}",
        rig.eval("globalThis.refused")
    );
    assert_eq!(inset(&mut rig), "0,0,0,0", "a client-drawn window keeps no room");
    let layers = rig.comp.children(3);
    assert_eq!(layers.len(), 2, "the underlay and the overlay, and no edge");
    let (under, over) = (layers[0], layers[1]);
    assert_eq!(rig.comp.value(under, "placement"), Some(text("under")));
    assert_eq!(rig.comp.value(over, "placement"), Some(text("over")));
    assert_eq!(
        rig.comp.flag(under, "input"),
        Some(false),
        "a layer lets the pointer through"
    );
    assert_eq!(rig.comp.flag(over, "input"), Some(false));
    assert_eq!(
        rig.comp.rect(under),
        "-20,-20,440,340",
        "the underlay is the frame and 20 pixels more on every side"
    );
    assert_eq!(rig.comp.rect(over), "0,0,400,300", "the overlay is the frame");
    let badge = rig.comp.children(over)[0];
    assert_eq!(
        rig.comp.flag(badge, "input"),
        Some(false),
        "a node added to a layer does not take the pointer unless the script says so"
    );
}

#[test]
fn a_layer_of_a_server_window_is_as_large_as_the_whole_frame() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 30; frame.left.size = 10;\n\
            frame.overlay.add(new FrameBorder(frame, { thickness: 2, color: (c) => c.focused ? '#89b4fa' : '#45475a' }));\n\
        } });",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let overlay = rig
        .comp
        .children(3)
        .into_iter()
        .last()
        .expect("the overlay is the last node");
    assert_eq!(
        rig.comp.rect(overlay),
        "-10,-30,410,330",
        "the frame: 10 left and 30 above the content"
    );
    let ring = rig.comp.children(overlay)[0];
    assert_eq!(rig.comp.rect(ring), "0,0,410,330", "the ring is as large as the layer");
    let list = painted(&rig, ring);
    assert!(
        matches!(&list.ops[..], [PaintOp::Rect { stroke_width, .. }] if *stroke_width == 2.0),
        "a ring of the width asked for: {list:?}"
    );
    let colour = |rig: &Rig| match &painted(rig, ring).ops[..] {
        [PaintOp::Rect { stroke, .. }] => *stroke,
        other => panic!("one rect, got {other:?}"),
    };
    let unfocused = colour(&rig);
    rig.comp.external_set(3, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_ne!(
        colour(&rig),
        unfocused,
        "the ring follows the focus, with no script running"
    );
}

// ---- the titlebar's zones and the stock parts ---------------------------------------------------------------------------

#[test]
fn the_three_zones_of_the_top_edge_take_their_size_the_room_that_is_left_and_their_size() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            frame.start.add(new Box({ width: 40, height: 16 }));\n\
            frame.center.add(new Box({ grow: 1, height: 16 }));\n\
            frame.end.add(new Box({ width: 20, height: 16 }));\n\
        } });",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let top = rig.comp.children(3)[0];
    assert_eq!(rig.comp.rect(top), "0,-28,400,28");
    let zones = rig.comp.children(top);
    assert_eq!(zones.len(), 3, "reading one zone made all three");
    let rects: Vec<String> = zones.iter().map(|zone| rig.comp.rect(*zone)).collect();
    assert_eq!(rects, ["0,0,40,28", "40,0,340,28", "380,0,20,28"]);
}

#[test]
fn the_stock_parts_follow_the_window_and_go_in_any_outlet_in_any_order() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            frame.start.add(new WindowButtons(frame, { buttons: ['close', 'minimise'], gap: 6 }));\n\
            frame.center.add(new TitleText(frame));\n\
            frame.end.add(new AppIcon(frame, { size: 16 }));\n\
        } });",
    );
    rig.comp.create_window(3, "term", "org.example.Files");
    rig.settle();
    let top = rig.comp.children(3)[0];
    let zones = rig.comp.children(top);
    let buttons = rig.comp.children(rig.comp.children(zones[0])[0]);
    let names: Vec<Vec<String>> = buttons.iter().map(|button| icons(&painted(&rig, *button))).collect();
    assert_eq!(
        names,
        [vec!["window-close-symbolic"], vec!["window-minimize-symbolic"]],
        "the buttons are in the order asked for, and the maximise button is left out"
    );
    let title = rig.comp.children(zones[1])[0];
    assert_eq!(texts(&painted(&rig, title)), ["term"]);
    let tile = rig.comp.children(zones[2])[0];
    assert_eq!(
        texts(&painted(&rig, tile)),
        ["F"],
        "the application's initial, from org.example.Files"
    );

    rig.comp.external_set(3, "title", text("renamed"));
    rig.settle();
    assert_eq!(
        texts(&painted(&rig, title)),
        ["renamed"],
        "the title follows the window"
    );
    rig.comp.pointer(buttons[0], "press", 4.0, 4.0, LEFT);
    rig.settle();
    assert!(rig.comp.timed_sets() > 0, "the close button fades the window out");
}

#[test]
fn a_drag_area_moves_the_window_and_a_resize_handle_resizes_it_from_its_side() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            const bar = new DragArea(frame, { grow: 1, direction: 'row', fill: '#333333' });\n\
            frame.top.add(bar);\n\
            frame.left.size = 0; frame.right.size = 0;\n\
            frame.left.add(new ResizeHandle(frame, 'left', { grow: 1 }));\n\
            frame.right.add(new ResizeHandle(frame, 'right', { grow: 1 }));\n\
            globalThis.nodes = { bar: bar.node, left: frame.left.children[0].node, right: frame.right.children[0].node };\n\
        } });",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let ids = rig.ids("[globalThis.nodes.bar, globalThis.nodes.left, globalThis.nodes.right]");
    let (bar, left, right) = (ids[0], ids[1], ids[2]);
    let start = content(&rig, 3);

    rig.comp.pointer(bar, "press", 20.0, 10.0, LEFT);
    rig.comp.pointer(bar, "motion", 50.0, 25.0, 0);
    rig.settle();
    let moved = content(&rig, 3);
    assert_eq!(
        (moved.0 - start.0, moved.1 - start.1),
        (30.0, 15.0),
        "the drag moved the window"
    );
    rig.comp.pointer(bar, "release", 50.0, 25.0, LEFT);

    // The right handle grows the window to the right; the window's left edge stays.
    rig.comp.pointer(right, "press", 0.0, 50.0, LEFT);
    rig.comp.pointer(right, "motion", 20.0, 50.0, 0);
    rig.settle();
    let wider = content(&rig, 3);
    assert_eq!(
        (wider.0, wider.2),
        (moved.0, moved.2 + 20.0),
        "the right side moved with the pointer"
    );
    rig.comp.pointer(right, "release", 20.0, 50.0, LEFT);

    // The left handle moves the left side: the window's right edge stays.
    rig.comp.pointer(left, "press", 0.0, 50.0, LEFT);
    rig.comp.pointer(left, "motion", -10.0, 50.0, 0);
    rig.settle();
    let leftward = content(&rig, 3);
    assert_eq!(leftward.0, wider.0 - 10.0);
    assert_eq!(leftward.0 + leftward.2, wider.0 + wider.2, "the right edge stayed");
    rig.comp.pointer(left, "release", -10.0, 50.0, LEFT);

    // A drag that would shrink the window past its least size stops there, with the far side held.
    rig.comp.pointer(left, "press", 0.0, 50.0, LEFT);
    rig.comp.pointer(left, "motion", 900.0, 50.0, 0);
    rig.settle();
    let least = content(&rig, 3);
    assert_eq!(least.2, 100.0, "the stock least width");
    assert_eq!(
        least.0 + least.2,
        leftward.0 + leftward.2,
        "and the right edge is where it was"
    );
}

#[test]
fn resize_handles_lie_round_the_frame_outside_the_window_geometry_and_not_while_it_is_maximised() {
    let mut rig = world(0, "startDecorations({ resizeHandles: 6 });");
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let outlets = rig.comp.children(3);
    assert_eq!(
        outlets.len(),
        2,
        "the top outlet's bar and the overlay the handles are in"
    );
    let overlay = outlets[1];
    assert_eq!(
        rig.comp.rect(overlay),
        "0,-28,400,328",
        "the overlay is the whole frame"
    );
    assert_eq!(rig.comp.flag(overlay, "input"), Some(false));
    let holder = rig.comp.children(overlay)[0];
    assert_eq!(
        rig.comp.flag(holder, "input"),
        Some(false),
        "only the handles take the pointer"
    );
    let handles: Vec<String> = rig
        .comp
        .children(holder)
        .iter()
        .map(|node| rig.comp.rect(*node))
        .collect();
    assert_eq!(
        handles,
        [
            "-6,-6,14,14",
            "392,-6,14,14",
            "-6,320,14,14",
            "392,320,14,14",
            "8,-6,384,8",
            "8,326,384,8",
            "-6,8,8,312",
            "398,8,8,312"
        ],
        "four corners and four edges, reaching 6 pixels outside the frame (a negative x is outside the window)"
    );
    let right = rig.comp.children(holder)[7];
    rig.comp.pointer(right, "press", 2.0, 50.0, LEFT);
    rig.comp.pointer(right, "motion", 12.0, 50.0, 0);
    rig.settle();
    assert_eq!(content(&rig, 3).2, 410.0, "the right handle grew the window");
    request(&mut rig, 3, "maximize");
    assert_eq!(
        rig.comp.flag(right, "visible"),
        Some(false),
        "a maximised window is not resized by its handles"
    );
}

// ---- who gets a frame ---------------------------------------------------------------------------------------------------

#[test]
fn rules_pick_the_frame_by_application_and_title_and_a_callback_can_give_a_window_none() {
    let mut rig = world(
        0,
        "startDecorations({\n\
            rules: [\n\
                { appId: 'mpv', frame: false },\n\
                { appId: /^org\\./, frame: (client, frame) => { frame.top.size = 10; } },\n\
                { title: 'scratch', frame: (client, frame) => false },\n\
            ],\n\
        });",
    );
    rig.comp.create_window(3, "video", "mpv");
    rig.comp.create_window(4, "files", "org.example.Files");
    rig.comp.create_window(5, "scratch", "foot");
    rig.comp.create_window(6, "plain", "foot");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    let insets: Vec<String> = (0..4)
        .map(|at| rig.eval(&format!("String(gnoblin.clients.at({at}).frameInset.top)")))
        .collect();
    assert_eq!(
        insets,
        ["0", "10", "0", "28"],
        "no frame, a 10 pixel edge, no frame, the stock titlebar"
    );
    assert!(rig.comp.children(3).is_empty());
    assert!(rig.comp.children(5).is_empty());
    assert_eq!(rig.comp.children(6).len(), 1);
}

#[test]
fn a_fill_that_throws_leaves_no_frame_and_no_room_behind() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) { frame.top.size = 30; frame.top.add(new Box({ height: 30 })); throw new Error('no'); } });",
    );
    rig.comp.create_window(3, "term", "foot");
    let diagnostics = rig.settle();
    assert!(
        diagnostics.iter().any(|line| line.contains("no")),
        "the throw is reported: {diagnostics:?}"
    );
    assert_eq!(inset(&mut rig), "0,0,0,0", "a half-built frame takes no room");
    assert!(rig.comp.children(3).is_empty(), "and leaves no node on the window");
}

#[test]
fn the_fill_runs_again_when_the_decoration_changes_and_nothing_is_left_when_the_window_goes() {
    let mut rig = world(
        0,
        "globalThis.fills = [];\n\
         startDecorations({ frame(client, frame) { globalThis.fills.push(frame.kind); frame.overlay.add(new Box({ width: 4, height: 4 })); } });",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    assert_eq!(rig.eval("globalThis.fills.join()"), "server");
    assert_eq!(rig.comp.children(3).len(), 1, "the overlay");
    decorate(&mut rig, 3, "client");
    assert_eq!(
        rig.eval("globalThis.fills.join()"),
        "server,layers",
        "the fill ran again for the kind of frame it has now"
    );
    assert_eq!(
        rig.comp.children(3).len(),
        1,
        "the server frame's overlay is gone and the layers frame has its own"
    );
    decorate(&mut rig, 3, "server");
    assert_eq!(rig.eval("globalThis.fills.join()"), "server,layers,server");
    assert_eq!(rig.comp.children(3).len(), 1);
    rig.comp.destroy(3);
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert!(
        rig.comp.canvases().is_empty(),
        "the compositor took the frame's nodes with the window"
    );
}

#[test]
fn a_reload_takes_every_node_the_generation_mounted_away() {
    let mut rig = world(1, "startDecorations({ resizeHandles: 6 });");
    rig.settle();
    assert!(
        rig.comp.canvases().len() > 10,
        "a titlebar and handles: {}",
        rig.comp.canvases().len()
    );
    rig.shell.retire().expect("retire");
    rig.settle();
    assert!(
        rig.comp.canvases().is_empty(),
        "no canvas of the old generation is left behind"
    );
}

// ---- reactive nodes -----------------------------------------------------------------------------------------------------

#[test]
fn a_node_in_an_outlet_follows_the_window_through_a_binding_without_the_script_running_again() {
    let mut rig = world(
        0,
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 20;\n\
            const dot = new Box({ width: 10, height: 10, fill: '#ff0000' });\n\
            dot.properties.bind('visible', () => !client.focused);\n\
            frame.top.add(dot);\n\
            globalThis.dot = dot.node;\n\
        } });",
    );
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let dot = rig.ids("[globalThis.dot]")[0];
    assert_ne!(
        rig.comp.flag(dot, "visible"),
        Some(false),
        "the window is not focused: the dot shows (visible is the default, so it is not written)"
    );
    rig.comp.external_set(3, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(rig.comp.flag(dot, "visible"), Some(false), "it follows the focus");
    rig.comp.destroy(3);
    let diagnostics = rig.settle();
    assert_eq!(
        diagnostics,
        Vec::<String>::new(),
        "a binding of a node that went with its window ends quietly"
    );
}

// ---- the three worked examples of docs/recipes/window-frame.md ----------------------------------------------------------

/// The code of the first `ts` block under `## heading` of the recipes page. The tests mount the page's own code, so an
/// example in the docs that stops working fails here.
fn recipe(heading: &str) -> String {
    let page = include_str!("../../../docs/recipes/window-frame.md");
    let at = page
        .find(&format!("## {heading}\n"))
        .unwrap_or_else(|| panic!("the page has the heading {heading}"));
    let rest = &page[at..];
    let open = rest.find("```ts\n").expect("a code block under the heading") + "```ts\n".len();
    let close = rest[open..].find("```").expect("the block ends");
    rest[open..open + close].to_owned()
}

#[test]
fn the_recipe_with_the_buttons_on_the_left_mounts_and_works() {
    let mut rig = world(0, &recipe("A bar with the buttons on the left"));
    rig.comp.create_window(3, "term", "foot");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert_eq!(
        inset(&mut rig),
        "34,0,0,0",
        "the top edge is 34 pixels and nothing else takes room"
    );
    let top = rig.comp.children(3)[0];
    assert_eq!(rig.comp.rect(top), "0,-34,400,34");
    let zones = rig.comp.children(top);
    assert_eq!(zones.len(), 3);
    let start = parts(&rig.comp.rect(zones[0]));
    let center = parts(&rig.comp.rect(zones[1]));
    assert!(start.2 > 0.0, "the buttons give the first zone a width");
    assert_eq!(center.0, start.2, "the center zone starts where the buttons end");
    let strip = rig.comp.children(zones[0])[0];
    let buttons = rig.comp.children(strip);
    let names: Vec<Vec<String>> = buttons.iter().map(|button| icons(&painted(&rig, *button))).collect();
    assert_eq!(
        names,
        [
            vec!["window-close-symbolic"],
            vec!["window-minimize-symbolic"],
            vec!["window-maximize-symbolic"]
        ],
        "the buttons are in the traffic-light order"
    );
    let xs: Vec<f64> = buttons.iter().map(|button| parts(&rig.comp.rect(*button)).0).collect();
    assert!(xs.windows(2).all(|pair| pair[0] < pair[1]), "left to right: {xs:?}");
    let bar = rig.comp.children(zones[1])[0];
    let title = rig.comp.children(bar)[0];
    assert_eq!(texts(&painted(&rig, title)), ["term"]);

    let start = content(&rig, 3);
    rig.comp.pointer(bar, "press", 20.0, 10.0, LEFT);
    rig.comp.pointer(bar, "motion", 50.0, 25.0, 0);
    rig.settle();
    let moved = content(&rig, 3);
    assert_eq!(
        (moved.0 - start.0, moved.1 - start.1),
        (30.0, 15.0),
        "the bar drags the window"
    );
    rig.comp.pointer(bar, "release", 50.0, 25.0, LEFT);
    rig.comp.pointer(buttons[0], "press", 4.0, 4.0, LEFT);
    rig.settle();
    assert!(rig.comp.timed_sets() > 0, "the close button fades the window out");
}

#[test]
fn the_recipe_with_a_border_and_a_glow_and_no_titlebar_mounts_and_works() {
    let mut rig = world(0, &recipe("A border and a glow without a titlebar"));
    rig.comp.create_window(3, "term", "foot");
    rig.comp.create_window_decorated(4, "own", "gtk", "client");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert_eq!(
        inset(&mut rig),
        "0,0,0,0",
        "no edge is touched, so the frame takes no room"
    );
    for window in [3, 4] {
        let layers = rig.comp.children(window);
        assert_eq!(
            layers.len(),
            2,
            "window {window}: the underlay and the overlay, and no titlebar"
        );
        let (under, over) = (layers[0], layers[1]);
        assert_eq!(rig.comp.value(under, "placement"), Some(text("under")));
        assert_eq!(rig.comp.value(over, "placement"), Some(text("over")));
        assert_eq!(
            rig.comp.rect(under),
            "-24,-24,448,348",
            "the glow reaches 24 pixels past the frame"
        );
        assert_eq!(rig.comp.rect(over), "0,0,400,300", "the overlay is the frame");
        let glow = rig.comp.children(under)[0];
        let rings = painted(&rig, glow)
            .ops
            .iter()
            .filter(|op| matches!(op, PaintOp::Rect { .. }))
            .count();
        assert_eq!(rings, 8, "the halo is eight rings");
        let ring = rig.comp.children(over)[0];
        let stroke = |rig: &Rig| match &painted(rig, ring).ops[..] {
            [
                PaintOp::Rect {
                    stroke, stroke_width, ..
                },
            ] => (*stroke, *stroke_width),
            other => panic!("one rect, got {other:?}"),
        };
        let (unfocused, width) = stroke(&rig);
        assert_eq!(width, 2.0);
        rig.comp.external_set(window, "focused", MirrorValue::Bool(true));
        rig.settle();
        assert_ne!(stroke(&rig).0, unfocused, "the ring follows the focus");
    }
}

#[test]
fn the_recipe_with_a_status_strip_and_a_dock_handle_mounts_and_works() {
    let mut rig = world(0, &recipe("A status strip at the bottom and a dock handle at the side"));
    rig.comp.create_window(3, "term", "foot");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert_eq!(
        inset(&mut rig),
        "28,12,22,0",
        "the stock titlebar, a 22 pixel strip and a 12 pixel handle"
    );
    let outlets: Vec<String> = rig.comp.children(3).iter().map(|node| rig.comp.rect(*node)).collect();
    assert_eq!(
        outlets,
        ["400,0,12,300", "0,-28,412,28", "0,300,412,22"],
        "the right edge, the titlebar's top edge and the bottom strip"
    );
    let [right, _, bottom] = rig.comp.children(3)[..] else {
        panic!("three outlets");
    };
    let status = rig.comp.children(bottom)[0];
    assert_eq!(texts(&painted(&rig, status)), ["foot  400 x 300"]);

    let handle = rig.comp.children(right)[0];
    rig.comp.pointer(handle, "press", 2.0, 50.0, LEFT);
    rig.comp.pointer(handle, "motion", 12.0, 50.0, 0);
    rig.settle();
    assert_eq!(content(&rig, 3).2, 410.0, "the handle on the right grew the window");
    assert_eq!(
        texts(&painted(&rig, status)),
        ["foot  410 x 300"],
        "and the strip follows the size"
    );
    rig.comp.pointer(handle, "release", 12.0, 50.0, LEFT);

    request(&mut rig, 3, "maximize");
    assert_eq!(
        inset(&mut rig),
        "28,12,0,0",
        "the strip gives its room back while the window is maximised"
    );
}

// ---- hot reload ---------------------------------------------------------------------------------------------------------

#[test]
fn a_reload_swaps_the_generation_and_leaves_no_node_of_the_old_one() {
    let comp = FakeCompositor::new();
    comp.add_desktop(1000.0, 800.0);
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = std::fs::canonicalize(dir.path()).expect("the temp dir exists");
    let entry = root.join("shell.ts");
    std::fs::write(
        &entry,
        format!("{LIBRARY_GLOBALS}startDecorations({{ resizeHandles: 6 }});\n"),
    )
    .expect("shell.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    // The shell's library is the std library over a core of its own, as the runtime's is.
    let core = common::native::NativeCore::new(&comp, Arc::clone(&config.shell.clock));
    config.shell.js.natives.push(core.std_native());
    let mut role = JsRole::over_link(core.link(), JsRoleLinks::default(), config).expect("the role boots");
    let settle = |role: &mut JsRole| {
        for _ in 0..6 {
            role.tick().expect("a frame");
            core.dispatch(role.shell_mut());
        }
    };
    // A window the first generation frames as it maps, the way a real one arrives.
    comp.create_window(3, "win1", "foot");
    settle(&mut role);
    let with_handles = comp.canvases().len();
    assert!(with_handles > 10, "a titlebar and eight handles: {with_handles}");
    assert_eq!(
        comp.number(3, "y"),
        28.0,
        "the window is placed below the titlebar of the first generation"
    );

    // The next generation has a frame of another shape: none of the first one's nodes may be left, and the window's
    // content moves to where the new frame wants it with the frame itself kept in place.
    std::fs::write(&entry, format!("{LIBRARY_GLOBALS}{FOUR}\n")).expect("shell.ts");
    let report = role.reload_now();
    assert!(report.ok, "{:?}", report.errors);
    settle(&mut role);
    assert_eq!(
        comp.canvases().len(),
        4,
        "the four outlets of the new frame, and nothing of the old one"
    );
    assert_eq!(comp.children(3).len(), 4);
    assert_eq!(
        (comp.number(3, "x"), comp.number(3, "y")),
        (10.0, 30.0),
        "the frame stayed where it was: the content moved to what the new edges leave"
    );

    // A generation that never frames a window takes the last frame away and gives the window its room back.
    std::fs::write(&entry, format!("{LIBRARY_GLOBALS}\n")).expect("shell.ts");
    assert!(role.reload_now().ok);
    settle(&mut role);
    assert!(
        comp.canvases().is_empty(),
        "no canvas of any earlier generation is left"
    );
    assert_eq!((comp.number(3, "x"), comp.number(3, "y")), (0.0, 0.0));
}
