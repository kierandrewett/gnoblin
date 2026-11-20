#![allow(unsafe_code)]
//! The window menu of `libgnoblin-std`, held to the recorded fixtures.
//!
//! Each test replays a scenario of `crates/gnoblin-js/tests/oracle/menus.rs` as a shell written over the object protocol: the same
//! windows, the same requests, keys and presses sent by the fake compositor, the same steps. What the fixture recorded of the TypeScript
//! run is what the native run must find at every step (`ui/rig.rs` says what is compared).
//!
//! The scenarios also started the window decorations (`startDecorations()`), which `libgnoblin-std` does not have natively yet (lane
//! NpStdFrame). The replay reads the fixture without what the decorations made (`Rig::foreign`: seven canvases a window, and the seat's
//! `decorations_available`), and the places where the fixture holds a number only a frame explains are named one by one (`Rig::allow_shared`
//! and the notes at each). The three titlebar clicks of `menus_at_the_pointer` are the titlebar's, so the replay makes the call the
//! TypeScript titlebar made.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::{Fail, Val};
use gnoblin_testkit::oracle::{Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{LEFT, RIGHT, Rig, W, b, callback, d, js, t};

/// What a window's decorations make: its titlebar, in seven canvases.
const FRAME: usize = 7;

/// A desktop of 1000 by 800 with `windows` windows as the oracle's menu scenarios had them (400 by 300), the desktop's workspaces
/// open, and the window menu started with the keyboard move and resize the policy gave the TypeScript shell.
fn shell(name: &'static str, windows: u64) -> (Rig, W) {
    shell_with(name, windows, &[], &[])
}

/// As [`shell`], for a scenario in which the TypeScript lock screen made canvases in the steps `steps` and set the `extra` properties.
fn shell_with(name: &'static str, windows: u64, steps: &[&str], extra: &[(u64, &str)]) -> (Rig, W) {
    let mut rig = Rig::with_plain_windows(name, 1000.0, 800.0, windows);
    let mut shared = vec![(21, "decorations_available")];
    shared.extend_from_slice(extra);
    rig.foreign(FRAME * usize::try_from(windows).expect("a count"), steps, &shared);
    let _workspaces = rig.make("workspaces", &[]);
    let menus = rig.make("window_menus", &[]);
    menus.call("start", &[]);
    for member in ["move_with_keys", "resize_with_keys"] {
        menus.set(member, callback(|_| Ok(Val::None)));
    }
    (rig, menus)
}

/// The open menu.
fn current(menus: &W) -> Option<W> {
    match menus.get("current") {
        Val::Object { class, obj } => Some(W {
            api: menus.api,
            class,
            obj,
        }),
        _ => None,
    }
}

/// The objects of a list property.
fn list(of: &W, name: &str) -> Vec<W> {
    of.get(name)
        .list()
        .expect("a list")
        .iter()
        .map(|value| match value {
            Val::Object { class, obj } => W {
                api: of.api,
                class: *class,
                obj: *obj,
            },
            other => panic!("{name} holds {other:?}"),
        })
        .collect()
}

/// What the scenario's `card()` read: where the card is, how big, whether it is secure, and its rows.
fn card(menus: &W) -> Value {
    let Some(menu) = current(menus) else {
        return Value::Null;
    };
    let panel = menu.obj("panel");
    let rows: Vec<Value> = list(&panel, "rows")
        .iter()
        .map(|row| json!([row.text("label"), row.flag("enabled"), row.flag("checked")]))
        .collect();
    json!({
        "x": js(panel.num("x")),
        "y": js(panel.num("y")),
        "width": js(panel.num("width")),
        "height": js(panel.num("height")),
        "secure": [panel.flag("secure"), menu.flag("secure")],
        "rows": rows,
    })
}

/// What the scenario's `sel()` read: the card the keyboard is in and the row it is on.
fn sel(menus: &W) -> Value {
    let Some(menu) = current(menus) else {
        return Value::Null;
    };
    let nav = menu.obj("nav");
    let depth = nav.get("depth").uint().expect("a depth");
    match nav.get("selected") {
        Val::Object { class, obj } => json!([
            depth,
            W {
                api: nav.api,
                class,
                obj
            }
            .text("label")
        ]),
        _ => json!([depth, Value::Null]),
    }
}

/// The compositor node of the open menu's layer.
fn layer(menus: &W) -> u64 {
    current(menus).expect("a menu is open").node()
}

/// The node of the row of the open menu with this label.
fn row(menus: &W, label: &str) -> u64 {
    let panel = current(menus).expect("a menu is open").obj("panel");
    list(&panel, "rows")
        .iter()
        .find(|row| row.text("label") == label)
        .unwrap_or_else(|| panic!("no row {label}"))
        .node()
}

fn panel_node(menus: &W) -> u64 {
    current(menus).expect("a menu is open").obj("panel").node()
}

/// A client asks for the menu of window `node` at (`x`, `y`), and the scenario reads what opened.
fn ask(rig: &mut Rig, menus: &W, node: u64, x: f64, y: f64, label: &str) {
    // The TypeScript toolkit asked again for the text of each widget it made and drew an estimate until the answer: nothing is answered
    // for the card this opens until the scenario says so.
    rig.unanswer_measures();
    rig.comp.request(node, "menu", x, y);
    rig.settle();
    rig.rec("card", card(menus));
    rig.rec("sel", sel(menus));
    rig.step(label);
}

/// A key pressed while the menu holds the keyboard, and the row it leaves selected.
fn key(rig: &mut Rig, menus: &W, name: &str) {
    rig.unanswer_measures();
    rig.comp.key(layer(menus), name, "", true);
    rig.settle();
    rig.rec("sel", sel(menus));
    rig.step(&format!("key_{name}"));
}

/// A key that typed a letter.
fn letter(rig: &mut Rig, menus: &W, name: &str, label: &str) {
    rig.unanswer_measures();
    rig.comp.key(layer(menus), name, name, true);
    rig.settle();
    rig.rec("sel", sel(menus));
    rig.step(label);
}

/// The pointer on `node`, and the row it leaves selected.
fn pointer(rig: &mut Rig, menus: &W, node: u64, kind: &str, at: (f64, f64), button: u64, label: &str) {
    rig.comp.pointer(node, kind, at.0, at.1, button);
    rig.settle();
    rig.rec("sel", sel(menus));
    rig.step(label);
}

/// The compositor changes a property of a window: a scenario's `set`.
fn set(rig: &mut Rig, node: u64, key: &str, value: MirrorValue) {
    rig.external(node, key, value);
    rig.settle();
}

fn flag(rig: &Rig, node: u64, key: &str) -> bool {
    rig.comp.flag(node, key).unwrap_or(false)
}

/// What the scenario's `uiCanvasCount()` read, as the TypeScript run counted it: the replay has no titlebars, so the canvases the two
/// windows' decorations made (seven each) are added back.
fn canvases(rig: &Rig, windows: usize) -> Value {
    let count = rig.function("ui", "canvas_count", &[]).uint().expect("a count");
    json!(count + u64::try_from(FRAME * windows).expect("a count"))
}

/// The nodes under a canvas the replay made for itself, so the comparison leaves them alone.
fn probe(rig: &mut Rig, canvas: &W) {
    rig.probe(canvas.node());
    for kid in canvas.kids() {
        probe(rig, &kid);
    }
}

// ---- the card -------------------------------------------------------------------------------------------------------------------------

#[test]
fn card_geometry_and_paint() {
    let (mut rig, menus) = shell("menus_card", 1);
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 300.0, 200.0, "open_at_anchor");
    rig.answer_measures();
    rig.step("measured");
    rig.tree("menu", layer(&menus));
    rig.paint("floating", layer(&menus));
    ask(&mut rig, &menus, 3, 990.0, 790.0, "open_near_the_corner");
    rig.answer_measures();
    rig.step("measured_again");
    rig.paint("near_corner", layer(&menus));
    ask(&mut rig, &menus, 3, 10.0, 10.0, "open_top_left");
    ask(&mut rig, &menus, 3, -50.0, -50.0, "open_off_screen");
    set(&mut rig, 3, "maximised", MirrorValue::Bool(true));
    rig.step("maximised");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "maximised_has_restore");
    rig.answer_measures();
    rig.step("measured_maximised");
    rig.paint("maximised", layer(&menus));
    set(&mut rig, 3, "maximised", MirrorValue::Bool(false));
    set(&mut rig, 3, "fullscreen", MirrorValue::Bool(true));
    rig.step("fullscreen");
    ask(
        &mut rig,
        &menus,
        3,
        50.0,
        50.0,
        "fullscreen_has_restore_and_no_workspace_move",
    );
    rig.answer_measures();
    rig.step("measured_fullscreen");
    rig.paint("fullscreen", layer(&menus));
    set(&mut rig, 3, "fullscreen", MirrorValue::Bool(false));
    set(&mut rig, 3, "secure", MirrorValue::Bool(true));
    rig.step("secure");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "secure_card");
    rig.tree("secure", layer(&menus));
    rig.finish();
}

// ---- the keyboard ----------------------------------------------------------------------------------------------------------------------

/// The TypeScript scenario's `{ do: .. }` for a key action, or null.
fn action_of(value: &Val) -> Value {
    let Val::Record { fields, .. } = value else {
        return Value::Null;
    };
    let kinds = ["escape", "move", "home", "end", "activate", "right", "left", "jump"];
    let Val::Enum { value: kind, .. } = &fields[0] else {
        panic!("a kind");
    };
    let mut out = json!({ "do": kinds[*kind as usize - 1] });
    if let Val::Int(step) = &fields[1] {
        out["step"] = json!(step);
    }
    if let Val::Text(letter) = &fields[2] {
        out["letter"] = json!(letter);
    }
    out
}

/// A function of the menu's rules.
fn metrics(rig: &Rig, name: &str, args: &[Val]) -> Val {
    rig.function("menu_metrics", name, args)
}

fn point(x: f64, y: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![d(x), d(y)],
    }
}

fn size(width: f64, height: f64) -> Val {
    point(width, height)
}

fn coordinates(value: &Val) -> Value {
    let Val::Record { fields, .. } = value else {
        panic!("a point");
    };
    json!({ "x": js(fields[0].double().expect("x")), "y": js(fields[1].double().expect("y")) })
}

#[test]
fn keyboard_navigation() {
    let (mut rig, menus) = shell("menus_keys", 1);
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open");
    for name in [
        "Down",
        "Down",
        "Tab",
        "Up",
        "ISO_Left_Tab",
        "Home",
        "End",
        "Down",
        "Up",
        "Up",
        "F5",
        "space",
        "BackSpace",
    ] {
        key(&mut rig, &menus, name);
    }
    letter(&mut rig, &menus, "c", "letter_c");
    letter(&mut rig, &menus, "m", "letter_m");
    letter(&mut rig, &menus, "m", "letter_m_again");
    rig.comp.key(layer(&menus), "Escape", "", false);
    rig.settle();
    rig.rec("sel", sel(&menus));
    rig.step("escape_released");
    key(&mut rig, &menus, "Escape");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_again");
    key(&mut rig, &menus, "Down");
    // The fixture's window is maximised to the work area less the titlebar's 28 pixels, which the frame's inset makes: the replay has
    // no frame, so the window takes the whole area. The library writes only what differs from the window's own, and the window was
    // at (0, 0) already, so the two positions the TypeScript run wrote are not written.
    rig.allow_shared("key_Return", 3, "x", json!(0.0), Value::Null);
    rig.allow_shared("key_Return", 3, "y", json!(28.0), Value::Null);
    rig.allow_shared("key_Return", 3, "height", json!(772.0), json!(800.0));
    key(&mut rig, &menus, "Return");
    rig.rec("maximised", json!(flag(&rig, 3, "maximised")));
    rig.step("maximised_by_return");

    let actions: Vec<Value> = [
        "Escape",
        "Down",
        "Tab",
        "Up",
        "ISO_Left_Tab",
        "Home",
        "End",
        "Return",
        "KP_Enter",
        "space",
        "Right",
        "Left",
        "F5",
    ]
    .iter()
    .map(|key| action_of(&metrics(&rig, "key_action", &[t(key), t("")])))
    .collect();
    rig.rec("actions", Value::Array(actions));
    rig.rec(
        "text_keys",
        json!([
            action_of(&metrics(&rig, "key_action", &[t("c"), t("c")])),
            action_of(&metrics(&rig, "key_action", &[t("x"), t(" ")])),
            action_of(&metrics(&rig, "key_action", &[t("ab"), t("ab")])),
        ]),
    );
    rig.settle();
    rig.step("key_actions");

    let entry = |label: &str| rig.make("menu_entry", &[("label", t(label))]);
    let labels = [
        "Maximise",
        "Minimise",
        "Move",
        "Resize",
        "Always on top",
        "Move to workspace",
        "Close",
    ];
    let rows: Vec<W> = labels
        .iter()
        .map(|label| rig.make("menu_item", &[("entry", entry(label).val())]))
        .collect();
    for item in &rows {
        probe(&mut rig, item);
    }
    let nav = rig.make(
        "menu_navigator",
        &[("rows", Val::List(rows.iter().map(W::val).collect()))],
    );
    let at = |nav: &W| match nav.get("selected") {
        Val::Object { class, obj } => json!(
            W {
                api: nav.api,
                class,
                obj
            }
            .text("label")
        ),
        _ => Value::Null,
    };
    let mut walked = Vec::new();
    nav.call("move", &[Val::Int(-1)]);
    walked.push(at(&nav));
    nav.call("home", &[]);
    walked.push(at(&nav));
    nav.call("end", &[]);
    walked.push(at(&nav));
    nav.call("move", &[Val::Int(1)]);
    walked.push(at(&nav));
    nav.call("move", &[Val::Int(-1)]);
    walked.push(at(&nav));
    for letter in ["c", "m", "m"] {
        nav.call("jump", &[t(letter)]);
        walked.push(at(&nav));
    }
    rig.rec("navigator", Value::Array(walked));
    let placed = |anchor: (f64, f64)| {
        coordinates(&metrics(
            &rig,
            "place_card",
            &[point(anchor.0, anchor.1), size(170.0, 220.0), size(1000.0, 800.0)],
        ))
    };
    rig.rec(
        "placement",
        json!([placed((300.0, 200.0)), placed((990.0, 790.0)), placed((-5.0, -5.0))]),
    );
    let make = |label: &str, separator: bool| {
        let entry = rig.make("menu_entry", &[("label", t(label))]);
        entry.set("separator", b(separator));
        entry.val()
    };
    let height = |entries: Vec<Val>| {
        metrics(&rig, "menu_height", &[Val::List(entries)])
            .double()
            .expect("a height")
    };
    rig.rec(
        "height",
        json!([
            js(height(vec![make("a", false), make("", true), make("b", false)])),
            js(height(Vec::new()))
        ]),
    );
    rig.settle();
    rig.step("navigator_and_metrics");
    rig.finish();
}

// ---- the pointer -----------------------------------------------------------------------------------------------------------------------

#[test]
fn pointer_selection_and_press() {
    let (mut rig, menus) = shell("menus_pointer", 1);
    rig.settle();
    rig.step("boot");
    set(&mut rig, 3, "maximised", MirrorValue::Bool(true));
    rig.step("maximised");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open");
    let (minimise, mv, close) = (row(&menus, "Minimise"), row(&menus, "Move"), row(&menus, "Close"));
    pointer(&mut rig, &menus, minimise, "enter", (4.0, 4.0), 0, "enter_minimise");
    pointer(&mut rig, &menus, mv, "enter", (4.0, 4.0), 0, "enter_disabled_move");
    pointer(&mut rig, &menus, mv, "press", (4.0, 4.0), LEFT, "press_disabled_move");
    let panel = panel_node(&menus);
    pointer(&mut rig, &menus, panel, "press", (1.0, 1.0), LEFT, "press_padding");
    pointer(&mut rig, &menus, close, "press", (5.0, 5.0), RIGHT, "right_press_entry");
    let backdrop = layer(&menus);
    pointer(&mut rig, &menus, backdrop, "press", (5.0, 5.0), LEFT, "press_outside");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_again");
    let backdrop = layer(&menus);
    pointer(
        &mut rig,
        &menus,
        backdrop,
        "press",
        (5.0, 5.0),
        RIGHT,
        "right_press_outside",
    );
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_third");
    let restore = row(&menus, "Restore");
    rig.comp.pointer(restore, "press", 5.0, 5.0, LEFT);
    rig.settle();
    rig.rec("maximised", json!(flag(&rig, 3, "maximised")));
    rig.rec("sel", sel(&menus));
    rig.step("press_restore");
    rig.finish();
}

// ---- the workspace submenu -------------------------------------------------------------------------------------------------------------

#[test]
fn workspace_submenu() {
    let (mut rig, menus) = shell("menus_submenu", 1);
    for id in ["1", "2", "3"] {
        let workspace = rig.make("workspace", &[("id", t(id))]);
        rig.make("workspaces", &[]).call("add", &[workspace.val()]);
    }
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open");
    key(&mut rig, &menus, "End");
    key(&mut rig, &menus, "Up");
    key(&mut rig, &menus, "Right");
    let menu = current(&menus).expect("open");
    let nav = menu.obj("nav");
    let sub = menu.obj("sub");
    let panel = menu.obj("panel");
    rig.rec(
        "submenu",
        json!({
            "depth": nav.get("depth").uint().expect("depth"),
            "rows": list(&nav, "sub_rows").iter().map(|row| json!([row.text("label"), row.flag("enabled"), row.flag("checked")])).collect::<Vec<_>>(),
            "x": js(sub.num("x")),
            "panel": [js(panel.num("x")), js(panel.num("width"))],
        }),
    );
    rig.answer_measures();
    rig.step("submenu_state");
    rig.tree("submenu", layer(&menus));
    rig.paint("submenu", layer(&menus));
    key(&mut rig, &menus, "Left");
    key(&mut rig, &menus, "Right");
    key(&mut rig, &menus, "Escape");
    rig.rec(
        "depth",
        json!(
            current(&menus)
                .expect("open")
                .obj("nav")
                .get("depth")
                .uint()
                .expect("depth")
        ),
    );
    rig.settle();
    rig.step("escape_in_submenu_goes_back");
    key(&mut rig, &menus, "Right");
    // The window leaves for a workspace that is not showing and is laid out there at the rectangle it has less its titlebar's 28
    // pixels, which the frame's inset makes.
    rig.allow_shared("key_Return", 3, "y", json!(28.0), json!(0.0));
    key(&mut rig, &menus, "Return");
    rig.rec("workspace", json!(workspace_id(&rig, 3)));
    rig.settle();
    rig.step("moved_to_workspace_2");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_on_the_new_workspace");
    rig.finish();
}

/// The id of the workspace window `node` is on.
fn workspace_id(rig: &Rig, node: u64) -> String {
    let state = rig.function("client_state", "of", &[t(&node.to_string())]);
    let Val::Object { class, obj } = state else {
        panic!("a client state");
    };
    W {
        api: rig.api,
        class,
        obj,
    }
    .obj("workspace")
    .text("id")
}

// ---- what the entries do ---------------------------------------------------------------------------------------------------------------

#[test]
fn entries_run() {
    let (mut rig, menus) = shell("menus_actions", 2);
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open");
    let panel = current(&menus).expect("open").obj("panel");
    rig.rec("above", json!(list(&panel, "rows")[4].flag("checked")));
    rig.settle();
    rig.step("not_pinned");
    key(&mut rig, &menus, "End");
    key(&mut rig, &menus, "Up");
    key(&mut rig, &menus, "Up");
    key(&mut rig, &menus, "Return");
    rig.rec(
        "above",
        json!(menus.call("is_above", &[t("3")]).bool().expect("a flag")),
    );
    rig.settle();
    rig.step("pinned");
    let other = client(&rig, 1);
    ccall(&rig, other, "raise", &[]);
    rig.advance(16);
    rig.step("raise_other_window");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_pinned");
    rig.answer_measures();
    rig.step("measured");
    rig.paint("pinned_tick", layer(&menus));
    key(&mut rig, &menus, "Escape");
    set(&mut rig, 3, "maximised", MirrorValue::Bool(true));
    rig.step("maximised");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_maximised");
    key(&mut rig, &menus, "Down");
    key(&mut rig, &menus, "Return");
    rig.rec("maximised", json!(flag(&rig, 3, "maximised")));
    rig.settle();
    rig.step("restored");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_for_minimise");
    key(&mut rig, &menus, "Down");
    key(&mut rig, &menus, "Down");
    key(&mut rig, &menus, "Return");
    rig.rec("minimised", json!(flag(&rig, 3, "minimised")));
    rig.settle();
    rig.step("minimised");
    set(&mut rig, 3, "minimised", MirrorValue::Bool(false));
    rig.step("unminimised");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_for_close");
    key(&mut rig, &menus, "End");
    key(&mut rig, &menus, "Return");
    assert_eq!(
        rig.comp
            .calls()
            .iter()
            .filter(|(node, method)| *node == 3 && method == "close")
            .count(),
        1,
        "Close asked the window to close"
    );
    rig.step("closed");
    rig.finish();
}

/// The window at `index` of `gnoblin.clients`, in the object model of the core.
fn client(rig: &Rig, index: i64) -> gnoblin_object::Obj {
    let class = rig.core.class("gnoblin").expect("the service class");
    let service = rig.core.create(class, &[]).expect("the service");
    let clients = cget(rig, service, "clients").object().expect("the clients");
    ccall(rig, clients, "at", &[Val::Int(index)])
        .object()
        .expect("a client")
}

fn cmember(rig: &Rig, obj: gnoblin_object::Obj, kind: u32, name: &str) -> u32 {
    let class = rig.core.class_of(obj).expect("the class of an object of the model");
    rig.core
        .member(class, kind, name)
        .unwrap_or_else(|fail| panic!("no member {name}: {fail}"))
}

fn cget(rig: &Rig, obj: gnoblin_object::Obj, name: &str) -> Val {
    rig.core
        .get(obj, cmember(rig, obj, abi::MEMBER_PROPERTY, name))
        .unwrap_or_else(|fail| panic!("reading {name}: {fail}"))
}

fn ccall(rig: &Rig, obj: gnoblin_object::Obj, name: &str, args: &[Val]) -> Val {
    rig.core
        .call(obj, cmember(rig, obj, abi::MEMBER_METHOD, name), args)
        .unwrap_or_else(|fail| panic!("calling {name}: {fail}"))
}

// ---- the menu's life -------------------------------------------------------------------------------------------------------------------

#[test]
fn two_menus_do_not_stack_and_a_window_that_goes_closes_its_menu() {
    let (mut rig, menus) = shell("menus_lifecycle", 2);
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_first");
    ask(&mut rig, &menus, 4, 400.0, 300.0, "open_second");
    rig.rec("canvases", canvases(&rig, 2));
    rig.step("two_menus_do_not_stack");
    ask(&mut rig, &menus, 4, 10.0, 10.0, "ask_again");
    rig.rec("canvases", canvases(&rig, 2));
    rig.step("same_count");
    rig.external_destroy(3);
    rig.settle();
    rig.rec("open", json!(current(&menus).is_some()));
    rig.step("other_window_closed");
    rig.external_destroy(4);
    rig.settle();
    rig.rec("open", json!(current(&menus).is_some()));
    rig.step("own_window_closed");
    rig.finish();
}

#[test]
fn a_secure_window_gets_a_secure_card_and_the_lock_closes_it() {
    // The lock screen the TypeScript runtime puts up when the session locks is NpStdShellUi's: its canvases, the lock's `engaged` and the
    // keyboard it takes are not compared.
    let (mut rig, menus) = shell_with("menus_secure_lock", 2, &["locking_closes"], &[(22, "engaged")]);
    rig.settle();
    rig.step("boot");
    set(&mut rig, 3, "secure", MirrorValue::Bool(true));
    rig.step("secure");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open_secure");
    ask(&mut rig, &menus, 4, 50.0, 50.0, "open_plain");
    set(&mut rig, 22, "state", MirrorValue::Text("pending".to_owned()));
    rig.rec("open", json!(current(&menus).is_some()));
    rig.step("locking_closes");
    ask(&mut rig, &menus, 4, 50.0, 50.0, "open_while_locked");
    set(&mut rig, 22, "state", MirrorValue::Text("not-locked".to_owned()));
    set(&mut rig, 4, "minimised", MirrorValue::Bool(true));
    rig.step("unlocked_and_minimised");
    ask(&mut rig, &menus, 4, 50.0, 50.0, "open_minimised");
    rig.finish();
}

/// A `menu_provider` the shell implements: the members it defines, by name, run `run` with their name.
fn provide(rig: &Rig, members: &[&str], run: impl FnMut(&str, &[Val]) -> Result<Val, Fail> + 'static) -> Val {
    rig.implement("menu_provider", members, run)
}

#[test]
fn a_shell_changes_the_entries_and_replaces_the_card() {
    let (mut rig, menus) = shell("menus_scripted", 1);
    let ran: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let comp = rig.comp.clone();
    let title_of = move |client: &str| match comp.value(client.parse().expect("a node"), "title") {
        Some(MirrorValue::Text(title)) => title,
        other => panic!("a title, got {other:?}"),
    };
    let only = rig.make("menu_entry", &[("label", t("Only"))]);
    let seen = Rc::clone(&ran);
    let title = title_of.clone();
    only.set(
        "run",
        callback(move |args| {
            seen.borrow_mut().push(title(args[0].text()?));
            Ok(Val::None)
        }),
    );
    let gap = rig.make("menu_entry", &[]);
    gap.set("separator", b(true));
    let off = rig.make("menu_entry", &[("label", t("Off"))]);
    off.set("enabled", b(false));
    let entries = Val::List(vec![only.val(), gap.val(), off.val()]);
    let items = entries.clone();
    menus.set("provider", provide(&rig, &["items"], move |_, _| Ok(items.clone())));
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open");
    rig.answer_measures();
    rig.step("measured");
    rig.paint("scripted", layer(&menus));
    key(&mut rig, &menus, "Down");
    key(&mut rig, &menus, "Return");
    rig.rec("ran", json!(ran.borrow().clone()));
    rig.settle();
    rig.step("ran");
    let shown: Rc<RefCell<Option<(String, f64, f64)>>> = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&shown);
    let items = entries;
    menus.set(
        "provider",
        provide(&rig, &["items", "show"], move |name, args| {
            if name == "show" {
                *seen.borrow_mut() = Some((title_of(args[0].text()?), args[1].double()?, args[2].double()?));
                Ok(Val::None)
            } else {
                Ok(items.clone())
            }
        }),
    );
    rig.settle();
    rig.step("show_replaced");
    ask(&mut rig, &menus, 3, 70.0, 80.0, "ask_show");
    let seen = shown.borrow().clone().map(|(title, x, y)| json!([title, js(x), js(y)]));
    rig.rec("seen", seen.unwrap_or(Value::Null));
    rig.settle();
    rig.step("show_seen");
    rig.finish();
}

#[test]
fn an_entry_that_throws_has_closed_the_menu_first() {
    let (mut rig, menus) = shell("menus_throwing_entry", 1);
    let boom = rig.make("menu_entry", &[("label", t("Boom"))]);
    boom.set("run", callback(|_| Err(Fail::err("boom"))));
    let items = Val::List(vec![boom.val()]);
    menus.set("provider", provide(&rig, &["items"], move |_, _| Ok(items.clone())));
    rig.settle();
    rig.step("boot");
    ask(&mut rig, &menus, 3, 50.0, 50.0, "open");
    key(&mut rig, &menus, "Down");
    key(&mut rig, &menus, "Return");
    rig.rec("open", json!(current(&menus).is_some()));
    rig.settle();
    rig.step("closed_before_the_entry_ran");
    rig.finish();
}

#[test]
fn the_menu_opens_at_the_pointer() {
    let (mut rig, menus) = shell("menus_at_the_pointer", 1);
    rig.settle();
    rig.step("boot");
    rig.external(21, "pointer_x", MirrorValue::Double(220.0));
    rig.external(21, "pointer_y", MirrorValue::Double(140.0));
    rig.settle();
    rig.step("pointer_moved");
    menus.call("show", &[t("3")]);
    rig.settle();
    rig.rec("card", card(&menus));
    rig.step("shown_at_the_pointer");
    key(&mut rig, &menus, "Escape");
    // The titlebar's clicks are the frame's: a middle click lowers the window and opens nothing; a right click makes the call below,
    // as `host/services.ts` made it when the seat says `menu` (its default); and with the seat saying `none` the titlebar makes none.
    rig.rec("open", json!(current(&menus).is_some()));
    rig.settle();
    rig.step("middle_click");
    menus.call("show", &[t("3")]);
    rig.settle();
    rig.rec("card", card(&menus));
    rig.step("right_click_opens");
    key(&mut rig, &menus, "Escape");
    rig.external(21, "right_click", MirrorValue::Text("none".to_owned()));
    rig.settle();
    rig.rec("open", json!(current(&menus).is_some()));
    rig.step("right_click_none");
    rig.finish();
}

#[test]
fn without_a_start_a_request_opens_nothing() {
    let mut rig = Rig::with_plain_windows("menus_not_started", 1000.0, 800.0, 1);
    rig.foreign(FRAME, &[], &[(21, "decorations_available")]);
    let _workspaces = rig.make("workspaces", &[]);
    let menus = rig.make("window_menus", &[]);
    rig.settle();
    rig.step("boot");
    rig.comp.request(3, "menu", 50.0, 50.0);
    rig.settle();
    rig.rec("started", json!(menus.flag("started")));
    rig.step("asked_without_a_menu");
    assert!(current(&menus).is_none());
    rig.finish();
}
