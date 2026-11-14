//! XR on the JS shell (`data/js/gnoblin/{scene,xr}.ts`, `design/js-xr.md`) over the fake compositor of `tests/common`.
//!
//! The XR program is out of process, so what the shell sees of it is nodes: the seat named `xr`, the camera named
//! `head`, and `controller` nodes. These tests make those nodes the way the program does (unparented, while the shell
//! runs) and check what the shell builds and writes. The cases of the retired shell's `retired corpus xr` are here with the same
//! golden numbers; the controller cases are new. No headset, OpenXR runtime or `gnoblin-xr` binary is involved: the
//! real program is tested in its own workspace (`extensions/gnoblin-xr`).

use std::fs;
use std::path::Path;

use gnoblin_js::JsShell;
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

mod common;
use common::*;

const SEAT: u64 = 21;
const XR_SEAT: u64 = 50;
const RIGHT_HAND: u64 = 60;
const LEFT_HAND: u64 = 61;
/// The first window's id: windows are 3, 4, 5, ...
const WIN: u64 = 3;

const IMPORTS: &str = "import * as g from \"gnoblin\";\nimport * as xrm from \"gnoblin/xr\";\nObject.assign(globalThis, { g, ...xrm });\n";

fn near(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

fn double(value: f64) -> MirrorValue {
    MirrorValue::Double(value)
}

/// A shell over a screen, a seat, a lock and one window per `(width, height)` (ids 3 on), whose `init.ts` is `body`.
fn world(sizes: &[(f64, f64)], body: &str) -> Rig {
    let sizes = sizes.to_vec();
    let mut rig = rig_setup(IMPORTS, body, move |comp| {
        comp.add_node(
            20,
            ROOT,
            "screen",
            vec![
                ("width", double(1000.0)),
                ("height", double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(22, ROOT, "lock", Vec::new());
        for (n, (width, height)) in sizes.iter().enumerate() {
            comp.add_node(
                WIN + n as u64,
                ROOT,
                "window",
                vec![
                    ("title", text(&format!("win{}", n + 1))),
                    ("app_id", text(&format!("app{}", n + 1))),
                    ("width", double(*width)),
                    ("height", double(*height)),
                ],
            );
        }
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// The program's session starts: it makes the seat named `xr`.
fn xr_arrives(rig: &mut Rig) {
    rig.comp.appear(XR_SEAT, 0, "seat", vec![("name", text("xr"))]);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// A tracked controller at `(x, y, z)` millimetres, facing down negative z.
fn hand(rig: &Rig, id: u64, name: &str, x: f64, y: f64, z: f64) {
    rig.comp.appear(
        id,
        0,
        "controller",
        vec![
            ("hand", text(name)),
            ("tracked", MirrorValue::Bool(true)),
            ("x", double(x)),
            ("y", double(y)),
            ("z", double(z)),
            ("rotation", MirrorValue::List(vec![0.0, 0.0, 0.0])),
            ("trigger", double(0.0)),
            ("grip", double(0.0)),
        ],
    );
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

fn flag(rig: &mut Rig, expression: &str) -> bool {
    rig.eval(&format!("String({expression})")) == "true"
}

fn room(rig: &Rig) -> u64 {
    rig.comp.named("room").expect("the room is in the tree")
}

/// The View that shows `window`.
fn view_of(rig: &Rig, window: u64) -> Option<u64> {
    rig.comp
        .ids_of_class("view")
        .into_iter()
        .find(|view| matches!(rig.comp.value(*view, "source"), Some(MirrorValue::Node(source)) if source == window))
}

/// The windows the room's Views show, in tree order: the order the program submits them.
fn shown(rig: &Rig) -> Vec<u64> {
    rig.comp
        .children(room(rig))
        .into_iter()
        .filter_map(|child| match rig.comp.value(child, "source") {
            Some(MirrorValue::Node(source)) => Some(source),
            _ => None,
        })
        .collect()
}

fn pose(rig: &Rig, view: u64) -> [f64; 3] {
    [
        rig.comp.number(view, "x"),
        rig.comp.number(view, "y"),
        rig.comp.number(view, "z"),
    ]
}

fn yaw(rig: &Rig, view: u64) -> f64 {
    match rig.comp.value(view, "rotation") {
        Some(MirrorValue::List(angles)) => angles[1],
        _ => 0.0,
    }
}

fn scale(rig: &Rig, view: u64) -> f64 {
    match rig.comp.value(view, "scale") {
        Some(MirrorValue::List(factors)) => factors[0],
        _ => 1.0,
    }
}

fn set_focus(rig: &mut Rig, window: u64) {
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(window));
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

// ---- Absent: the desktop does not know XR exists -----------------------------------------------------------------

#[test]
fn with_no_xr_program_the_extension_is_null_and_the_desktop_works_untouched() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)], "");
    assert_eq!(rig.eval("String(gnoblin.extensions.xr)"), "null");
    assert_eq!(rig.eval("String(gnoblin.extensions.xr?.session)"), "undefined");
    assert_eq!(rig.eval("String(gnoblin.clients.length)"), "2");
    rig.settle();
    for class in ["view", "camera"] {
        assert!(rig.comp.ids_of_class(class).is_empty(), "no {class} was made");
    }
    let made: Vec<String> = rig
        .comp
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            TxOp::New { class, .. } => Some(class),
            _ => None,
        })
        .collect();
    assert!(
        !made.iter().any(|class| class == "view" || class == "camera"),
        "the shell made nothing for XR: {made:?}"
    );
    assert!(rig.comp.named("room").is_none(), "no room");
}

/// A shell over a fresh fake compositor with `setup` run first, whose `init.ts` is `body`; the entry's load result.
fn load(declared: bool, body: &str, setup: impl FnOnce(&FakeCompositor)) -> Result<Vec<String>, String> {
    let comp = FakeCompositor::new();
    setup(&comp);
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), body).expect("init.ts is written");
    let mut config = honest_config(&root);
    config.xr_declared = declared;
    let mut shell = JsShell::new(Box::new(comp), config).expect("the shell boots");
    shell
        .load_entry(Path::new("init.ts"))
        .map_err(|error| error.to_string())
}

const STATIC_IMPORT: &str = "import { XRCamera3D, ViewMoveBinding } from \"gnoblin/extensions/xr\";\nconst made = [XRCamera3D, ViewMoveBinding];\n";

#[test]
fn a_static_import_of_the_extension_module_fails_clearly_when_it_is_absent() {
    let error = load(false, STATIC_IMPORT, |_| {}).expect_err("the import fails");
    for needle in [
        "gnoblin/extensions/xr",
        "not available",
        "GNOBLIN_XR=1",
        "gnoblin.extensions.xr",
    ] {
        assert!(error.contains(needle), "the message names `{needle}`: {error}");
    }
}

#[test]
fn the_extension_module_imports_when_the_session_declared_xr_or_the_program_already_has_a_session() {
    load(true, STATIC_IMPORT, |_| {}).expect("a declared session may import it before the headset starts");
    load(false, STATIC_IMPORT, |comp| {
        comp.add_node(XR_SEAT, 0, "seat", vec![("name", text("xr"))]);
    })
    .expect("a program that has a session is there to import");
    load(false, STATIC_IMPORT, |comp| {
        comp.add_node(XR_SEAT, 0, "seat", vec![("name", text("other"))]);
    })
    .expect_err("a seat with another name is no XR program");
}

// ---- XR1: the class relations ------------------------------------------------------------------------------------

#[test]
fn xr1_the_classes_relate_as_the_spec_says() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    for (expression, want) in [
        ("g.Camera3D === Camera3D && g.View === View && g.Node === Node", true),
        ("new XRCamera3D() instanceof Camera3D", true),
        ("new XRCamera3D() instanceof XRCamera3D", true),
        ("gnoblin.extensions.xr.Camera3D === XRCamera3D", true),
        ("gnoblin.extensions.xr.Viewport === XRViewport", true),
        ("gnoblin.extensions.xr.ViewMoveBinding === ViewMoveBinding", true),
        ("gnoblin.extensions.xr.ViewResizeBinding === ViewResizeBinding", true),
        ("new ViewMoveBinding() instanceof XRControllerDragBinding", true),
        ("new ViewMoveBinding() instanceof g.Binding", true),
        ("new ViewResizeBinding() instanceof g.Binding", true),
        ("new View() instanceof Node", true),
        ("new XRRoom() instanceof Node", true),
        ("gnoblin.extensions.xr.session instanceof XRSession", true),
        ("gnoblin.extensions.xr instanceof XRExtension", true),
        ("gnoblin.extensions.xr.sessions.size === 1", true),
        ("gnoblin.extensions.xr.session.viewport instanceof XRViewport", true),
        (
            "gnoblin.extensions.xr.session.viewport.content === gnoblin.extensions.xr.room",
            true,
        ),
        (
            "gnoblin.extensions.xr.session.viewport.camera === gnoblin.extensions.xr.room.head",
            true,
        ),
        ("gnoblin.extensions.xr.room.head instanceof XRCamera3D", true),
        ("gnoblin.extensions.xr.room.head instanceof Camera3D", true),
        // The session's input is the `xr` seat's ordinary input context.
        (
            "gnoblin.extensions.xr.session.input === gnoblin.input.contexts.find((c) => c.name === 'xr')",
            true,
        ),
        ("gnoblin.extensions.xr.session.input !== gnoblin.input.primary", true),
        // The bindings are `Binding`s of the room layout's collection, so the ordinary collection API applies.
        (
            "gnoblin.extensions.xr.room.layout.bindings instanceof g.BindingCollection",
            true,
        ),
        (
            "gnoblin.extensions.xr.room.layout.bindings.get('move') instanceof ViewMoveBinding",
            true,
        ),
        (
            "gnoblin.extensions.xr.room.layout.bindings.get('resize') instanceof ViewResizeBinding",
            true,
        ),
    ] {
        assert_eq!(flag(&mut rig, expression), want, "{expression}");
    }
}

#[test]
fn a_xr_camera_is_the_stereo_head_camera_and_converts_metres_to_the_cameras_units() {
    let mut rig = world(&[], "");
    xr_arrives(&mut rig);
    rig.eval("globalThis.cam = new XRCamera3D({ near: 0.05, far: 100 }); 1");
    rig.settle();
    assert_eq!(rig.eval("cam.name"), "head");
    assert_eq!(rig.eval("cam.projection"), "stereo");
    near(number(&mut rig, "cam.metres"), 0.001, "metres");
    near(number(&mut rig, "cam.near"), 50.0, "near is 0.05 m in millimetres");
    near(number(&mut rig, "cam.far"), 100_000.0, "far is 100 m in millimetres");
    assert_eq!(rig.eval("cam.referenceSpace"), "local");
    let error = rig.eval("try { new XRCamera3D({ referenceSpace: 'stage' }); 'no error' } catch (e) { e.message }");
    assert!(error.contains("stage") && error.contains("local"), "{error}");
}

// ---- The room: the retired shell's cases -----------------------------------------------------------------------------

#[test]
fn a_seat_with_another_name_builds_no_room() {
    let mut rig = world(&[(800.0, 600.0)], "");
    rig.comp.appear(XR_SEAT, 0, "seat", vec![("name", text("seat1"))]);
    rig.settle();
    assert!(rig.comp.named("room").is_none());
    assert_eq!(rig.eval("String(gnoblin.extensions.xr)"), "null");
}

#[test]
fn the_seat_named_xr_builds_the_room_under_root_a_stereo_head_camera_in_millimetres_and_a_view_per_window() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)], "");
    assert!(rig.comp.named("room").is_none(), "no room before the seat");
    xr_arrives(&mut rig);
    let room = room(&rig);
    assert_eq!(rig.comp.parent_of(room), Some(ROOT), "the room is a child of root");
    near(
        rig.comp.number(room, "y"),
        -1400.0,
        "the floor is an eye height below the viewer",
    );
    let head = rig.comp.named("head").expect("a camera named head");
    assert_eq!(rig.comp.children(room)[0], head, "the head camera comes first");
    assert_eq!(rig.comp.value(head, "projection"), Some(text("stereo")));
    // 0.001 is the schema's default, so it is not written and the program reads the default.
    assert!(matches!(
        rig.comp.value(head, "metres"),
        None | Some(MirrorValue::Double(0.001))
    ));
    near(
        number(&mut rig, "gnoblin.extensions.xr.room.head.metres"),
        0.001,
        "the room is in millimetres",
    );
    let views: Vec<u64> = rig.comp.ids_of_class("view");
    assert_eq!(views.len(), 2, "one View per window");
    for window in [WIN, WIN + 1] {
        let view = view_of(&rig, window).expect("a View for the window");
        near(rig.comp.number(view, "width"), 800.0, "view width");
        near(rig.comp.number(view, "height"), 600.0, "view height");
        assert_ne!(rig.comp.flag(view, "visible"), Some(false), "visible");
        assert_ne!(rig.comp.flag(view, "input"), Some(false), "takes input");
        assert_eq!(rig.comp.parent_of(view), Some(room));
        assert_eq!(
            rig.comp.parent_of(window),
            Some(ROOT),
            "the window itself is where it was"
        );
    }
    assert!(flag(&mut rig, "gnoblin.extensions.xr.session.alive"));
    assert!(flag(&mut rig, "gnoblin.extensions.xr.session.connected"));
}

#[test]
fn a_seat_in_the_boot_snapshot_builds_the_room_on_the_first_frames() {
    let mut rig = rig_setup(IMPORTS, "", |comp| {
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(XR_SEAT, 0, "seat", vec![("name", text("xr"))]);
        comp.add_window(WIN, "one", "app");
    });
    rig.settle();
    assert!(rig.comp.named("room").is_some());
    assert!(view_of(&rig, WIN).is_some());
    assert!(flag(&mut rig, "gnoblin.extensions.xr !== null"));
}

#[test]
fn a_room_already_in_the_tree_is_the_users_and_is_left_alone() {
    let mut rig = rig_setup(IMPORTS, "", |comp| {
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(9, ROOT, "node", vec![("name", text("room"))]);
        comp.add_window(WIN, "one", "app");
    });
    rig.settle();
    xr_arrives(&mut rig);
    assert_eq!(rig.comp.ids_of_class("view").len(), 0, "no View is made");
    assert!(
        rig.comp.children(9).is_empty(),
        "the existing room gets no head camera or Views"
    );
    assert!(flag(&mut rig, "gnoblin.extensions.xr.room === null"));
    assert!(
        flag(&mut rig, "gnoblin.extensions.xr.session.alive"),
        "the session itself is still there"
    );
}

#[test]
fn configure_room_false_builds_no_room_of_the_shells() {
    let mut rig = world(
        &[(800.0, 600.0)],
        "gnoblin.extensions.configure('xr', { room: false });",
    );
    xr_arrives(&mut rig);
    assert!(rig.comp.named("room").is_none());
    assert!(flag(&mut rig, "gnoblin.extensions.xr.session.alive"));
    let error = rig.eval("try { gnoblin.extensions.configure('xr', { rooom: false }); 'ok' } catch (e) { e.message }");
    assert!(error.contains("rooom") && error.contains("room"), "{error}");
}

#[test]
fn destroying_the_seat_ends_the_session_takes_the_room_down_and_the_windows_stay() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)], "");
    xr_arrives(&mut rig);
    rig.eval("globalThis.held = gnoblin.extensions.xr.session; globalThis.heldRoom = gnoblin.extensions.xr.room; 1");
    assert!(rig.comp.named("room").is_some());
    rig.comp.destroy(XR_SEAT);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(rig.comp.named("room").is_none(), "the room is gone");
    assert!(rig.comp.ids_of_class("view").is_empty() && rig.comp.ids_of_class("camera").is_empty());
    assert_eq!(
        rig.eval("String(gnoblin.extensions.xr)"),
        "null",
        "the extension is absent again"
    );
    // No half-alive session: the object a script held says so and refuses writes.
    assert!(!flag(&mut rig, "held.alive"));
    assert!(!flag(&mut rig, "held.connected"));
    assert!(!flag(&mut rig, "held.controllers.size > 0"));
    assert!(!flag(&mut rig, "heldRoom.alive"));
    assert_eq!(rig.eval("String(held.input)"), "null");
    let error = rig.eval("try { held.viewport = new XRViewport({ content: new XRRoom(), camera: new Camera3D() }); 'wrote' } catch (e) { e.message }");
    assert!(error.contains("ended"), "{error}");
    let error = rig.eval("try { heldRoom.x = 1; 'wrote' } catch (e) { e.message }");
    assert!(error.contains("destroyed"), "{error}");
    for window in [WIN, WIN + 1] {
        assert!(rig.comp.exists(window), "the window is still there");
        assert_eq!(rig.comp.parent_of(window), Some(ROOT));
    }
    // A new seat is a new session with a new room.
    xr_arrives(&mut rig);
    assert!(rig.comp.named("room").is_some());
    assert_eq!(rig.comp.ids_of_class("view").len(), 2);
    assert!(flag(
        &mut rig,
        "gnoblin.extensions.xr.session !== held && gnoblin.extensions.xr.session.alive"
    ));
}

#[test]
fn a_listener_hears_the_session_begin_and_end() {
    let mut rig = world(
        &[(800.0, 600.0)],
        "globalThis.heard = []; gnoblin.extensions.addListener({ onSessionStarted: (s) => heard.push('start:' + s.alive), onSessionEnded: (s) => heard.push('end:' + s.alive) });",
    );
    xr_arrives(&mut rig);
    rig.comp.destroy(XR_SEAT);
    rig.settle();
    assert_eq!(rig.eval("heard.join(',')"), "start:true,end:false");
}

#[test]
fn a_window_that_opens_gets_a_view_follows_its_size_and_loses_it_when_destroyed() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    rig.comp.create_window(WIN + 1, "late", "late");
    rig.settle();
    let late = view_of(&rig, WIN + 1).expect("the window that opened has a View");
    near(rig.comp.number(late, "width"), 400.0, "width");
    near(rig.comp.number(late, "height"), 300.0, "height");
    rig.comp.external_set(WIN + 1, "width", double(1200.0));
    rig.comp.external_set(WIN + 1, "height", double(700.0));
    rig.settle();
    near(
        rig.comp.number(late, "width"),
        1200.0,
        "the View follows the window's width",
    );
    near(rig.comp.number(late, "height"), 700.0, "and its height");
    near(
        rig.comp.number(view_of(&rig, WIN).expect("view"), "width"),
        800.0,
        "the other View is unchanged",
    );
    rig.comp.destroy(WIN);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(
        view_of(&rig, WIN).is_none(),
        "the compositor's destroy removes the View"
    );
    assert_eq!(rig.comp.ids_of_class("view").len(), 1);
}

#[test]
fn a_minimised_window_hides_its_view_lists_it_first_and_leaves_the_ring_until_shown_again() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0), (800.0, 600.0)], "");
    xr_arrives(&mut rig);
    set_focus(&mut rig, WIN);
    let third = view_of(&rig, WIN + 2).expect("view");
    let before = rig.comp.number(third, "x");
    rig.comp.external_set(WIN + 1, "minimised", MirrorValue::Bool(true));
    rig.settle();
    let hidden = view_of(&rig, WIN + 1).expect("the View stays");
    assert_eq!(rig.comp.flag(hidden, "visible"), Some(false), "and is hidden");
    assert_eq!(shown(&rig)[0], WIN + 1, "a hidden View lists first: it draws nothing");
    assert!(
        (rig.comp.number(third, "x") - before).abs() > 1.0,
        "the other panels close the gap"
    );
    rig.comp.external_set(WIN + 1, "minimised", MirrorValue::Bool(false));
    rig.settle();
    assert_eq!(rig.comp.flag(hidden, "visible"), Some(true));
    near(rig.comp.number(third, "x"), before, "and the ring is as it was");
}

#[test]
fn a_dialog_has_no_view_of_its_own() {
    let mut rig = rig_setup(IMPORTS, "", |comp| {
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_window(WIN, "parent", "app");
        // The compositor puts a dialog's node under its parent window.
        comp.add_node(
            WIN + 1,
            WIN,
            "window",
            vec![("width", double(300.0)), ("height", double(200.0))],
        );
    });
    rig.settle();
    xr_arrives(&mut rig);
    assert!(view_of(&rig, WIN).is_some());
    assert!(view_of(&rig, WIN + 1).is_none(), "the parent's View shows the dialog");
    assert_eq!(rig.comp.ids_of_class("view").len(), 1);
}

#[test]
fn the_accept_policy_keeps_a_window_out_of_the_room_and_a_change_of_mind_brings_it_in() {
    let mut rig = world(
        &[(800.0, 600.0), (800.0, 600.0)],
        "globalThis.keepOut = new Set(['app2']); gnoblin.extensions.configure('xr', { accept: (c) => !keepOut.has(c.appId) });",
    );
    xr_arrives(&mut rig);
    assert!(view_of(&rig, WIN).is_some());
    assert!(view_of(&rig, WIN + 1).is_none(), "the policy leaves the window out");
    rig.eval("keepOut.clear(); 1");
    rig.settle();
    assert!(view_of(&rig, WIN + 1).is_some(), "it comes in when the policy lets it");
}

// ---- XR6: a secure client never appears in a View ----------------------------------------------------------------

#[test]
fn xr6_a_secure_client_never_gets_a_view_and_one_that_turns_secure_loses_it() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)], "");
    rig.comp.external_set(WIN + 1, "secure", MirrorValue::Bool(true));
    rig.settle();
    xr_arrives(&mut rig);
    assert!(view_of(&rig, WIN).is_some());
    assert!(view_of(&rig, WIN + 1).is_none(), "the secure window has no View");
    rig.comp.external_set(WIN, "secure", MirrorValue::Bool(true));
    rig.settle();
    assert!(
        view_of(&rig, WIN).is_none(),
        "a window that turns secure loses its View"
    );
    rig.comp.external_set(WIN, "secure", MirrorValue::Bool(false));
    rig.settle();
    assert!(view_of(&rig, WIN).is_some(), "and gets one back when it stops");
    // A head-locked panel follows the same rule.
    rig.eval("globalThis.panel = gnoblin.extensions.xr.room.pinToHead(gnoblin.clients.at(0)); 1");
    rig.settle();
    assert!(rig.comp.ids_of_class("view").len() >= 2);
    rig.comp.external_set(WIN, "secure", MirrorValue::Bool(true));
    rig.settle();
    assert!(
        !flag(&mut rig, "panel.alive"),
        "the head-locked panel of a window that turned secure is gone"
    );
}

// ---- The ring, with the retired shell's golden numbers ----------------------------------------------------------------

fn ring(rig: &mut Rig, count: usize, width: f64, focus: &str) -> Vec<f64> {
    numbers(
        rig,
        &format!(
            "(() => {{ const r = arrangeRing(Array.from({{ length: {count} }}, (_, i) => ({{ id: i + 1, width: {width}, height: 600 }})), {focus}); \
             return r.poses.flatMap((p) => [p.x, p.y, p.z, p.rotation[1], p.scale, p.focused ? 1 : 0, p.behind]).concat(r.order); }})()"
        ),
    )
}

#[test]
fn the_ring_puts_one_panel_ahead_and_a_focused_one_larger_and_closer() {
    let mut rig = world(&[], "");
    let plain = ring(&mut rig, 1, 800.0, "null");
    for (got, want) in plain.iter().zip([0.0, 1400.0, -1600.0, 0.0, 0.75, 0.0, 0.0, 0.0]) {
        near(*got, want, "unfocused");
    }
    let focused = ring(&mut rig, 1, 800.0, "1");
    for (got, want) in focused.iter().zip([0.0, 1400.0, -1500.0, 0.0, 0.8625, 1.0, 0.0, 0.0]) {
        near(*got, want, "focused");
    }
    assert_eq!(
        rig.eval("String(arrangeRing([], null).order.length)"),
        "0",
        "an empty list has an empty order"
    );
}

#[test]
fn three_panels_form_a_flat_arc_with_the_focused_one_at_the_front() {
    let mut rig = world(&[], "");
    let got = ring(&mut rig, 3, 800.0, "2");
    let side = (703.8364939904852, -1436.8765394866678, 0.45548487755982986);
    let want = [
        -side.0, 1400.0, side.1, side.2, 0.75, 0.0, 0.0, // left
        0.0, 1400.0, -1500.0, 0.0, 0.8625, 1.0, 0.0, // front
        side.0, 1400.0, side.1, -side.2, 0.75, 0.0, 0.0, // right
        0.0, 2.0, 1.0, // the focused panel draws last
    ];
    for (at, (got, want)) in got.iter().zip(want).enumerate() {
        near(*got, want, &format!("entry {at}"));
    }
    // The ring is a carousel: focus 1 puts its neighbours on either side, wrapping round.
    let carousel = ring(&mut rig, 3, 800.0, "1");
    near(carousel[0], 0.0, "front x");
    near(carousel[7], side.0, "the next panel is on the right");
    near(carousel[14], -side.0, "and the one before wraps to the left");
    assert_eq!(&carousel[21..], &[1.0, 2.0, 0.0], "order");
}

#[test]
fn a_focus_that_names_no_view_puts_the_first_view_at_the_front_unenlarged() {
    let mut rig = world(&[], "");
    let got = ring(&mut rig, 3, 800.0, "'nobody'");
    near(got[0], 0.0, "x");
    near(got[2], -1600.0, "z");
    near(got[4], 0.75, "scale");
}

#[test]
fn twelve_wide_panels_shrink_together_to_fit_150_degrees_to_either_side() {
    let mut rig = world(&[], "");
    let got = ring(&mut rig, 12, 1920.0, "1");
    let scale = 0.291994247481941;
    near(got[4], 0.3357933846042322, "the front panel's scale");
    near(got[7], 665.5858301153864, "first right x");
    near(got[11], scale, "scale");
    near(got[42], 1029.27333286744, "last right x");
    near(got[77], -665.5858301153864, "first left x");
    for i in 1..12 {
        let (x, z, yaw) = (got[i * 7], got[i * 7 + 2], got[i * 7 + 3]);
        near((x * x + z * z).sqrt(), 1600.0, &format!("radius of panel {i}"));
        assert!(
            yaw.abs() <= 150f64.to_radians() + 1e-9,
            "panel {i} is within 150 degrees"
        );
    }
    let order: Vec<f64> = got[84..].to_vec();
    assert_eq!(order, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 0.0]);
}

#[test]
fn panels_past_the_twelfth_stack_behind_the_ring_and_a_focused_one_takes_the_rings_last_place() {
    let mut rig = world(&[], "");
    let got = ring(&mut rig, 14, 800.0, "14");
    near(got[13 * 7 + 2], -1500.0, "the focused extra moves onto the ring");
    near(got[13 * 7 + 6], 0.0, "and is not behind");
    near(got[84], 769.5836160709155, "behind view 1: x");
    near(got[12 * 7 + 2], -1682.332029616988, "behind view 1: z");
    near(got[12 * 7 + 6], 1.0, "behind view 1: tier");
    let order: Vec<f64> = got[98..].to_vec();
    assert_eq!(
        order,
        [12.0, 11.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 13.0],
        "stacked panels draw first"
    );
    let tiers = ring(&mut rig, 25, 800.0, "null");
    let radius = |at: usize| (tiers[at * 7].powi(2) + tiers[at * 7 + 2].powi(2)).sqrt();
    near(radius(12), 1850.0, "tier one is 250 mm behind the ring");
    near(radius(24), 2100.0, "tier two is 500 mm behind the ring");
}

#[test]
fn the_focused_window_moves_to_the_front_and_last_in_tree_order_and_the_ring_turns_with_the_focus() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0), (800.0, 600.0)], "");
    xr_arrives(&mut rig);
    set_focus(&mut rig, WIN + 1);
    let front = view_of(&rig, WIN + 1).expect("view");
    let [x, y, z] = pose(&rig, front);
    near(x, 0.0, "x");
    near(y, 1400.0, "y");
    near(z, -1500.0, "z");
    near(yaw(&rig, front), 0.0, "yaw");
    near(scale(&rig, front), 0.8625, "a focused panel is larger");
    assert_eq!(shown(&rig)[2], WIN + 1, "the focused View is last, so it draws on top");
    near(
        pose(&rig, view_of(&rig, WIN).expect("view"))[0],
        -703.8364939904852,
        "the previous window is on the left",
    );
    near(
        pose(&rig, view_of(&rig, WIN + 2).expect("view"))[0],
        703.8364939904852,
        "the next on the right",
    );
    set_focus(&mut rig, WIN + 2);
    let next = view_of(&rig, WIN + 2).expect("view");
    near(pose(&rig, next)[0], 0.0, "the new focus is at the front");
    near(scale(&rig, next), 0.8625, "and larger");
    near(scale(&rig, front), 0.75, "the old focus is back to normal size");
    assert_eq!(shown(&rig)[2], WIN + 2, "the new focus is last");
}

#[test]
fn a_view_tree_order_is_back_to_front_so_panels_stacked_behind_the_ring_draw_first() {
    let sizes = vec![(800.0, 600.0); 14];
    let mut rig = world(&sizes, "");
    xr_arrives(&mut rig);
    assert_eq!(rig.comp.ids_of_class("view").len(), 14);
    let order = shown(&rig);
    assert_eq!(order[0], WIN + 12, "the stacked panels come first");
    assert_eq!(order[1], WIN + 13);
    let behind = pose(&rig, view_of(&rig, WIN + 12).expect("view"));
    near(
        (behind[0].powi(2) + behind[2].powi(2)).sqrt(),
        1850.0,
        "and sit 250 mm behind the ring",
    );
}

#[test]
fn a_frame_in_which_nothing_changes_writes_nothing_to_the_room() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0), (800.0, 600.0)], "");
    xr_arrives(&mut rig);
    rig.comp.clear_sent();
    rig.comp.external_set(WIN, "title", text("a new title"));
    rig.settle();
    let writes: Vec<TxOp> = rig
        .comp
        .ops()
        .into_iter()
        .filter(|op| matches!(op, TxOp::Set { key, .. } if ["x", "y", "z", "rotation", "scale", "width", "height", "visible"].contains(&key.as_str())))
        .collect();
    assert!(writes.is_empty(), "a title change writes no pose: {writes:?}");
}

#[test]
fn a_view_shows_a_client_without_moving_or_reparenting_it() {
    let mut rig = world(&[(800.0, 600.0)], "");
    rig.comp.clear_sent();
    rig.eval("globalThis.v = new View({ content: gnoblin.clients.at(0), width: 640, height: 480, x: 5 }); 1");
    rig.settle();
    let view = rig.comp.ids_of_class("view")[0];
    assert_eq!(rig.comp.value(view, "source"), Some(MirrorValue::Node(WIN)));
    assert!(flag(&mut rig, "v.content === gnoblin.clients.at(0)"));
    assert!(rig.comp.sets_of(WIN).is_empty(), "nothing is written to the window");
    assert_eq!(rig.comp.parent_of(WIN), Some(ROOT), "it is not reparented");
    // Many Views may show one client.
    rig.eval("globalThis.w = new View({ content: gnoblin.clients.at(0) }); 1");
    rig.settle();
    assert_eq!(rig.comp.ids_of_class("view").len(), 2);
    let error = rig.eval("try { v.content = gnoblin.workspaces.at(0); 'ok' } catch (e) { e.message }");
    assert!(
        error.contains("workspace"),
        "a workspace is not a tree node yet: {error}"
    );
    // Destroying the View leaves the client where it was.
    rig.eval("v.destroy(); w.destroy(); 1");
    rig.settle();
    assert!(rig.comp.ids_of_class("view").is_empty());
    assert!(rig.comp.exists(WIN));
}

#[test]
fn a_head_locked_panel_is_a_view_under_the_head_camera_at_metres_in_front_of_the_eyes() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    rig.eval(
        "globalThis.panel = gnoblin.extensions.xr.room.pinToHead(gnoblin.clients.at(0), { x: 0.1, y: -0.2, z: -1 }); 1",
    );
    rig.settle();
    let head = rig.comp.named("head").expect("head");
    let panel = rig.comp.children(head)[0];
    assert_eq!(rig.comp.value(panel, "source"), Some(MirrorValue::Node(WIN)));
    near(rig.comp.number(panel, "x"), 100.0, "x in millimetres");
    near(rig.comp.number(panel, "y"), -200.0, "y");
    near(rig.comp.number(panel, "z"), -1000.0, "z");
    near(rig.comp.number(panel, "width"), 800.0, "the panel follows the window");
    rig.comp.external_set(WIN, "width", double(900.0));
    rig.settle();
    near(rig.comp.number(panel, "width"), 900.0, "and follows its size");
    assert_eq!(
        rig.comp.ids_of_class("view").len(),
        2,
        "beside the ring's own View of the window"
    );
    rig.comp.destroy(WIN);
    rig.settle();
    assert!(!flag(&mut rig, "panel.alive"), "the panel goes with its window");
}

// ---- The session's view of the head, the controllers and the layer budget -----------------------------------------

#[test]
fn the_session_reads_the_head_the_layer_budget_and_the_controllers_from_the_tree() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    assert_eq!(rig.eval("String(gnoblin.extensions.xr.session.controllers.size)"), "0");
    assert_eq!(rig.eval("gnoblin.extensions.xr.session.tracking"), "untracked");
    near(
        number(&mut rig, "gnoblin.extensions.xr.session.layerBudget"),
        0.0,
        "no budget yet",
    );
    let head = rig.comp.named("head").expect("head");
    // The program writes the head's pose with y mirrored and the turns about x and z reversed, and the budget.
    rig.comp.external_sets(
        head,
        vec![
            ("x", double(100.0)),
            ("y", double(-1500.0)),
            ("z", double(-300.0)),
            ("rotation", MirrorValue::List(vec![0.1, 0.5, -0.2])),
            ("layer_budget", MirrorValue::Uint(16)),
        ],
    );
    rig.settle();
    near(
        number(&mut rig, "gnoblin.extensions.xr.session.layerBudget"),
        16.0,
        "the budget",
    );
    let head_pose = numbers(
        &mut rig,
        "(() => { const p = gnoblin.extensions.xr.session.head.pose; return [p.position.x, p.position.y, p.position.z, p.rotation.x, p.rotation.y, p.rotation.z]; })()",
    );
    for (got, want) in head_pose.iter().zip([100.0, 1500.0, -300.0, -0.1, 0.5, 0.2]) {
        near(*got, want, "the head pose in the placement frame");
    }
    rig.eval("globalThis.log = []; gnoblin.extensions.xr.session.controllers.addListener({ onAdded: (c) => log.push('+' + c.hand), onRemoved: (c) => log.push('-' + c.hand) }); 1");
    hand(&rig, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    rig.settle();
    assert_eq!(rig.eval("String(gnoblin.extensions.xr.session.controllers.size)"), "1");
    assert_eq!(rig.eval("gnoblin.extensions.xr.session.tracking"), "tracked");
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.8)), ("grip", double(0.5))]);
    rig.settle();
    assert_eq!(
        rig.eval("[...gnoblin.extensions.xr.session.controllers.at(0).buttons].join('+')"),
        "Trigger"
    );
    // A pull is down at 0.75 and up at 0.25, so a half pull neither presses nor chatters.
    rig.comp.external_sets(RIGHT_HAND, vec![("trigger", double(0.5))]);
    rig.settle();
    assert_eq!(
        rig.eval("[...gnoblin.extensions.xr.session.controllers.at(0).buttons].join('+')"),
        "Trigger"
    );
    rig.comp.external_sets(RIGHT_HAND, vec![("trigger", double(0.2))]);
    rig.settle();
    assert_eq!(
        rig.eval("[...gnoblin.extensions.xr.session.controllers.at(0).buttons].join('+')"),
        ""
    );
    rig.comp.destroy(RIGHT_HAND);
    rig.settle();
    assert_eq!(
        rig.eval("log.join(',')"),
        "+right,-right",
        "a removed controller still says which hand it was"
    );
}

#[test]
fn an_xr_trigger_is_parsed_in_one_canonical_spelling_and_a_bad_one_names_the_token() {
    let mut rig = world(&[], "");
    for (written, canonical) in [
        ("Grip+Drag", "Grip+Drag"),
        ("grip+trigger+drag", "Trigger+Grip+Drag"),
        ("Right.Grip+Drag", "Right.Grip+Drag"),
        ("left.Trigger+right.Trigger+Drag", ""),
        ("Secondary+Primary+Drag", "Primary+Secondary+Drag"),
    ] {
        let got = rig.eval(&format!(
            "try {{ parseControllerTrigger('{written}').text }} catch (e) {{ 'error' }}"
        ));
        assert_eq!(got, if canonical.is_empty() { "error" } else { canonical }, "{written}");
    }
    for (written, token) in [
        ("Grip", "Grip"),
        ("Squeeze+Drag", "Squeeze"),
        ("Grip+Grip+Drag", "Grip"),
        ("Drag", "Drag"),
        ("Middle.Grip+Drag", "Middle.Grip"),
    ] {
        let message = rig.eval(&format!(
            "try {{ parseControllerTrigger('{written}'); 'ok' }} catch (e) {{ e.message }}"
        ));
        assert!(message.contains(token), "{written}: {message}");
    }
    // The ordinary grammar still refuses an XR trigger, and an XR binding refuses a pointer trigger.
    assert!(
        rig.eval("try { new g.Binding({ id: 'x', trigger: 'Grip+Drag' }); 'ok' } catch (e) { e.message }")
            .contains("Drag")
    );
    assert!(
        rig.eval("try { new ViewMoveBinding({ trigger: 'Super+MouseLeft+Drag' }); 'ok' } catch (e) { e.message }")
            .contains("Super")
    );
    // An XR binding cannot be bound on a seat: no seat claims it.
    let error = rig.eval(
        "try { gnoblin.input.primary.bind('Grip+Drag', new ViewMoveBinding()); 'bound' } catch (e) { e.message }",
    );
    assert!(error.contains("XR"), "{error}");
}

// ---- Dragging a panel ---------------------------------------------------------------------------------------------

/// A session with one 800 by 600 window whose panel is straight ahead, 1600 mm away (nothing is focused), and a right
/// hand at the origin. The settle time is 0, so a drag starts on the frame the buttons go down.
fn drag_world(body: &str) -> Rig {
    let mut rig = world(&[(800.0, 600.0)], body);
    xr_arrives(&mut rig);
    hand(&rig, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    rig.eval("gnoblin.extensions.xr.session.chordSettleMs = 0; 1");
    rig.settle();
    rig
}

fn grip(rig: &mut Rig, value: f64) {
    rig.comp.external_sets(RIGHT_HAND, vec![("grip", double(value))]);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

fn move_hand(rig: &mut Rig, x: f64, y: f64, z: f64) {
    rig.comp
        .external_sets(RIGHT_HAND, vec![("x", double(x)), ("y", double(y)), ("z", double(z))]);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

fn window_rect(rig: &Rig) -> Vec<Option<MirrorValue>> {
    ["x", "y", "width", "height"]
        .iter()
        .map(|key| rig.comp.value(WIN, key))
        .collect()
}

#[test]
fn xr3_a_grip_drag_moves_the_view_and_changes_nothing_of_the_client() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    assert_eq!(pose(&rig, view), [0.0, 1400.0, -1600.0], "the panel is straight ahead");
    let rect = window_rect(&rig);
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    // The hand moves 200 mm to the right and 50 up: the panel goes with it.
    move_hand(&mut rig, 200.0, 50.0, 0.0);
    rig.settle();
    let [x, y, z] = pose(&rig, view);
    near(x, 200.0, "x follows the hand");
    near(y, 1450.0, "y follows the hand");
    near(z, -1600.0, "z is as it was");
    near(yaw(&rig, view), 0.0, "the panel does not turn");
    near(scale(&rig, view), 0.75, "or change size");
    assert!(flag(
        &mut rig,
        "gnoblin.extensions.xr.room.isPinned(gnoblin.extensions.xr.room.views[0])"
    ));
    assert!(
        rig.comp.sets_of(WIN).is_empty(),
        "no write reached the window: {:?}",
        rig.comp.ops()
    );
    assert_eq!(
        window_rect(&rig),
        rect,
        "the client's rectangle on the desktop is unchanged"
    );
    assert_eq!(rig.comp.parent_of(WIN), Some(ROOT));
    grip(&mut rig, 0.0);
    // The panel stays where it was left: the ring leaves a pinned View alone, even when the focus moves.
    set_focus(&mut rig, WIN);
    let [x, y, z] = pose(&rig, view);
    near(x, 200.0, "x after release and a focus change");
    near(y, 1450.0, "y");
    near(z, -1600.0, "z is not the focused ring's -1500");
    assert!(rig.comp.sets_of(WIN).is_empty());
    // `unpin` gives the View back to the ring.
    rig.eval("gnoblin.extensions.xr.room.unpin(gnoblin.extensions.xr.room.views[0]); 1");
    rig.settle();
    near(pose(&rig, view)[2], -1500.0, "the ring places it again");
}

#[test]
fn a_hand_that_turns_turns_the_panel_about_it_and_keep_holds_the_panels_own_rotation() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    grip(&mut rig, 1.0);
    rig.comp
        .external_sets(RIGHT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, 0.5, 0.0]))]);
    rig.frame();
    rig.settle();
    let [x, y, z] = pose(&rig, view);
    near(x, -1600.0 * 0.5f64.sin(), "the panel swings round the hand");
    near(y, 1400.0, "y");
    near(z, -1600.0 * 0.5f64.cos(), "z");
    near(yaw(&rig, view), 0.5, "and turns with it");
    grip(&mut rig, 0.0);

    let mut kept = drag_world("gnoblin.extensions.configure('xr', { layout: { triggers: { move: null } } });");
    kept.eval(
        "gnoblin.extensions.xr.room.layout.bindings.add(new ViewMoveBinding({ id: 'move', rotation: 'keep' })); 1",
    );
    let view = view_of(&kept, WIN).expect("view");
    grip(&mut kept, 1.0);
    kept.comp
        .external_sets(RIGHT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, 0.5, 0.0]))]);
    kept.frame();
    kept.settle();
    near(yaw(&kept, view), 0.0, "rotation keep leaves the panel's own turn");
    near(pose(&kept, view)[0], -1600.0 * 0.5f64.sin(), "but it still swings");
}

#[test]
fn a_ray_drag_moves_a_distant_panel_along_the_ray_at_the_distance_it_was_grabbed() {
    let mut rig = drag_world("gnoblin.extensions.configure('xr', { layout: { triggers: { move: null } } });");
    rig.eval(
        "gnoblin.extensions.xr.room.layout.bindings.add(new ViewMoveBinding({ id: 'move', trigger: 'Grip+Drag', tracking: 'ray', rotation: 'keep', distance: 'preserve' })); 1",
    );
    let view = view_of(&rig, WIN).expect("view");
    // Grab the panel 100 mm right of its centre, so the panel must not jump to the ray.
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    grip(&mut rig, 1.0);
    rig.comp.clear_sent();
    // Turn the hand 0.2 rad about y: the ray now points to the right of straight ahead.
    rig.comp
        .external_sets(RIGHT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, -0.2, 0.0]))]);
    rig.frame();
    rig.settle();
    let hit_distance = 1600.0;
    // The grabbed point sat at (100, 0, -1600) from a hand at (100, 0, 0): 1600 along the ray. It is now 1600 along
    // the turned ray, and the panel's centre keeps its offset (-100, 0, 0) from that point.
    let along = [0.2f64.sin(), 0.0, -0.2f64.cos()];
    let want = [100.0 + hit_distance * along[0] - 100.0, 1400.0, hit_distance * along[2]];
    let got = pose(&rig, view);
    near(got[0], want[0], "x");
    near(got[1], want[1], "y");
    near(got[2], want[2], "z");
    near(yaw(&rig, view), 0.0, "keep: the panel does not turn");
    assert!(rig.comp.sets_of(WIN).is_empty(), "the client is untouched");
}

#[test]
fn xr4_a_resize_changes_the_views_size_on_every_update_asks_the_client_for_nothing_while_the_hand_moves_and_once_on_release()
 {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    rig.eval("gnoblin.extensions.xr.session.chordSettleMs = 0; 1");
    // Grab the top right: the ray lands 200 mm right of and 100 mm above the panel's centre.
    move_hand(&mut rig, 200.0, 100.0, 0.0);
    rig.comp.clear_sent();
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    rig.frame();
    // Drag outward in three steps. Every update changes the View and none touches the window.
    let mut last = scale(&rig, view);
    for (x, y) in [(300.0, 200.0), (400.0, 300.0), (350.0, 250.0)] {
        move_hand(&mut rig, x, y, 0.0);
        rig.settle();
        let now = scale(&rig, view);
        assert!(
            (now - last).abs() > 1e-6,
            "the View changed on this update ({x}, {y}): {last} -> {now}"
        );
        last = now;
        assert!(
            rig.comp.sets_of(WIN).is_empty(),
            "no configure during the drag: {:?}",
            rig.comp.sets_of(WIN)
        );
        near(
            rig.comp.number(view, "width"),
            800.0,
            "the View's picture is the client's, unchanged",
        );
        near(rig.comp.number(view, "height"), 600.0, "and so is its height");
    }
    // The last step is the (400, 300) of the arithmetic below.
    move_hand(&mut rig, 400.0, 300.0, 0.0);
    rig.settle();
    let k = (700.0f64 / (133.33333333333334 + 300.0)).max((533.3333333333334 + 400.0) / (266.6666666666667 + 400.0));
    near(
        scale(&rig, view),
        0.75 * k,
        "the View grew evenly to the larger of the two asked-for sizes",
    );
    // The opposite corner (bottom left) stays where it was: (-300, -225) from the panel's centre, in the room's frame.
    let [x, y, _] = pose(&rig, view);
    near(x - 400.0 * scale(&rig, view), -300.0, "the anchored corner's x stays");
    near((y - 1400.0) - 300.0 * scale(&rig, view), -225.0, "and its y");
    assert!(rig.comp.sets_of(WIN).is_empty());
    // Release: the asked-for size changes the aspect, so the client is asked once, in one transaction.
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.0)), ("grip", double(0.0))]);
    rig.frame();
    rig.settle();
    let carrying: Vec<Vec<TxOp>> = rig
        .comp
        .transactions()
        .into_iter()
        .filter(|ops| {
            ops.iter()
                .any(|op| matches!(op, TxOp::Set { node, .. } if *node == WIN))
        })
        .collect();
    assert_eq!(carrying.len(), 1, "one transaction carries the request: {carrying:?}");
    let keys: Vec<String> = rig.comp.sets_of(WIN);
    assert_eq!(keys, ["width", "height"], "and it is the size alone");
    let kx = (533.3333333333334 + 400.0) / (266.6666666666667 + 400.0);
    let ky = 700.0 / (133.33333333333334 + 300.0);
    near(
        rig.comp.number(WIN, "width"),
        (800.0 * kx * 0.75 * 0.001 * 1200.0f64).round(),
        "the width asked for",
    );
    near(
        rig.comp.number(WIN, "height"),
        (600.0 * ky * 0.75 * 0.001 * 1200.0f64).round(),
        "the height asked for",
    );
    // The panel keeps the size the hand drew: one pixel is now 1 / 1200 of a metre.
    near(
        scale(&rig, view),
        1.0 / (1200.0 * 0.001),
        "mm per pixel after the request",
    );
    // The View follows the window's new size on the next frames, and nothing more is asked of it.
    rig.settle();
    assert_eq!(rig.comp.sets_of(WIN).len(), 2, "nothing else is written to the window");
}

#[test]
fn a_resize_that_keeps_the_aspect_and_the_density_asks_the_client_for_nothing_at_all() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    move_hand(&mut rig, 200.0, 150.0, 0.0);
    rig.comp.clear_sent();
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    rig.frame();
    // A 10% larger panel: the ray is 10% further from the anchor corner on each axis, so the aspect is the same and the
    // client has the pixels.
    move_hand(&mut rig, 250.0, 187.5, 0.0);
    rig.settle();
    near(scale(&rig, view), 0.75 * 1.1, "the preview is 10% larger");
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.0)), ("grip", double(0.0))]);
    rig.frame();
    rig.settle();
    assert!(
        rig.comp.sets_of(WIN).is_empty(),
        "no request on release: {:?}",
        rig.comp.ops()
    );
    near(scale(&rig, view), 0.75 * 1.1, "the View keeps the size it was given");
}

#[test]
fn a_drag_cancelled_by_an_untracked_hand_puts_the_view_back_and_no_client_is_asked_anything() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 300.0, 0.0, 0.0);
    rig.settle();
    near(pose(&rig, view)[0], 300.0, "the panel is moved");
    rig.comp
        .external_sets(RIGHT_HAND, vec![("tracked", MirrorValue::Bool(false))]);
    rig.frame();
    rig.settle();
    assert_eq!(pose(&rig, view), [0.0, 1400.0, -1600.0], "onCancel puts the View back");
    assert!(!flag(
        &mut rig,
        "gnoblin.extensions.xr.room.isPinned(gnoblin.extensions.xr.room.views[0])"
    ));
    assert!(rig.comp.sets_of(WIN).is_empty());
}

#[test]
fn a_session_that_ends_mid_drag_cancels_it_once() {
    let mut rig = drag_world(
        "globalThis.log = []; gnoblin.extensions.configure('xr', { layout: { triggers: { move: null } } });",
    );
    rig.eval(
        "class Spy extends XRControllerDragBinding { \
           onStart(e) { log.push('start:' + (e.view !== undefined)); return {}; } \
           onUpdate(e) { log.push('update:' + e.deltaTime.toFixed(0)); } \
           onEnd() { log.push('end'); } \
           onCancel() { log.push('cancel'); } } \
         gnoblin.extensions.xr.room.layout.bindings.add(new Spy({ id: 'spy', trigger: 'Grip+Drag' })); 1",
    );
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 10.0, 0.0, 0.0);
    rig.comp.destroy(XR_SEAT);
    rig.settle();
    assert_eq!(rig.eval("log.join(',')"), "start:true,update:0,cancel");
}

#[test]
fn the_binding_lifecycle_is_start_update_end_with_one_update_a_frame_and_a_throw_cancels_only_that_drag() {
    let mut rig = drag_world(
        "globalThis.log = []; gnoblin.extensions.configure('xr', { layout: { triggers: { move: null, resize: null } } });",
    );
    rig.eval(
        "class Spy extends XRControllerDragBinding { \
           onStart(e) { log.push('start'); return { n: 0 }; } \
           onUpdate(e, s) { s.n++; log.push('update' + s.n); if (globalThis.boom && s.n === 2) throw new Error('boom'); } \
           onEnd(e, s) { log.push('end' + s.n); } \
           onCancel(e, s) { log.push('cancel' + s.n); } } \
         gnoblin.extensions.xr.room.layout.bindings.add(new Spy({ id: 'spy', trigger: 'Grip+Drag' })); 1",
    );
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 1.0, 0.0, 0.0);
    move_hand(&mut rig, 2.0, 0.0, 0.0);
    grip(&mut rig, 0.0);
    assert_eq!(rig.eval("log.join(',')"), "start,update1,update2,end2");
    // An update that throws is reported once and cancels that drag; the next press starts a fresh one.
    rig.eval("log.length = 0; globalThis.boom = true; 1");
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 3.0, 0.0, 0.0);
    rig.comp.external_sets(RIGHT_HAND, vec![("x", double(4.0))]);
    let diagnostics = rig.frame();
    assert!(
        diagnostics.iter().any(|line| line.contains("boom")),
        "the throw is reported: {diagnostics:?}"
    );
    assert_eq!(rig.eval("log.join(',')"), "start,update1,update2,cancel2");
    rig.eval("globalThis.boom = false; log.length = 0; 1");
    grip(&mut rig, 0.0);
    grip(&mut rig, 1.0);
    assert_eq!(rig.eval("log.join(',')"), "start", "a fresh drag after the release");
}

#[test]
fn a_binding_disables_removes_and_replaces_like_any_binding() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    let binding = "gnoblin.extensions.xr.room.layout.bindings";
    // Disabled: the grab does nothing.
    rig.eval(&format!("{binding}.get('move').enabled = false; 1"));
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 0.0, "a disabled binding does not move the panel");
    grip(&mut rig, 0.0);
    move_hand(&mut rig, 0.0, 0.0, 0.0);
    // Removed.
    rig.eval(&format!(
        "{binding}.get('move').enabled = true; {binding}.remove('move'); 1"
    ));
    assert!(!flag(&mut rig, &format!("{binding}.has('move')")));
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 0.0, "a removed binding does not move the panel");
    grip(&mut rig, 0.0);
    move_hand(&mut rig, 0.0, 0.0, 0.0);
    // Replaced by one on the left hand only: the right hand's grip does nothing, the left's moves the panel.
    rig.eval(&format!(
        "{binding}.add(new gnoblin.extensions.xr.ViewMoveBinding({{ id: 'move', trigger: 'Left.Grip+Drag', tracking: 'ray', rotation: 'keep' }})); 1"
    ));
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 0.0, "the right hand no longer grabs");
    grip(&mut rig, 0.0);
    hand(&rig, LEFT_HAND, "left", 0.0, 0.0, 0.0);
    rig.settle();
    rig.comp.external_sets(LEFT_HAND, vec![("grip", double(1.0))]);
    rig.frame();
    rig.comp
        .external_sets(LEFT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, -0.2, 0.0]))]);
    rig.frame();
    rig.settle();
    assert!(
        pose(&rig, view)[0] > 100.0,
        "the left hand grabs by ray: {:?}",
        pose(&rig, view)
    );
    // A trigger can be moved: `trigger` is a setter that reparses.
    rig.comp.external_sets(LEFT_HAND, vec![("grip", double(0.0))]);
    rig.frame();
    rig.eval(&format!("{binding}.get('move').trigger = 'Right.Trigger+Drag'; 1"));
    assert_eq!(
        rig.eval(&format!("{binding}.get('move').parsed.text")),
        "Right.Trigger+Drag"
    );
}

#[test]
fn a_two_button_chord_that_goes_down_together_is_not_taken_for_its_first_button() {
    let mut rig = drag_world("globalThis.log = [];");
    rig.eval(
        "const b = gnoblin.extensions.xr.room.layout.bindings; \
         for (const id of ['move', 'resize']) { const old = b.get(id); const hook = old.onStart.bind(old); old.onStart = (e) => { log.push(id); return hook(e); }; } 1",
    );
    // Trigger and grip in one frame: a resize, never a move.
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    rig.frame();
    assert_eq!(rig.eval("log.join(',')"), "resize");
    // With the default settle time a chord that is not yet stable has not started.
    let mut slow = world(&[(800.0, 600.0)], "globalThis.log = [];");
    xr_arrives(&mut slow);
    hand(&slow, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    slow.settle();
    assert_eq!(slow.eval("gnoblin.extensions.xr.session.chordSettleMs"), "50");
    slow.eval(
        "const old = gnoblin.extensions.xr.room.layout.bindings.get('move'); const hook = old.onStart.bind(old); old.onStart = (e) => { log.push('move'); return hook(e); }; 1",
    );
    slow.comp.external_sets(RIGHT_HAND, vec![("grip", double(1.0))]);
    slow.frame();
    assert_eq!(
        slow.eval("log.join(',')"),
        "",
        "the grip has not been down for the settle time yet"
    );
}

#[test]
fn a_hand_that_points_at_nothing_starts_no_drag() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    // Aimed far to the side of the only panel.
    move_hand(&mut rig, 5000.0, 0.0, 0.0);
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 5100.0, 0.0, 0.0);
    assert_eq!(pose(&rig, view), [0.0, 1400.0, -1600.0], "no panel was under the ray");
    assert!(!flag(
        &mut rig,
        "gnoblin.extensions.xr.room.isPinned(gnoblin.extensions.xr.room.views[0])"
    ));
}

// ---- The snippets of docs/xr.md ----------------------------------------------------------------------------------

#[test]
fn the_configure_snippet_of_the_docs_shapes_the_stock_room() {
    let mut rig = world(
        &[(800.0, 600.0)],
        "gnoblin.extensions.configure('xr', { room: true, layout: { radius: 1.8, triggers: { move: 'Right.Grip+Drag', resize: null } } });",
    );
    xr_arrives(&mut rig);
    let view = view_of(&rig, WIN).expect("view");
    near(rig.comp.number(view, "z"), -1800.0, "the ring's radius is 1.8 m");
    assert_eq!(
        rig.eval("gnoblin.extensions.xr.room.layout.bindings.get('move').parsed.text"),
        "Right.Grip+Drag"
    );
    assert!(!flag(
        &mut rig,
        "gnoblin.extensions.xr.room.layout.bindings.has('resize')"
    ));
}

#[test]
fn the_viewport_snippet_of_the_docs_replaces_the_stock_room_and_names_the_nodes_for_the_program() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    let stock = room(&rig);
    rig.eval(
        "const xr = gnoblin.extensions.xr; \
         if (xr?.session) { \
           const head = new xr.Camera3D({ tracking: xr.session.head, near: 0.05, far: 100 }); \
           xr.session.viewport = new xr.Viewport({ content: new xr.Room(), camera: head, composition: 'auto' }); \
         } 1",
    );
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(!rig.comp.exists(stock), "the stock room is gone");
    let rooms: Vec<u64> = rig
        .comp
        .ids_of_class("node")
        .into_iter()
        .filter(|id| rig.comp.value(*id, "name") == Some(text("room")))
        .collect();
    assert_eq!(rooms.len(), 1, "one room: {rooms:?}");
    assert_eq!(rig.comp.parent_of(rooms[0]), Some(ROOT), "mounted under the root");
    let heads = rig.comp.ids_of_class("camera");
    assert_eq!(heads.len(), 1, "one head camera");
    assert_eq!(rig.comp.value(heads[0], "name"), Some(text("head")));
    assert_eq!(rig.comp.parent_of(heads[0]), Some(rooms[0]), "under the new room");
    near(rig.comp.number(heads[0], "near"), 50.0, "near is 0.05 m in millimetres");
    assert!(view_of(&rig, WIN).is_some(), "the new room has the window's panel");
    assert!(flag(
        &mut rig,
        "gnoblin.extensions.xr.session.viewport.composition === 'auto'"
    ));
}

// ---- Claims: a drag takes its buttons from the window under the ray ----------------------------------------------

/// What the node says is claimed: the shell's last write of `claimed`, in the order it wrote it.
fn claimed(rig: &Rig, hand: u64) -> Vec<String> {
    match rig.comp.value(hand, "claimed") {
        Some(MirrorValue::Texts(names)) => names,
        None => Vec::new(),
        other => panic!("`claimed` is a list of names, not {other:?}"),
    }
}

/// How many times the shell wrote `claimed` on `hand` since `clear_sent`.
fn claim_writes(rig: &Rig, hand: u64) -> usize {
    rig.comp.sets_of(hand).iter().filter(|key| *key == "claimed").count()
}

/// Runs `body` in the script and lets the frame send what it wrote.
fn run(rig: &mut Rig, body: &str) {
    rig.eval(&format!("{body}; 1"));
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// Both hands tracked at the origin, a drag world's right hand being the one that grabs.
fn two_hands(body: &str) -> Rig {
    let mut rig = drag_world(body);
    hand(&rig, LEFT_HAND, "left", 0.0, 0.0, 0.0);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    rig
}

const SPY: &str = "globalThis.log = []; \
    gnoblin.extensions.configure('xr', { layout: { triggers: { move: null, resize: null } } }); \
    globalThis.Spy = class Spy extends XRControllerDragBinding { \
       onStart(e) { log.push('start'); return { n: 0 }; } \
       onUpdate(e, s) { s.n++; log.push('update' + s.n + ':' + e.controller.claimed.join('+')); \
           if (globalThis.boom) throw new Error('boom'); } \
       onEnd(e, s) { log.push('end'); } \
       onCancel(e, s) { log.push('cancel'); } };";

#[test]
fn a_grip_drag_claims_the_grip_of_the_hand_that_started_it_for_the_length_of_the_drag_and_no_longer() {
    let mut rig = two_hands("");
    assert!(claimed(&rig, RIGHT_HAND).is_empty() && claimed(&rig, LEFT_HAND).is_empty());
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"], "the grip is the drag's");
    assert!(claimed(&rig, LEFT_HAND).is_empty(), "the other hand is not touched");
    assert_eq!(rig.comp.sets_of(LEFT_HAND), Vec::<String>::new());
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    move_hand(&mut rig, 200.0, 0.0, 0.0);
    rig.settle();
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["grip"],
        "still claimed while the hand moves"
    );
    assert_eq!(
        claim_writes(&rig, RIGHT_HAND),
        1,
        "one write claims it, not one for each frame of the drag"
    );
    assert!(rig.comp.sets_of(WIN).is_empty(), "and the window is untouched");
    grip(&mut rig, 0.0);
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "the end of the drag gives the grip back"
    );
    assert_eq!(
        claim_writes(&rig, RIGHT_HAND),
        2,
        "one write to claim and one to release"
    );
    // The node says what a script reads.
    assert_eq!(
        rig.eval("gnoblin.extensions.xr.session.controllers.get('60').claimed.length"),
        "0"
    );
}

#[test]
fn a_ray_drag_and_a_resize_claim_the_buttons_of_their_triggers() {
    let mut rig = drag_world("gnoblin.extensions.configure('xr', { layout: { triggers: { move: null } } });");
    rig.eval(
        "gnoblin.extensions.xr.room.layout.bindings.add(new ViewMoveBinding({ id: 'move', trigger: 'Right.Grip+Drag', tracking: 'ray', rotation: 'keep', distance: 'preserve' })); 1",
    );
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"], "a ray drag takes the grip too");
    grip(&mut rig, 0.0);
    assert!(claimed(&rig, RIGHT_HAND).is_empty());

    let mut resizing = drag_world("");
    move_hand(&mut resizing, 200.0, 100.0, 0.0);
    resizing
        .comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    resizing.frame();
    assert_eq!(
        claimed(&resizing, RIGHT_HAND),
        ["trigger", "grip"],
        "a resize holds both buttons of its chord, in the node's order"
    );
    resizing
        .comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.0)), ("grip", double(0.0))]);
    resizing.frame();
    resizing.settle();
    assert!(claimed(&resizing, RIGHT_HAND).is_empty());
}

#[test]
fn a_hand_that_points_at_nothing_or_a_binding_that_declines_claims_nothing() {
    let mut rig = drag_world("");
    // Out of reach of every panel: no View is under the ray, so no drag starts.
    move_hand(&mut rig, 100_000.0, 0.0, 0.0);
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 0);

    let mut declined = drag_world(&format!(
        "{SPY} globalThis.Decline = class extends Spy {{ onStart() {{ return false; }} }};"
    ));
    declined.eval("gnoblin.extensions.xr.room.layout.bindings.add(new Decline({ id: 'no', trigger: 'Grip+Drag' })); 1");
    declined.comp.clear_sent();
    grip(&mut declined, 1.0);
    assert!(claimed(&declined, RIGHT_HAND).is_empty());
    assert_eq!(
        claim_writes(&declined, RIGHT_HAND),
        0,
        "a binding that said no wrote nothing"
    );
}

#[test]
fn every_way_a_drag_ends_gives_the_claim_back() {
    // Cancelled: the hand is lost.
    let mut rig = drag_world("");
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
    rig.comp
        .external_sets(RIGHT_HAND, vec![("tracked", MirrorValue::Bool(false))]);
    rig.frame();
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "an untracked hand's drag is cancelled"
    );

    // Cancelled: the drag's binding is removed from the room.
    let mut removed = drag_world("");
    grip(&mut removed, 1.0);
    assert_eq!(claimed(&removed, RIGHT_HAND), ["grip"]);
    run(
        &mut removed,
        "gnoblin.extensions.xr.room.layout.bindings.remove('move')",
    );
    assert!(
        claimed(&removed, RIGHT_HAND).is_empty(),
        "a removed binding holds nothing"
    );

    // Cancelled: the controller's hook throws, which is reported and ends only that drag.
    let mut crashed = drag_world(SPY);
    crashed.eval("gnoblin.extensions.xr.room.layout.bindings.add(new Spy({ id: 'spy', trigger: 'Grip+Drag' })); 1");
    crashed.eval("globalThis.boom = true; 1");
    grip(&mut crashed, 1.0);
    crashed.comp.external_sets(RIGHT_HAND, vec![("x", double(1.0))]);
    let diagnostics = crashed.frame();
    assert!(
        diagnostics.iter().any(|line| line.contains("boom")),
        "the throw is reported: {diagnostics:?}"
    );
    assert!(
        claimed(&crashed, RIGHT_HAND).is_empty(),
        "a crashed drag does not keep the grip"
    );
    assert!(
        crashed.eval("log.join(',')").ends_with(",cancel"),
        "{}",
        crashed.eval("log.join(',')")
    );

    // Cancelled: the session ends under the drag.
    let mut ended = drag_world("");
    grip(&mut ended, 1.0);
    assert_eq!(claimed(&ended, RIGHT_HAND), ["grip"]);
    ended.comp.destroy(XR_SEAT);
    ended.settle();
    assert!(
        claimed(&ended, RIGHT_HAND).is_empty(),
        "the end of the session releases what it claimed"
    );
}

#[test]
fn a_generation_that_is_retired_mid_drag_gives_the_claim_back_in_the_transaction_it_sends() {
    let mut rig = drag_world("");
    grip(&mut rig, 1.0);
    // The claim's transaction is answered by the next frame; `retire` sends its own at once only when none is in flight.
    rig.settle();
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
    // `retire` is what a reload and a shutdown call before the old generation goes.
    let frame = rig.shell.retire().expect("the generation retires");
    assert!(frame.diagnostics.is_empty(), "{:?}", frame.diagnostics);
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "the hold is released before the next generation runs"
    );
}

#[test]
fn two_claimants_on_one_hand_keep_their_own_and_the_node_says_the_union() {
    let mut rig = drag_world("");
    rig.eval(
        "globalThis.hand = gnoblin.extensions.xr.session.controllers.get('60'); \
         globalThis.panel = {}; globalThis.menu = {}; 1",
    );
    run(&mut rig, "hand.claim(panel, ['trigger'])");
    assert_eq!(claimed(&rig, RIGHT_HAND), ["trigger"]);
    // The drag arrives while the script holds the trigger: both stand, whichever came first.
    grip(&mut rig, 1.0);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["trigger", "grip"],
        "the drag did not take the script's claim"
    );
    run(&mut rig, "hand.claim(menu, ['thumbstick', 'grip'])");
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["trigger", "grip", "thumbstick"],
        "three owners, one list"
    );
    grip(&mut rig, 0.0);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["trigger", "grip", "thumbstick"],
        "the drag ending does not release a name another owner also holds"
    );
    run(&mut rig, "hand.release(menu)");
    assert_eq!(claimed(&rig, RIGHT_HAND), ["trigger"]);
    // A claim replaces the same owner's earlier one.
    run(&mut rig, "hand.claim(panel, ['primary'])");
    assert_eq!(claimed(&rig, RIGHT_HAND), ["primary"]);
    run(&mut rig, "hand.claim(panel, [])");
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    // Nothing changed, so nothing is written.
    rig.comp.clear_sent();
    run(
        &mut rig,
        "hand.release(panel); hand.release(menu); hand.claim(panel, [])",
    );
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 0);
    assert_eq!(claimed(&rig, LEFT_HAND), Vec::<String>::new());
}

/// What `body` throws, as `Name|message`, or `none`.
fn thrown(rig: &mut Rig, body: &str) -> String {
    rig.eval(&format!(
        "(() => {{ try {{ {body}; return 'none'; }} catch (e) {{ return e.name + '|' + e.message; }} }})()"
    ))
}

#[test]
fn a_claim_that_is_not_a_list_of_lowercase_input_names_is_a_typed_error_naming_the_controllers_claimed() {
    let mut rig = drag_world("");
    rig.eval("globalThis.hand = gnoblin.extensions.xr.session.controllers.get('60'); globalThis.me = {}; 1");
    rig.comp.clear_sent();
    for (call, expect) in [
        ("hand.claim(me, 'grip')", "a list"),
        ("hand.claim(me, null)", "a list"),
        ("hand.claim(me, { 0: 'grip' })", "a list"),
        ("hand.claim(me, ['Grip'])", "lowercase"),
        ("hand.claim(me, ['GRIP'])", "lowercase"),
        ("hand.claim(me, ['grip', 'bumper'])", "not an input"),
        ("hand.claim(me, [''])", "not an input"),
        ("hand.claim(me, [1])", "string"),
        ("hand.claim(null, ['grip'])", "owner"),
        ("hand.claim(undefined, ['grip'])", "owner"),
    ] {
        let got = thrown(&mut rig, call);
        assert!(got.starts_with("TypeError|Controller#60.claimed: "), "{call}: {got}");
        assert!(got.contains(expect), "{call}: {got}");
    }
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 0, "a refused claim writes nothing");
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    // A good call still works afterwards, and the refused ones left nothing behind.
    assert_eq!(thrown(&mut rig, "hand.claim(me, ['grip'])"), "none");
    rig.frame();
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
}

#[test]
fn a_controller_that_is_gone_refuses_a_claim_with_a_scene_error_and_forgets_a_release() {
    let mut rig = drag_world("");
    run(
        &mut rig,
        "globalThis.hand = gnoblin.extensions.xr.session.controllers.get('60'); globalThis.me = {}; hand.claim(me, ['grip'])",
    );
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
    rig.comp.destroy(RIGHT_HAND);
    rig.settle();
    assert!(flag(&mut rig, "!hand.alive"));
    let got = thrown(&mut rig, "hand.claim(me, ['trigger'])");
    assert!(got.starts_with("SceneError|Controller#60.claimed: "), "{got}");
    assert_eq!(
        thrown(&mut rig, "hand.release(me)"),
        "none",
        "a release of a gone controller is not an error"
    );
}

// ---- The settle time and the program's hold ------------------------------------------------------------------------

#[test]
fn the_chord_settle_time_is_a_number_of_milliseconds_the_programs_hold_can_cover() {
    let mut rig = drag_world("");
    let settle = |rig: &mut Rig, value: &str| {
        rig.eval(&format!("gnoblin.extensions.xr.session.chordSettleMs = {value}; 1"));
        rig.eval("String(gnoblin.extensions.xr.session.chordSettleMs)")
    };
    assert_eq!(settle(&mut rig, "40"), "40");
    assert_eq!(
        settle(&mut rig, "10000"),
        "60",
        "a settle time past 60 ms would claim after the 150 ms the program holds a press for"
    );
    assert_eq!(settle(&mut rig, "-5"), "0");
    assert_eq!(settle(&mut rig, "Infinity"), "60");
    for bad in ["'50'", "NaN", "null", "undefined"] {
        let got = thrown(
            &mut rig,
            &format!("gnoblin.extensions.xr.session.chordSettleMs = {bad}"),
        );
        assert!(
            got.starts_with("TypeError|session.chordSettleMs must be a number"),
            "{bad}: {got}"
        );
    }
    assert_eq!(
        rig.eval("String(gnoblin.extensions.xr.session.chordSettleMs)"),
        "60",
        "a refused value changes nothing"
    );
}

/// A world with the default settle time and a right hand at the origin, on the manual clock that stands still until the
/// test moves it.
fn settling_world() -> Rig {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    hand(&rig, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    rig.settle();
    rig
}

/// Moves the clock `ms` milliseconds and runs a frame.
fn after(rig: &mut Rig, ms: u64) {
    rig.advance(ms);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn a_drag_claims_its_grip_after_the_settle_time_and_not_on_the_press() {
    let mut rig = settling_world();
    assert_eq!(rig.eval("String(gnoblin.extensions.xr.session.chordSettleMs)"), "50");
    grip(&mut rig, 1.0);
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "the claim is not made on the frame of the press: the chord has to settle first"
    );
    after(&mut rig, 49);
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "49 ms is short of the 50 ms settle time, which is what the program's hold has to cover"
    );
    after(&mut rig, 1);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["grip"],
        "claimed once the chord has settled"
    );
}

#[test]
fn a_settle_time_past_the_cap_claims_at_the_cap_so_the_claim_still_fits_the_programs_hold() {
    let mut rig = settling_world();
    rig.eval("gnoblin.extensions.xr.session.chordSettleMs = 10000; 1");
    grip(&mut rig, 1.0);
    after(&mut rig, 59);
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    after(&mut rig, 1);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["grip"],
        "claimed at 60 ms, not at the 10 s that was asked for"
    );
}

// ---- The specification's XR surface (design/js-spec-native.md, "XR extension") --------------------------------------

/// A code fence of the owner's specification, verbatim.
fn fence(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/spec/snippets")
        .join(name);
    fs::read_to_string(path).expect("the fence is checked in")
}

/// The ids and triggers of the room layout's bindings, in order.
const BINDINGS: &str = "[...gnoblin.extensions.xr.room.layout.bindings].map((b) => b.id + '=' + b.trigger).join(',')";

const ROOM_VIEW_PINNED: &str = "gnoblin.extensions.xr.room.isPinned(gnoblin.extensions.xr.room.views[0])";

/// Presses or releases the trigger and the thumbstick of the right hand together, and runs a frame.
fn trigger_and_stick(rig: &mut Rig, down: bool) {
    rig.comp.external_sets(
        RIGHT_HAND,
        vec![
            ("trigger", double(if down { 1.0 } else { 0.0 })),
            ("thumbstick_click", MirrorValue::Bool(down)),
        ],
    );
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// Presses or releases the grip and the thumbstick of the right hand together, and runs a frame.
fn grip_and_stick(rig: &mut Rig, down: bool) {
    rig.comp.external_sets(
        RIGHT_HAND,
        vec![
            ("grip", double(if down { 1.0 } else { 0.0 })),
            ("thumbstick_click", MirrorValue::Bool(down)),
        ],
    );
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

fn turn_hand(rig: &mut Rig, yaw: f64) {
    rig.comp
        .external_sets(RIGHT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, yaw, 0.0]))]);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn the_extension_offers_the_drag_base_and_the_four_bindings_of_the_specs_hierarchy() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    for expression in [
        "gnoblin.extensions.xr.ControllerDragBinding === XRControllerDragBinding",
        "gnoblin.extensions.xr.ViewRotateBinding === ViewRotateBinding",
        "gnoblin.extensions.xr.ViewScaleBinding === ViewScaleBinding",
        "new ViewRotateBinding() instanceof XRControllerDragBinding",
        "new ViewScaleBinding() instanceof XRControllerDragBinding",
        "new ViewRotateBinding() instanceof g.Binding",
        "new ViewRotateBinding().id === 'rotate' && new ViewRotateBinding().trigger === 'Trigger+Thumbstick+Drag'",
        "new ViewScaleBinding().id === 'scale' && new ViewScaleBinding().trigger === 'Grip+Thumbstick+Drag'",
        "new ViewResizeBinding().edges === 'quadrant' && new ViewResizeBinding({ edges: 'top-left' }).edges === 'top-left'",
        "gnoblin.extensions.xr.room.layout.bindings.get('rotate') instanceof ViewRotateBinding",
    ] {
        assert!(flag(&mut rig, expression), "{expression}");
    }
    assert!(
        thrown(&mut rig, "new ViewResizeBinding({ edges: 'left' })").starts_with("TypeError|ViewResizeBinding: edges"),
    );
    assert!(thrown(&mut rig, "new ViewScaleBinding({ minScale: 2, maxScale: 1 })").starts_with("RangeError|"));
    assert!(thrown(&mut rig, "new ViewScaleBinding({ maxScale: 0 })").starts_with("RangeError|"));
}

#[test]
fn the_spatial_layout_triggers_name_rotate_and_snippet_065_runs() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    assert_eq!(
        rig.eval(BINDINGS),
        "move=Grip+Drag,resize=Grip+Trigger+Drag,rotate=Trigger+Thumbstick+Drag"
    );
    // Snippet 065 verbatim, over the room the shell built.
    let body = fence("065-xr-scene-and-view-creation.js.txt");
    rig.eval(&format!(
        "(() => {{ const xr = gnoblin.extensions.xr; const room = xr.room; {body} return 1; }})()"
    ));
    assert_eq!(
        rig.eval(BINDINGS),
        "move=Grip+Drag,resize=Grip+Trigger+Drag,rotate=Trigger+Thumbstick+Drag"
    );
    rig.eval(
        "gnoblin.extensions.xr.room.layout = new SpatialLayout({ triggers: { rotate: 'Left.Trigger+Thumbstick+Drag', move: null } }); 1",
    );
    assert_eq!(
        rig.eval(BINDINGS),
        "resize=Grip+Trigger+Drag,rotate=Left.Trigger+Thumbstick+Drag"
    );
    rig.eval("gnoblin.extensions.xr.room.layout = new SpatialLayout({ triggers: { rotate: null } }); 1");
    assert_eq!(rig.eval(BINDINGS), "move=Grip+Drag,resize=Grip+Trigger+Drag");
    let got = thrown(&mut rig, "new SpatialLayout({ triggers: { scale: 'Grip+Drag' } })");
    assert_eq!(
        got,
        "TypeError|SpatialLayout: unknown trigger \"scale\"; expected one of move, resize, rotate"
    );
    let got = thrown(
        &mut rig,
        "new SpatialLayout({ triggers: { rotate: 'Trigger+Thumbstick' } })",
    );
    assert!(got.starts_with("TypeError|SpatialLayout trigger \"rotate\""), "{got}");
}

#[test]
fn a_binding_added_under_a_taken_id_replaces_it_and_snippets_066_068_and_117_run() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    // 066: move gets another trigger, rotate goes, and a ray move takes the id `move`, on the chord resize holds.
    let body = fence("066-controller-inputs-and-trigger-grammar.js.txt");
    rig.eval(&format!(
        "(() => {{ const xr = gnoblin.extensions.xr; const spatial = xr.room.layout; {body} return 1; }})()"
    ));
    assert_eq!(rig.eval(BINDINGS), "resize=Grip+Trigger+Drag,move=Trigger+Grip+Drag");
    assert_eq!(
        rig.eval("gnoblin.extensions.xr.room.layout.bindings.get('move').tracking"),
        "ray"
    );
    // 068: a ray move under the default id.
    let body = fence("068-ray-dragging-a-distant-panel.js.txt");
    rig.eval(&format!(
        "(() => {{ const xr = gnoblin.extensions.xr; const spatial = xr.room.layout; {body} return 1; }})()"
    ));
    assert_eq!(rig.eval(BINDINGS), "resize=Grip+Trigger+Drag,move=Grip+Drag");
    // 117 declares `xr` itself and assumes `spatial`.
    let body = fence("117-x-xr-preview-resize-defer-real-client-configure-until-.js.txt");
    rig.eval(&format!(
        "(() => {{ const spatial = gnoblin.extensions.xr.room.layout; {body} return 1; }})()"
    ));
    assert_eq!(rig.eval(BINDINGS), "move=Grip+Drag,resize=Grip+Trigger+Drag");
    assert_eq!(rig.eval("gnoblin.extensions.xr.room.layout.bindings.size"), "2");
}

#[test]
fn two_bindings_on_one_chord_are_offered_the_newest_first_and_a_decline_falls_through() {
    let mut rig = drag_world(
        "gnoblin.extensions.configure('xr', { layout: { triggers: { move: null, resize: null, rotate: null } } });",
    );
    rig.eval(
        "globalThis.log = [];
         globalThis.make = (name, take) => class extends XRControllerDragBinding {
             constructor() { super({ id: name, trigger: 'Grip+Drag' }); }
             onStart() { log.push(name); return take ? { n: 1 } : false; }
         };
         globalThis.bindings = gnoblin.extensions.xr.room.layout.bindings;
         bindings.add(new (make('old', true))());
         bindings.add(new (make('new', false))());
         1",
    );
    grip(&mut rig, 1.0);
    assert_eq!(
        rig.eval("log.join(',')"),
        "new,old",
        "the newest declines, so the older one takes it"
    );
    grip(&mut rig, 0.0);
    rig.eval("log.length = 0; bindings.add(new (make('new', true))()); 1");
    assert_eq!(rig.eval("bindings.size"), "2", "the same id replaced the binding");
    grip(&mut rig, 1.0);
    assert_eq!(
        rig.eval("log.join(',')"),
        "new",
        "the newest takes it and the older is not asked"
    );
}

#[test]
fn a_rotate_drag_turns_the_panel_about_its_own_centre_by_what_the_hand_turns_and_leaves_it_there() {
    let mut rig = drag_world("");
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    trigger_and_stick(&mut rig, true);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["trigger", "thumbstick"],
        "a rotate holds both buttons"
    );
    turn_hand(&mut rig, 0.5);
    rig.settle();
    near(yaw(&rig, view), 0.5, "the panel turns with the hand");
    assert_eq!(
        pose(&rig, view),
        [0.0, 1400.0, -1600.0],
        "about its own centre: it does not swing"
    );
    near(scale(&rig, view), 0.75, "and keeps its size");
    assert!(rig.comp.sets_of(WIN).is_empty(), "the client is untouched");
    turn_hand(&mut rig, -0.25);
    near(
        yaw(&rig, view),
        -0.25,
        "the turn is the hand's turn since the drag began, not a running sum",
    );
    trigger_and_stick(&mut rig, false);
    rig.settle();
    near(yaw(&rig, view), -0.25, "the panel stays as it was turned");
    assert!(flag(&mut rig, ROOM_VIEW_PINNED), "and the ring leaves it alone");
    assert!(claimed(&rig, RIGHT_HAND).is_empty());

    // Another drag starts from the panel's own turn: the hand's turn adds to it.
    turn_hand(&mut rig, 0.0);
    trigger_and_stick(&mut rig, true);
    turn_hand(&mut rig, 0.5);
    near(yaw(&rig, view), 0.25, "-0.25 and a half radian");
    // An untracked hand cancels it and puts the panel back, unpinned only if it was not pinned before.
    rig.comp
        .external_sets(RIGHT_HAND, vec![("tracked", MirrorValue::Bool(false))]);
    rig.frame();
    rig.settle();
    near(yaw(&rig, view), -0.25, "the cancel puts back the turn the drag found");
    assert!(
        flag(&mut rig, ROOM_VIEW_PINNED),
        "it was pinned by the first drag and still is"
    );
}

#[test]
fn a_scale_drag_adds_the_metres_the_hand_pushed_along_its_aim_and_stays_within_the_limits() {
    let mut rig = drag_world("");
    rig.eval("gnoblin.extensions.xr.room.layout.bindings.add(new ViewScaleBinding()); 1");
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    grip_and_stick(&mut rig, true);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip", "thumbstick"]);
    move_hand(&mut rig, 0.0, 0.0, -100.0);
    near(scale(&rig, view), 0.85, "100 mm along the aim is 0.1 added to 0.75");
    assert_eq!(pose(&rig, view), [0.0, 1400.0, -1600.0], "the panel does not move");
    move_hand(&mut rig, 500.0, 0.0, -100.0);
    near(scale(&rig, view), 0.85, "sideways travel adds nothing");
    move_hand(&mut rig, 0.0, 0.0, 1000.0);
    near(scale(&rig, view), 0.2, "pulling back past the limit stops at minScale");
    move_hand(&mut rig, 0.0, 0.0, -9000.0);
    near(scale(&rig, view), 3.0, "pushing out past the limit stops at maxScale");
    grip_and_stick(&mut rig, false);
    rig.settle();
    near(scale(&rig, view), 3.0, "the panel keeps the scale");
    assert!(flag(&mut rig, ROOM_VIEW_PINNED));
    assert!(rig.comp.sets_of(WIN).is_empty(), "the client is untouched");
}

#[test]
fn the_owners_move_binding_of_snippet_067_extends_the_extensions_drag_base_and_the_panel_stays_put() {
    let mut rig = drag_world("gnoblin.extensions.configure('xr', { layout: { triggers: { move: null } } });");
    let class = fence("067-xr-viewmovebinding-pure-js-6-dof-grab.js.txt");
    rig.eval(&format!(
        "globalThis.Owner = (() => {{ const xr = gnoblin.extensions.xr; {class} return ViewMoveBinding; }})();
         gnoblin.extensions.xr.room.layout.bindings.add(new Owner());
         1"
    ));
    assert!(flag(
        &mut rig,
        "new Owner() instanceof gnoblin.extensions.xr.ControllerDragBinding"
    ));
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["grip"],
        "the owner's binding claims its trigger's buttons too"
    );
    move_hand(&mut rig, 200.0, 50.0, 0.0);
    rig.settle();
    let [x, y, z] = pose(&rig, view);
    near(x, 200.0, "x follows the hand");
    near(y, 1450.0, "y follows the hand");
    near(z, -1600.0, "z");
    // The class never pins the panel; the runtime does, so the ring does not put it back a frame later.
    rig.settle();
    near(pose(&rig, view)[0], 200.0, "the ring left it alone");
    grip(&mut rig, 0.0);
    set_focus(&mut rig, WIN);
    near(
        pose(&rig, view)[0],
        200.0,
        "onEnd keeps the pose, even when the focus moves",
    );
    assert!(flag(&mut rig, ROOM_VIEW_PINNED));
    assert!(rig.comp.sets_of(WIN).is_empty());

    // A cancel goes through the owner's onCancel and gives the panel back to the ring if it was not pinned.
    rig.eval("gnoblin.extensions.xr.room.unpin(gnoblin.extensions.xr.room.views[0]); 1");
    rig.settle();
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 300.0, 0.0, 0.0);
    rig.comp
        .external_sets(RIGHT_HAND, vec![("tracked", MirrorValue::Bool(false))]);
    rig.frame();
    rig.settle();
    assert!(!flag(&mut rig, ROOM_VIEW_PINNED), "cancelled: back with the ring");
    near(pose(&rig, view)[0], 0.0, "and the ring placed it");
}

#[test]
fn the_owners_other_drag_bindings_of_snippets_069_to_071_define_over_the_extensions_base() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    for (file, class) in [
        (
            "069-xr-viewresizebinding-live-view-preview-client-resize-o.js.txt",
            "ViewResizeBinding",
        ),
        (
            "070-xr-view-rotation-and-scaling-small-fully-js.js.txt",
            "ViewRotateBinding",
        ),
        ("071-user-defined-throw-inertia-interaction.js.txt", "ThrowViewBinding"),
    ] {
        let body = fence(file);
        let got = rig.eval(&format!(
            "(() => {{ const xr = gnoblin.extensions.xr; {body} const one = new {class}(); \
             return (one instanceof xr.ControllerDragBinding) + ',' + one.parsed.kind; }})()"
        ));
        assert_eq!(got, "true,controller", "{class}");
    }
}

#[test]
fn a_virtual_output_is_checked_when_it_is_made_and_the_compositor_refuses_to_create_it_with_a_typed_error() {
    let mut rig = world(&[(800.0, 600.0)], "");
    xr_arrives(&mut rig);
    // Snippet 118 verbatim.
    let body = fence("118-y-head-locked-bingux-panel-without-coupling-bingux-to-.js.txt");
    let got = thrown(&mut rig, &format!("const {{ VirtualOutput }} = g; {body}"));
    assert_eq!(
        got,
        "GnoblinError|Collection.add: virtual outputs are not supported by this backend"
    );
    assert_eq!(
        rig.eval(
            "(() => { try { gnoblin.outputs.add(new g.VirtualOutput({ name: 'XR-HUD', width: 1600, height: 900 })); } \
             catch (e) { return e.code; } })()"
        ),
        "unsupported"
    );
    assert_eq!(rig.eval("[...gnoblin.outputs].length"), "1", "nothing was added");
    let hud = "new g.VirtualOutput({ name: 'XR-HUD', width: 1600, height: 900, scale: 2, attachment: 'viewer' })";
    assert_eq!(
        rig.eval(&format!(
            "(() => {{ const h = {hud}; return [h.name, h.width, h.height, h.scale, h.attachment].join(','); }})()"
        )),
        "XR-HUD,1600,900,2,viewer"
    );
    assert_eq!(
        rig.eval("new g.VirtualOutput({ name: 'A', width: 2, height: 3 }).attachment === null"),
        "true"
    );
    for (options, want) in [
        ("{ width: 1, height: 1 }", "TypeError|VirtualOutput: name"),
        ("{ name: '', width: 1, height: 1 }", "TypeError|VirtualOutput: name"),
        ("{ name: 'A', width: 0, height: 1 }", "RangeError|VirtualOutput: width"),
        (
            "{ name: 'A', width: 1.5, height: 1 }",
            "RangeError|VirtualOutput: width",
        ),
        (
            "{ name: 'A', width: 1, height: -2 }",
            "RangeError|VirtualOutput: height",
        ),
        (
            "{ name: 'A', width: 1, height: 1, scale: 0 }",
            "RangeError|VirtualOutput: scale",
        ),
        (
            "{ name: 'A', width: 1, height: 1, attachment: 'world' }",
            "TypeError|VirtualOutput: attachment",
        ),
    ] {
        let got = thrown(&mut rig, &format!("new g.VirtualOutput({options})"));
        assert!(got.starts_with(want), "{options}: {got}");
    }
    // Only a virtual output can be added, and the compositor's own outputs cannot be removed or cleared.
    let got = thrown(&mut rig, "gnoblin.outputs.add({ name: 'A' })");
    assert!(
        got.starts_with("TypeError|gnoblin.outputs.add: expected a VirtualOutput"),
        "{got}"
    );
    assert_eq!(rig.eval("gnoblin.outputs.remove([...gnoblin.outputs][0])"), "false");
    assert!(thrown(&mut rig, "gnoblin.outputs.clear()").starts_with("GnoblinError|Collection.clear"));
}
