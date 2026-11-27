//! The scene API of the owner's specification (sections 8, 14, 15, 16) over the fake compositor of `tests/common`:
//! script nodes with writable transforms, the `Scene`, a `View` of a client and of a workspace, `ArcLayout` and the
//! `transform` placements of a layout, and property bindings that follow what they read (`data/js/gnoblin-std/scene/`,
//! `properties.ts`, `scene_mirror.ts`, `spatial.ts`, `layouts/arc.ts`).

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

/// The first window's id: windows are 3, 4, 5, ...
const WIN: u64 = 3;

fn double(value: f64) -> MirrorValue {
    MirrorValue::Double(value)
}

/// A shell over a screen of 1000 by 800, a seat and one 800 by 600 window per entry of `count` (ids 3 on), whose
/// `shell.ts` is `body`.
fn world(count: u64, body: &str) -> Rig {
    let (rig, diagnostics) = boot(count, body);
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// Like [`world`], returning the diagnostics of the boot frames instead of requiring none.
fn boot(count: u64, body: &str) -> (Rig, Vec<String>) {
    let mut rig = rig_setup(LIBRARY_GLOBALS, body, move |comp| {
        comp.add_desktop(1000.0, 800.0);
        for n in 0..count {
            comp.add_node(
                WIN + n,
                ROOT,
                "window",
                vec![
                    ("title", text(&format!("win{}", n + 1))),
                    ("app_id", text(&format!("app{}", n + 1))),
                    ("width", double(800.0)),
                    ("height", double(600.0)),
                ],
            );
        }
    });
    let diagnostics = rig.settle();
    (rig, diagnostics)
}

fn number(rig: &mut Rig, expression: &str) -> f64 {
    rig.eval(&format!("String({expression})")).parse().expect("a number")
}

fn numbers(rig: &mut Rig, expression: &str) -> Vec<f64> {
    rig.eval(&format!("({expression}).join(',')"))
        .split(',')
        .map(|part| part.parse().expect("a number"))
        .collect()
}

fn list(rig: &Rig, node: u64, key: &str) -> Vec<f64> {
    match rig.comp.value(node, key) {
        Some(MirrorValue::List(parts)) => parts,
        other => panic!("node {node} has no list {key}: {other:?}"),
    }
}

/// The turn about Y of a node; none is written for the default of 0.
fn turn(rig: &Rig, node: u64) -> f64 {
    match rig.comp.value(node, "rotation") {
        Some(MirrorValue::List(angles)) => angles[1],
        _ => 0.0,
    }
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-6, "{what}: got {got}, want {want}");
}

fn all_close(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} against {want:?}");
    for (g, w) in got.iter().zip(want) {
        close(*g, *w, what);
    }
}

fn thrown(rig: &mut Rig, source: &str) -> String {
    rig.eval(&format!(
        "(() => {{ try {{ {source}; return 'did not throw'; }} catch (e) {{ return e.message; }} }})()"
    ))
}

// ---- Nodes: placement ---------------------------------------------------------------------------------------------

#[test]
fn a_node_takes_an_id_a_position_an_euler_rotation_and_a_scale_as_the_specification_writes_them() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.room = new SceneNode({ id: 'xr-room', position: { x: 1, y: 2, z: 3 }, \
         rotation: { x: 0.1, y: 0.2, z: 0.3 } }); room.place({ uniformScale: 2 }); room.metresPerUnit = 0.5; 1",
    );
    rig.settle();
    let node = rig.comp.named("xr-room").expect("the node is in the tree under its id");
    all_close(
        &[
            rig.comp.number(node, "x"),
            rig.comp.number(node, "y"),
            rig.comp.number(node, "z"),
        ],
        &[1.0, 2.0, 3.0],
        "position",
    );
    all_close(&list(&rig, node, "rotation"), &[0.1, 0.2, 0.3], "rotation");
    all_close(
        &list(&rig, node, "scale"),
        &[2.0, 2.0, 2.0],
        "a uniform scale scales every axis",
    );
    assert_eq!(rig.eval("room.id"), "xr-room");
    close(
        number(&mut rig, "room.rotation.y"),
        0.2,
        "rotation answers to its axis names",
    );
    close(
        number(
            &mut rig,
            "(() => { const a = room.position, b = new SceneNode({ position: { x: 1, y: 2, z: 7 } }).position; return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z); })()",
        ),
        4.0,
        "position is a vector",
    );
    close(number(&mut rig, "room.metresPerUnit"), 0.5, "metresPerUnit is kept");
    close(
        number(&mut rig, "new SceneNode().metresPerUnit"),
        1.0,
        "and is 1 by default",
    );
}

#[test]
#[ignore = "vrnw: needs SceneNode.rotation to answer to an index (rotation[1]) and Vec3.distanceTo"]
fn a_rotation_answers_to_an_index_and_a_position_measures_a_distance() {
    let mut rig = world(1, "");
    rig.eval("globalThis.room = new SceneNode({ id: 'xr-room', rotation: { x: 0.1, y: 0.2, z: 0.3 } }); 1");
    close(number(&mut rig, "room.rotation[1]"), 0.2, "and to an index");
    close(
        number(
            &mut rig,
            "room.position.distanceTo(new SceneNode({ position: { x: 1, y: 2, z: 7 } }).position)",
        ),
        0.0,
        "position is a vector",
    );
}

#[test]
fn a_rotation_may_be_given_as_a_quaternion() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.n = new SceneNode({ id: 'q' }); n.place({ orientation: { x: 0, y: Math.sin(0.25), z: 0, w: Math.cos(0.25) } }); 1",
    );
    rig.settle();
    let node = rig.comp.named("q").expect("the node");
    all_close(
        &list(&rig, node, "rotation"),
        &[0.0, 0.5, 0.0],
        "a quaternion is stored as Euler angles",
    );
}

#[test]
fn a_bad_rotation_or_scale_names_the_node_and_the_field() {
    let mut rig = world(1, "");
    rig.eval("globalThis.n = new SceneNode({ id: 'q' }); 1");
    let message = thrown(&mut rig, "n.rotation = { x: 0, y: NaN, z: 0 }");
    assert!(message.contains("Node#") && message.contains(".rotation"), "{message}");
    let message = thrown(&mut rig, "n.scale = 'big'");
    assert!(message.contains("expected a Vec3 object"), "{message}");
}

#[test]
fn a_transform_given_as_parts_writes_only_the_parts_it_has_and_checks_them_all_first() {
    let mut rig = world(1, "");
    rig.eval("globalThis.n = new SceneNode({ id: 'p', position: { x: 1, y: 1, z: 1 }, rotation: { x: 0, y: 0.5, z: 0 } }); 1");
    rig.settle();
    let node = rig.comp.named("p").expect("the node");
    rig.eval("n.place({ position: { x: 5, y: 6, z: 7 } }); 1");
    rig.settle();
    all_close(
        &[
            rig.comp.number(node, "x"),
            rig.comp.number(node, "y"),
            rig.comp.number(node, "z"),
        ],
        &[5.0, 6.0, 7.0],
        "position",
    );
    all_close(
        &list(&rig, node, "rotation"),
        &[0.0, 0.5, 0.0],
        "a part left out is not written",
    );
    // A bad part refuses the whole transform: the valid position is not applied.
    let message = thrown(
        &mut rig,
        "n.place({ position: { x: 9, y: 9, z: 9 }, rotation: { x: 'no', y: 0, z: 0 } })",
    );
    assert!(message.contains("rotation.x must be a number"), "{message}");
    rig.settle();
    close(
        rig.comp.number(node, "x"),
        5.0,
        "nothing of a refused transform is written",
    );
    // A transform reads back as the plain parts.
    close(number(&mut rig, "n.transform.position.x"), 5.0, "the getter");
    close(number(&mut rig, "n.transform.rotation.y"), 0.5, "the getter's rotation");
}

#[test]
fn a_transform_with_a_part_it_has_not_is_refused() {
    let mut rig = world(1, "");
    rig.eval("globalThis.n = new SceneNode({ id: 'p' }); 1");
    let message = thrown(&mut rig, "n.place({ position: { x: 0, y: 0, z: 0 }, pose: 1 })");
    // The generated record refuses a part it has in its own words, which name the record and the part.
    assert!(message.contains("TransformParts: no field \"pose\""), "{message}");
}

#[test]
#[ignore = "vrnw: needs client.set"]
fn a_client_takes_a_transform_in_set_as_one_atomic_write_and_reads_position_and_committed_size() {
    let mut rig = world(1, "");
    let mark = rig.comp.sent_count();
    rig.eval(
        "globalThis.c = gnoblin.clients.at(0); \
         c.set({ opacity: 0.92, visible: true, transform: { rotation: { x: 0, y: 0.12, z: 0 } } }, { duration: 250, easing: 'ease-out' }); 1",
    );
    rig.frame();
    assert_eq!(rig.comp.sent_count() - mark, 1, "one transaction");
    let sets = rig.comp.sets_of(WIN);
    for key in ["opacity", "rotation"] {
        assert!(sets.iter().any(|set| set == key), "{key} is in it: {sets:?}");
    }
    rig.settle();
    close(
        number(&mut rig, "c.rotation[1]"),
        0.12,
        "the rotation is the transform's",
    );
    // Without options it is atomic and immediate.
    let mark = rig.comp.sent_count();
    rig.eval("c.set({ opacity: 1, transform: { position: { x: 30, y: 40, z: -5 }, scale: 1.5 } }); 1");
    rig.frame();
    assert_eq!(rig.comp.sent_count() - mark, 1, "one transaction");
    rig.settle();
    all_close(
        &[
            rig.comp.number(WIN, "x"),
            rig.comp.number(WIN, "y"),
            rig.comp.number(WIN, "z"),
        ],
        &[30.0, 40.0, -5.0],
        "position",
    );
    all_close(&list(&rig, WIN, "scale"), &[1.5, 1.5, 1.5], "scale");
    all_close(&numbers(&mut rig, "c.position"), &[30.0, 40.0, -5.0], "Client.position");
    // committedSize is what the compositor published: the 800 by 600 the window has, not what was asked of it.
    all_close(
        &numbers(&mut rig, "[c.committedSize.width, c.committedSize.height]"),
        &[800.0, 600.0],
        "committed",
    );
    rig.eval("c.width = 400; 1");
    rig.frame();
    close(number(&mut rig, "c.width"), 400.0, "the asked width");
    let message = thrown(&mut rig, "c.set({ transform: { pose: 1 } })");
    assert!(
        message.contains("a transform has position, rotation and scale only"),
        "{message}"
    );
}

// ---- Nodes: input, the scene and Views ----------------------------------------------------------------------------

#[test]
fn input_is_forward_capture_or_none() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.v = new View({ id: 'panel', size: { width: 0.9, height: 0.55 }, fit: 'contain', input: 'forward' }); \
         v.showClient(gnoblin.clients.at(0)); 1",
    );
    rig.settle();
    let view = rig.comp.named("panel").expect("the view");
    assert_eq!(rig.eval("v.input"), "forward");
    assert!(rig.comp.flag(view, "input").unwrap_or(true));
    rig.eval("v.input = 'none'");
    rig.settle();
    assert_eq!(rig.eval("v.input"), "none");
    assert_eq!(rig.comp.flag(view, "input"), Some(false), "none is a miss in the tree");
    rig.eval("v.input = 'capture'");
    rig.settle();
    assert_eq!(rig.eval("v.input"), "capture");
    assert_eq!(rig.comp.flag(view, "input"), Some(true), "capture is hit in the tree");
    assert_eq!(
        rig.comp.value(view, "input_mode"),
        Some(text("capture")),
        "and the mode the compositor routes by is written natively"
    );
    rig.eval("v.input = 'forward'");
    rig.settle();
    assert_eq!(rig.comp.value(view, "input_mode"), Some(text("forward")));
    let message = thrown(&mut rig, "v.input = 'bogus'");
    assert!(message.contains("(forward, capture, none)"), "{message}");
}

#[test]
#[ignore = "vrnw: needs View.input to take the old booleans (false is none, true is forward)"]
fn the_old_booleans_still_write_an_input_mode() {
    let mut rig = world(1, "");
    rig.eval("globalThis.v = new View({ id: 'panel', input: 'capture' }); 1");
    rig.eval("v.input = false");
    assert_eq!(rig.eval("v.input"), "none");
    rig.eval("v.input = true");
    assert_eq!(rig.eval("v.input"), "forward");
}

#[test]
fn a_view_of_a_client_has_a_size_a_content_fit_and_a_rotation_in_euler_form() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.v = new View({ id: 'panel', size: { width: 0.9, height: 0.55 }, fit: 'contain' }); \
         v.showClient(gnoblin.clients.at(0)); \
         gnoblin.requestTransition(() => { v.position = { x: 0.5, y: 0.2, z: -1.4 }; \
         v.rotation = { x: 0, y: 0.3, z: 0 }; v.size = { width: 1.2, height: 0.75 }; }, { duration: 550, easing: 'ease-out' }); 1",
    );
    rig.settle();
    let view = rig.comp.named("panel").expect("the view");
    close(rig.comp.number(view, "width"), 1.2, "size is the view's width");
    close(rig.comp.number(view, "height"), 0.75, "and height");
    close(rig.comp.number(view, "z"), -1.4, "position");
    all_close(&list(&rig, view, "rotation"), &[0.0, 0.3, 0.0], "an Euler object");
    assert_eq!(rig.eval("v.fit"), "contain");
    assert_eq!(rig.eval("String(v.source)"), WIN.to_string());
    all_close(
        &numbers(&mut rig, "[v.size.width, v.size.height]"),
        &[1.2, 0.75],
        "size reads back",
    );
    let message = thrown(&mut rig, "v.fit = 'fill'");
    assert!(message.contains("(contain, cover, stretch)"), "{message}");
}

#[test]
#[ignore = "vrnw: needs View.content (a client, a workspace or null; the view reads and writes what it shows)"]
fn a_views_content_is_a_client_a_workspace_or_null() {
    let mut rig = world(1, "");
    rig.eval("globalThis.v = new View({ id: 'panel' }); v.showClient(gnoblin.clients.at(0)); 1");
    assert_eq!(rig.eval("v.content === gnoblin.clients.at(0)"), "true");
    let message = thrown(&mut rig, "v.content = {}");
    assert!(message.contains("must be a client, a workspace or null"), "{message}");
}

#[test]
fn the_scene_root_mounts_nodes_and_cannot_be_destroyed() {
    let mut rig = world(1, "");
    assert_eq!(rig.eval("gnoblin.scene instanceof Scene"), "true");
    rig.eval("globalThis.room = new SceneNode({ id: 'xr-room' }); gnoblin.scene.root.add(room); 1");
    rig.settle();
    let node = rig.comp.named("xr-room").expect("the room");
    assert_eq!(
        rig.comp.parent_of(node),
        Some(ROOT),
        "the room is under the tree's root"
    );
    assert_eq!(rig.eval("room.parent === gnoblin.scene.root"), "true");
    assert_eq!(rig.eval("gnoblin.scene.root.children.includes(room)"), "true");
    close(
        number(&mut rig, "gnoblin.scene.metresPerUnit"),
        1.0,
        "the default unit is logical units",
    );
    rig.eval("gnoblin.scene.metresPerUnit = 0.001");
    close(number(&mut rig, "gnoblin.scene.metresPerUnit"), 0.001, "it is settable");
    let message = thrown(&mut rig, "gnoblin.scene.metresPerUnit = 0");
    assert!(message.contains("above 0"), "{message}");
}

#[test]
fn the_scene_root_cannot_be_destroyed() {
    let mut rig = world(1, "");
    let message = thrown(&mut rig, "gnoblin.scene.root.destroy()");
    assert!(message.contains("cannot be destroyed"), "{message}");
}

#[test]
fn a_second_scene_cannot_be_made() {
    let mut rig = world(1, "");
    let message = thrown(&mut rig, "new Scene()");
    assert!(message.contains("`gnoblin.scene`"), "{message}");
}

/// The nodes under the holder of the View named `name`: the picture of a workspace, one child View per client.
fn picture(rig: &Rig, name: &str) -> (u64, Vec<u64>) {
    let view = rig.comp.named(name).expect("the view");
    let holders = rig.comp.children(view);
    assert_eq!(holders.len(), 1, "one holder under the View: {holders:?}");
    (holders[0], rig.comp.children(holders[0]))
}

fn source_of(rig: &Rig, node: u64) -> u64 {
    match rig.comp.value(node, "source") {
        Some(MirrorValue::Node(source)) => source,
        other => panic!("node {node} has no source: {other:?}"),
    }
}

#[test]
fn a_view_of_a_workspace_draws_one_child_view_per_client_fitted_to_its_size() {
    let mut rig = world(
        2,
        "const desktop = new Workspace('desktop', { layout: new FloatingLayout() });\n\
         gnoblin.workspaces.add(desktop);\n",
    );
    rig.eval(
        "globalThis.overview = new SceneNode({ id: 'overview' });\
         globalThis.big = overview.add(new View({ id: 'big', size: { width: 700, height: 400 } })); big.showWorkspace(gnoblin.workspaces.byId('desktop'));\
         globalThis.small = overview.add(new View({ id: 'small', size: { width: 350, height: 200 } })); small.showWorkspace(gnoblin.workspaces.byId('desktop')); 1",
    );
    rig.settle();
    let (holder, cards) = picture(&rig, "big");
    let shown: Vec<u64> = cards.iter().map(|card| source_of(&rig, *card)).collect();
    assert_eq!(
        shown,
        vec![WIN, WIN + 1],
        "a child View per client, sourcing the client's own node"
    );
    // The screen is 1000 by 800 and the View 700 by 400: stretch scales each axis on its own.
    all_close(&list(&rig, holder, "scale"), &[0.7, 0.5, 1.0], "stretch");
    let (small_holder, small_cards) = picture(&rig, "small");
    assert_eq!(small_cards.len(), 2, "the same workspace twice");
    all_close(
        &list(&rig, small_holder, "scale"),
        &[0.35, 0.25, 1.0],
        "the second View's fit",
    );
    assert_eq!(rig.eval("big.workspace"), "desktop");
    assert_eq!(rig.eval("big.orphaned"), "false");
    assert_eq!(
        rig.eval("big.children.length"),
        "0",
        "the picture's nodes are not the script's children"
    );
    // contain keeps the aspect and centres the picture.
    rig.eval("big.fit = 'contain'");
    rig.settle();
    let (holder, _) = picture(&rig, "big");
    all_close(&list(&rig, holder, "scale"), &[0.5, 0.5, 1.0], "contain");
    close(rig.comp.number(holder, "x"), 100.0, "centred across");
    // A window that opens joins the picture, and one that closes leaves it.
    rig.comp.create_window(900, "late", "late");
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("gnoblin.workspaces.byId('desktop').clients.length"), "3");
    let (_, cards) = picture(&rig, "big");
    assert_eq!(cards.len(), 3, "the new window is in the picture");
    rig.comp.destroy(WIN);
    rig.settle();
    let (_, cards) = picture(&rig, "big");
    let shown: Vec<u64> = cards.iter().map(|card| source_of(&rig, *card)).collect();
    assert_eq!(shown, vec![WIN + 1, 900]);
    // Destroying the View removes its picture; the other View is not affected.
    rig.eval("big.destroy(); 1");
    rig.settle();
    assert!(rig.comp.named("big").is_none());
    assert_eq!(picture(&rig, "small").1.len(), 2, "the other View is not affected");
    // A View given a client takes the picture away.
    rig.eval("small.showClient(gnoblin.clients.at(0)); 1");
    rig.settle();
    assert!(
        rig.comp.children(rig.comp.named("small").expect("small")).is_empty(),
        "no picture under a client's View"
    );
}

// ---- Layouts: ArcLayout and transform placements --------------------------------------------------------------------

fn arc_world(options: &str) -> Rig {
    let mut rig = world(
        3,
        &format!(
            "const desk = new Workspace('desk', {{ layout: new ArcLayout({options}) }});\n\
             gnoblin.workspaces.add(desk);\n"
        ),
    );
    rig.comp.external_set(WIN, "focused", MirrorValue::Bool(true));
    rig.settle();
    rig
}

#[test]
fn an_arc_layout_puts_each_window_on_the_arc_and_a_flat_workspace_shows_depth_turn_and_offset() {
    let mut rig = arc_world("{ radius: 1000, spacing: 0.5 }");
    // The focused window is the front one: angle 0. The others are one and two steps round.
    let z: Vec<f64> = (0..3).map(|n| rig.comp.number(WIN + n, "z")).collect();
    all_close(
        &z,
        &[-1000.0, -(0.5_f64).cos() * 1000.0, -(1.0_f64).cos() * 1000.0],
        "z is -cos(angle) * radius",
    );
    all_close(
        &[turn(&rig, WIN), turn(&rig, WIN + 1), turn(&rig, WIN + 2)],
        &[0.0, -0.5, -1.0],
        "the turn about Y faces the centre",
    );
    let xs: Vec<f64> = (0..3).map(|n| rig.comp.number(WIN + n, "x")).collect();
    close(xs[0], 100.0, "the front window is centred in a 1000 wide area");
    close(
        xs[1] - xs[0],
        (0.5_f64.sin() * 1000.0).round(),
        "an offset of sin(angle) * radius across",
    );
    close(xs[2] - xs[0], (1.0_f64.sin() * 1000.0).round(), "and two steps further");
    // Focus moves the front of the arc.
    rig.comp.external_set(WIN, "focused", MirrorValue::Bool(false));
    rig.comp.external_set(WIN + 2, "focused", MirrorValue::Bool(true));
    rig.settle();
    close(
        rig.comp.number(WIN + 2, "z"),
        -1000.0,
        "the newly focused window is at the front",
    );
    close(
        rig.comp.number(WIN, "z"),
        -(1.0_f64).cos() * 1000.0,
        "the first is two steps away",
    );
    close(turn(&rig, WIN), 1.0, "on the other side");
}

#[test]
fn an_arc_layouts_fields_are_plain_and_a_change_recomputes_it() {
    let mut rig = arc_world("{ radius: 1000, spacing: 0.5 }");
    rig.eval("gnoblin.workspaces.byId('desk').layout.radius = 500; 1");
    rig.settle();
    close(
        rig.comp.number(WIN + 1, "z"),
        -(0.5_f64).cos() * 500.0,
        "the new radius is laid out",
    );
    // Refused in the constructor, in the words it always had.
    let message = thrown(&mut rig, "new ArcLayout({ radius: -1 })");
    assert!(message.contains("radius must be a finite number above 0"), "{message}");
    let message = thrown(&mut rig, "new ArcLayout({ spacing: NaN })");
    assert!(message.contains("spacing must be a finite number"), "{message}");
}

#[test]
fn a_placement_with_bounds_and_flat_depth_turn_scale_and_offset_fields_offsets_the_bounds() {
    let mut rig = world(
        2,
        "class Offset extends Layout {\n\
           compute({ children }: any) {\n\
             return [\n\
               { node: children[0], bounds: { x: 100, y: 50, width: 300, height: 200 },\n\
                 offsetX: 10, offsetY: 20, z: -5, rotation: [0, 0.25, 0], scale: [2, 2, 2] },\n\
               { node: children[1], bounds: { x: 0, y: 0, width: 100, height: 100 },\n\
                 z: -9, scale: [1, 2, 3] },\n\
             ];\n\
           }\n\
         }\n\
         gnoblin.workspaces.add(new Workspace('o', { layout: new Offset() }));\n",
    );
    rig.settle();
    assert_eq!(
        rig.comp.rect(WIN),
        "110,70,300,200",
        "bounds moved by the offset's x and y"
    );
    close(rig.comp.number(WIN, "z"), -5.0, "z is the placement's");
    all_close(&list(&rig, WIN, "rotation"), &[0.0, 0.25, 0.0], "rotation");
    all_close(&list(&rig, WIN, "scale"), &[2.0, 2.0, 2.0], "a uniform scale");
    close(rig.comp.number(WIN + 1, "z"), -9.0, "a flat z");
    all_close(&list(&rig, WIN + 1, "scale"), &[1.0, 2.0, 3.0], "a per-axis scale");
}

#[test]
#[ignore = "vrnw: needs LayoutPlacement.transform (a position, rotation and scale record the flat fields win over)"]
fn a_placement_with_a_transform_offsets_the_bounds_and_the_flat_fields_win() {
    let mut rig = world(
        2,
        "class Offset extends Layout {\n\
           compute({ children }: any) {\n\
             return [\n\
               { node: children[0], bounds: { x: 100, y: 50, width: 300, height: 200 },\n\
                 transform: { position: { x: 10, y: 20, z: -5 }, rotation: [0, 0.25, 0], scale: 2 } },\n\
               { node: children[1], bounds: { x: 0, y: 0, width: 100, height: 100 },\n\
                 z: -9, transform: { position: { x: 0, y: 0, z: -1 }, scale: { x: 1, y: 2, z: 3 } } },\n\
             ];\n\
           }\n\
         }\n\
         gnoblin.workspaces.add(new Workspace('o', { layout: new Offset() }));\n",
    );
    rig.settle();
    assert_eq!(rig.comp.rect(WIN), "110,70,300,200");
    close(
        rig.comp.number(WIN + 1, "z"),
        -9.0,
        "a flat z wins over the transform's",
    );
}

#[test]
#[ignore = "vrnw: needs the notice 'placement <n> rotation ...' for a placement a layout got wrong in the shell's diagnostics"]
fn a_placement_with_a_bad_rotation_is_refused_with_the_placement_named() {
    let (_rig, diagnostics) = boot(
        1,
        "class Bad extends Layout {\n\
           compute({ children }: any) { return [{ node: children[0], rotation: [1, 2] }]; }\n\
         }\n\
         gnoblin.workspaces.add(new Workspace('b', { layout: new Bad() }));\n",
    );
    let diagnostics = diagnostics.join("\n");
    assert!(diagnostics.contains("placement 0 rotation"), "{diagnostics}");
}

// ---- Property bindings -----------------------------------------------------------------------------------------------

#[test]
fn a_bound_property_is_set_now_and_runs_again_only_when_what_it_read_changes() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.runs = 0; globalThis.c = gnoblin.clients.at(0); \
         globalThis.binding = c.properties.bindDouble('opacity', () => { runs++; return Math.max(0.25, 1 - c.x / 1000); }); 1",
    );
    rig.settle();
    close(number(&mut rig, "runs"), 1.0, "it runs once as it is made");
    close(number(&mut rig, "c.opacity"), 1.0, "and has set the property");
    rig.settle();
    close(
        number(&mut rig, "runs"),
        1.0,
        "no polling: frames pass and it does not run",
    );
    // The compositor moves the window: the binding reads x, so it runs.
    rig.comp.external_set(WIN, "x", double(200.0));
    rig.settle();
    close(number(&mut rig, "runs"), 2.0, "one run for the change");
    close(rig.comp.number(WIN, "opacity"), 0.8, "the tree has the new opacity");
    // Something it did not read changes: it does not run.
    rig.comp.external_set(WIN, "title", text("renamed"));
    rig.comp.external_set(WIN, "y", double(50.0));
    rig.settle();
    close(number(&mut rig, "runs"), 2.0, "an unrelated property is not a reason");
    // A script's own write of what it reads is one.
    rig.eval("c.x = 400; 1");
    rig.settle();
    close(number(&mut rig, "runs"), 3.0, "a script's write counts");
    close(rig.comp.number(WIN, "opacity"), 0.6, "opacity follows");
    // Ending it keeps the last value and stops the runs.
    rig.eval("binding.dispose(); 1");
    rig.comp.external_set(WIN, "x", double(900.0));
    rig.settle();
    close(number(&mut rig, "runs"), 3.0, "a disposed binding does not run");
    assert_eq!(rig.eval("String(binding.alive)"), "false");
    close(rig.comp.number(WIN, "opacity"), 0.6, "the last value stays");
}

#[test]
#[ignore = "vrnw: needs Workspace.properties (property bindings on a workspace)"]
fn a_binding_that_reads_a_collection_follows_its_membership() {
    let mut rig = world(
        2,
        "const development = new Workspace('development', { layout: new FloatingLayout() });\n\
         gnoblin.workspaces.add(development);\n",
    );
    rig.eval(
        "globalThis.ws = gnoblin.workspaces.byId('development'); \
         ws.properties.bindText('name', () => (ws.clients.length > 2 ? 'busy' : 'quiet')); 1",
    );
    assert_eq!(rig.eval("ws.name"), "quiet");
    rig.comp.create_window(30, "third", "third");
    rig.settle();
    assert_eq!(rig.eval("ws.name"), "busy", "a window joined and the binding ran");
    rig.comp.destroy(30);
    rig.settle();
    assert_eq!(rig.eval("ws.name"), "quiet", "and again when it left");
}

#[test]
fn a_binding_on_a_property_that_does_not_exist_is_reported_once_and_the_others_still_run() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.c = gnoblin.clients.at(0); globalThis.background = new SceneNode({ id: 'background' }); \
         globalThis.camera2D = { zoom: 2 }; \
         background.properties.bindNumber('vignette', () => Math.min(1, camera2D.zoom / 3)); \
         c.properties.bindDouble('opacity', () => { if (c.x > 100) throw new Error('too far'); return 0.5 + c.x / 1000; }); 1",
    );
    rig.settle();
    close(rig.comp.number(WIN, "opacity"), 0.5, "the other binding has run");
    rig.comp.external_set(WIN, "x", double(300.0));
    rig.settle();
    // It keeps following its reads and recovers.
    rig.comp.external_set(WIN, "x", double(10.0));
    rig.settle();
    close(rig.comp.number(WIN, "opacity"), 0.51, "it ran again and set the value");
}

#[test]
#[ignore = "vrnw: needs the notice of a binding that throws (the message, once) in the shell's diagnostics"]
fn a_throwing_binding_is_reported_once() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.c = gnoblin.clients.at(0); \
         c.properties.bindDouble('opacity', () => { if (c.x > 100) throw new Error('too far'); return 0.5 + c.x / 1000; }); 1",
    );
    rig.settle();
    rig.comp.external_set(WIN, "x", double(300.0));
    let diagnostics = rig.settle().join("\n");
    assert!(
        diagnostics.contains("too far"),
        "a throwing binding is reported: {diagnostics}"
    );
    // The same failure is not reported again when the binding runs and fails the same way.
    rig.comp.external_set(WIN, "x", double(400.0));
    let again = rig.settle().join("\n");
    assert!(!again.contains("too far"), "reported once: {again}");
}

#[test]
#[ignore = "vrnw: needs the notice '<key> is not a writable property' for a binding on a property a node has not"]
fn a_binding_on_a_property_that_does_not_exist_is_reported() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.background = new SceneNode({ id: 'background' }); \
         background.properties.bindNumber('vignette', () => 1); 1",
    );
    let diagnostics = rig.settle().join("\n");
    assert!(
        diagnostics.contains("vignette is not a writable property"),
        "the missing property is reported: {diagnostics}"
    );
}

#[test]
#[ignore = "vrnw: needs the notice 'still changing after 8 passes' naming what the bindings read in the shell's diagnostics"]
fn bindings_that_feed_each_other_stop_after_a_bounded_number_of_passes_and_name_what_they_read() {
    let mut rig = world(1, "");
    rig.eval(
        "globalThis.a = new SceneNode({ id: 'a' }); globalThis.b = new SceneNode({ id: 'b' }); \
         a.properties.bindNumber('x', () => b.x + 1); b.properties.bindNumber('x', () => a.x + 1); 1",
    );
    let diagnostics = rig.settle().join("\n");
    assert!(diagnostics.contains("still changing after 8 passes"), "{diagnostics}");
    assert!(
        diagnostics.contains(".x"),
        "the report names what it reads: {diagnostics}"
    );
}

#[test]
fn a_binding_ends_when_its_owner_is_destroyed_and_a_second_binding_of_a_property_replaces_the_first() {
    let mut rig = world(2, "");
    rig.eval(
        "globalThis.runs = 0; globalThis.n = new SceneNode({ id: 'n' }); \
         globalThis.first = n.properties.bindNumber('x', () => { runs++; return gnoblin.clients.at(0).x; }); \
         globalThis.second = n.properties.bindNumber('x', () => 7); 1",
    );
    rig.settle();
    assert_eq!(rig.eval("String(first.alive)"), "false", "replaced");
    assert_eq!(rig.eval("String(second.alive)"), "true");
    rig.eval(
        "globalThis.c2 = gnoblin.clients.at(1); globalThis.bound = c2.properties.bindDouble('opacity', () => 0.4); 1",
    );
    rig.comp.destroy(WIN + 1);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("String(bound.alive)"), "false", "the owner is gone");
    rig.eval("n.destroy(); 1");
    assert_eq!(
        rig.eval("String(second.alive)"),
        "false",
        "a destroyed node ends its bindings"
    );
    let message = thrown(&mut rig, "n.properties.bindNumber('x', () => 1)");
    assert!(message.contains("the object is gone"), "{message}");
}

#[test]
#[ignore = "vrnw: needs SceneNode.properties.bindNumber('x', ...) to write the value (the tree keeps 0)"]
fn a_node_binding_writes_its_value() {
    let mut rig = world(1, "");
    rig.eval("globalThis.n = new SceneNode({ id: 'n' }); n.properties.bindNumber('x', () => 7); 1");
    rig.settle();
    close(number(&mut rig, "n.x"), 7.0, "the binding's value");
}
