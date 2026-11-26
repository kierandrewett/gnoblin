#![allow(unsafe_code)]
//! XR of `libgnoblin-std` (`xr.toml`), held to what the TypeScript XR extension did (`crates/gnoblin-js/tests/xr.rs`, retired with it).
//!
//! The XR program is out of process, so what the shell sees of it is nodes: the seat named `xr`, the camera named `head` and
//! `controller` nodes. These tests make those nodes the way the program does (unparented, while the shell runs) over the fake compositor,
//! drive the library through the object protocol as a shell of any language does, and check what it builds and writes. The golden
//! numbers are the TypeScript cases'. No headset, OpenXR runtime or `gnoblin-xr` binary is involved: the real program is tested in its own
//! workspace (`extensions/gnoblin-xr`).
//!
//! Where the native API differs from the TypeScript one the case says so: a script's drag hooks are a `handler` (`xr_drag_handler`)
//! rather than a subclass, a claim's owner is a name, a layout's trigger left out is an empty string, the collection's lookup is `find`,
//! and `gnoblin.extensions.require("xr")` replaces the deep import `gnoblin-std/extensions/xr`. A focus of `arrange_ring` is an index.
//! The TypeScript case of `VirtualOutput` is core API, not XR, and is `a_virtual_output_is_checked_when_it_is_made_and_the_compositor_
//! refuses_to_create_it` of `crates/gnoblin-core-impl/tests/services.rs`.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, E_REFUSED, E_TYPE, MEMBER_METHOD, MEMBER_PROPERTY};
use gnoblin_core_impl::host;
use gnoblin_object::{Fail, Val};
use gnoblin_testkit::compositor::{ROOT, text};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};
use rig::{Rig, W, b, callback, d, t};

const SEAT: u64 = 21;
const XR_SEAT: u64 = 50;
const RIGHT_HAND: u64 = 60;
const LEFT_HAND: u64 = 61;
/// The first window's id: windows are 3, 4, 5, ...
const WIN: u64 = 3;

/// A point or a turn: x, y and z.
type Vec3 = [f64; 3];

// ---- the world ------------------------------------------------------------------------------------------------------------------

fn near(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

fn double(value: f64) -> MirrorValue {
    MirrorValue::Double(value)
}

fn rec(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

/// Runs the turns and asserts the library reported nothing.
fn quiet(rig: &mut Rig) {
    rig.settle();
    let reported = rig.reported();
    assert!(reported.is_empty(), "{reported:?}");
}

/// A shell over a screen, a seat, a lock and one window per `(width, height)` (ids 3 on, titled `win<n>`, app id `app<n>`).
fn world(sizes: &[(f64, f64)]) -> Rig {
    let sizes = sizes.to_vec();
    let mut rig = Rig::scratch_boot(move |comp| {
        comp.add_desktop(1000.0, 800.0);
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
    quiet(&mut rig);
    rig
}

/// The program's session starts: it makes the seat named `xr`.
fn xr_arrives(rig: &mut Rig) {
    rig.comp.appear(XR_SEAT, 0, "seat", vec![("name", text("xr"))]);
    quiet(rig);
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

fn wrap(rig: &Rig, value: Val) -> Option<W> {
    match value {
        Val::None => None,
        Val::Object { class, obj } => Some(W {
            api: rig.api,
            class,
            obj,
        }),
        other => panic!("an object or none, got {other:?}"),
    }
}

/// `gnoblin.extensions`.
fn ext(rig: &Rig) -> W {
    rig.make("extensions", &[])
}

/// `gnoblin.extensions.xr`, or none.
fn xr(rig: &Rig) -> Option<W> {
    wrap(rig, ext(rig).get("xr"))
}

fn session(rig: &Rig) -> W {
    xr(rig).expect("the extension").obj("session")
}

fn room_obj(rig: &Rig) -> W {
    xr(rig).expect("the extension").obj("room")
}

/// `gnoblin.extensions.xr.room.layout.bindings`.
fn bindings(rig: &Rig) -> W {
    room_obj(rig).obj("layout").obj("bindings")
}

fn binding(rig: &Rig, id: &str) -> W {
    wrap(rig, bindings(rig).call("find", &[t(id)])).unwrap_or_else(|| panic!("a binding {id}"))
}

/// The ids and triggers of the room layout's bindings, in order.
fn listed(rig: &Rig) -> String {
    bindings(rig)
        .get("list")
        .list()
        .expect("a list")
        .iter()
        .map(|item| {
            let item = wrap(rig, item.clone()).expect("a binding");
            format!("{}={}", item.text("id"), item.text("trigger"))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Makes an object of `class`, or the refusal.
fn try_make(rig: &Rig, class: &str, inits: &[(&str, Val)]) -> Result<W, Fail> {
    let class_id = rig.api.class(class).expect("the class");
    let ids: Vec<(u32, Val)> = inits
        .iter()
        .map(|(name, value)| {
            let member = rig
                .api
                .member(class_id, MEMBER_PROPERTY, name)
                .unwrap_or_else(|fail| panic!("{class} has no {name}: {fail}"));
            (member, value.clone())
        })
        .collect();
    rig.api.create(class_id, &ids).map(|obj| W {
        api: rig.api,
        class: class_id,
        obj,
    })
}

/// The class `name` is `class` or one it extends.
fn is_a(rig: &Rig, class: u32, name: &str) -> bool {
    let want = rig.api.class(name).expect("the class");
    let bases: HashMap<u32, u32> = rig
        .api
        .catalog()
        .expect("the catalog")
        .into_iter()
        .filter(|item| item.entry == abi::CATALOG_CLASS)
        .map(|item| (item.class, item.base))
        .collect();
    let mut at = class;
    while at != 0 {
        if at == want {
            return true;
        }
        at = bases.get(&at).copied().unwrap_or(0);
    }
    false
}

/// `gnoblin.extensions.configure("xr", { room, layout }, accept)`.
fn configure(rig: &Rig, room: Option<bool>, layout: Option<&W>, accept: Option<Val>) {
    ext(rig).call(
        "configure",
        &[
            t("xr"),
            rec(vec![room.map_or(Val::None, b), layout.map_or(Val::None, W::val)]),
            accept.unwrap_or(Val::None),
        ],
    );
}

/// A `spatial_triggers` record: `None` keeps the default, an empty string leaves the binding out.
fn triggers(move_: Option<&str>, resize: Option<&str>, rotate: Option<&str>) -> Val {
    let field = |value: Option<&str>| value.map_or(Val::None, t);
    rec(vec![field(move_), field(resize), field(rotate)])
}

/// A layout with `inits`, configured as the stock room's.
fn configure_layout(rig: &Rig, inits: &[(&str, Val)]) {
    let layout = rig.make("spatial_layout", inits);
    configure(rig, None, Some(&layout), None);
}

fn room_node(rig: &Rig) -> u64 {
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
        .children(room_node(rig))
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
    quiet(rig);
}

/// The first of the room's Views, as `room.views[0]`.
fn first_view(rig: &Rig) -> Val {
    room_obj(rig).get("views").list().expect("a list")[0].clone()
}

fn pinned(rig: &Rig) -> bool {
    room_obj(rig)
        .call("is_pinned", &[first_view(rig)])
        .bool()
        .expect("a flag")
}

/// A field of a binding's `parsed` (`controller_trigger`: text, kind, hand, buttons).
fn parsed_field(binding: &W, at: usize) -> String {
    binding.get("parsed").record().expect("a controller_trigger")[at]
        .text()
        .expect("text")
        .to_owned()
}

/// What a refusal says, without the code the client puts in front of it (`E_TYPE: `).
fn said(fail: &Fail) -> &str {
    match fail.message.split_once(": ") {
        Some((code, rest)) if code.starts_with("E_") => rest,
        _ => &fail.message,
    }
}

/// Whether a handle's object says it is gone: it answers `alive` false, or it ended with what it stood for.
fn gone(object: &W) -> bool {
    let member = object
        .api
        .member(object.class, MEMBER_PROPERTY, "alive")
        .expect("an alive member");
    match object.api.get(object.obj, member) {
        Ok(alive) => !alive.bool().expect("a flag"),
        Err(fail) => fail.code == abi::E_STALE,
    }
}

// ---- Absent: the desktop does not know XR exists ---------------------------------------------------------------------------------

#[test]
fn with_no_xr_program_the_extension_is_null_and_the_desktop_works_untouched() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)]);
    assert!(xr(&rig).is_none(), "no extension");
    quiet(&mut rig);
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

#[test]
fn require_fails_clearly_when_the_extension_is_absent() {
    let rig = world(&[]);
    let fail = ext(&rig)
        .try_call("require", &[t("xr")])
        .expect_err("the extension is absent");
    assert_eq!(fail.code, E_REFUSED);
    for needle in ["gnoblin.extensions.xr", "not available", "GNOBLIN_XR=1"] {
        assert!(
            fail.message.contains(needle),
            "the message names `{needle}`: {}",
            fail.message
        );
    }
}

#[test]
fn require_succeeds_when_the_session_declared_xr_or_the_program_already_has_a_session() {
    let with_seat = |name: &'static str| {
        Rig::scratch_boot(move |comp| {
            comp.add_desktop(1000.0, 800.0);
            comp.add_node(XR_SEAT, 0, "seat", vec![("name", text(name))]);
        })
    };
    let rig = with_seat("xr");
    ext(&rig).call("require", &[t("xr")]);
    drop(rig);
    let rig = with_seat("other");
    ext(&rig)
        .try_call("require", &[t("xr")])
        .expect_err("a seat with another name is no XR program");
    // SAFETY: the rig's process lock is held, and nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("GNOBLIN_XR", "1") };
    let declared = ext(&rig).try_call("require", &[t("xr")]);
    // SAFETY: as above.
    unsafe { std::env::remove_var("GNOBLIN_XR") };
    declared.expect("a declared session may require it before the headset starts");
}

#[test]
fn extensions_list_names_xr_with_its_status() {
    let mut rig = world(&[(800.0, 600.0)]);
    let status = |rig: &Rig| -> Vec<(String, u32)> {
        ext(rig)
            .get("list")
            .list()
            .expect("a list")
            .iter()
            .map(|item| {
                let fields = item.record().expect("an extension_info");
                (
                    fields[0].text().expect("an id").to_owned(),
                    fields[1].enum_value().expect("a status"),
                )
            })
            .collect()
    };
    // `extension_status`: ready is 1, unloaded 2.
    assert_eq!(status(&rig), [("xr".to_owned(), 2)]);
    xr_arrives(&mut rig);
    assert_eq!(status(&rig), [("xr".to_owned(), 1)]);
}

// ---- XR1: the class relations ------------------------------------------------------------------------------------------------------

#[test]
fn xr1_the_classes_relate_as_the_spec_says() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let class = |name: &str| rig.api.class(name).expect("the class");
    for (derived, base) in [
        ("xr_camera3d", "camera3d"),
        ("xr_camera3d", "scene_node"),
        ("xr_room", "scene_node"),
        ("view_move_binding", "xr_drag_binding"),
        ("view_resize_binding", "xr_drag_binding"),
        ("view_rotate_binding", "xr_drag_binding"),
        ("view_scale_binding", "xr_drag_binding"),
    ] {
        assert!(is_a(&rig, class(derived), base), "{derived} extends {base}");
    }
    let extension = xr(&rig).expect("the extension");
    assert_eq!(extension.class, class("xr_extension"));
    let session = session(&rig);
    assert_eq!(session.class, class("xr_session"));
    assert_eq!(extension.get("sessions").list().expect("a list").len(), 1);
    let viewport = session.obj("viewport");
    assert_eq!(viewport.class, class("xr_viewport"));
    let room = room_obj(&rig);
    assert_eq!(viewport.obj("content").obj, room.obj, "the viewport shows the room");
    let head = room.obj("head");
    assert_eq!(viewport.obj("camera").obj, head.obj, "through the room's head");
    assert!(is_a(&rig, head.class, "xr_camera3d"), "the head is an XR camera");
    // The session's input is the `xr` seat's own input context, not the primary one.
    let input = match session.get("input") {
        Val::Foreign { namespace, obj } if namespace == "gnoblin.model" => obj,
        other => panic!("the core's input context, got {other:?}"),
    };
    let core = rig.core;
    let seat = core
        .member(core.class("input_context").expect("the class"), MEMBER_PROPERTY, "seat")
        .expect("the seat");
    assert_eq!(core.get(input, seat).and_then(|value| value.uint()).ok(), Some(XR_SEAT));
    let bindings = bindings(&rig);
    assert_eq!(bindings.class, class("xr_bindings"));
    assert_eq!(binding(&rig, "move").class, class("view_move_binding"));
    assert_eq!(binding(&rig, "resize").class, class("view_resize_binding"));
}

#[test]
fn a_xr_camera_is_the_stereo_head_camera_and_converts_metres_to_the_cameras_units() {
    let mut rig = world(&[]);
    xr_arrives(&mut rig);
    let cam = rig.make("xr_camera3d", &[("near", d(0.05)), ("far", d(100.0))]);
    quiet(&mut rig);
    assert_eq!(cam.text("name"), "head");
    assert_eq!(rig.comp.value(cam.node(), "projection"), Some(text("stereo")));
    near(cam.num("metres"), 0.001, "metres");
    near(cam.num("near"), 50.0, "near is 0.05 m in millimetres");
    near(cam.num("far"), 100_000.0, "far is 100 m in millimetres");
    assert_eq!(cam.text("reference_space"), "local");
    let fail = try_make(&rig, "xr_camera3d", &[("reference_space", t("stage"))])
        .err()
        .expect("refused");
    assert!(
        fail.message.contains("stage") && fail.message.contains("local"),
        "{}",
        fail.message
    );
}

// ---- The room ------------------------------------------------------------------------------------------------------------------

#[test]
fn a_seat_with_another_name_builds_no_room() {
    let mut rig = world(&[(800.0, 600.0)]);
    rig.comp.appear(XR_SEAT, 0, "seat", vec![("name", text("seat1"))]);
    quiet(&mut rig);
    assert!(rig.comp.named("room").is_none());
    assert!(xr(&rig).is_none());
}

#[test]
fn the_seat_named_xr_builds_the_room_under_root_a_stereo_head_camera_in_millimetres_and_a_view_per_window() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)]);
    assert!(rig.comp.named("room").is_none(), "no room before the seat");
    xr_arrives(&mut rig);
    let room = room_node(&rig);
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
        room_obj(&rig).obj("head").num("metres"),
        0.001,
        "the room is in millimetres",
    );
    assert_eq!(rig.comp.ids_of_class("view").len(), 2, "one View per window");
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
    let session = session(&rig);
    assert!(session.flag("alive"));
    assert!(session.flag("connected"));
}

#[test]
fn a_seat_in_the_boot_snapshot_builds_the_room_on_the_first_frames() {
    let mut rig = Rig::scratch_boot(|comp| {
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(XR_SEAT, 0, "seat", vec![("name", text("xr"))]);
        comp.add_window(WIN, "one", "app");
    });
    quiet(&mut rig);
    assert!(rig.comp.named("room").is_some());
    assert!(view_of(&rig, WIN).is_some());
    assert!(xr(&rig).is_some());
}

#[test]
fn a_room_already_in_the_tree_is_the_users_and_is_left_alone() {
    let mut rig = Rig::scratch_boot(|comp| {
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(9, ROOT, "node", vec![("name", text("room"))]);
        comp.add_window(WIN, "one", "app");
    });
    quiet(&mut rig);
    xr_arrives(&mut rig);
    assert_eq!(rig.comp.ids_of_class("view").len(), 0, "no View is made");
    assert!(
        rig.comp.children(9).is_empty(),
        "the existing room gets no head camera or Views"
    );
    assert!(xr(&rig).expect("the extension").get("room").is_none());
    assert!(session(&rig).flag("alive"), "the session itself is still there");
}

#[test]
fn configure_room_false_builds_no_room_of_the_shells() {
    let mut rig = world(&[(800.0, 600.0)]);
    configure(&rig, Some(false), None, None);
    xr_arrives(&mut rig);
    assert!(rig.comp.named("room").is_none());
    assert!(session(&rig).flag("alive"));
    let fail = ext(&rig)
        .try_call("configure", &[t("xrr"), rec(vec![b(false)]), Val::None])
        .expect_err("an unknown extension");
    assert_eq!(fail.code, E_TYPE);
    assert!(
        fail.message.contains("xrr") && fail.message.contains("\"xr\""),
        "{}",
        fail.message
    );
}

#[test]
fn destroying_the_seat_ends_the_session_takes_the_room_down_and_the_windows_stay() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)]);
    xr_arrives(&mut rig);
    let held = session(&rig);
    let held_room = room_obj(&rig);
    assert!(rig.comp.named("room").is_some());
    rig.comp.destroy(XR_SEAT);
    quiet(&mut rig);
    assert!(rig.comp.named("room").is_none(), "the room is gone");
    assert!(rig.comp.ids_of_class("view").is_empty() && rig.comp.ids_of_class("camera").is_empty());
    assert!(xr(&rig).is_none(), "the extension is absent again");
    // No half-alive session: the object a script held says so and refuses writes.
    assert!(!held.flag("alive"));
    assert!(!held.flag("connected"));
    assert!(held.get("controllers").list().expect("a list").is_empty());
    assert!(gone(&held_room), "the stock room ended with the session");
    assert!(held.get("input").is_none());
    let content = rig.make("xr_room", &[]);
    let camera = rig.make("camera3d", &[]);
    let viewport = rig.make("xr_viewport", &[("content", content.val()), ("camera", camera.val())]);
    let fail = held.try_set("viewport", viewport.val()).expect_err("the session ended");
    assert!(fail.message.contains("ended"), "{}", fail.message);
    held_room.try_set("x", d(1.0)).expect_err("the room is destroyed");
    for window in [WIN, WIN + 1] {
        assert!(rig.comp.exists(window), "the window is still there");
        assert_eq!(rig.comp.parent_of(window), Some(ROOT));
    }
    content.destroy();
    camera.destroy();
    quiet(&mut rig);
    // A new seat is a new session with a new room.
    xr_arrives(&mut rig);
    assert!(rig.comp.named("room").is_some());
    assert_eq!(rig.comp.ids_of_class("view").len(), 2);
    let again = session(&rig);
    assert_ne!(again.obj, held.obj);
    assert!(again.flag("alive"));
}

#[test]
fn a_listener_hears_the_session_begin_and_end() {
    let mut rig = world(&[(800.0, 600.0)]);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let api = rig.api;
    let log = Rc::clone(&heard);
    let listener = rig.implement(
        "xr_listener",
        &["on_session_started", "on_session_ended"],
        move |name, args| {
            let session = args[0].object()?;
            let class = api.class_of(session)?;
            let alive = api.get(session, api.member(class, MEMBER_PROPERTY, "alive")?)?.bool()?;
            let what = if name == "on_session_started" { "start" } else { "end" };
            log.borrow_mut().push(format!("{what}:{alive}"));
            Ok(Val::None)
        },
    );
    ext(&rig).call("add_listener", &[listener]);
    xr_arrives(&mut rig);
    rig.comp.destroy(XR_SEAT);
    quiet(&mut rig);
    assert_eq!(heard.borrow().join(","), "start:true,end:false");
}

#[test]
fn a_window_that_opens_gets_a_view_follows_its_size_and_loses_it_when_destroyed() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    rig.comp.create_window(WIN + 1, "late", "late");
    quiet(&mut rig);
    let late = view_of(&rig, WIN + 1).expect("the window that opened has a View");
    near(rig.comp.number(late, "width"), 400.0, "width");
    near(rig.comp.number(late, "height"), 300.0, "height");
    rig.comp.external_set(WIN + 1, "width", double(1200.0));
    rig.comp.external_set(WIN + 1, "height", double(700.0));
    quiet(&mut rig);
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
    quiet(&mut rig);
    assert!(
        view_of(&rig, WIN).is_none(),
        "the compositor's destroy removes the View"
    );
    assert_eq!(rig.comp.ids_of_class("view").len(), 1);
}

#[test]
fn a_minimised_window_hides_its_view_lists_it_first_and_leaves_the_ring_until_shown_again() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0), (800.0, 600.0)]);
    xr_arrives(&mut rig);
    set_focus(&mut rig, WIN);
    let third = view_of(&rig, WIN + 2).expect("view");
    let before = rig.comp.number(third, "x");
    rig.comp.external_set(WIN + 1, "minimised", MirrorValue::Bool(true));
    quiet(&mut rig);
    let hidden = view_of(&rig, WIN + 1).expect("the View stays");
    assert_eq!(rig.comp.flag(hidden, "visible"), Some(false), "and is hidden");
    assert_eq!(shown(&rig)[0], WIN + 1, "a hidden View lists first: it draws nothing");
    assert!(
        (rig.comp.number(third, "x") - before).abs() > 1.0,
        "the other panels close the gap"
    );
    rig.comp.external_set(WIN + 1, "minimised", MirrorValue::Bool(false));
    quiet(&mut rig);
    assert_eq!(rig.comp.flag(hidden, "visible"), Some(true));
    near(rig.comp.number(third, "x"), before, "and the ring is as it was");
}

/// The TypeScript room assumed a dialog's node lay under its parent window's and gave it no View. The native room follows the tree's
/// rule instead: a dialog is a window of its own whose `parent_window` names its parent, and it gets a panel of its own under the
/// parent's, where it sits on the parent and just in front of it.
#[test]
fn a_dialog_gets_a_panel_of_its_own_on_its_parents_panel() {
    let mut rig = Rig::scratch_boot(|comp| {
        comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_window(WIN, "parent", "app");
        comp.add_node(
            WIN + 1,
            ROOT,
            "window",
            vec![
                ("parent_window", MirrorValue::Node(WIN)),
                ("x", double(150.0)),
                ("y", double(50.0)),
                ("width", double(200.0)),
                ("height", double(100.0)),
            ],
        );
    });
    quiet(&mut rig);
    xr_arrives(&mut rig);
    let parent = view_of(&rig, WIN).expect("the parent's panel");
    let dialog = view_of(&rig, WIN + 1).expect("the dialog's panel");
    assert_eq!(rig.comp.parent_of(dialog), Some(parent), "under the parent's panel");
    assert_eq!(shown(&rig), [WIN], "the ring holds the parent alone");
    // The parent is 400 by 300 at the origin, so its centre is (200, 150); the dialog's is (250, 100): 50 right and 50 up.
    let [x, y, z] = pose(&rig, dialog);
    near(x, 50.0, "x on the parent, in its pixels");
    near(y, 50.0, "y, up in the room");
    near(z, 2.0, "just in front of the parent");
    near(rig.comp.number(dialog, "width"), 200.0, "the dialog's size");
    rig.comp.destroy(WIN + 1);
    quiet(&mut rig);
    assert!(view_of(&rig, WIN + 1).is_none(), "the dialog's panel goes with it");
}

#[test]
fn the_accept_policy_keeps_a_window_out_of_the_room_and_a_change_of_mind_brings_it_in() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)]);
    let keep_out = Rc::new(RefCell::new(vec!["app2".to_owned()]));
    let held = Rc::clone(&keep_out);
    let accept = callback(move |args| {
        // `xr_client`: id, app_id, title, width, height, minimised, dialog.
        let fields = args[0].record()?;
        let app = fields[1].text()?.to_owned();
        Ok(Val::Bool(!held.borrow().contains(&app)))
    });
    configure(&rig, None, None, Some(accept));
    xr_arrives(&mut rig);
    assert!(view_of(&rig, WIN).is_some());
    assert!(view_of(&rig, WIN + 1).is_none(), "the policy leaves the window out");
    keep_out.borrow_mut().clear();
    // The policy is asked each turn; a change of the window's asks for one.
    rig.comp.external_set(WIN + 1, "title", text("again"));
    quiet(&mut rig);
    assert!(view_of(&rig, WIN + 1).is_some(), "it comes in when the policy lets it");
}

// ---- XR6: a secure client never appears in a View ----------------------------------------------------------------------------------

#[test]
fn xr6_a_secure_client_never_gets_a_view_and_one_that_turns_secure_loses_it() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0)]);
    rig.comp.external_set(WIN + 1, "secure", MirrorValue::Bool(true));
    quiet(&mut rig);
    xr_arrives(&mut rig);
    assert!(view_of(&rig, WIN).is_some());
    assert!(view_of(&rig, WIN + 1).is_none(), "the secure window has no View");
    rig.comp.external_set(WIN, "secure", MirrorValue::Bool(true));
    quiet(&mut rig);
    assert!(
        view_of(&rig, WIN).is_none(),
        "a window that turns secure loses its View"
    );
    rig.comp.external_set(WIN, "secure", MirrorValue::Bool(false));
    quiet(&mut rig);
    assert!(view_of(&rig, WIN).is_some(), "and gets one back when it stops");
    // A head-locked panel follows the same rule.
    let panel = wrap(&rig, room_obj(&rig).call("pin_to_head", &[rig.client(WIN), Val::None])).expect("a panel");
    quiet(&mut rig);
    assert!(rig.comp.ids_of_class("view").len() >= 2);
    rig.comp.external_set(WIN, "secure", MirrorValue::Bool(true));
    quiet(&mut rig);
    assert!(
        gone(&panel),
        "the head-locked panel of a window that turned secure is gone"
    );
}

// ---- The ring, with the retired shell's golden numbers -------------------------------------------------------------------------------

/// `SpatialLayout.arrangeRing` of `count` views of `width` by 600 with the focus at the index `focus`, flattened as
/// `x, y, z, yaw, scale, focused, behind` a pose and then the order.
fn ring(rig: &Rig, count: usize, width: f64, focus: Option<i64>) -> Vec<f64> {
    let views = Val::List((0..count).map(|_| rec(vec![d(width), d(600.0)])).collect());
    let result = rig.function(
        "spatial_layout",
        "arrange_ring",
        &[views, focus.map_or(Val::None, Val::Int), Val::None],
    );
    let fields = result.record().expect("a ring_result").to_vec();
    let mut out = Vec::new();
    for pose in fields[0].list().expect("the poses") {
        let p = pose.record().expect("a ring_pose");
        for part in &p[..5] {
            out.push(part.double().expect("a number"));
        }
        out.push(if p[5].bool().expect("focused") { 1.0 } else { 0.0 });
        out.push(p[6].uint().expect("behind") as f64);
    }
    for at in fields[1].list().expect("the order") {
        out.push(at.uint().expect("an index") as f64);
    }
    out
}

#[test]
fn the_ring_puts_one_panel_ahead_and_a_focused_one_larger_and_closer() {
    let rig = world(&[]);
    let plain = ring(&rig, 1, 800.0, None);
    for (got, want) in plain.iter().zip([0.0, 1400.0, -1600.0, 0.0, 0.75, 0.0, 0.0, 0.0]) {
        near(*got, want, "unfocused");
    }
    let focused = ring(&rig, 1, 800.0, Some(0));
    for (got, want) in focused.iter().zip([0.0, 1400.0, -1500.0, 0.0, 0.8625, 1.0, 0.0, 0.0]) {
        near(*got, want, "focused");
    }
    assert!(
        ring(&rig, 0, 800.0, None).is_empty(),
        "an empty list has an empty order"
    );
}

#[test]
fn three_panels_form_a_flat_arc_with_the_focused_one_at_the_front() {
    let rig = world(&[]);
    let got = ring(&rig, 3, 800.0, Some(1));
    let side = (
        703.836_493_990_485_2,
        -1_436.876_539_486_667_8,
        0.455_484_877_559_829_86,
    );
    let want = [
        -side.0, 1400.0, side.1, side.2, 0.75, 0.0, 0.0, // left
        0.0, 1400.0, -1500.0, 0.0, 0.8625, 1.0, 0.0, // front
        side.0, 1400.0, side.1, -side.2, 0.75, 0.0, 0.0, // right
        0.0, 2.0, 1.0, // the focused panel draws last
    ];
    assert_eq!(got.len(), want.len());
    for (at, (got, want)) in got.iter().zip(want).enumerate() {
        near(*got, want, &format!("entry {at}"));
    }
    // The ring is a carousel: focus on the first puts its neighbours on either side, wrapping round.
    let carousel = ring(&rig, 3, 800.0, Some(0));
    near(carousel[0], 0.0, "front x");
    near(carousel[7], side.0, "the next panel is on the right");
    near(carousel[14], -side.0, "and the one before wraps to the left");
    assert_eq!(&carousel[21..], &[1.0, 2.0, 0.0], "order");
}

#[test]
fn a_focus_that_names_no_view_puts_the_first_view_at_the_front_unenlarged() {
    let rig = world(&[]);
    let got = ring(&rig, 3, 800.0, Some(99));
    near(got[0], 0.0, "x");
    near(got[2], -1600.0, "z");
    near(got[4], 0.75, "scale");
}

#[test]
fn twelve_wide_panels_shrink_together_to_fit_150_degrees_to_either_side() {
    let rig = world(&[]);
    let got = ring(&rig, 12, 1920.0, Some(0));
    let scale = 0.291_994_247_481_941;
    near(got[4], 0.335_793_384_604_232_2, "the front panel's scale");
    near(got[7], 665.585_830_115_386_4, "first right x");
    near(got[11], scale, "scale");
    near(got[42], 1_029.273_332_867_44, "last right x");
    near(got[77], -665.585_830_115_386_4, "first left x");
    for i in 1..12 {
        let (x, z, yaw) = (got[i * 7], got[i * 7 + 2], got[i * 7 + 3]);
        near((x * x + z * z).sqrt(), 1600.0, &format!("radius of panel {i}"));
        assert!(
            yaw.abs() <= 150f64.to_radians() + 1e-9,
            "panel {i} is within 150 degrees"
        );
    }
    assert_eq!(
        got[84..],
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 0.0]
    );
}

#[test]
fn panels_past_the_twelfth_stack_behind_the_ring_and_a_focused_one_takes_the_rings_last_place() {
    let rig = world(&[]);
    let got = ring(&rig, 14, 800.0, Some(13));
    near(got[13 * 7 + 2], -1500.0, "the focused extra moves onto the ring");
    near(got[13 * 7 + 6], 0.0, "and is not behind");
    near(got[84], 769.583_616_070_915_5, "behind view 1: x");
    near(got[12 * 7 + 2], -1_682.332_029_616_988, "behind view 1: z");
    near(got[12 * 7 + 6], 1.0, "behind view 1: tier");
    assert_eq!(
        got[98..],
        [12.0, 11.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 13.0],
        "stacked panels draw first"
    );
    let tiers = ring(&rig, 25, 800.0, None);
    let radius = |at: usize| (tiers[at * 7].powi(2) + tiers[at * 7 + 2].powi(2)).sqrt();
    near(radius(12), 1850.0, "tier one is 250 mm behind the ring");
    near(radius(24), 2100.0, "tier two is 500 mm behind the ring");
}

#[test]
fn the_focused_window_moves_to_the_front_and_last_in_tree_order_and_the_ring_turns_with_the_focus() {
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0), (800.0, 600.0)]);
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
        -703.836_493_990_485_2,
        "the previous window is on the left",
    );
    near(
        pose(&rig, view_of(&rig, WIN + 2).expect("view"))[0],
        703.836_493_990_485_2,
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
    let mut rig = world(&[(800.0, 600.0); 14]);
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
    let mut rig = world(&[(800.0, 600.0), (800.0, 600.0), (800.0, 600.0)]);
    xr_arrives(&mut rig);
    rig.comp.clear_sent();
    rig.comp.external_set(WIN, "title", text("a new title"));
    quiet(&mut rig);
    let writes: Vec<TxOp> = rig
        .comp
        .ops()
        .into_iter()
        .filter(|op| {
            matches!(op, TxOp::Set { key, .. }
                if ["x", "y", "z", "rotation", "scale", "width", "height", "visible"].contains(&key.as_str()))
        })
        .collect();
    assert!(writes.is_empty(), "a title change writes no pose: {writes:?}");
}

#[test]
fn a_view_shows_a_client_without_moving_or_reparenting_it() {
    let mut rig = world(&[(800.0, 600.0)]);
    rig.comp.clear_sent();
    let v = rig.make("view", &[("width", d(640.0)), ("height", d(480.0)), ("x", d(5.0))]);
    v.call("show_client", &[rig.client(WIN)]);
    quiet(&mut rig);
    let view = rig.comp.ids_of_class("view")[0];
    assert_eq!(rig.comp.value(view, "source"), Some(MirrorValue::Node(WIN)));
    assert!(rig.comp.sets_of(WIN).is_empty(), "nothing is written to the window");
    assert_eq!(rig.comp.parent_of(WIN), Some(ROOT), "it is not reparented");
    // Many Views may show one client.
    let w = rig.make("view", &[]);
    w.call("show_client", &[rig.client(WIN)]);
    quiet(&mut rig);
    assert_eq!(rig.comp.ids_of_class("view").len(), 2);
    // Destroying the View leaves the client where it was.
    v.destroy();
    w.destroy();
    quiet(&mut rig);
    assert!(rig.comp.ids_of_class("view").is_empty());
    assert!(rig.comp.exists(WIN));
}

#[test]
fn a_head_locked_panel_is_a_view_under_the_head_camera_at_metres_in_front_of_the_eyes() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let options = rec(vec![d(0.1), d(-0.2), d(-1.0), Val::None]);
    let panel = wrap(&rig, room_obj(&rig).call("pin_to_head", &[rig.client(WIN), options])).expect("a panel");
    quiet(&mut rig);
    let head = rig.comp.named("head").expect("head");
    let node = rig.comp.children(head)[0];
    assert_eq!(rig.comp.value(node, "source"), Some(MirrorValue::Node(WIN)));
    near(rig.comp.number(node, "x"), 100.0, "x in millimetres");
    near(rig.comp.number(node, "y"), -200.0, "y");
    near(rig.comp.number(node, "z"), -1000.0, "z");
    near(rig.comp.number(node, "width"), 800.0, "the panel follows the window");
    rig.comp.external_set(WIN, "width", double(900.0));
    quiet(&mut rig);
    near(rig.comp.number(node, "width"), 900.0, "and follows its size");
    assert_eq!(
        rig.comp.ids_of_class("view").len(),
        2,
        "beside the ring's own View of the window"
    );
    rig.comp.destroy(WIN);
    quiet(&mut rig);
    assert!(gone(&panel), "the panel goes with its window");
}

// ---- The session's view of the head, the controllers and the layer budget ---------------------------------------------------------

fn buttons(rig: &Rig) -> String {
    let session = session(rig);
    let controller = wrap(rig, session.get("controllers").list().expect("a list")[0].clone()).expect("a controller");
    controller
        .get("buttons")
        .list()
        .expect("a list")
        .iter()
        .map(|name| name.text().expect("a name").to_owned())
        .collect::<Vec<_>>()
        .join("+")
}

#[test]
fn the_session_reads_the_head_the_layer_budget_and_the_controllers_from_the_tree() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let session = session(&rig);
    assert!(session.get("controllers").list().expect("a list").is_empty());
    // `xr_tracking`: tracked is 1, untracked 2.
    assert_eq!(session.get("tracking").enum_value().expect("an enum"), 2);
    assert_eq!(
        session.get("layer_budget").uint().expect("a budget"),
        0,
        "no budget yet"
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
    quiet(&mut rig);
    assert_eq!(session.get("layer_budget").uint().expect("a budget"), 16, "the budget");
    let pose = session.obj("head").get("pose");
    let fields = pose.record().expect("a pose");
    let vector = |value: &Val| -> Vec<f64> {
        value
            .record()
            .expect("a vec3")
            .iter()
            .map(|part| part.double().expect("a number"))
            .collect()
    };
    let mut got = vector(&fields[0]);
    got.extend(vector(&fields[1]));
    for (got, want) in got.iter().zip([100.0, 1500.0, -300.0, -0.1, 0.5, 0.2]) {
        near(*got, want, "the head pose in the placement frame");
    }
    let log = Rc::new(RefCell::new(Vec::new()));
    let api = rig.api;
    for signal in ["controller_added", "controller_removed"] {
        let member = api
            .member(session.class, abi::MEMBER_SIGNAL, signal)
            .expect("the signal");
        let log = Rc::clone(&log);
        let sign = if signal == "controller_added" { '+' } else { '-' };
        api.listen(session.obj, member, move |_, payload| {
            let controller = payload[0].object()?;
            let class = api.class_of(controller)?;
            let hand = api.get(controller, api.member(class, MEMBER_PROPERTY, "hand")?)?;
            log.borrow_mut().push(format!("{sign}{}", hand.text()?));
            let _ = api.release(controller);
            Ok(Val::None)
        })
        .expect("the listener");
    }
    hand(&rig, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    quiet(&mut rig);
    assert_eq!(session.get("controllers").list().expect("a list").len(), 1);
    assert_eq!(session.get("tracking").enum_value().expect("an enum"), 1);
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.8)), ("grip", double(0.5))]);
    quiet(&mut rig);
    assert_eq!(buttons(&rig), "Trigger");
    // A pull is down at 0.75 and up at 0.25, so a half pull neither presses nor chatters.
    rig.comp.external_sets(RIGHT_HAND, vec![("trigger", double(0.5))]);
    quiet(&mut rig);
    assert_eq!(buttons(&rig), "Trigger");
    rig.comp.external_sets(RIGHT_HAND, vec![("trigger", double(0.2))]);
    quiet(&mut rig);
    assert_eq!(buttons(&rig), "");
    rig.comp.destroy(RIGHT_HAND);
    quiet(&mut rig);
    assert_eq!(
        log.borrow().join(","),
        "+right,-right",
        "a removed controller still says which hand it was"
    );
}

#[test]
fn an_xr_trigger_is_parsed_in_one_canonical_spelling_and_a_bad_one_names_the_token() {
    let rig = world(&[]);
    let parse = |written: &str| -> Result<String, Fail> {
        let class = rig.api.class("view_move_binding").expect("the class");
        let member = rig
            .api
            .member(class, MEMBER_METHOD, "parse_trigger")
            .expect("the function");
        rig.api.call_static(class, member, &[t(written)]).map(|value| {
            value.record().expect("a controller_trigger")[0]
                .text()
                .expect("text")
                .to_owned()
        })
    };
    for (written, canonical) in [
        ("Grip+Drag", "Grip+Drag"),
        ("grip+trigger+drag", "Trigger+Grip+Drag"),
        ("Right.Grip+Drag", "Right.Grip+Drag"),
        ("Secondary+Primary+Drag", "Primary+Secondary+Drag"),
    ] {
        assert_eq!(parse(written).expect(written), canonical, "{written}");
    }
    parse("left.Trigger+right.Trigger+Drag").expect_err("two hands");
    for (written, token) in [
        ("Grip", "Grip"),
        ("Squeeze+Drag", "Squeeze"),
        ("Grip+Grip+Drag", "Grip"),
        ("Drag", "Drag"),
        ("Middle.Grip+Drag", "Middle.Grip"),
    ] {
        let fail = parse(written).expect_err(written);
        assert_eq!(fail.code, E_TYPE, "{written}");
        assert!(fail.message.contains(token), "{written}: {}", fail.message);
    }
    // An XR binding refuses a pointer trigger.
    let fail = try_make(&rig, "view_move_binding", &[("trigger", t("Super+MouseLeft+Drag"))])
        .err()
        .expect("refused");
    assert!(fail.message.contains("Super"), "{}", fail.message);
}

// ---- Dragging a panel ----------------------------------------------------------------------------------------------------------

/// A session with one 800 by 600 window whose panel is straight ahead, 1600 mm away (nothing is focused), and a right hand at the
/// origin. `before` runs before the session starts (the shell's configuration). The settle time is 0, so a drag starts on the frame
/// the buttons go down.
fn drag_world_with(before: impl FnOnce(&Rig)) -> Rig {
    let mut rig = world(&[(800.0, 600.0)]);
    before(&rig);
    xr_arrives(&mut rig);
    hand(&rig, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    session(&rig).set("chord_settle_ms", d(0.0));
    quiet(&mut rig);
    rig
}

fn drag_world() -> Rig {
    drag_world_with(|_| {})
}

/// A drag world whose layout has no stock `move` binding.
fn drag_world_without_move() -> Rig {
    drag_world_with(|rig| configure_layout(rig, &[("triggers", triggers(Some(""), None, None))]))
}

fn grip(rig: &mut Rig, value: f64) {
    rig.comp.external_sets(RIGHT_HAND, vec![("grip", double(value))]);
    quiet(rig);
}

fn move_hand(rig: &mut Rig, x: f64, y: f64, z: f64) {
    rig.comp
        .external_sets(RIGHT_HAND, vec![("x", double(x)), ("y", double(y)), ("z", double(z))]);
    quiet(rig);
}

fn turn_hand(rig: &mut Rig, yaw: f64) {
    rig.comp
        .external_sets(RIGHT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, yaw, 0.0]))]);
    quiet(rig);
}

fn lose_hand(rig: &mut Rig) {
    rig.comp
        .external_sets(RIGHT_HAND, vec![("tracked", MirrorValue::Bool(false))]);
    quiet(rig);
}

fn window_rect(rig: &Rig) -> Vec<Option<MirrorValue>> {
    ["x", "y", "width", "height"]
        .iter()
        .map(|key| rig.comp.value(WIN, key))
        .collect()
}

#[test]
fn xr3_a_grip_drag_moves_the_view_and_changes_nothing_of_the_client() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    assert_eq!(pose(&rig, view), [0.0, 1400.0, -1600.0], "the panel is straight ahead");
    let rect = window_rect(&rig);
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    // The hand moves 200 mm to the right and 50 up: the panel goes with it.
    move_hand(&mut rig, 200.0, 50.0, 0.0);
    let [x, y, z] = pose(&rig, view);
    near(x, 200.0, "x follows the hand");
    near(y, 1450.0, "y follows the hand");
    near(z, -1600.0, "z is as it was");
    near(yaw(&rig, view), 0.0, "the panel does not turn");
    near(scale(&rig, view), 0.75, "or change size");
    assert!(pinned(&rig));
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
    room_obj(&rig).call("unpin", &[first_view(&rig)]);
    quiet(&mut rig);
    near(pose(&rig, view)[2], -1500.0, "the ring places it again");
}

#[test]
fn a_hand_that_turns_turns_the_panel_about_it_and_keep_holds_the_panels_own_rotation() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    grip(&mut rig, 1.0);
    turn_hand(&mut rig, 0.5);
    let [x, y, z] = pose(&rig, view);
    near(x, -1600.0 * 0.5f64.sin(), "the panel swings round the hand");
    near(y, 1400.0, "y");
    near(z, -1600.0 * 0.5f64.cos(), "z");
    near(yaw(&rig, view), 0.5, "and turns with it");
    grip(&mut rig, 0.0);

    drop(rig);
    let mut kept = drag_world_without_move();
    let keep = kept.make("view_move_binding", &[("id", t("move")), ("rotation", rig::en(2))]);
    bindings(&kept).call("add", &[keep.val()]);
    let view = view_of(&kept, WIN).expect("view");
    grip(&mut kept, 1.0);
    turn_hand(&mut kept, 0.5);
    near(yaw(&kept, view), 0.0, "rotation keep leaves the panel's own turn");
    near(pose(&kept, view)[0], -1600.0 * 0.5f64.sin(), "but it still swings");
}

/// A move binding of the ray: `view_tracking` ray is 2, `view_rotation` keep is 2.
fn ray_move(rig: &Rig, trigger: &str) -> W {
    rig.make(
        "view_move_binding",
        &[
            ("id", t("move")),
            ("trigger", t(trigger)),
            ("tracking", rig::en(2)),
            ("rotation", rig::en(2)),
            ("distance", t("preserve")),
        ],
    )
}

#[test]
fn a_ray_drag_moves_a_distant_panel_along_the_ray_at_the_distance_it_was_grabbed() {
    let mut rig = drag_world_without_move();
    bindings(&rig).call("add", &[ray_move(&rig, "Grip+Drag").val()]);
    let view = view_of(&rig, WIN).expect("view");
    // Grab the panel 100 mm right of its centre, so the panel must not jump to the ray.
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    grip(&mut rig, 1.0);
    rig.comp.clear_sent();
    // Turn the hand 0.2 rad about y: the ray now points to the right of straight ahead.
    turn_hand(&mut rig, -0.2);
    let hit_distance = 1600.0;
    // The grabbed point sat at (100, 0, -1600) from a hand at (100, 0, 0): 1600 along the ray. It is now 1600 along the turned ray,
    // and the panel's centre keeps its offset (-100, 0, 0) from that point.
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
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    // Grab the top right: the ray lands 200 mm right of and 100 mm above the panel's centre.
    move_hand(&mut rig, 200.0, 100.0, 0.0);
    rig.comp.clear_sent();
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    quiet(&mut rig);
    // Drag outward in three steps. Every update changes the View and none touches the window.
    let mut last = scale(&rig, view);
    for (x, y) in [(300.0, 200.0), (400.0, 300.0), (350.0, 250.0)] {
        move_hand(&mut rig, x, y, 0.0);
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
    let k = (700.0f64 / (133.333_333_333_333_34 + 300.0))
        .max((533.333_333_333_333_4 + 400.0) / (266.666_666_666_666_7 + 400.0));
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
    quiet(&mut rig);
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
    assert_eq!(rig.comp.sets_of(WIN), ["width", "height"], "and it is the size alone");
    let kx = (533.333_333_333_333_4 + 400.0) / (266.666_666_666_666_7 + 400.0);
    let ky = 700.0 / (133.333_333_333_333_34 + 300.0);
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
    // The View follows the window's new size on the next turns, and nothing more is asked of it.
    quiet(&mut rig);
    assert_eq!(rig.comp.sets_of(WIN).len(), 2, "nothing else is written to the window");
}

#[test]
fn a_resize_that_keeps_the_aspect_and_the_density_asks_the_client_for_nothing_at_all() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    move_hand(&mut rig, 200.0, 150.0, 0.0);
    rig.comp.clear_sent();
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    quiet(&mut rig);
    // A 10% larger panel: the ray is 10% further from the anchor corner on each axis, so the aspect is the same and the client has the
    // pixels.
    move_hand(&mut rig, 250.0, 187.5, 0.0);
    near(scale(&rig, view), 0.75 * 1.1, "the preview is 10% larger");
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.0)), ("grip", double(0.0))]);
    quiet(&mut rig);
    assert!(
        rig.comp.sets_of(WIN).is_empty(),
        "no request on release: {:?}",
        rig.comp.ops()
    );
    near(scale(&rig, view), 0.75 * 1.1, "the View keeps the size it was given");
}

#[test]
fn a_drag_cancelled_by_an_untracked_hand_puts_the_view_back_and_no_client_is_asked_anything() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 300.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 300.0, "the panel is moved");
    lose_hand(&mut rig);
    assert_eq!(
        pose(&rig, view),
        [0.0, 1400.0, -1600.0],
        "the cancel puts the View back"
    );
    assert!(!pinned(&rig));
    assert!(rig.comp.sets_of(WIN).is_empty());
}

/// A script's drag hooks, logged: a `handler` that answers `start` with `answer` and fails an update with `boom` while `boom` is set.
struct Spy {
    log: Rc<RefCell<Vec<String>>>,
    boom: Rc<Cell<bool>>,
    handler: Val,
}

fn spy(rig: &Rig, answer: Val) -> Spy {
    let log = Rc::new(RefCell::new(Vec::new()));
    let boom = Rc::new(Cell::new(false));
    let updates = Rc::new(Cell::new(0));
    let (held, failing, api) = (Rc::clone(&log), Rc::clone(&boom), rig.api);
    let handler = rig.implement(
        "xr_drag_handler",
        &["on_start", "on_update", "on_end", "on_cancel"],
        move |name, args| {
            // `xr_drag_event`: timestamp, delta_time, controller, view, ray, hit, hand, trigger, room, unit.
            let event = rig::fields(args);
            let line = match name {
                "on_start" => {
                    updates.set(0);
                    format!("start:{}", !event[3].is_none())
                }
                "on_update" => {
                    updates.set(updates.get() + 1);
                    let controller = event[2].object()?;
                    let class = api.class_of(controller)?;
                    let claimed = api.get(controller, api.member(class, MEMBER_PROPERTY, "claimed")?)?;
                    let claimed: Vec<String> = claimed
                        .list()?
                        .iter()
                        .map(|name| name.text().map(str::to_owned))
                        .collect::<Result<_, _>>()?;
                    format!(
                        "update{}:{:.0}:{}",
                        updates.get(),
                        event[1].double()?,
                        claimed.join("+")
                    )
                }
                "on_end" => format!("end{}", updates.get()),
                _ => format!("cancel{}", updates.get()),
            };
            held.borrow_mut().push(line);
            if name == "on_update" && failing.get() && updates.get() == 2 {
                return Err(Fail::new(abi::E_ERR, "boom"));
            }
            Ok(if name == "on_start" { answer.clone() } else { Val::None })
        },
    );
    Spy { log, boom, handler }
}

impl Spy {
    fn take(&self) -> String {
        std::mem::take(&mut *self.log.borrow_mut()).join(",")
    }

    /// A plain drag binding of `trigger` with these hooks, added to the room's layout, and the turn the addition asks for run, so the
    /// next turn is the one a change of the hand asks for.
    fn bind(&self, rig: &mut Rig, id: &str, trigger: &str) {
        let made = rig.make(
            "xr_drag_binding",
            &[
                ("id", t(id)),
                ("trigger", t(trigger)),
                ("handler", self.handler.clone()),
            ],
        );
        bindings(rig).call("add", &[made.val()]);
        quiet(rig);
    }
}

#[test]
fn a_session_that_ends_mid_drag_cancels_it_once() {
    let mut rig = drag_world_without_move();
    let spy = spy(&rig, Val::None);
    spy.bind(&mut rig, "spy", "Grip+Drag");
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 10.0, 0.0, 0.0);
    rig.comp.destroy(XR_SEAT);
    quiet(&mut rig);
    assert_eq!(spy.take(), "start:true,update1:0:grip,cancel1");
}

#[test]
fn the_binding_lifecycle_is_start_update_end_with_one_update_a_frame_and_a_throw_cancels_only_that_drag() {
    let mut rig = drag_world_with(|rig| {
        configure_layout(rig, &[("triggers", triggers(Some(""), Some(""), None))]);
    });
    let spy = spy(&rig, Val::None);
    spy.bind(&mut rig, "spy", "Grip+Drag");
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 1.0, 0.0, 0.0);
    move_hand(&mut rig, 2.0, 0.0, 0.0);
    grip(&mut rig, 0.0);
    assert_eq!(spy.take(), "start:true,update1:0:grip,update2:0:grip,end2");
    // An update that throws is reported once and cancels that drag; the next press starts a fresh one.
    spy.boom.set(true);
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 3.0, 0.0, 0.0);
    rig.comp.external_sets(RIGHT_HAND, vec![("x", double(4.0))]);
    rig.settle();
    let reported = rig.reported();
    // The library names the hook that threw; the shell's own runtime reports what it threw.
    let threw = reported
        .iter()
        .filter(|line| line.contains("spy.onUpdate threw"))
        .count();
    assert_eq!(threw, 1, "the throw is reported once: {reported:?}");
    assert_eq!(spy.take(), "start:true,update1:0:grip,update2:0:grip,cancel2");
    spy.boom.set(false);
    grip(&mut rig, 0.0);
    grip(&mut rig, 1.0);
    assert_eq!(spy.take(), "start:true", "a fresh drag after the release");
}

#[test]
fn a_binding_disables_removes_and_replaces_like_any_binding() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    // Disabled: the grab does nothing.
    binding(&rig, "move").set("enabled", b(false));
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 0.0, "a disabled binding does not move the panel");
    grip(&mut rig, 0.0);
    move_hand(&mut rig, 0.0, 0.0, 0.0);
    // Removed.
    binding(&rig, "move").set("enabled", b(true));
    assert!(bindings(&rig).call("remove", &[t("move")]).bool().expect("a flag"));
    assert!(!bindings(&rig).call("has", &[t("move")]).bool().expect("a flag"));
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 0.0, "a removed binding does not move the panel");
    grip(&mut rig, 0.0);
    move_hand(&mut rig, 0.0, 0.0, 0.0);
    // Replaced by one on the left hand only: the right hand's grip does nothing, the left's moves the panel.
    let left = rig.make(
        "view_move_binding",
        &[
            ("id", t("move")),
            ("trigger", t("Left.Grip+Drag")),
            ("tracking", rig::en(2)),
            ("rotation", rig::en(2)),
        ],
    );
    bindings(&rig).call("add", &[left.val()]);
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    near(pose(&rig, view)[0], 0.0, "the right hand no longer grabs");
    grip(&mut rig, 0.0);
    hand(&rig, LEFT_HAND, "left", 0.0, 0.0, 0.0);
    quiet(&mut rig);
    rig.comp.external_sets(LEFT_HAND, vec![("grip", double(1.0))]);
    quiet(&mut rig);
    rig.comp
        .external_sets(LEFT_HAND, vec![("rotation", MirrorValue::List(vec![0.0, -0.2, 0.0]))]);
    quiet(&mut rig);
    assert!(
        pose(&rig, view)[0] > 100.0,
        "the left hand grabs by ray: {:?}",
        pose(&rig, view)
    );
    // A trigger can be moved: `trigger` is a setter that reparses.
    rig.comp.external_sets(LEFT_HAND, vec![("grip", double(0.0))]);
    quiet(&mut rig);
    let moved = binding(&rig, "move");
    moved.set("trigger", t("Right.Trigger+Drag"));
    assert_eq!(parsed_field(&moved, 0), "Right.Trigger+Drag");
}

/// A handler that only logs `name` when a drag starts and leaves the binding's own drag as it is.
fn logging(rig: &Rig, log: &Rc<RefCell<Vec<String>>>, name: &'static str) -> Val {
    let log = Rc::clone(log);
    rig.implement("xr_drag_handler", &["on_start"], move |_, _| {
        log.borrow_mut().push(name.to_owned());
        Ok(Val::None)
    })
}

#[test]
fn a_two_button_chord_that_goes_down_together_is_not_taken_for_its_first_button() {
    let mut rig = drag_world();
    let log = Rc::new(RefCell::new(Vec::new()));
    for id in ["move", "resize"] {
        binding(&rig, id).set("handler", logging(&rig, &log, id));
    }
    // Trigger and grip in one frame: a resize, never a move.
    rig.comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    quiet(&mut rig);
    assert_eq!(log.borrow().join(","), "resize");
    // With the default settle time a chord that is not yet stable has not started.
    drop(rig);
    let mut slow = world(&[(800.0, 600.0)]);
    xr_arrives(&mut slow);
    hand(&slow, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    quiet(&mut slow);
    near(session(&slow).num("chord_settle_ms"), 50.0, "the default settle time");
    let log = Rc::new(RefCell::new(Vec::new()));
    binding(&slow, "move").set("handler", logging(&slow, &log, "move"));
    slow.comp.external_sets(RIGHT_HAND, vec![("grip", double(1.0))]);
    quiet(&mut slow);
    assert!(
        log.borrow().is_empty(),
        "the grip has not been down for the settle time yet"
    );
}

#[test]
fn a_hand_that_points_at_nothing_starts_no_drag() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    // Aimed far to the side of the only panel.
    move_hand(&mut rig, 5000.0, 0.0, 0.0);
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 5100.0, 0.0, 0.0);
    assert_eq!(pose(&rig, view), [0.0, 1400.0, -1600.0], "no panel was under the ray");
    assert!(!pinned(&rig));
}

// ---- The snippets of docs/xr.md ----------------------------------------------------------------------------------------------------

#[test]
fn the_configure_snippet_of_the_docs_shapes_the_stock_room() {
    let mut rig = world(&[(800.0, 600.0)]);
    let layout = rig.make(
        "spatial_layout",
        &[
            ("radius", d(1.8)),
            ("triggers", triggers(Some("Right.Grip+Drag"), Some(""), None)),
        ],
    );
    configure(&rig, Some(true), Some(&layout), None);
    xr_arrives(&mut rig);
    let view = view_of(&rig, WIN).expect("view");
    near(rig.comp.number(view, "z"), -1800.0, "the ring's radius is 1.8 m");
    assert_eq!(parsed_field(&binding(&rig, "move"), 0), "Right.Grip+Drag");
    assert!(!bindings(&rig).call("has", &[t("resize")]).bool().expect("a flag"));
}

#[test]
fn the_viewport_snippet_of_the_docs_replaces_the_stock_room_and_names_the_nodes_for_the_program() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let stock = room_node(&rig);
    let session = session(&rig);
    let head = rig.make(
        "xr_camera3d",
        &[("tracking", session.get("head")), ("near", d(0.05)), ("far", d(100.0))],
    );
    let room = rig.make("xr_room", &[]);
    // `xr_composition`: auto is 1.
    let viewport = rig.make(
        "xr_viewport",
        &[
            ("content", room.val()),
            ("camera", head.val()),
            ("composition", rig::en(1)),
        ],
    );
    session.set("viewport", viewport.val());
    quiet(&mut rig);
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
    assert_eq!(
        session
            .obj("viewport")
            .get("composition")
            .enum_value()
            .expect("an enum"),
        1
    );
}

// ---- Claims: a drag takes its buttons from the window under the ray -------------------------------------------------------------------

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

/// Both hands tracked at the origin, a drag world's right hand being the one that grabs.
fn two_hands() -> Rig {
    let mut rig = drag_world();
    hand(&rig, LEFT_HAND, "left", 0.0, 0.0, 0.0);
    quiet(&mut rig);
    rig
}

fn controller(rig: &Rig, id: u64) -> W {
    wrap(rig, session(rig).call("controller", &[t(&id.to_string())])).expect("the controller")
}

fn names(list: &[&str]) -> Val {
    Val::List(list.iter().map(|name| t(name)).collect())
}

#[test]
fn a_grip_drag_claims_the_grip_of_the_hand_that_started_it_for_the_length_of_the_drag_and_no_longer() {
    let mut rig = two_hands();
    assert!(claimed(&rig, RIGHT_HAND).is_empty() && claimed(&rig, LEFT_HAND).is_empty());
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"], "the grip is the drag's");
    assert!(claimed(&rig, LEFT_HAND).is_empty(), "the other hand is not touched");
    assert!(rig.comp.sets_of(LEFT_HAND).is_empty());
    move_hand(&mut rig, 100.0, 0.0, 0.0);
    move_hand(&mut rig, 200.0, 0.0, 0.0);
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
    assert!(
        controller(&rig, RIGHT_HAND)
            .get("claimed")
            .list()
            .expect("a list")
            .is_empty()
    );
}

#[test]
fn a_ray_drag_and_a_resize_claim_the_buttons_of_their_triggers() {
    let mut rig = drag_world_without_move();
    bindings(&rig).call("add", &[ray_move(&rig, "Right.Grip+Drag").val()]);
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"], "a ray drag takes the grip too");
    grip(&mut rig, 0.0);
    assert!(claimed(&rig, RIGHT_HAND).is_empty());

    drop(rig);
    let mut resizing = drag_world();
    move_hand(&mut resizing, 200.0, 100.0, 0.0);
    resizing
        .comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(1.0)), ("grip", double(1.0))]);
    quiet(&mut resizing);
    assert_eq!(
        claimed(&resizing, RIGHT_HAND),
        ["trigger", "grip"],
        "a resize holds both buttons of its chord, in the node's order"
    );
    resizing
        .comp
        .external_sets(RIGHT_HAND, vec![("trigger", double(0.0)), ("grip", double(0.0))]);
    quiet(&mut resizing);
    assert!(claimed(&resizing, RIGHT_HAND).is_empty());
}

#[test]
fn a_hand_that_points_at_nothing_or_a_binding_that_declines_claims_nothing() {
    let mut rig = drag_world();
    // Out of reach of every panel: no View is under the ray, so no drag starts.
    move_hand(&mut rig, 100_000.0, 0.0, 0.0);
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 0);

    drop(rig);
    let mut declined = drag_world_with(|rig| {
        configure_layout(rig, &[("triggers", triggers(Some(""), Some(""), None))]);
    });
    let spy = spy(&declined, Val::Bool(false));
    spy.bind(&mut declined, "no", "Grip+Drag");
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
    let mut rig = drag_world();
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
    lose_hand(&mut rig);
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "an untracked hand's drag is cancelled"
    );

    // Cancelled: the drag's binding is removed from the room.
    drop(rig);
    let mut removed = drag_world();
    grip(&mut removed, 1.0);
    assert_eq!(claimed(&removed, RIGHT_HAND), ["grip"]);
    bindings(&removed).call("remove", &[t("move")]);
    quiet(&mut removed);
    assert!(
        claimed(&removed, RIGHT_HAND).is_empty(),
        "a removed binding holds nothing"
    );

    // Cancelled: the binding's hook throws, which is reported and ends only that drag.
    drop(removed);
    let mut crashed = drag_world_with(|rig| {
        configure_layout(rig, &[("triggers", triggers(Some(""), Some(""), None))]);
    });
    let spy = spy(&crashed, Val::None);
    spy.bind(&mut crashed, "spy", "Grip+Drag");
    spy.boom.set(true);
    grip(&mut crashed, 1.0);
    move_hand(&mut crashed, 1.0, 0.0, 0.0);
    crashed.comp.external_sets(RIGHT_HAND, vec![("x", double(2.0))]);
    crashed.settle();
    let reported = crashed.reported();
    assert!(
        reported.iter().any(|line| line.contains("spy.onUpdate threw")),
        "the throw is reported: {reported:?}"
    );
    assert!(
        claimed(&crashed, RIGHT_HAND).is_empty(),
        "a crashed drag does not keep the grip"
    );
    assert!(spy.take().ends_with(",cancel2"));

    // Cancelled: the session ends under the drag.
    drop(crashed);
    let mut ended = drag_world();
    grip(&mut ended, 1.0);
    assert_eq!(claimed(&ended, RIGHT_HAND), ["grip"]);
    ended.comp.destroy(XR_SEAT);
    quiet(&mut ended);
    assert!(
        claimed(&ended, RIGHT_HAND).is_empty(),
        "the end of the session releases what it claimed"
    );
}

#[test]
fn a_generation_that_ends_mid_drag_gives_the_claim_back_in_the_transaction_it_sends() {
    let mut rig = drag_world();
    grip(&mut rig, 1.0);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
    rig.comp.clear_sent();
    // What a reload and a shutdown do before the old generation goes: its shutdown hooks run.
    host::end_generation(rig.ctx);
    assert!(
        claimed(&rig, RIGHT_HAND).is_empty(),
        "the hold is released before the next generation runs"
    );
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 1, "in one write");
}

#[test]
fn two_claimants_on_one_hand_keep_their_own_and_the_node_says_the_union() {
    let mut rig = drag_world();
    let hand_obj = controller(&rig, RIGHT_HAND);
    let claim = |rig: &mut Rig, owner: &str, list: &[&str]| {
        hand_obj.call("claim", &[t(owner), names(list)]);
        quiet(rig);
    };
    let release = |rig: &mut Rig, owner: &str| {
        hand_obj.call("release", &[t(owner)]);
        quiet(rig);
    };
    claim(&mut rig, "panel", &["trigger"]);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["trigger"]);
    // The drag arrives while the script holds the trigger: both stand, whichever came first.
    grip(&mut rig, 1.0);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["trigger", "grip"],
        "the drag did not take the script's claim"
    );
    claim(&mut rig, "menu", &["thumbstick", "grip"]);
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
    release(&mut rig, "menu");
    assert_eq!(claimed(&rig, RIGHT_HAND), ["trigger"]);
    // A claim replaces the same owner's earlier one.
    claim(&mut rig, "panel", &["primary"]);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["primary"]);
    claim(&mut rig, "panel", &[]);
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    // Nothing changed, so nothing is written.
    rig.comp.clear_sent();
    release(&mut rig, "panel");
    release(&mut rig, "menu");
    claim(&mut rig, "panel", &[]);
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 0);
    assert!(claimed(&rig, LEFT_HAND).is_empty());
}

#[test]
fn a_claim_that_is_not_a_list_of_lowercase_input_names_is_a_typed_error_naming_the_controllers_claimed() {
    let mut rig = drag_world();
    let hand_obj = controller(&rig, RIGHT_HAND);
    rig.comp.clear_sent();
    for (owner, list, expect) in [
        ("me", names(&["Grip"]), "lowercase"),
        ("me", names(&["GRIP"]), "lowercase"),
        ("me", names(&["grip", "bumper"]), "not an input"),
        ("me", names(&[""]), "not an input"),
        ("", names(&["grip"]), "owner"),
    ] {
        let fail = hand_obj
            .try_call("claim", &[t(owner), list.clone()])
            .expect_err("refused");
        assert_eq!(fail.code, E_TYPE, "{list:?}");
        assert!(
            said(&fail).starts_with("Controller#60.claimed: "),
            "{list:?}: {}",
            fail.message
        );
        assert!(fail.message.contains(expect), "{list:?}: {}", fail.message);
    }
    // What is not a list of text the protocol refuses before the library sees it.
    for list in [t("grip"), Val::None, Val::List(vec![d(1.0)])] {
        hand_obj
            .try_call("claim", &[t("me"), list])
            .expect_err("not a list of names");
    }
    quiet(&mut rig);
    assert_eq!(claim_writes(&rig, RIGHT_HAND), 0, "a refused claim writes nothing");
    assert!(claimed(&rig, RIGHT_HAND).is_empty());
    // A good call still works afterwards, and the refused ones left nothing behind.
    hand_obj.call("claim", &[t("me"), names(&["grip"])]);
    quiet(&mut rig);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
}

#[test]
fn a_controller_that_is_gone_refuses_a_claim_and_forgets_a_release() {
    let mut rig = drag_world();
    let hand_obj = controller(&rig, RIGHT_HAND);
    hand_obj.call("claim", &[t("me"), names(&["grip"])]);
    quiet(&mut rig);
    assert_eq!(claimed(&rig, RIGHT_HAND), ["grip"]);
    rig.comp.destroy(RIGHT_HAND);
    quiet(&mut rig);
    assert!(!hand_obj.flag("alive"));
    let fail = hand_obj
        .try_call("claim", &[t("me"), names(&["trigger"])])
        .expect_err("gone");
    assert_eq!(fail.code, E_REFUSED);
    assert!(said(&fail).starts_with("Controller#60.claimed: "), "{}", fail.message);
    hand_obj
        .try_call("release", &[t("me")])
        .expect("a release of a gone controller is not an error");
}

// ---- The settle time and the program's hold ------------------------------------------------------------------------------------------

#[test]
fn the_chord_settle_time_is_a_number_of_milliseconds_the_programs_hold_can_cover() {
    let rig = drag_world();
    let session = session(&rig);
    let settle = |value: f64| {
        session.set("chord_settle_ms", d(value));
        session.num("chord_settle_ms")
    };
    near(settle(40.0), 40.0, "within the cap");
    near(
        settle(10_000.0),
        60.0,
        "a settle time past 60 ms would claim after the 150 ms the program holds a press for",
    );
    near(settle(-5.0), 0.0, "no less than nothing");
    near(settle(f64::INFINITY), 60.0, "infinity is the cap");
    let fail = session.try_set("chord_settle_ms", d(f64::NAN)).expect_err("NaN");
    assert!(
        said(&fail).starts_with("session.chordSettleMs must be a number"),
        "{}",
        fail.message
    );
    session.try_set("chord_settle_ms", t("50")).expect_err("text");
    near(session.num("chord_settle_ms"), 60.0, "a refused value changes nothing");
}

/// A world with the default settle time and a right hand at the origin, on the manual clock that stands still until the test moves it.
fn settling_world() -> Rig {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    hand(&rig, RIGHT_HAND, "right", 0.0, 0.0, 0.0);
    quiet(&mut rig);
    rig
}

/// Moves the clock `ms` milliseconds and runs the turns.
fn after(rig: &mut Rig, ms: u64) {
    rig.advance(ms);
    let reported = rig.reported();
    assert!(reported.is_empty(), "{reported:?}");
}

#[test]
fn a_drag_claims_its_grip_after_the_settle_time_and_not_on_the_press() {
    let mut rig = settling_world();
    near(session(&rig).num("chord_settle_ms"), 50.0, "the default");
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
    session(&rig).set("chord_settle_ms", d(10_000.0));
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

// ---- The specification's XR surface --------------------------------------------------------------------------------------------------

/// Presses or releases `button` and the thumbstick of the right hand together.
fn with_stick(rig: &mut Rig, button: &str, down: bool) {
    rig.comp.external_sets(
        RIGHT_HAND,
        vec![
            (button, double(if down { 1.0 } else { 0.0 })),
            ("thumbstick_click", MirrorValue::Bool(down)),
        ],
    );
    quiet(rig);
}

#[test]
fn the_extension_offers_the_drag_base_and_the_four_bindings_of_the_specs_hierarchy() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let rotate = rig.make("view_rotate_binding", &[]);
    assert_eq!(
        (rotate.text("id"), rotate.text("trigger")),
        ("rotate".to_owned(), "Trigger+Thumbstick+Drag".to_owned())
    );
    let scale = rig.make("view_scale_binding", &[]);
    assert_eq!(
        (scale.text("id"), scale.text("trigger")),
        ("scale".to_owned(), "Grip+Thumbstick+Drag".to_owned())
    );
    // `resize_corner`: quadrant is 1, top-left 2.
    assert_eq!(
        rig.make("view_resize_binding", &[])
            .get("edges")
            .enum_value()
            .expect("an enum"),
        1
    );
    assert_eq!(
        rig.make("view_resize_binding", &[("edges", rig::en(2))])
            .get("edges")
            .enum_value()
            .expect("an enum"),
        2
    );
    assert_eq!(
        binding(&rig, "rotate").class,
        rig.api.class("view_rotate_binding").expect("the class")
    );
    for inits in [
        vec![("min_scale", d(2.0)), ("max_scale", d(1.0))],
        vec![("max_scale", d(0.0))],
    ] {
        let fail = try_make(&rig, "view_scale_binding", &inits).err().expect("refused");
        assert_eq!(fail.code, E_TYPE);
        assert!(said(&fail).starts_with("ViewScaleBinding: "), "{}", fail.message);
    }
}

#[test]
fn the_spatial_layout_triggers_name_rotate_and_a_new_layout_replaces_the_stock_bindings() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    assert_eq!(
        listed(&rig),
        "move=Grip+Drag,resize=Grip+Trigger+Drag,rotate=Trigger+Thumbstick+Drag"
    );
    // Snippet 065: a layout of the spec's triggers, over the room the shell built.
    let spatial = rig.make(
        "spatial_layout",
        &[
            ("radius", d(1.6)),
            ("spacing", d(0.35)),
            (
                "triggers",
                triggers(
                    Some("Grip+Drag"),
                    Some("Grip+Trigger+Drag"),
                    Some("Trigger+Thumbstick+Drag"),
                ),
            ),
        ],
    );
    room_obj(&rig).set("layout", spatial.val());
    assert_eq!(
        listed(&rig),
        "move=Grip+Drag,resize=Grip+Trigger+Drag,rotate=Trigger+Thumbstick+Drag"
    );
    let layout = rig.make(
        "spatial_layout",
        &[(
            "triggers",
            triggers(Some(""), None, Some("Left.Trigger+Thumbstick+Drag")),
        )],
    );
    room_obj(&rig).set("layout", layout.val());
    assert_eq!(
        listed(&rig),
        "resize=Grip+Trigger+Drag,rotate=Left.Trigger+Thumbstick+Drag"
    );
    let layout = rig.make("spatial_layout", &[("triggers", triggers(None, None, Some("")))]);
    room_obj(&rig).set("layout", layout.val());
    assert_eq!(listed(&rig), "move=Grip+Drag,resize=Grip+Trigger+Drag");
    let fail = try_make(
        &rig,
        "spatial_layout",
        &[("triggers", triggers(None, None, Some("Trigger+Thumbstick")))],
    )
    .err()
    .expect("refused");
    assert_eq!(fail.code, E_TYPE);
    assert!(
        said(&fail).starts_with("SpatialLayout trigger \"rotate\""),
        "{}",
        fail.message
    );
}

#[test]
fn a_duplicate_binding_id_or_chord_on_the_spatial_layout_is_refused() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let already = |id: &str| format!("a binding with id \"{id}\" is already in this collection");
    // Snippets 066 and 068 add a `move` of their own beside the stock one, and 117 a `resize`: the owner has not decided whether `add`
    // replaces a binding that has the id, so it is refused, as in every layout.
    for (class, id, inits) in [
        (
            "view_move_binding",
            "move",
            vec![("trigger", t("Trigger+Grip+Drag")), ("tracking", rig::en(2))],
        ),
        (
            "view_move_binding",
            "move",
            vec![("trigger", t("Grip+Drag")), ("distance", t("preserve"))],
        ),
        (
            "view_resize_binding",
            "resize",
            vec![("trigger", t("Grip+Trigger+Drag")), ("pixels_per_metre", d(1200.0))],
        ),
    ] {
        let made = rig.make(class, &inits);
        let fail = bindings(&rig)
            .try_call("add", &[made.val()])
            .expect_err("a duplicate id");
        assert_eq!(fail.code, E_REFUSED);
        assert_eq!(said(&fail), already(id));
    }
    let other = rig.make(
        "view_move_binding",
        &[("id", t("other")), ("trigger", t("Grip+Trigger+Drag"))],
    );
    bindings(&rig)
        .try_call("add", &[other.val()])
        .expect_err("a second binding on the chord of `resize` at the same priority is refused");
    // Removing the stock binding first is the way to replace it.
    bindings(&rig).call("remove", &[t("move")]);
    bindings(&rig).call("add", &[ray_move(&rig, "Grip+Drag").val()]);
    assert_eq!(
        listed(&rig),
        "resize=Grip+Trigger+Drag,rotate=Trigger+Thumbstick+Drag,move=Grip+Drag"
    );
}

#[test]
fn a_rotate_drag_turns_the_panel_about_its_own_centre_by_what_the_hand_turns_and_leaves_it_there() {
    let mut rig = drag_world();
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    with_stick(&mut rig, "trigger", true);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["trigger", "thumbstick"],
        "a rotate holds both buttons"
    );
    turn_hand(&mut rig, 0.5);
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
    with_stick(&mut rig, "trigger", false);
    near(yaw(&rig, view), -0.25, "the panel stays as it was turned");
    assert!(pinned(&rig), "and the ring leaves it alone");
    assert!(claimed(&rig, RIGHT_HAND).is_empty());

    // Another drag starts from the panel's own turn: the hand's turn adds to it.
    turn_hand(&mut rig, 0.0);
    with_stick(&mut rig, "trigger", true);
    turn_hand(&mut rig, 0.5);
    near(yaw(&rig, view), 0.25, "-0.25 and a half radian");
    // An untracked hand cancels it and puts the panel back, unpinned only if it was not pinned before.
    lose_hand(&mut rig);
    near(yaw(&rig, view), -0.25, "the cancel puts back the turn the drag found");
    assert!(pinned(&rig), "it was pinned by the first drag and still is");
}

#[test]
fn a_scale_drag_adds_the_metres_the_hand_pushed_along_its_aim_and_stays_within_the_limits() {
    let mut rig = drag_world();
    let binding = rig.make("view_scale_binding", &[]);
    bindings(&rig).call("add", &[binding.val()]);
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    with_stick(&mut rig, "grip", true);
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
    with_stick(&mut rig, "grip", false);
    near(scale(&rig, view), 3.0, "the panel keeps the scale");
    assert!(pinned(&rig));
    assert!(rig.comp.sets_of(WIN).is_empty(), "the client is untouched");
}

/// Snippet 067's grab, as a handler: the panel follows the hand's change of place since the drag began. It moves the View through the
/// protocol, as a script of any language does.
#[test]
fn a_scripts_move_handler_on_the_drag_base_moves_the_panel_and_it_stays_put() {
    let mut rig = drag_world_without_move();
    let api = rig.api;
    // Where the hand and the panel were when the drag began.
    let start: Rc<Cell<Option<(Vec3, Vec3)>>> = Rc::new(Cell::new(None));
    let held = Rc::clone(&start);
    let read3 = move |obj: u64, name: &str| -> gnoblin_object::Res<Vec3> {
        let class = api.class_of(obj)?;
        let value = api.get(obj, api.member(class, MEMBER_PROPERTY, name)?)?;
        let fields = value.record()?;
        Ok([fields[0].double()?, fields[1].double()?, fields[2].double()?])
    };
    let handler = rig.implement(
        "xr_drag_handler",
        &["on_start", "on_update", "on_cancel"],
        move |name, args| {
            let event = rig::fields(args);
            let controller = event[2].object()?;
            let class = api.class_of(controller)?;
            let pose = api.get(controller, api.member(class, MEMBER_PROPERTY, "pose")?)?;
            let hand = pose.record()?[0]
                .record()?
                .iter()
                .map(Val::double)
                .collect::<Result<Vec<_>, _>>()?;
            let hand = [hand[0], hand[1], hand[2]];
            let view = event[3].object()?;
            match name {
                "on_start" => held.set(Some((hand, read3(view, "position")?))),
                "on_update" => {
                    let (from, at) = held.get().expect("a drag that started");
                    let to = Val::Record {
                        class: 0,
                        fields: (0..3).map(|i| Val::Double(at[i] + hand[i] - from[i])).collect(),
                    };
                    let class = api.class_of(view)?;
                    api.set(view, api.member(class, MEMBER_PROPERTY, "position")?, &to)?;
                }
                _ => {
                    let (_, at) = held.get().expect("a drag that started");
                    let back = Val::Record {
                        class: 0,
                        fields: at.iter().map(|part| Val::Double(*part)).collect(),
                    };
                    let class = api.class_of(view)?;
                    api.set(view, api.member(class, MEMBER_PROPERTY, "position")?, &back)?;
                }
            }
            Ok(Val::None)
        },
    );
    let owner = rig.make(
        "xr_drag_binding",
        &[("id", t("move")), ("trigger", t("Grip+Drag")), ("handler", handler)],
    );
    bindings(&rig).call("add", &[owner.val()]);
    let view = view_of(&rig, WIN).expect("view");
    rig.comp.clear_sent();
    grip(&mut rig, 1.0);
    assert_eq!(
        claimed(&rig, RIGHT_HAND),
        ["grip"],
        "the script's binding claims its trigger's buttons too"
    );
    move_hand(&mut rig, 200.0, 50.0, 0.0);
    let [x, y, z] = pose(&rig, view);
    near(x, 200.0, "x follows the hand");
    near(y, 1450.0, "y follows the hand");
    near(z, -1600.0, "z");
    // The handler never pins the panel; the runtime does, so the ring does not put it back a turn later.
    quiet(&mut rig);
    near(pose(&rig, view)[0], 200.0, "the ring left it alone");
    grip(&mut rig, 0.0);
    set_focus(&mut rig, WIN);
    near(
        pose(&rig, view)[0],
        200.0,
        "the end keeps the pose, even when the focus moves",
    );
    assert!(pinned(&rig));
    assert!(rig.comp.sets_of(WIN).is_empty());

    // A cancel goes through the handler's onCancel and gives the panel back to the ring if it was not pinned.
    room_obj(&rig).call("unpin", &[first_view(&rig)]);
    quiet(&mut rig);
    grip(&mut rig, 1.0);
    move_hand(&mut rig, 300.0, 0.0, 0.0);
    lose_hand(&mut rig);
    assert!(!pinned(&rig), "cancelled: back with the ring");
    near(pose(&rig, view)[0], 0.0, "and the ring placed it");
}

/// Snippets 069 to 071 define bindings of their own over the drag base: on the native surface each is a drag binding with a trigger of
/// its kind and the script's hooks in `handler`.
#[test]
fn a_scripts_bindings_over_the_drag_base_read_as_controller_triggers() {
    let mut rig = world(&[(800.0, 600.0)]);
    xr_arrives(&mut rig);
    let handler = rig.implement("xr_drag_handler", &["on_update"], |_, _| Ok(Val::None));
    for (id, trigger) in [
        ("resize", "Grip+Trigger+Drag"),
        ("rotate", "Trigger+Thumbstick+Drag"),
        ("throw", "Right.Grip+Drag"),
    ] {
        let made = rig.make(
            "xr_drag_binding",
            &[("id", t(id)), ("trigger", t(trigger)), ("handler", handler.clone())],
        );
        // `controller_trigger`: text, kind, hand, buttons.
        assert_eq!(parsed_field(&made, 1), "controller", "{id}");
        assert!(is_a(&rig, made.class, "xr_drag_binding"));
    }
}

// ---- change gating -------------------------------------------------------------------------------------------------------------------

/// How many callbacks one turn of the core ran.
fn turn_runs(rig: &Rig) -> i32 {
    // SAFETY: the core's table is static and `dispatch` takes plain values.
    unsafe { (host::api().dispatch)(rig.ctx, 0) }
}

#[test]
fn a_running_session_keeps_a_turn_every_frame_and_its_end_lets_the_loop_rest() {
    let mut rig = world(&[(800.0, 600.0)]);
    rig.clock.advance(std::time::Duration::from_millis(16));
    assert_eq!(turn_runs(&rig), 0, "with no session nothing is due");
    xr_arrives(&mut rig);
    for _ in 0..10 {
        rig.clock.advance(std::time::Duration::from_millis(16));
        assert!(turn_runs(&rig) > 0, "a session that is up skipped a frame");
    }
    rig.comp.destroy(XR_SEAT);
    quiet(&mut rig);
    assert!(rig.comp.named("room").is_none(), "the room came down");
    rig.clock.advance(std::time::Duration::from_millis(16));
    quiet(&mut rig);
    rig.clock.advance(std::time::Duration::from_millis(16));
    assert_eq!(turn_runs(&rig), 0, "with the session gone nothing is due");
}
