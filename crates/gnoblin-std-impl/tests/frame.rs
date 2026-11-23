#![allow(unsafe_code)]
//! The window frame of `libgnoblin-std`, held to what the TypeScript frame did (`data/js/gnoblin-std/frame/*`, `titlebar.ts`).
//!
//! Each test replays a scenario of `crates/gnoblin-js/tests/oracle/frame.rs` as a shell written over the object protocol: the same
//! decorations started with the same fill, the same windows opened by the fake compositor in the same order, the same pointer events and
//! the same steps. What the fixture recorded of the TypeScript run is what the native run must find at every step (`rig.rs` says what is
//! compared): the log, the nodes the shell made whole, the snapshots of the trees and the bytes of every paint list.
//!
//! The windows' policy is the policy lane's (NpStdPolicy), not the frame's. The replay does what the TypeScript library's policy did at
//! the moments the scenarios needed it, as a shell that keeps its own policy would write it: the seat's focus goes to a window that opens
//! or that a button went down on, `maximize` and `unmaximize` fill the work area less the frame and put the window back, and `minimize`
//! passes the focus on. They are the shell's closures (`Shell::hooks`), the `press`, `maximise`, `minimise`, `lower` and `menu` hooks of
//! the decorations.
//!
//! **Allowances of the replay**, each with its reason where it is made:
//!
//! 1. `frame_titlebar_clicks` is replayed through its step `middle_click_lowers`. The next step opens the window menu (`startWindowMenu`),
//!    which is the menus lane's (NpStdMenus), and every later step of the fixture holds the menu's canvases. What those steps tested of
//!    the frame, the seat's `double_click`, `middle_click` and `right_click` actions and an action the schema does not name, is asserted
//!    by `seat_actions` below.
//! 2. `frame_bound_node` bound a canvas's `visible` to the window's focus with `properties.bind`, which is the core's mechanism on a
//!    node of the TypeScript library. The replay follows the focus from the outlet's `on_refresh`, which reads the window once a frame as
//!    the binding does, and holds the same nodes.
//! 3. A fixture's window positions come from the TypeScript policy where it filled the work area (`maximize`) or kept a window inside the
//!    area; the replay's policy does the same arithmetic, so the rectangles are the fixture's.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi::MEMBER_PROPERTY;
use gnoblin_object::client::Api;
use gnoblin_object::{Fail, Val};
use gnoblin_testkit::oracle::{Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{LEFT, MIDDLE, RIGHT, Rig, W, b, callback, client_of, d, en, js, t};

type Comp = gnoblin_testkit::compositor::FakeCompositor;

// ---- the shell's side -----------------------------------------------------------------------------------------------------------

fn desktop(name: &'static str) -> Rig {
    Rig::new(name, 1000.0, 800.0)
}

/// An object a value holds.
fn wrap(api: Api, value: &Val) -> W {
    match value {
        Val::Object { class, obj } => W {
            api,
            class: *class,
            obj: *obj,
        },
        other => panic!("{other:?} is not an object"),
    }
}

/// Makes an object of `class` with `inits`, as a shell's `new Class({..})`.
fn make(api: Api, class: &str, inits: &[(&str, Val)]) -> W {
    let class_id = api.class(class).expect("the class");
    let mut ids = Vec::new();
    for (name, value) in inits {
        let member = api
            .member(class_id, MEMBER_PROPERTY, name)
            .unwrap_or_else(|fail| panic!("{class} has no {name}: {fail}"));
        ids.push((member, value.clone()));
    }
    let obj = api
        .create(class_id, &ids)
        .unwrap_or_else(|fail| panic!("a {class}: {fail}"));
    W {
        api,
        class: class_id,
        obj,
    }
}

/// A box with `inits`.
fn boxed(api: Api, inits: &[(&str, Val)]) -> W {
    make(api, "box", inits)
}

/// A function of a class.
fn function(api: Api, class: &str, name: &str, args: &[Val]) -> Val {
    let class = api.class(class).expect("the class");
    let member = api
        .member(class, gnoblin_core_abi::MEMBER_METHOD, name)
        .unwrap_or_else(|fail| panic!("{name}: {fail}"));
    api.call_static(class, member, args)
        .unwrap_or_else(|fail| panic!("{name}: {fail}"))
}

/// A property read that may fail, as a shell's `try { frame.top } catch`.
fn try_get(w: &W, name: &str) -> Result<Val, Fail> {
    let member = w.api.member(w.class, MEMBER_PROPERTY, name)?;
    w.api.get(w.obj, member)
}

/// The room of a `frame_inset` value: top, right, bottom, left.
fn inset_of(value: &Val) -> [f64; 4] {
    let fields = value.record().expect("an inset");
    [0, 1, 2, 3].map(|at| fields[at].double().expect("a number"))
}

/// What a script logs of an inset: `[top, right, bottom, left]`.
fn inset_json(value: &Val) -> Value {
    json!(inset_of(value).map(js))
}

/// The node a `frame_client` value is of.
fn node_of(value: &Val) -> u64 {
    let fields = value.record().expect("a client");
    fields[0].text().expect("an id").parse().expect("a node number")
}

/// The index of a field of `frame_client`.
const MAXIMISED: usize = 5;
const FOCUSED: usize = 3;

fn field_flag(client: &Val, at: usize) -> bool {
    client.record().expect("a client")[at].bool().expect("a flag")
}

/// A property of a window, `default` while the compositor has not written it.
fn prop(comp: &Comp, node: u64, key: &str, default: f64) -> f64 {
    match comp.borrow().nodes.get(&node).and_then(|entry| entry.props.get(key)) {
        Some(MirrorValue::Double(value)) => *value,
        _ => default,
    }
}

fn rect_of(comp: &Comp, node: u64) -> [f64; 4] {
    ["x", "y", "width", "height"].map(|key| prop(comp, node, key, 0.0))
}

/// The windows' policy the replay plays: where the maximised windows floated, and the clock the actions are timed by.
struct Policy {
    comp: Comp,
    deco: W,
    core: Api,
}

impl Policy {
    /// `windowState.maximise`: keeps the rectangle the window floated at and fills the work area less the frame.
    fn maximise(&self, node: u64) {
        let before = rect_of(&self.comp, node);
        let state = function(self.deco.api, "client_state", "of", &[client_of(self.core, node)]);
        wrap(self.deco.api, &state).set("floating_bounds", rect_val(before[0], before[1], before[2], before[3]));
        self.comp.external_set(node, "maximised", MirrorValue::Bool(true));
        let [top, right, bottom, left] = inset_of(&self.deco.call("inset_of", &[client_of(self.core, node)]));
        for (key, value) in [
            ("x", left),
            ("y", top),
            ("width", 1000.0 - left - right),
            ("height", 800.0 - top - bottom),
        ] {
            self.comp.external_set(node, key, MirrorValue::Double(value));
        }
    }

    /// `windowState.unmaximise`: leaves maximised and puts the window back where it floated. What the script kept moved with the frame
    /// (`keepFramePlace` of the TypeScript host), and the library keeps the same rectangle (`floating_bounds`), so it is read from there.
    fn unmaximise(&self, node: u64) {
        self.comp.external_set(node, "maximised", MirrorValue::Bool(false));
        let state = wrap(
            self.deco.api,
            &function(self.deco.api, "client_state", "of", &[client_of(self.core, node)]),
        );
        let kept = state.get("floating_bounds");
        if let Val::Record { fields, .. } = kept {
            for (key, at) in [("x", 0), ("y", 1), ("width", 2), ("height", 3)] {
                self.comp
                    .external_set(node, key, MirrorValue::Double(fields[at].double().expect("a number")));
            }
        }
    }
}

/// What the decorations run: the shell's decorations object, the policy of the windows and the clock.
struct Shell {
    deco: W,
    clock: Rc<Cell<f64>>,
    core: Api,
}

impl Shell {
    /// `startDecorations(options)` of the TypeScript library: the hooks of the policy and `setup`, then `start`.
    fn start(rig: &mut Rig, setup: impl FnOnce(&W)) -> Shell {
        let deco = make(rig.api, "decorations", &[]);
        let clock = Rc::new(Cell::new(0.0));
        Shell::hooks(rig, &deco, &clock);
        setup(&deco);
        deco.call("start", &[]);
        rig.settle();
        Shell {
            deco,
            clock,
            core: rig.core,
        }
    }

    /// The window policy the TypeScript host handed the frames: `press`, `maximise`, `minimise`, `lower` and `menu`, and the requests
    /// the compositor makes of a window.
    fn hooks(rig: &Rig, deco: &W, clock: &Rc<Cell<f64>>) {
        let policy = Rc::new(Policy {
            comp: rig.comp.clone(),
            deco: *deco,
            core: rig.core,
        });
        let comp = rig.comp.clone();
        deco.set(
            "press",
            callback(move |args| {
                // The focus policy hears a press on a window's frame as it hears one on the window.
                comp.external_set(21, "focus", MirrorValue::Node(node_of(&args[0])));
                Ok(Val::None)
            }),
        );
        let held = Rc::clone(&policy);
        deco.set(
            "maximise",
            callback(move |args| {
                let node = node_of(&args[0]);
                if args[1].bool()? {
                    held.maximise(node);
                } else {
                    held.unmaximise(node);
                }
                Ok(Val::None)
            }),
        );
        let comp = rig.comp.clone();
        deco.set(
            "minimise",
            callback(move |args| {
                comp.external_set(node_of(&args[0]), "minimised", MirrorValue::Bool(true));
                // The keyboard goes to another window, or to nobody.
                comp.external_set(21, "focus", MirrorValue::Node(0));
                Ok(Val::None)
            }),
        );
        // Lowering a window is a restack of the compositor's, which no property of the tree shows; the window menu is the menus lane's.
        deco.set("lower", callback(move |_| Ok(Val::None)));
        deco.set("menu", callback(move |_| Ok(Val::None)));
        let time = Rc::clone(clock);
        deco.set("now", callback(move |_| Ok(d(time.get()))));
        // `maximize` and `unmaximize` are answered as the policy answers them.
        let answer = callback(move |args| {
            let request = args[0].record().expect("a request");
            let node = request[0].uint()?;
            match request[1].text()? {
                "maximize" => policy.maximise(node),
                "unmaximize" => policy.unmaximise(node),
                _ => {}
            }
            Ok(Val::None)
        });
        rig.function("ui", "on_request", &[answer]);
    }

    /// The clock the actions are timed by moves on with the shell's.
    fn advance(&self, rig: &mut Rig, ms: u64) {
        self.clock.set(self.clock.get() + ms as f64);
        rig.advance(ms);
    }

    /// `client.frameInset` of a window.
    fn inset(&self, node: u64) -> Val {
        self.deco.call("inset_of", &[client_of(self.core, node)])
    }

    /// The log entry `inset` of the scenarios: the frame inset of window 3.
    fn log_inset(&self, rig: &Rig) {
        rig.rec("inset", inset_json(&self.inset(3)));
    }

    /// The log entry `inset` of the scenarios that log `client.frameInset` itself: an object.
    fn log_inset_record(&self, rig: &Rig) {
        let [top, right, bottom, left] = inset_of(&self.inset(3));
        rig.rec(
            "inset",
            json!({ "top": js(top), "right": js(right), "bottom": js(bottom), "left": js(left) }),
        );
    }
}

// ---- the compositor's side -----------------------------------------------------------------------------------------------------

/// The compositor maps a window and the focus policy gives it the keyboard.
fn open(rig: &mut Rig, id: u64, title: &str, app_id: &str) {
    rig.comp.create_window(id, title, app_id);
    rig.external(21, "focus", MirrorValue::Node(id));
    rig.settle();
}

fn open_decorated(rig: &mut Rig, id: u64, title: &str, app_id: &str, decoration: &str) {
    rig.comp.create_window_decorated(id, title, app_id, decoration);
    rig.external(21, "focus", MirrorValue::Node(id));
    rig.settle();
}

/// The compositor changes a property of a window.
fn set(rig: &mut Rig, id: u64, key: &str, value: MirrorValue) {
    rig.external(id, key, value);
    rig.settle();
}

/// The node `path` (a child index at each step) under the window `id`.
fn under(rig: &Rig, id: u64, path: &[usize]) -> u64 {
    let mut node = id;
    for at in path {
        node = *rig
            .comp
            .children(node)
            .get(*at)
            .unwrap_or_else(|| panic!("node {node} has no child {at}"));
    }
    node
}

fn pointer(rig: &mut Rig, at: u64, kind: &str, x: f64, y: f64, button: u64) {
    rig.comp.pointer(at, kind, x, y, button);
    rig.settle();
}

fn request(rig: &mut Rig, id: u64, name: &str) {
    rig.comp.request(id, name, 0.0, 0.0);
    rig.settle();
}

/// `[x, y, width, height]` of window 3, as a script logs it.
fn log_bounds(rig: &Rig) {
    rig.rec("bounds", json!(rect_of(&rig.comp, 3).map(js)));
}

/// `[x, y, width, height, opacity, maximised, minimised]` of every window, as the titlebar scenarios log them.
fn log_states(rig: &Rig, ids: &[u64]) {
    let list: Vec<Value> = ids
        .iter()
        .map(|id| {
            let [x, y, width, height] = rect_of(&rig.comp, *id);
            json!([
                js(x),
                js(y),
                js(width),
                js(height),
                js(prop(&rig.comp, *id, "opacity", 1.0)),
                rig.comp.flag(*id, "maximised").unwrap_or(false),
                rig.comp.flag(*id, "minimised").unwrap_or(false)
            ])
        })
        .collect();
    rig.rec("bounds", Value::Array(list));
}

/// What the fill of every scenario that has one starts with: a client-drawn window gets no frame.
fn server_only(frame: &W) -> bool {
    frame.flag("server")
}

// ---- scenarios ------------------------------------------------------------------------------------------------------------------

#[test]
fn zones() {
    let mut rig = desktop("frame_zones");
    let api = rig.api;
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !server_only(&frame) {
                    return Ok(b(false));
                }
                frame.obj("top").set("size", d(28.0));
                frame
                    .obj("start")
                    .add(boxed(api, &[("width", d(40.0)), ("height", d(16.0))]));
                frame
                    .obj("center")
                    .add(boxed(api, &[("grow", d(1.0)), ("height", d(16.0))]));
                frame
                    .obj("end")
                    .add(boxed(api, &[("width", d(20.0)), ("height", d(16.0))]));
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    rig.step("opened");
    rig.tree("frame", 3);
    rig.finish();
}

#[test]
fn stock_parts() {
    let mut rig = desktop("frame_parts");
    let api = rig.api;
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !server_only(&frame) {
                    return Ok(b(false));
                }
                frame.obj("top").set("size", d(28.0));
                let start = frame.obj("start");
                let buttons = make(
                    api,
                    "window_buttons",
                    &[("frame", frame.val()), ("buttons", Val::List(vec![en(3), en(1)]))],
                );
                buttons.set("gap", d(6.0));
                start.add(buttons);
                let center = frame.obj("center");
                center.add(make(api, "title_text", &[("frame", frame.val())]));
                let end = frame.obj("end");
                let icon = make(api, "app_icon", &[("frame", frame.val())]);
                icon.set("size", d(16.0));
                end.add(icon);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "org.example.Files");
    open(&mut rig, 4, "other", "org.gnome.Nautilus");
    open(&mut rig, 5, "three", "firefox");
    open(&mut rig, 6, "four", "");
    rig.answer_measures();
    rig.step("opened");
    rig.tree("frame", 3);
    for (label, id) in [("files", 3), ("nautilus", 4), ("firefox", 5), ("no_app_id", 6)] {
        let top = under(&rig, id, &[0]);
        rig.paint(label, top);
    }
    set(&mut rig, 3, "title", MirrorValue::Text("renamed".to_owned()));
    rig.answer_measures();
    rig.step("renamed");
    let top = under(&rig, 3, &[0]);
    rig.paint("renamed", top);
    rig.finish();
}

#[test]
fn stock_titlebar() {
    let mut rig = desktop("frame_titlebar");
    let shell = Shell::start(&mut rig, |_| {});
    rig.step("boot");
    open(&mut rig, 3, "terminal", "foot");
    rig.answer_measures();
    log_states(&rig, &[3]);
    rig.step("opened");
    rig.tree("frame", 3);
    let bar = under(&rig, 3, &[0]);
    rig.paint("unfocused", bar);
    set(&mut rig, 3, "focused", MirrorValue::Bool(true));
    rig.answer_measures();
    rig.step("focused");
    rig.paint("focused", bar);
    set(
        &mut rig,
        3,
        "title",
        MirrorValue::Text(
            "A very long window title that cannot fit in the room the drag area has left over for it".to_owned(),
        ),
    );
    rig.answer_measures();
    rig.step("long_title");
    rig.paint("long_title", bar);
    set(&mut rig, 3, "width", MirrorValue::Double(200.0));
    rig.answer_measures();
    rig.step("narrow");
    rig.paint("narrow", bar);
    rig.external(3, "width", MirrorValue::Double(400.0));
    set(&mut rig, 3, "title", MirrorValue::Text("terminal".to_owned()));
    rig.answer_measures();
    rig.step("restored");
    request(&mut rig, 3, "maximize");
    rig.answer_measures();
    log_states(&rig, &[3]);
    rig.step("maximised");
    rig.paint("maximised", bar);
    request(&mut rig, 3, "unmaximize");
    log_states(&rig, &[3]);
    rig.step("unmaximised");
    set(&mut rig, 3, "fullscreen", MirrorValue::Bool(true));
    rig.step("fullscreen");
    rig.paint("fullscreen", bar);
    set(&mut rig, 3, "fullscreen", MirrorValue::Bool(false));
    rig.step("windowed");

    // The buttons: maximise toggles, minimise hides, close fades the window and then closes it.
    let minimise = under(&rig, 3, &[0, 0, 1]);
    let maximise = under(&rig, 3, &[0, 0, 2]);
    let close = under(&rig, 3, &[0, 0, 3]);
    pointer(&mut rig, maximise, "press", 4.0, 4.0, LEFT);
    log_states(&rig, &[3]);
    rig.step("maximise_pressed");
    pointer(&mut rig, maximise, "press", 4.0, 4.0, RIGHT);
    rig.step("maximise_right_pressed");
    pointer(&mut rig, maximise, "press", 4.0, 4.0, LEFT);
    log_states(&rig, &[3]);
    rig.step("maximise_pressed_again");
    pointer(&mut rig, minimise, "press", 4.0, 4.0, LEFT);
    log_states(&rig, &[3]);
    rig.step("minimise_pressed");
    set(&mut rig, 3, "minimised", MirrorValue::Bool(false));
    rig.step("unminimised");
    pointer(&mut rig, close, "press", 4.0, 4.0, LEFT);
    log_states(&rig, &[3]);
    rig.step("close_pressed");
    shell.advance(&mut rig, 100);
    log_states(&rig, &[3]);
    rig.step("fading");
    shell.advance(&mut rig, 100);
    log_states(&rig, &[3]);
    rig.step("fade_ended");
    shell.advance(&mut rig, 1500);
    log_states(&rig, &[3]);
    rig.step("restored_after_staying");
    pointer(&mut rig, close, "press", 4.0, 4.0, LEFT);
    pointer(&mut rig, close, "press", 4.0, 4.0, LEFT);
    shell.advance(&mut rig, 200);
    rig.step("close_twice");
    rig.external_destroy(3);
    rig.settle();
    rig.forget("closed", &["n1", "n2", "n3", "n4", "n5", "n6", "n7"]);
    rig.step("closed");
    rig.finish();
}

#[test]
fn titlebar_clicks_and_drag() {
    let mut rig = desktop("frame_titlebar_clicks");
    let shell = Shell::start(&mut rig, |_| {});
    rig.step("boot");
    // A press on the bar itself: the stock titlebar is a drag area, and a press on its title reaches it too.
    open(&mut rig, 3, "terminal", "foot");
    rig.answer_measures();
    log_states_plain(&rig);
    rig.step("opened");
    let drag = under(&rig, 3, &[0, 0]);
    pointer(&mut rig, drag, "press", 20.0, 10.0, LEFT);
    pointer(&mut rig, drag, "motion", 50.0, 25.0, 0);
    log_states_plain(&rig);
    rig.step("dragged");
    pointer(&mut rig, drag, "release", 50.0, 25.0, LEFT);
    rig.step("released");
    shell.advance(&mut rig, 1000);
    pointer(&mut rig, drag, "press", 20.0, 10.0, LEFT);
    shell.advance(&mut rig, 200);
    pointer(&mut rig, drag, "press", 20.0, 10.0, LEFT);
    log_states_plain(&rig);
    rig.step("double_click_toggles_maximise");
    shell.advance(&mut rig, 1000);
    pointer(&mut rig, drag, "press", 20.0, 10.0, MIDDLE);
    rig.step("middle_click_lowers");
    // Allowance 1: the next step of the fixture opens the window menu, which is the menus lane's.
    rig.stop_after("middle_click_lowers");
    rig.finish();
}

/// `[x, y, width, height, maximised, minimised]` of every window, as the click scenarios log them.
fn log_states_plain(rig: &Rig) {
    let list: Vec<Value> = [3_u64]
        .iter()
        .map(|id| {
            let [x, y, width, height] = rect_of(&rig.comp, *id);
            json!([
                js(x),
                js(y),
                js(width),
                js(height),
                rig.comp.flag(*id, "maximised").unwrap_or(false),
                rig.comp.flag(*id, "minimised").unwrap_or(false)
            ])
        })
        .collect();
    rig.rec("bounds", Value::Array(list));
}

#[test]
fn layers_of_a_client_drawn_window() {
    let mut rig = desktop("frame_layers");
    let api = rig.api;
    let log = Rc::clone(&rig.log);
    let shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                let underlay = frame.obj("underlay");
                underlay.set("bleed", d(20.0));
                underlay.add(boxed(
                    api,
                    &[
                        ("x", d(20.0)),
                        ("y", d(20.0)),
                        ("width", d(400.0)),
                        ("height", d(300.0)),
                        ("fill", t("#102030")),
                    ],
                ));
                let badge = boxed(
                    api,
                    &[
                        ("x", d(4.0)),
                        ("y", d(4.0)),
                        ("width", d(10.0)),
                        ("height", d(10.0)),
                        ("fill", t("#ff0000")),
                    ],
                );
                frame.obj("overlay").add(badge);
                if let Err(fail) = try_get(&frame, "top") {
                    log.borrow_mut()
                        .push(json!({ "label": "refused", "value": fail.message.trim_start_matches("E_TYPE: ") }));
                }
                let kind = match frame.get("kind").enum_value().expect("a kind") {
                    1 => "server",
                    _ => "layers",
                };
                log.borrow_mut().push(json!({ "label": "kind", "value": kind }));
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open_decorated(&mut rig, 3, "term", "foot", "client");
    shell.log_inset_record(&rig);
    rig.step("opened");
    rig.tree("frame", 3);
    rig.paint("layers", 3);
    rig.finish();
}

#[test]
fn a_ring_in_the_overlay() {
    let mut rig = desktop("frame_ring");
    let api = rig.api;
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !server_only(&frame) {
                    return Ok(b(false));
                }
                frame.obj("top").set("size", d(30.0));
                frame.obj("left").set("size", d(10.0));
                let overlay = frame.obj("overlay");
                let ring = make(api, "frame_border", &[("frame", frame.val())]);
                ring.set("thickness", d(2.0));
                ring.set(
                    "color_of",
                    callback(|args| {
                        Ok(t(if field_flag(&args[0], FOCUSED) {
                            "#89b4fa"
                        } else {
                            "#45475a"
                        }))
                    }),
                );
                overlay.add(ring);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    rig.step("opened");
    rig.tree("frame", 3);
    rig.paint("unfocused", 3);
    set(&mut rig, 3, "focused", MirrorValue::Bool(true));
    rig.step("focused");
    rig.paint("focused", 3);
    rig.finish();
}

/// A frame with a different room on every side, so no side can be mistaken for another: 30 above, 20 right, 40 below and 10 left.
fn four_sides(deco: &W, api: Api) {
    deco.set(
        "frame",
        callback(move |args| {
            let frame = wrap(api, &args[1]);
            if !server_only(&frame) {
                return Ok(b(false));
            }
            frame.obj("top").set("size", d(30.0));
            frame.obj("right").set("size", d(20.0));
            frame.obj("bottom").set("size", d(40.0));
            let left = frame.obj("left");
            left.set("size", d(10.0));
            left.set("hide_when", callback(|args| Ok(b(field_flag(&args[0], MAXIMISED)))));
            Ok(Val::None)
        }),
    );
}

/// The log entries `inset` and `bounds` of the four-sides scenario.
fn log_inset_and_bounds(rig: &Rig, shell: &Shell) {
    shell.log_inset(rig);
    log_bounds(rig);
}

#[test]
fn four_edges() {
    let mut rig = desktop("frame_four_edges");
    let api = rig.api;
    let shell = Shell::start(&mut rig, |deco| four_sides(deco, api));
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    log_inset_and_bounds(&rig, &shell);
    rig.step("opened");
    rig.tree("frame", 3);
    set(&mut rig, 3, "fullscreen", MirrorValue::Bool(true));
    log_inset_and_bounds(&rig, &shell);
    rig.step("fullscreen");
    rig.tree("frame", 3);
    set(&mut rig, 3, "fullscreen", MirrorValue::Bool(false));
    log_inset_and_bounds(&rig, &shell);
    rig.step("windowed");
    request(&mut rig, 3, "maximize");
    log_inset_and_bounds(&rig, &shell);
    rig.step("maximised");
    rig.tree("frame", 3);
    request(&mut rig, 3, "unmaximize");
    log_inset_and_bounds(&rig, &shell);
    rig.step("unmaximised");
    move_to(&mut rig, 3, [300.0, 200.0, 400.0, 300.0]);
    log_inset_and_bounds(&rig, &shell);
    rig.step("moved");
    set(&mut rig, 3, "decoration", MirrorValue::Text("client".to_owned()));
    log_inset_and_bounds(&rig, &shell);
    rig.step("client_drawn");
    rig.tree("frame", 3);
    set(&mut rig, 3, "decoration", MirrorValue::Text("server".to_owned()));
    log_inset_and_bounds(&rig, &shell);
    rig.step("server_drawn");
    rig.tree("frame", 3);
    move_to(&mut rig, 3, [600.0, 500.0, 400.0, 300.0]);
    set(&mut rig, 3, "decoration", MirrorValue::Text("client".to_owned()));
    set(&mut rig, 3, "decoration", MirrorValue::Text("server".to_owned()));
    log_inset_and_bounds(&rig, &shell);
    rig.step("kept_inside_the_area");
    rig.external_destroy(3);
    rig.settle();
    rig.forget("closed", &["n9", "n10", "n11", "n12"]);
    rig.step("closed");
    rig.finish();
}

/// `c.bounds = rect; c.floatingBounds = { ...rect }`: the shell places the window and says where it floats.
fn move_to(rig: &mut Rig, node: u64, [x, y, width, height]: [f64; 4]) {
    for (key, value) in [("x", x), ("y", y), ("width", width), ("height", height)] {
        rig.external(node, key, MirrorValue::Double(value));
    }
    let state = function(rig.api, "client_state", "of", &[rig.client(node)]);
    let state = wrap(rig.api, &state);
    state.set("floating_bounds", rect_val(x, y, width, height));
    rig.settle();
}

fn rect_val(x: f64, y: f64, width: f64, height: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![d(x), d(y), d(width), d(height)],
    }
}

/// `gnoblin.workspaces.add(new Workspace({ id: '1', layout: new MasterStackLayout() }))`.
fn master_stack_workspace(api: Api) {
    let layout = make(api, "master_stack_layout", &[]);
    let workspace = make(api, "workspace", &[("id", t("1")), ("layout", layout.val())]);
    let list = make(api, "workspaces", &[]);
    list.call("add", &[workspace.val()]);
}

/// `[x, y, width, height]` of each window, as `gnoblin.clients.map` logs them.
fn log_all_bounds(rig: &Rig, ids: &[u64]) {
    let list: Vec<Value> = ids.iter().map(|id| json!(rect_of(&rig.comp, *id).map(js))).collect();
    rig.rec("bounds", Value::Array(list));
}

fn rect_arg(x: f64, y: f64, width: f64, height: f64) -> Val {
    rect_val(x, y, width, height)
}

#[test]
fn edge_size_changes() {
    let mut rig = desktop("frame_edge_resize");
    let api = rig.api;
    let top = Rc::new(RefCell::new(None::<W>));
    let held = Rc::clone(&top);
    master_stack_workspace(api);
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                let edge = frame.obj("top");
                edge.set("size", d(30.0));
                *held.borrow_mut() = Some(edge);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    log_bounds(&rig);
    rig.step("tiled");
    top.borrow().expect("the top outlet").set("size", d(50.0));
    rig.settle();
    log_bounds(&rig);
    rig.step("top_grew");
    open(&mut rig, 4, "second", "foot");
    log_all_bounds(&rig, &[3, 4]);
    rig.step("two_tiles");
    rig.tree("frame", 3);
    rig.finish();
}

#[test]
fn snapped_frames() {
    let mut rig = desktop("frame_tiled_and_snapped");
    let api = rig.api;
    let _shell = Shell::start(&mut rig, |deco| four_sides(deco, api));
    rig.step("boot");
    open_decorated(&mut rig, 3, "term", "foot", "client");
    let state = wrap(api, &function(api, "client_state", "of", &[rig.client(3)]));
    state.call("snap_to", &[en(1), rect_arg(0.0, 0.0, 1000.0, 800.0), b(true)]);
    rig.settle();
    // The TypeScript script read the window before its snap reached the compositor: the log of this step is the window's old place, and
    // the state the step leaves is the snapped one.
    rig.skip(&["bounds"]);
    rig.step("snapped_client_drawn");
    set(&mut rig, 3, "decoration", MirrorValue::Text("server".to_owned()));
    log_all_bounds(&rig, &[3]);
    rig.step("snapped_framed");
    set(&mut rig, 3, "decoration", MirrorValue::Text("client".to_owned()));
    log_all_bounds(&rig, &[3]);
    rig.step("snapped_unframed");
    rig.finish();
}

#[test]
fn tiled_frames() {
    let mut rig = desktop("frame_tiled");
    let api = rig.api;
    master_stack_workspace(api);
    let _shell = Shell::start(&mut rig, |deco| four_sides(deco, api));
    rig.step("boot");
    open_decorated(&mut rig, 3, "term", "foot", "client");
    log_all_bounds(&rig, &[3]);
    rig.step("tile_client_drawn");
    set(&mut rig, 3, "decoration", MirrorValue::Text("server".to_owned()));
    log_all_bounds(&rig, &[3]);
    rig.step("tile_framed");
    open(&mut rig, 4, "second", "foot");
    open(&mut rig, 5, "third", "foot");
    log_all_bounds(&rig, &[3, 4, 5]);
    rig.step("three_tiles");
    rig.finish();
}

#[test]
fn drag_areas_and_resize_handles() {
    let mut rig = desktop("frame_handles");
    let api = rig.api;
    let shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !server_only(&frame) {
                    return Ok(b(false));
                }
                let top = frame.obj("top");
                top.set("size", d(28.0));
                let bar = make(api, "drag_area", &[("frame", frame.val())]);
                bar.set("grow", d(1.0));
                bar.set("direction", en(rig::ROW));
                bar.set("fill", t("#333333"));
                top.add(bar);
                let left = frame.obj("left");
                left.set("size", d(0.0));
                let right = frame.obj("right");
                right.set("size", d(0.0));
                let handle = make(api, "resize_handle", &[("frame", frame.val()), ("side", en(5))]);
                handle.set("grow", d(1.0));
                left.add(handle);
                let handle = make(api, "resize_handle", &[("frame", frame.val()), ("side", en(6))]);
                handle.set("grow", d(1.0));
                right.add(handle);
                Ok(Val::None)
            }),
        );
    });
    let _ = shell;
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    log_bounds(&rig);
    rig.step("opened");
    rig.tree("frame", 3);
    let bar = under(&rig, 3, &[2, 0]);
    pointer(&mut rig, bar, "press", 20.0, 10.0, LEFT);
    pointer(&mut rig, bar, "motion", 50.0, 25.0, 0);
    log_bounds(&rig);
    rig.step("dragged");
    pointer(&mut rig, bar, "release", 50.0, 25.0, LEFT);
    let right = under(&rig, 3, &[1, 0]);
    pointer(&mut rig, right, "press", 0.0, 50.0, LEFT);
    pointer(&mut rig, right, "motion", 20.0, 50.0, 0);
    log_bounds(&rig);
    rig.step("right_handle");
    pointer(&mut rig, right, "release", 20.0, 50.0, LEFT);
    let left = under(&rig, 3, &[0, 0]);
    pointer(&mut rig, left, "press", 0.0, 50.0, LEFT);
    pointer(&mut rig, left, "motion", -10.0, 50.0, 0);
    log_bounds(&rig);
    rig.step("left_handle");
    pointer(&mut rig, left, "release", -10.0, 50.0, LEFT);
    pointer(&mut rig, left, "press", 0.0, 50.0, LEFT);
    pointer(&mut rig, left, "motion", 900.0, 50.0, 0);
    log_bounds(&rig);
    rig.step("left_handle_to_the_least");
    rig.finish();
}

#[test]
fn resize_handles_round_the_frame() {
    let mut rig = desktop("frame_resize_handles");
    let _shell = Shell::start(&mut rig, |deco| deco.set("resize_handles", d(6.0)));
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    log_bounds(&rig);
    rig.step("opened");
    rig.tree("frame", 3);
    let right = under(&rig, 3, &[1, 0, 7]);
    pointer(&mut rig, right, "press", 2.0, 50.0, LEFT);
    pointer(&mut rig, right, "motion", 12.0, 50.0, 0);
    log_bounds(&rig);
    rig.step("right_handle");
    request(&mut rig, 3, "maximize");
    rig.step("maximised");
    rig.tree("frame", 3);
    rig.finish();
}

#[test]
fn who_gets_a_frame() {
    let mut rig = desktop("frame_rules");
    let api = rig.api;
    let shell = Shell::start(&mut rig, |deco| {
        deco.call("rule", &[t("mpv"), Val::None, Val::None, Val::None]);
        deco.call(
            "rule",
            &[
                Val::None,
                Val::None,
                callback(|args| Ok(b(args[0].record().expect("a client")[2].text()?.starts_with("org.")))),
                callback(move |args| {
                    wrap(api, &args[1]).obj("top").set("size", d(10.0));
                    Ok(Val::None)
                }),
            ],
        );
        deco.call(
            "rule",
            &[Val::None, t("scratch"), Val::None, callback(|_| Ok(b(false)))],
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "video", "mpv");
    open(&mut rig, 4, "files", "org.example.Files");
    open(&mut rig, 5, "scratch", "foot");
    open(&mut rig, 6, "plain", "foot");
    let tops: Vec<Value> = (3..=6).map(|id| js(inset_of(&shell.inset(id))[0])).collect();
    rig.rec("tops", Value::Array(tops));
    rig.step("opened");
    for id in [3, 4, 5, 6] {
        rig.tree(&format!("window_{id}"), id);
    }
    rig.finish();
}

#[test]
fn a_fill_that_throws() {
    let mut rig = desktop("frame_fill_throws");
    let api = rig.api;
    let shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                let top = frame.obj("top");
                top.set("size", d(30.0));
                top.add(boxed(api, &[("height", d(30.0))]));
                Err(Fail::err("no"))
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    shell.log_inset_record(&rig);
    rig.step("opened");
    rig.tree("window", 3);
    rig.finish();
}

#[test]
fn a_fill_runs_again_when_the_kind_changes() {
    let mut rig = desktop("frame_fill_again");
    let api = rig.api;
    let fills = Rc::new(RefCell::new(Vec::<String>::new()));
    let held = Rc::clone(&fills);
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                held.borrow_mut()
                    .push(if frame.flag("server") { "server" } else { "layers" }.to_owned());
                frame
                    .obj("overlay")
                    .add(boxed(api, &[("width", d(4.0)), ("height", d(4.0))]));
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    rig.rec("fills", json!(fills.borrow().join(",")));
    rig.step("opened");
    rig.tree("window", 3);
    set(&mut rig, 3, "decoration", MirrorValue::Text("client".to_owned()));
    rig.rec("fills", json!(fills.borrow().join(",")));
    rig.step("client_drawn");
    rig.tree("window", 3);
    set(&mut rig, 3, "decoration", MirrorValue::Text("server".to_owned()));
    rig.rec("fills", json!(fills.borrow().join(",")));
    rig.step("server_drawn");
    rig.external_destroy(3);
    rig.settle();
    rig.forget("closed", &["n5", "n6"]);
    rig.step("closed");
    rig.finish();
}

#[test]
fn a_canvas_that_follows_the_focus() {
    let mut rig = desktop("frame_bound_node");
    let api = rig.api;
    let comp = rig.comp.clone();
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !server_only(&frame) {
                    return Ok(b(false));
                }
                let top = frame.obj("top");
                top.set("size", d(20.0));
                let dot = boxed(api, &[("width", d(10.0)), ("height", d(10.0)), ("fill", t("#ff0000"))]);
                // Allowance 2: `properties.bind('visible', () => !client.focused)` is the core's mechanism; the outlet's `on_refresh` reads the
                // window once a frame as the binding does.
                let node = node_of(&args[0]);
                let comp = comp.clone();
                top.set(
                    "on_refresh",
                    callback(move |_| {
                        dot.set("visible", b(!comp.flag(node, "focused").unwrap_or(false)));
                        Ok(Val::None)
                    }),
                );
                top.add(dot);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    rig.step("opened");
    rig.tree("window", 3);
    set(&mut rig, 3, "focused", MirrorValue::Bool(true));
    rig.step("focused");
    rig.tree("window", 3);
    rig.external_destroy(3);
    rig.settle();
    rig.forget("closed", &["n1", "n2"]);
    rig.step("closed");
    rig.finish();
}

#[test]
fn recipe_buttons_on_the_left() {
    let mut rig = desktop("frame_recipe_buttons_left");
    let api = rig.api;
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !server_only(&frame) {
                    return Ok(b(false));
                }
                let top = frame.obj("top");
                top.set("size", d(34.0));
                top.set("fill", t("#2a2b3c"));
                let start = frame.obj("start");
                let buttons = make(
                    api,
                    "window_buttons",
                    &[
                        ("frame", frame.val()),
                        ("buttons", Val::List(vec![en(3), en(1), en(2)])),
                    ],
                );
                buttons.set("gap", d(8.0));
                start.add(buttons);
                let bar = make(api, "drag_area", &[("frame", frame.val())]);
                bar.set("grow", d(1.0));
                bar.set("direction", en(rig::ROW));
                bar.add(make(api, "title_text", &[("frame", frame.val())]));
                frame.obj("center").add(bar);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    rig.answer_measures();
    rig.step("opened");
    rig.tree("frame", 3);
    let bar = under(&rig, 3, &[0]);
    rig.paint("bar", bar);
    rig.finish();
}

#[test]
fn recipe_glow_and_ring() {
    let mut rig = desktop("frame_recipe_glow");
    let api = rig.api;
    let _shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                let underlay = frame.obj("underlay");
                underlay.set("bleed", d(24.0));
                let glow = make(api, "frame_glow", &[]);
                glow.set("color", t("#000000"));
                glow.set("strength", d(0.45));
                glow.set("radius", d(12.0));
                underlay.add(glow);
                let overlay = frame.obj("overlay");
                let ring = make(api, "frame_border", &[("frame", frame.val())]);
                ring.set("thickness", d(2.0));
                ring.set("radius", d(12.0));
                ring.set(
                    "color_of",
                    callback(|args| {
                        Ok(t(if field_flag(&args[0], FOCUSED) {
                            "#89b4fa"
                        } else {
                            "#45475a"
                        }))
                    }),
                );
                overlay.add(ring);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    open_decorated(&mut rig, 4, "own", "gtk", "client");
    rig.step("opened");
    rig.tree("server", 3);
    rig.tree("client", 4);
    rig.paint("server_layers", 3);
    set(&mut rig, 3, "focused", MirrorValue::Bool(true));
    rig.step("focused");
    rig.paint("server_layers_focused", 3);
    rig.finish();
}

#[test]
fn recipe_status_strip() {
    let mut rig = desktop("frame_recipe_status_strip");
    let api = rig.api;
    let comp = rig.comp.clone();
    let shell = Shell::start(&mut rig, |deco| {
        deco.set(
            "frame",
            callback(move |args| {
                let frame = wrap(api, &args[1]);
                if !function(api, "titlebar", "install", &[frame.val()]).bool()? {
                    return Ok(b(false));
                }
                let bottom = frame.obj("bottom");
                bottom.set("size", d(22.0));
                bottom.set("fill", t("#181825"));
                bottom.set("padding", d(4.0));
                bottom.set("hide_when", callback(|args| Ok(b(field_flag(&args[0], MAXIMISED)))));
                // `class StatusText extends Text`: its `refresh` reads the window into its text.
                let text = make(
                    api,
                    "text",
                    &[("grow", d(1.0)), ("size", d(11.0)), ("color", t("#a6adc8"))],
                );
                let node = node_of(&args[0]);
                let comp = comp.clone();
                let widget = api.class("widget").expect("the interface");
                let refresh = api
                    .member(widget, gnoblin_core_abi::MEMBER_METHOD, "refresh")
                    .expect("refresh");
                let imp = api.implement(widget, &[refresh], move |_, _| {
                    let [_, _, width, height] = rect_of(&comp, node);
                    text.set("text", t(&format!("foot  {} x {}", width.round(), height.round())));
                    Ok(Val::None)
                })?;
                text.set(
                    "behaviour",
                    Val::Object {
                        class: widget,
                        obj: imp,
                    },
                );
                bottom.add(text);
                let right = frame.obj("right");
                right.set("size", d(12.0));
                right.set("fill", t("#313244"));
                let handle = make(api, "resize_handle", &[("frame", frame.val()), ("side", en(6))]);
                handle.set("grow", d(1.0));
                right.add(handle);
                Ok(Val::None)
            }),
        );
    });
    rig.step("boot");
    open(&mut rig, 3, "term", "foot");
    rig.answer_measures();
    shell.log_inset(&rig);
    rig.step("opened");
    rig.tree("frame", 3);
    rig.paint("strip", 3);
    let handle = under(&rig, 3, &[0, 0]);
    pointer(&mut rig, handle, "press", 2.0, 50.0, LEFT);
    pointer(&mut rig, handle, "motion", 12.0, 50.0, 0);
    rig.answer_measures();
    rig.step("handle_dragged");
    rig.paint("strip_after_resize", 3);
    request(&mut rig, 3, "maximize");
    shell.log_inset(&rig);
    rig.step("maximised");
    rig.finish();
}

/// What `frame_titlebar_clicks` tested after the menu opened (allowance 1): the seat's `double_click`, `middle_click` and `right_click`
/// actions, and an action the schema does not name.
#[test]
fn seat_actions() {
    let mut rig = Rig::scratch(1000.0, 800.0);
    let shell = Shell::start(&mut rig, |_| {});
    open(&mut rig, 3, "terminal", "foot");
    let drag = under(&rig, 3, &[0, 0]);
    let click = |rig: &mut Rig, shell: &Shell, button: u64| {
        shell.advance(rig, 1000);
        pointer(rig, drag, "press", 20.0, 10.0, button);
    };
    let double = |rig: &mut Rig, shell: &Shell| {
        shell.advance(rig, 1000);
        pointer(rig, drag, "press", 20.0, 10.0, LEFT);
        shell.advance(rig, 100);
        pointer(rig, drag, "press", 20.0, 10.0, LEFT);
    };
    // The stock actions: a double click maximises, a right click opens the menu (the hook does nothing here), a middle click lowers.
    double(&mut rig, &shell);
    assert_eq!(rig.comp.flag(3, "maximised"), Some(true));
    click(&mut rig, &shell, MIDDLE);
    click(&mut rig, &shell, RIGHT);
    rig.external(21, "double_click", MirrorValue::Text("minimize".to_owned()));
    rig.external(21, "middle_click", MirrorValue::Text("none".to_owned()));
    rig.external(21, "right_click", MirrorValue::Text("toggle-maximize".to_owned()));
    rig.settle();
    double(&mut rig, &shell);
    assert_eq!(rig.comp.flag(3, "minimised"), Some(true), "double click minimises");
    click(&mut rig, &shell, MIDDLE);
    click(&mut rig, &shell, RIGHT);
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(false),
        "right click toggles maximise"
    );
    rig.external(21, "double_click", MirrorValue::Text("bogus".to_owned()));
    rig.settle();
    let _ = rig.reported();
    double(&mut rig, &shell);
    let lines = rig.reported();
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("double_click must name a titlebar action, got bogus")),
        "{lines:?}"
    );
}
