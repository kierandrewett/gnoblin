#![allow(unsafe_code)]
//! The UI toolkit of `libgnoblin-std`, held to what the TypeScript toolkit did (`data/js/gnoblin-std/ui/*`, `widgets.ts`, `panel.ts`).
//!
//! Each test replays a scenario of `crates/gnoblin-js/tests/oracle/ui.rs` as a shell written over the object protocol: the same boxes,
//! texts and widgets made in the same order, the same events sent by the fake compositor, the same steps. What the fixture recorded of
//! the TypeScript run is what the native run must find at every step (`rig.rs` says what is compared). Text is measured by the rule the
//! oracle's compositor used, and answered when the scenario answers it.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_object::Val;
use gnoblin_testkit::oracle::{Map, Value, json};
use rig::{COLUMN, LEFT, MIDDLE, RIGHT, ROW, Rig, W, b, callback, d, en, fields, js, t};

fn desktop(name: &'static str) -> Rig {
    Rig::new(name, 1000.0, 800.0)
}

fn boxed(rig: &Rig, inits: &[(&str, Val)]) -> W {
    rig.make("box", inits)
}

// ---- layout ----------------------------------------------------------------------------------------------------------------------

#[test]
fn layout() {
    let mut rig = desktop("ui_layout");
    let row = boxed(
        &rig,
        &[
            ("width", d(300.0)),
            ("height", d(50.0)),
            ("direction", en(ROW)),
            ("gap", d(10.0)),
            ("padding", d(5.0)),
        ],
    );
    row.add(boxed(&rig, &[("width", d(40.0)), ("height", d(10.0))]))
        .add(boxed(&rig, &[("width", d(60.0)), ("height", d(10.0))]));
    let grow_row = boxed(
        &rig,
        &[
            ("width", d(300.0)),
            ("height", d(50.0)),
            ("direction", en(ROW)),
            ("gap", d(10.0)),
            ("padding", d(5.0)),
        ],
    );
    grow_row
        .add(boxed(&rig, &[("width", d(40.0))]))
        .add(boxed(&rig, &[("grow", d(1.0))]))
        .add(boxed(&rig, &[("grow", d(2.0))]));
    let col = boxed(
        &rig,
        &[
            ("width", d(100.0)),
            ("height", d(200.0)),
            ("direction", en(COLUMN)),
            ("gap", d(6.0)),
            ("padding", d(4.0)),
        ],
    );
    col.add(boxed(&rig, &[("height", d(30.0))]))
        .add(boxed(&rig, &[("grow", d(1.0))]))
        .add(boxed(&rig, &[("height", d(20.0))]));
    let none = boxed(&rig, &[("width", d(100.0)), ("height", d(100.0))]);
    let inner = boxed(
        &rig,
        &[
            ("x", d(7.0)),
            ("y", d(9.0)),
            ("width", d(30.0)),
            ("height", d(20.0)),
            ("direction", en(COLUMN)),
            ("padding", d(2.0)),
        ],
    );
    none.add(inner);
    inner.add(boxed(&rig, &[("height", d(5.0))]));
    let grid = rig.make(
        "grid",
        &[
            ("width", d(100.0)),
            ("height", d(100.0)),
            ("padding", d(4.0)),
            ("gap", d(2.0)),
        ],
    );
    for _ in 0..3 {
        grid.add(boxed(&rig, &[]));
    }
    let tight = boxed(
        &rig,
        &[
            ("width", d(50.0)),
            ("height", d(50.0)),
            ("direction", en(ROW)),
            ("padding", d(5.0)),
        ],
    );
    tight
        .add(boxed(&rig, &[("width", d(100.0))]))
        .add(boxed(&rig, &[("grow", d(1.0))]));
    let stray = boxed(
        &rig,
        &[
            ("width", d(80.0)),
            ("height", d(40.0)),
            ("direction", en(ROW)),
            ("gap", d(3.0)),
        ],
    );
    stray.add(boxed(&rig, &[("width", d(10.0))])).add(rig.make(
        "canvas",
        &[("x", d(9.0)), ("y", d(9.0)), ("width", d(4.0)), ("height", d(4.0))],
    ));
    stray.add(boxed(&rig, &[("width", d(20.0))]));
    let roots = [
        ("row", row),
        ("grow_row", grow_row),
        ("column", col),
        ("none", none),
        ("grid", grid),
        ("tight", tight),
        ("stray", stray),
    ];

    rig.settle();
    rig.step("boot");
    for (label, root) in roots {
        rig.tree(label, root.node());
    }

    grow_row.set("width", d(500.0));
    col.set("height", d(120.0));
    grid.add(boxed(&rig, &[]));
    grid.set("padding", d(0.0));
    grid.set("gap", d(6.0));
    rig.settle();
    rig.step("resized");

    row.kid(0).set("visible", b(false));
    row.kid(1).set("opacity", d(0.5));
    row.set("gap", d(0.0));
    col.kid(1).set("grow", d(3.0));
    rig.settle();
    rig.step("changed");

    col.call("remove", &[col.kid(0).val()]);
    row.add(grow_row.kid(0));
    rig.settle();
    rig.step("moved_and_removed");

    row.set("padding", d(12.0));
    row.set("direction", en(COLUMN));
    rig.settle();
    rig.step("turned");

    grow_row.kid(1).set("width", d(30.0));
    grow_row.kid(1).set("grow", d(0.0));
    rig.settle();
    rig.step("grow_off");
    for (label, root) in roots {
        rig.tree(&format!("final_{label}"), root.node());
    }
    rig.finish();
}

// ---- text ------------------------------------------------------------------------------------------------------------------------

/// `[width, height, ascent, shown]` of each text, as the scenario's `metrics()` reads them.
fn metrics(texts: &[W]) -> Value {
    Value::Array(
        texts
            .iter()
            .map(|text| {
                json!([
                    js(text.num("width")),
                    js(text.num("height")),
                    js(text.num("ascent")),
                    text.call("shown", &[]).text().expect("text")
                ])
            })
            .collect(),
    )
}

#[test]
fn text_metrics_and_painting() {
    let mut rig = desktop("ui_text");
    let holder = boxed(&rig, &[("width", d(400.0)), ("height", d(400.0))]);
    let put = |y: f64, w: W| {
        w.set("y", d(y));
        holder.add(w);
        w
    };
    let text = |inits: &[(&str, Val)]| rig.make("text", inits);
    let t1 = put(0.0, text(&[("text", t("Hello")), ("size", d(10.0))]));
    let multi = put(20.0, text(&[("text", t("a\nbbbb\ncc")), ("size", d(10.0))]));
    let roomy = put(
        80.0,
        text(&[
            ("text", t("A long window title")),
            ("size", d(10.0)),
            ("ellipsize", b(true)),
        ]),
    );
    let cut = put(
        100.0,
        text(&[
            ("text", t("A long window title")),
            ("size", d(10.0)),
            ("ellipsize", b(true)),
            ("width", d(50.0)),
        ]),
    );
    let plain = put(
        120.0,
        text(&[
            ("text", t("A long window title")),
            ("size", d(10.0)),
            ("width", d(50.0)),
        ]),
    );
    let mono = put(
        140.0,
        text(&[
            ("text", t("ab")),
            ("size", d(13.0)),
            ("color", t("#c6d0f5")),
            ("font", t("mono")),
        ]),
    );
    let squeezed = put(
        160.0,
        text(&[("text", t("ab")), ("size", d(13.0)), ("height", d(13.0))]),
    );
    let wide = put(
        180.0,
        text(&[("text", t("h\u{e9}llo w\u{f6}rld \u{2713}")), ("size", d(16.0))]),
    );
    let empty = put(210.0, text(&[("text", t("")), ("size", d(12.0))]));
    let icon = put(
        230.0,
        rig.make(
            "icon",
            &[
                ("icon", t("window-close-symbolic")),
                ("size", d(16.0)),
                ("color", t("#fff")),
            ],
        ),
    );
    let icon_default = put(260.0, rig.make("icon", &[("icon", t("folder"))]));
    let texts = [t1, multi, roomy, cut, plain, mono, squeezed, wide, empty];

    rig.settle();
    rig.step("boot");

    rig.rec("estimate", metrics(&texts));
    rig.settle();
    rig.step("estimate");
    rig.paint("estimate", holder.node());

    rig.answer_measures();
    rig.rec("measured", metrics(&texts));
    rig.settle();
    rig.step("measured");
    rig.paint("measured", holder.node());

    t1.set("text", t("Hello, world"));
    rig.rec("after_change", metrics(&texts));
    rig.settle();
    rig.step("text_changed");

    rig.answer_measures();
    rig.rec("answered", metrics(&texts));
    rig.settle();
    rig.step("text_answered");

    t1.set("size", d(20.0));
    t1.set("font", t("mono"));
    icon.set("size", d(32.0));
    icon_default.set("icon", t("user-trash"));
    rig.rec("restyled", metrics(&texts));
    rig.settle();
    rig.step("restyled");

    rig.answer_measures();
    rig.rec("restyled_answered", metrics(&texts));
    rig.settle();
    rig.step("restyled_answered");
    rig.paint("restyled", holder.node());
    rig.finish();
}

// ---- paint -----------------------------------------------------------------------------------------------------------------------

/// A `paint_op` record with the fields given, by name.
fn op(kind: u32, given: &[(&str, Val)]) -> Val {
    const NAMES: [&str; 19] = [
        "op",
        "x",
        "y",
        "width",
        "height",
        "radius",
        "fill",
        "stroke",
        "stroke_width",
        "x1",
        "y1",
        "x2",
        "y2",
        "size",
        "color",
        "text",
        "font",
        "name",
        "alpha",
    ];
    let mut fields = vec![Val::None; NAMES.len()];
    fields[0] = en(kind);
    for (name, value) in given {
        let at = NAMES.iter().position(|held| held == name).expect("a field of paint_op");
        fields[at] = value.clone();
    }
    Val::Record { class: 0, fields }
}

const RECT: u32 = 1;
const LINE: u32 = 2;
const TEXT: u32 = 3;
const ICON: u32 = 4;
const PUSH_CLIP: u32 = 5;
const POP_CLIP: u32 = 6;
const PUSH_OPACITY: u32 = 7;
const POP_OPACITY: u32 = 8;

/// A canvas whose shell paints it: the interface `widget`, implemented by `paint`.
fn painted(rig: &Rig, width: f64, height: f64, ops: Vec<Val>) -> W {
    let canvas = rig.make("canvas", &[("width", d(width)), ("height", d(height))]);
    let widget = rig.api.class("widget").expect("the interface");
    let paint = rig
        .api
        .member(widget, gnoblin_core_abi::MEMBER_METHOD, "paint")
        .expect("paint");
    let imp = rig
        .api
        .implement(widget, &[paint], move |_, _| Ok(Val::List(ops.clone())))
        .expect("the shell implements the interface");
    canvas.set(
        "behaviour",
        Val::Object {
            class: widget,
            obj: imp,
        },
    );
    canvas
}

#[test]
fn paint_ops_and_colours() {
    let mut rig = desktop("ui_paint");
    let holder = boxed(&rig, &[("width", d(300.0)), ("height", d(200.0))]);
    let chart = painted(
        &rig,
        60.0,
        40.0,
        vec![
            op(
                RECT,
                &[
                    ("x", d(1.0)),
                    ("y", d(2.0)),
                    ("width", d(30.0)),
                    ("height", d(20.0)),
                    ("radius", d(4.0)),
                    ("fill", t("#ff000080")),
                    ("stroke", t("#00ff00")),
                    ("stroke_width", d(2.0)),
                ],
            ),
            op(
                RECT,
                &[("x", d(0.0)), ("y", d(0.0)), ("width", d(5.0)), ("height", d(5.0))],
            ),
            op(
                LINE,
                &[
                    ("x1", d(0.0)),
                    ("y1", d(0.0)),
                    ("x2", d(30.0)),
                    ("y2", d(20.0)),
                    ("size", d(2.0)),
                    ("color", t("#0000ff")),
                ],
            ),
            op(LINE, &[("x1", d(1.0)), ("y1", d(1.0)), ("x2", d(2.0)), ("y2", d(2.0))]),
            op(
                TEXT,
                &[
                    ("text", t("hi \u{2713}")),
                    ("x", d(3.0)),
                    ("y", d(4.0)),
                    ("size", d(12.0)),
                    ("color", t("#abc")),
                    ("font", t("mono")),
                ],
            ),
            op(TEXT, &[("text", t("plain"))]),
            op(
                ICON,
                &[
                    ("name", t("folder")),
                    ("x", d(5.0)),
                    ("y", d(6.0)),
                    ("size", d(24.0)),
                    ("color", t("#fff8")),
                ],
            ),
            op(ICON, &[("name", t("user-home"))]),
            op(
                PUSH_CLIP,
                &[
                    ("x", d(0.0)),
                    ("y", d(0.0)),
                    ("width", d(10.0)),
                    ("height", d(10.0)),
                    ("radius", d(3.0)),
                ],
            ),
            op(PUSH_OPACITY, &[("alpha", d(0.5))]),
            op(
                RECT,
                &[
                    ("x", d(0.0)),
                    ("y", d(0.0)),
                    ("width", d(5.0)),
                    ("height", d(5.0)),
                    ("fill", t("#123")),
                ],
            ),
            op(POP_OPACITY, &[]),
            op(POP_CLIP, &[]),
        ],
    );
    holder.add(chart);
    let swatch = |x: f64, fill: &str| {
        holder.add(boxed(
            &rig,
            &[
                ("x", d(x)),
                ("y", d(50.0)),
                ("width", d(8.0)),
                ("height", d(8.0)),
                ("fill", t(fill)),
            ],
        ));
    };
    swatch(0.0, "#f00");
    swatch(10.0, "#f008");
    swatch(20.0, "#ff8800");
    swatch(30.0, "#ff880044");
    holder.add(boxed(
        &rig,
        &[
            ("x", d(40.0)),
            ("y", d(50.0)),
            ("width", d(20.0)),
            ("height", d(10.0)),
            ("radius", d(5.0)),
            ("fill", t("#112233")),
        ],
    ));
    holder.add(painted(
        &rig,
        5.0,
        5.0,
        vec![op(
            RECT,
            &[
                ("x", d(0.0)),
                ("y", d(0.0)),
                ("width", d(4.0)),
                ("height", d(4.0)),
                ("fill", t("red")),
            ],
        )],
    ));
    holder.add(painted(
        &rig,
        5.0,
        5.0,
        vec![op(
            RECT,
            &[("x", d(f64::NAN)), ("y", d(0.0)), ("width", d(4.0)), ("height", d(4.0))],
        )],
    ));
    rig.settle();
    rig.step("boot");
    rig.paint("ops", holder.node());

    holder.kid(0).set("width", d(80.0));
    holder.kid(5).set("fill", t("#00ff0080"));
    rig.settle();
    rig.step("resized");
    rig.paint("resized", holder.node());
    rig.finish();
}

// ---- events ----------------------------------------------------------------------------------------------------------------------

const HANDLERS: [&str; 6] = ["onPress", "onRelease", "onMotion", "onScroll", "onEnter", "onLeave"];
const KINDS: [&str; 6] = ["press", "release", "motion", "scroll", "enter", "leave"];

#[test]
fn events_bubble_and_stop() {
    let mut rig = desktop("ui_events");
    let outer = boxed(
        &rig,
        &[
            ("x", d(10.0)),
            ("y", d(20.0)),
            ("width", d(200.0)),
            ("height", d(100.0)),
        ],
    );
    let mid = boxed(
        &rig,
        &[("x", d(5.0)), ("y", d(5.0)), ("width", d(100.0)), ("height", d(60.0))],
    );
    let leaf = boxed(
        &rig,
        &[("x", d(7.0)), ("y", d(3.0)), ("width", d(20.0)), ("height", d(20.0))],
    );
    outer.add(mid);
    mid.add(leaf);
    let stop_at = Rc::new(RefCell::new(String::new()));
    let throws = Rc::new(Cell::new(false));
    let leaf_obj = leaf.obj;
    for (name, canvas) in [("outer", outer), ("mid", mid), ("leaf", leaf)] {
        let log = Rc::clone(&rig.log);
        let stop_at = Rc::clone(&stop_at);
        let throws = Rc::clone(&throws);
        canvas.set(
            "on_pointer",
            callback(move |args| {
                let event = fields(args);
                let kind = event[0].enum_value()? as usize - 1;
                let handler = HANDLERS[kind];
                if name == "leaf" && handler == "onRelease" && throws.get() {
                    return Err(gnoblin_object::Fail::err("a handler that throws"));
                }
                log.borrow_mut().push(json!({
                    "label": format!("{name}.{handler}"),
                    "value": {
                        "x": js(event[1].double()?),
                        "y": js(event[2].double()?),
                        "button": event[3].uint()?,
                        "kind": KINDS[kind],
                        "target": matches!(event[6], Val::Object { obj, .. } if obj == leaf_obj),
                    },
                }));
                Ok(Val::Bool(name == "mid" && stop_at.borrow().as_str() == handler))
            }),
        );
    }
    rig.settle();
    rig.step("boot");
    for (kind, button) in [
        ("press", LEFT),
        ("release", LEFT),
        ("motion", 0),
        ("scroll", 0),
        ("enter", 0),
        ("leave", 0),
        ("press", RIGHT),
        ("press", MIDDLE),
    ] {
        rig.comp.pointer(leaf.node(), kind, 4.0, 5.0, button);
        rig.settle();
        rig.step(&format!("leaf_{kind}_{button}"));
    }
    *stop_at.borrow_mut() = "onPress".to_owned();
    rig.comp.pointer(leaf.node(), "press", 4.0, 5.0, LEFT);
    rig.settle();
    rig.step("stopped_at_mid");

    stop_at.borrow_mut().clear();
    throws.set(true);
    rig.comp.pointer(leaf.node(), "release", 1.0, 1.0, LEFT);
    rig.settle();
    rig.step("handler_throws");

    rig.comp.pointer(mid.node(), "press", 50.0, 30.0, LEFT);
    rig.settle();
    rig.step("mid_press");

    let gone = leaf.node();
    outer.call("remove", &[mid.val()]);
    rig.comp.pointer(gone, "press", 1.0, 1.0, LEFT);
    rig.settle();
    rig.step("after_destroy");
    rig.finish();
}

// ---- keys and focus --------------------------------------------------------------------------------------------------------------

fn key_logger(rig: &Rig, name: &'static str, stop: Option<&'static str>) -> Val {
    let log = Rc::clone(&rig.log);
    callback(move |args| {
        let event = fields(args);
        let key = event[0].text()?.to_owned();
        log.borrow_mut().push(json!({
            "label": format!("{name}.key"),
            "value": { "key": key, "text": event[1].text()?, "pressed": event[2].bool()? },
        }));
        Ok(Val::Bool(stop == Some(key.as_str())))
    })
}

#[test]
fn keys_and_focus() {
    let mut rig = desktop("ui_keys");
    let a = boxed(&rig, &[("width", d(100.0)), ("height", d(50.0))]);
    let b_ = boxed(&rig, &[("width", d(50.0)), ("height", d(20.0))]);
    let c = boxed(&rig, &[("width", d(50.0)), ("height", d(20.0)), ("x", d(50.0))]);
    a.add(b_);
    a.add(c);
    a.set("on_key", key_logger(&rig, "a", None));
    b_.set("on_key", key_logger(&rig, "b", Some("x")));
    a.mount_top();
    b_.focus();
    let focus = |rig: &Rig, names: &[(&str, W)]| {
        let value: Map<String, Value> = names
            .iter()
            .map(|(name, canvas)| ((*name).to_owned(), json!(canvas.flag("focused"))))
            .collect();
        rig.rec("focus", Value::Object(value));
    };
    rig.settle();
    rig.step("boot");

    focus(&rig, &[("a", a), ("b", b_), ("c", c)]);
    rig.settle();
    rig.step("focus_on_b");

    for (key, text) in [("y", "y"), ("x", "x")] {
        for pressed in [true, false] {
            rig.comp.key(b_.node(), key, text, pressed);
            rig.settle();
        }
        rig.step(if key == "y" { "key_bubbles" } else { "key_stopped" });
    }
    rig.comp.key(b_.node(), "Return", "", true);
    rig.settle();
    rig.step("return_pressed");

    c.focus();
    focus(&rig, &[("a", a), ("b", b_), ("c", c)]);
    rig.settle();
    rig.step("focus_on_c");

    for pressed in [true, false] {
        rig.comp.key(c.node(), "z", "z", pressed);
        rig.settle();
    }
    rig.step("key_to_c");

    a.call("remove", &[c.val()]);
    focus(&rig, &[("a", a), ("b", b_)]);
    rig.settle();
    rig.step("c_destroyed");

    b_.focus();
    a.call("remove", &[b_.val()]);
    focus(&rig, &[("a", a), ("b", b_)]);
    rig.settle();
    rig.step("b_destroyed_while_focused");
    rig.finish();
}

// ---- the entry -------------------------------------------------------------------------------------------------------------------

fn press(rig: &mut Rig, at: &W, key: &str, text: &str) {
    rig.comp.key(at.node(), key, text, true);
    rig.settle();
}

fn release(rig: &mut Rig, at: &W, key: &str, text: &str) {
    rig.comp.key(at.node(), key, text, false);
    rig.settle();
}

fn tap(rig: &mut Rig, at: &W, key: &str, text: &str) {
    press(rig, at, key, text);
    release(rig, at, key, text);
}

/// `[caret, anchor, selected text]` of an entry's model, as the scenario records it.
fn selection(entry: &W) -> Value {
    let model = entry.obj("model");
    json!([
        model.get("caret").uint().expect("caret"),
        model.get("anchor").uint().expect("anchor"),
        model.text("selected_text")
    ])
}

#[test]
fn entry_states() {
    let mut rig = desktop("ui_entry");
    let e = rig.make("entry", &[("width", d(300.0)), ("height", d(40.0))]);
    e.mount_top();
    e.focus();
    let log = Rc::clone(&rig.log);
    e.set(
        "on_submit",
        callback(move |args| {
            log.borrow_mut()
                .push(json!({ "label": "submit", "value": args[0].text()? }));
            Ok(Val::None)
        }),
    );
    let idle = rig.make(
        "entry",
        &[
            ("width", d(200.0)),
            ("height", d(30.0)),
            ("text", t("caf\u{e9} \u{2713}")),
            ("size", d(12.0)),
            ("color", t("#ffffff")),
        ],
    );
    idle.mount_top();

    rig.settle();
    rig.step("boot");
    rig.paint("empty", e.node());
    rig.answer_measures();
    rig.step("measured_empty");
    for (key, text) in [("f", "f"), ("o", "o"), ("o", "o"), ("BackSpace", ""), ("x", "x")] {
        tap(&mut rig, &e, key, text);
    }
    rig.answer_measures();
    rig.rec(
        "text",
        json!([
            e.text("text"),
            e.obj("model").get("caret").uint().expect("caret"),
            e.obj("model").text("selected_text")
        ]),
    );
    rig.settle();
    rig.step("typed");
    rig.paint("typed", e.node());

    press(&mut rig, &e, "Left", "");
    press(&mut rig, &e, "Left", "");
    rig.answer_measures();
    rig.step("moved_left");
    rig.paint("caret_moved", e.node());

    press(&mut rig, &e, "Shift_L", "");
    press(&mut rig, &e, "Left", "");
    release(&mut rig, &e, "Shift_L", "");
    rig.answer_measures();
    rig.rec("selection", selection(&e));
    rig.settle();
    rig.step("selected");
    rig.paint("selected", e.node());

    press(&mut rig, &e, "Control_L", "");
    press(&mut rig, &e, "a", "a");
    release(&mut rig, &e, "Control_L", "");
    rig.answer_measures();
    rig.rec("selection", selection(&e));
    rig.settle();
    rig.step("select_all");
    rig.paint("select_all", e.node());

    press(&mut rig, &e, "End", "");
    press(&mut rig, &e, "Home", "");
    press(&mut rig, &e, "Delete", "");
    rig.answer_measures();
    rig.allow("home_delete", "n4", "height", 21.0, 17.0);
    rig.step("home_delete");
    rig.paint("home_delete", e.node());

    press(&mut rig, &e, "Return", "");
    rig.step("submit");

    e.set("secret", b(true));
    rig.settle();
    rig.answer_measures();
    rig.step("secret");
    rig.paint("secret", e.node());

    e.set("selection_fill", t("#ff0000"));
    e.set("color", t("#00ff00"));
    e.set("size", d(20.0));
    e.set("font", t("mono"));
    e.obj("model").call("select_all", &[]);
    rig.settle();
    rig.answer_measures();
    rig.step("restyled");
    rig.paint("restyled", e.node());

    e.set("text", t("h\u{e9}llo"));
    rig.settle();
    rig.answer_measures();
    rig.step("unicode");
    rig.paint("unicode", e.node());

    e.focus();
    rig.settle();
    rig.step("refocus");
    rig.paint("idle_unfocused", idle.node());

    idle.focus();
    rig.settle();
    rig.answer_measures();
    rig.step("idle_focused");
    rig.paint("idle_focused", idle.node());
    rig.finish();
}

// ---- the entry's model -----------------------------------------------------------------------------------------------------------

#[test]
fn entry_model() {
    let mut rig = desktop("ui_entry_model");
    let m = rig.make("entry_model", &[]);
    let at = |rig: &Rig| {
        rig.rec(
            "state",
            json!([
                m.text("text"),
                m.get("caret").uint().expect("caret"),
                m.get("anchor").uint().expect("anchor"),
                m.text("selected_text"),
                m.get("length").uint().expect("length")
            ]),
        );
    };
    rig.settle();
    rig.step("boot");
    let say = |name: &str, args: &[Val]| {
        m.call(name, args);
    };
    let extend = [b(true)];
    #[allow(clippy::type_complexity)]
    let script: Vec<Box<dyn Fn()>> = vec![
        Box::new(|| say("insert", &[t("h\u{e9}llo")])),
        Box::new(|| say("backspace", &[])),
        Box::new(|| {
            say("left", &[]);
            say("left", &[]);
            say("insert", &[t("X\n")]);
        }),
        Box::new(|| say("home", &extend)),
        Box::new(|| say("insert", &[t("Z")])),
        Box::new(|| say("select_all", &[])),
        Box::new(|| say("delete", &[])),
        Box::new(|| {
            say("insert", &[t("ab")]);
            say("left", &[]);
            say("delete", &[]);
        }),
        Box::new(|| {
            say("end", &[]);
            say("right", &[]);
            say("left", &extend);
        }),
        Box::new(|| m.set("text", t("new"))),
        Box::new(|| {
            say("home", &[]);
            say("right", &extend);
            say("right", &extend);
            say("backspace", &[]);
        }),
        Box::new(|| {
            say("insert", &[t("\u{1f600}x")]);
            say("left", &[]);
            say("left", &[]);
            say("delete", &[]);
        }),
        Box::new(|| {
            say("end", &extend);
            say("insert", &[t("")]);
        }),
    ];
    for (n, edit) in script.iter().enumerate() {
        edit();
        at(&rig);
        rig.settle();
        rig.step(&format!("edit_{}", n + 1));
    }
    rig.finish();
}

// ---- the clock, the bar and the toast --------------------------------------------------------------------------------------------

/// Milliseconds since the Unix epoch of a UTC time of day.
fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> f64 {
    // Days from the civil date, as the proleptic Gregorian calendar counts them.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + hour * 3600 + minute * 60 + second) * 1000) as f64
}

fn a_clock(now: &Rc<Cell<f64>>) -> Val {
    let now = Rc::clone(now);
    callback(move |_| Ok(Val::Double(now.get())))
}

fn bar_rows(bars: &[W]) -> Value {
    Value::Array(
        bars.iter()
            .map(|bar| {
                json!([
                    js(bar.num("x")),
                    js(bar.num("y")),
                    js(bar.num("width")),
                    js(bar.num("height")),
                    bar.flag("visible")
                ])
            })
            .collect(),
    )
}

fn insets(rig: &Rig) -> (f64, f64) {
    let Val::Record { fields, .. } = rig.function("bar", "insets", &[]) else {
        panic!("a record")
    };
    (fields[0].double().expect("top"), fields[1].double().expect("bottom"))
}

#[test]
fn clock_bar_toast() {
    let mut rig = desktop("ui_widgets");
    let time = Rc::new(Cell::new(utc(2024, 1, 5, 9, 30, 7)));
    let clock = rig.make(
        "clock",
        &[("now", a_clock(&time)), ("size", d(18.0)), ("color", t("#ff0000"))],
    );
    let seconds = rig.make(
        "clock",
        &[("now", a_clock(&time)), ("format", t("%a %d %b %Y %I:%M:%S %p"))],
    );
    let holder = boxed(
        &rig,
        &[
            ("width", d(400.0)),
            ("height", d(60.0)),
            ("direction", en(ROW)),
            ("gap", d(8.0)),
            ("padding", d(4.0)),
        ],
    );
    holder.add(clock).add(seconds);
    holder.mount_top();
    rig.settle();
    rig.step("boot");

    rig.answer_measures();
    rig.rec("clocks", json!([clock.text("text"), seconds.text("text")]));
    rig.settle();
    rig.step("clocks");
    rig.paint("clocks", holder.node());

    time.set(utc(2024, 12, 31, 23, 59, 59));
    // The clocks follow the system's minute: the shell's timer fires when it comes.
    rig.advance(61_000);
    rig.answer_measures();
    rig.rec("clocks", json!([clock.text("text"), seconds.text("text")]));
    rig.settle();
    rig.step("clocks_later");
    rig.paint("clocks_later", holder.node());

    let friday = utc(2024, 1, 5, 14, 7, 9);
    rig.rec(
        "formats",
        Value::Array(
            [
                "%H:%M",
                "%I:%M %p",
                "%a %d %b %Y",
                "%T %%",
                "%Q",
                "%F %j",
                "%A %B %e %y %R",
                "%S %m",
            ]
            .iter()
            .map(|format| rig.function("time_format", "format_time", &[t(format), d(friday)]))
            .map(|text| json!(text.text().expect("text")))
            .collect(),
        ),
    );
    rig.settle();
    rig.step("formats");

    let top = rig.make("bar", &[("height", d(32.0))]);
    let low = rig.make("bar", &[("edge", en(2)), ("height", d(20.0))]);
    let second = rig.make("bar", &[("height", d(10.0)), ("fill", t("#ff000080"))]);
    top.add(rig.make("text", &[("text", t("left")), ("grow", d(1.0))]));
    top.add(rig.make("text", &[("text", t("right"))]));
    rig.settle();
    rig.answer_measures();
    let (above, below) = insets(&rig);
    rig.rec("insets", json!({ "top": js(above), "bottom": js(below) }));
    // The workspaces' area is the screen less what the bars take: the library's workspaces report it, and the bars' insets are what
    // they read.
    rig.rec(
        "area",
        json!({ "x": 0, "y": js(above), "width": 1000, "height": js(800.0 - above - below) }),
    );
    rig.rec("bars", bar_rows(&[top, second, low]));
    rig.settle();
    rig.step("bars");
    rig.paint("bars", top.node());

    let hidden = Rc::new(Cell::new(false));
    let question = Rc::clone(&hidden);
    top.set("hide_when", callback(move |_| Ok(Val::Bool(question.get()))));
    hidden.set(true);
    rig.settle();
    let (above, below) = insets(&rig);
    rig.rec("insets", json!({ "top": js(above), "bottom": js(below) }));
    rig.rec("bars", bar_rows(&[top, second, low]));
    rig.settle();
    rig.step("top_hidden");

    second.destroy();
    top.destroy();
    low.destroy();
    let (above, below) = insets(&rig);
    rig.rec("insets", json!({ "top": js(above), "bottom": js(below) }));
    rig.settle();
    rig.step("bars_gone");

    let now = Rc::new(Cell::new(0.0));
    let card = match rig.function(
        "toast",
        "show",
        &[t("saved"), d(100.0), d(200.0), d(100.0), a_clock(&now)],
    ) {
        Val::Object { class, obj } => W {
            api: rig.api,
            class,
            obj,
        },
        other => panic!("a toast, got {other:?}"),
    };
    rig.settle();
    rig.answer_measures();
    rig.rec(
        "toast",
        json!([
            js(card.num("x")),
            js(card.num("y")),
            js(card.num("width")),
            js(card.num("height")),
            js(card.num("opacity"))
        ]),
    );
    rig.settle();
    rig.step("toast_start");
    rig.paint("toast_start", card.node());

    for (at, label) in [
        (50.0, "fading_in"),
        (150.0, "full"),
        (350.0, "fading_out"),
        (500.0, "gone"),
    ] {
        now.set(at);
        // The card's fade follows its age, and the toolkit's timer asks for the next frame.
        rig.advance(20);
        let count = rig.function("ui", "canvas_count", &[]).uint().expect("a count");
        rig.rec("toast", json!([card.flag("alive"), js(card.num("opacity")), count]));
        rig.settle();
        rig.step(label);
    }
    rig.finish();
}
