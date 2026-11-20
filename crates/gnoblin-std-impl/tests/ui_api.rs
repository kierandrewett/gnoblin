#![allow(unsafe_code)]
//! The UI toolkit of `libgnoblin-std` on its own terms: what a shell's `widget` may replace, what the signals and the hooks tell, what
//! happens to a canvas when its node goes, what the session values read, and what the toolkit refuses. The fixtures of `tests/ui.rs`
//! hold the toolkit to what the TypeScript toolkit did; these hold the parts of the object protocol that toolkit never had.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi::{MEMBER_METHOD, MEMBER_SIGNAL};
use gnoblin_object::{Res, Val};
use gnoblin_paint::{PaintOp, decode};
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{COLUMN, Rig, W, b, callback, d, en, fields, t};

fn scratch() -> Rig {
    Rig::scratch(1000.0, 800.0)
}

/// A `widget` the shell implements: the members it defines, by name, run `run` with their name.
fn implement(rig: &Rig, members: &[&str], mut run: impl FnMut(&str, &[Val]) -> Res<Val> + 'static) -> Val {
    let widget = rig.api.class("widget").expect("the interface");
    let ids: Vec<(u32, String)> = members
        .iter()
        .map(|name| {
            (
                rig.api.member(widget, MEMBER_METHOD, name).expect("a member of widget"),
                (*name).to_owned(),
            )
        })
        .collect();
    let defined: Vec<u32> = ids.iter().map(|(id, _)| *id).collect();
    let obj = rig
        .api
        .implement(widget, &defined, move |member, args| {
            let name = ids
                .iter()
                .find(|(id, _)| *id == member)
                .map(|(_, name)| name.clone())
                .expect("a member the shell defined");
            run(&name, args)
        })
        .expect("the shell implements the interface");
    Val::Object { class: widget, obj }
}

/// Hears a signal of `canvas` from the library's turns.
fn hear(canvas: &W, signal: &str, run: impl FnMut(&[Val]) + 'static) {
    let member = canvas
        .api
        .member(canvas.class, MEMBER_SIGNAL, signal)
        .expect("a signal");
    let mut run = run;
    canvas
        .api
        .listen(canvas.obj, member, move |_, args| {
            run(args);
            Ok(Val::None)
        })
        .expect("a listener");
}

/// Hears the changes of a property of `canvas`.
fn hear_property(canvas: &W, property: &str, run: impl FnMut(&[Val]) + 'static) {
    let member = canvas
        .api
        .member(canvas.class, gnoblin_core_abi::MEMBER_PROPERTY, property)
        .expect("a property");
    let mut run = run;
    canvas
        .api
        .listen(canvas.obj, member, move |_, args| {
            run(args);
            Ok(Val::None)
        })
        .expect("a listener");
}

fn content_of(rig: &Rig, canvas: &W) -> Vec<PaintOp> {
    match rig.comp.value(canvas.node(), "content") {
        Some(MirrorValue::Bytes(bytes)) => decode(&bytes).expect("a paint list the compositor accepts").ops,
        other => panic!("no content: {other:?}"),
    }
}

#[test]
fn a_shell_widget_arranges_paints_and_sizes_in_place_of_the_library() {
    let mut rig = scratch();
    let parent = rig.make("box", &[("width", d(200.0)), ("height", d(50.0)), ("direction", en(1))]);
    let kids = Rc::new(RefCell::new(Vec::<W>::new()));
    let refreshed = Rc::new(Cell::new(0_u32));
    let arranged = Rc::new(Cell::new(0_u32));
    let hooks = {
        let kids = Rc::clone(&kids);
        let refreshed = Rc::clone(&refreshed);
        let arranged = Rc::clone(&arranged);
        implement(
            &rig,
            &["refresh", "arrange", "paint", "natural_width", "natural_height"],
            move |name, _| match name {
                "refresh" => {
                    refreshed.set(refreshed.get() + 1);
                    Ok(Val::None)
                }
                "arrange" => {
                    arranged.set(arranged.get() + 1);
                    for (at, kid) in kids.borrow().iter().enumerate() {
                        kid.set("x", d(7.0 * at as f64));
                        kid.set("y", d(3.0));
                    }
                    Ok(Val::None)
                }
                "paint" => Ok(Val::List(vec![Val::Record {
                    class: 0,
                    fields: {
                        let mut fields = vec![Val::None; 19];
                        fields[0] = en(1);
                        fields[3] = d(9.0);
                        fields[4] = d(9.0);
                        fields[6] = t("#112233");
                        fields
                    },
                }])),
                "natural_width" => Ok(d(33.0)),
                "natural_height" => Ok(d(44.0)),
                other => panic!("{other}"),
            },
        )
    };
    parent.set("behaviour", hooks);
    for _ in 0..3 {
        let kid = rig.make("box", &[("width", d(10.0)), ("height", d(10.0))]);
        kids.borrow_mut().push(kid);
        parent.add(kid);
    }
    rig.settle();
    assert!(refreshed.get() >= 1, "the shell's refresh ran");
    assert!(arranged.get() >= 1, "the shell's arrangement ran");
    // The row of the library would have put the children at 0, 10 and 20: the shell's hook put them where it said.
    let placed: Vec<f64> = kids.borrow().iter().map(|kid| kid.num("x")).collect();
    assert_eq!(placed, [0.0, 7.0, 14.0]);
    assert_eq!(parent.kid(0).num("y"), 3.0);
    // The shell's paint is what the node holds, not the box's background.
    assert_eq!(
        content_of(&rig, &parent),
        [PaintOp::Rect {
            x: 0.0,
            y: 0.0,
            width: 9.0,
            height: 9.0,
            radius: 0.0,
            fill: gnoblin_paint::Rgba8::new(0x11, 0x22, 0x33, 255),
            stroke: gnoblin_paint::Rgba8::TRANSPARENT,
            stroke_width: 0.0,
        }]
    );
    // What the library would have painted is still there to ask for, and so is its arrangement.
    let own = parent.call("paint_default", &[]);
    assert_eq!(own.list().expect("a list").len(), 1);
    parent.call("arrange_default", &[]);
    assert_eq!(
        kids.borrow()[1].num("x"),
        10.0,
        "the row of the library puts the second child at 10"
    );

    // A canvas given no width of its own takes the natural width the shell gave.
    let sized = rig.make("canvas", &[]);
    sized.set(
        "behaviour",
        implement(&rig, &["natural_width", "natural_height"], |name, _| {
            Ok(if name == "natural_width" { d(33.0) } else { d(44.0) })
        }),
    );
    assert_eq!((sized.num("width"), sized.num("height")), (33.0, 44.0));
    sized.set("width", d(5.0));
    assert_eq!(sized.num("width"), 5.0);
    sized.call("clear_width", &[]);
    assert_eq!(sized.num("width"), 33.0, "a width cleared is the natural one again");
    assert_eq!(sized.num("natural_width"), 33.0);
}

#[test]
fn a_handler_stops_an_event_and_a_signal_tells_what_reached_a_canvas() {
    let mut rig = scratch();
    let outer = rig.make("box", &[("width", d(100.0)), ("height", d(100.0))]);
    let inner = rig.make(
        "box",
        &[("x", d(10.0)), ("y", d(10.0)), ("width", d(20.0)), ("height", d(20.0))],
    );
    outer.add(inner);
    outer.mount_top();
    let reached = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let log = Rc::clone(&reached);
        inner.set(
            "behaviour",
            implement(&rig, &["on_pointer", "on_key"], move |name, args| {
                let event = fields(args);
                log.borrow_mut().push(format!("hook {name}"));
                // Only a press is the hook's: the rest go on.
                Ok(Val::Bool(name == "on_pointer" && event[0].enum_value()? == 1))
            }),
        );
    }
    {
        let log = Rc::clone(&reached);
        outer.set(
            "on_pointer",
            callback(move |_| {
                log.borrow_mut().push("outer".to_owned());
                Ok(Val::Bool(false))
            }),
        );
    }
    let told = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let told = Rc::clone(&told);
        hear(&outer, "pointer", move |args| {
            let event = fields(args);
            told.borrow_mut().push(format!(
                "pointer {:?} at {}",
                event[0].enum_value().unwrap_or(0),
                event[1].double().unwrap_or(0.0)
            ));
        });
    }
    rig.settle();
    rig.comp.pointer(inner.node(), "press", 1.0, 2.0, 272);
    rig.settle();
    assert_eq!(
        *reached.borrow(),
        ["hook on_pointer"],
        "the hook answered true: the outer canvas heard nothing"
    );
    assert!(
        told.borrow().is_empty(),
        "a canvas the event did not reach tells nothing"
    );
    rig.comp.pointer(inner.node(), "motion", 1.0, 2.0, 0);
    rig.settle();
    assert_eq!(*reached.borrow(), ["hook on_pointer", "hook on_pointer", "outer"]);
    assert_eq!(
        *told.borrow(),
        ["pointer 3 at 11"],
        "the outer canvas's signal: a motion, in its own coordinates"
    );
    // A focus change is told to the canvas that lost it and the one that gained it.
    let focus = Rc::new(RefCell::new(Vec::<bool>::new()));
    {
        let focus = Rc::clone(&focus);
        hear(&inner, "focus_changed", move |args| {
            focus.borrow_mut().push(args[0].bool().unwrap_or(false))
        });
    }
    inner.focus();
    rig.settle();
    assert_eq!(*focus.borrow(), [true]);
    assert!(inner.flag("focused"));
    rig.comp.key(inner.node(), "q", "q", true);
    rig.settle();
    assert_eq!(reached.borrow().last().map(String::as_str), Some("hook on_key"));
    outer.focus();
    rig.settle();
    assert_eq!(*focus.borrow(), [true, false]);
}

#[test]
fn a_canvas_whose_node_goes_reads_as_it_was_and_says_so() {
    let mut rig = scratch();
    rig.comp.create_window(3, "win", "app");
    let card = rig.make("box", &[("x", d(4.0)), ("width", d(30.0)), ("height", d(20.0))]);
    let label = rig.make("text", &[("text", t("hello"))]);
    card.add(label);
    card.call("mount", &[Val::Uint(3)]);
    rig.settle();
    assert!(card.flag("alive"));
    assert_eq!(rig.comp.children(3), [card.node()]);
    let alive = Rc::new(RefCell::new(Vec::<bool>::new()));
    let gone = Rc::new(Cell::new(0_u32));
    {
        let alive = Rc::clone(&alive);
        hear_property(&card, "alive", move |args| {
            alive.borrow_mut().push(args[0].bool().unwrap_or(true))
        });
        let gone = Rc::clone(&gone);
        hear(&label, "gone", move |_| gone.set(gone.get() + 1));
    }
    let before = rig.function("ui", "canvas_count", &[]).uint().expect("a count");
    assert_eq!(before, 2);
    // The compositor takes the window, and its canvases with it.
    rig.comp.destroy(3);
    rig.settle();
    assert!(!card.flag("alive"));
    assert!(!label.flag("alive"));
    assert_eq!(card.num("x"), 4.0, "what the canvas was stays readable");
    assert_eq!(label.text("text"), "hello");
    assert!(card.kids().is_empty(), "a canvas that is gone holds no children");
    assert_eq!(*alive.borrow(), [false]);
    assert_eq!(gone.get(), 1);
    assert_eq!(rig.function("ui", "canvas_count", &[]).uint().expect("a count"), 0);
    // What a canvas that is gone refuses.
    assert!(card.try_call("mount_top", &[]).is_err());
    assert!(card.try_call("focus", &[]).is_err());
    assert!(card.try_call("add", &[label.val()]).is_err());
    // Its object goes when the shell says so, and nothing is left behind.
    card.destroy();
    assert!(card.try_set("x", d(1.0)).is_err(), "the handle is stale");
}

#[test]
fn the_session_values_and_the_hooks_of_the_frame() {
    let mut rig = scratch();
    let now = rig.function("ui", "now", &[]).double().expect("a time");
    rig.advance(5);
    assert!(rig.function("ui", "now", &[]).double().expect("a time") >= now);
    let screen = rig.function("ui", "screen", &[]);
    assert_eq!(numbers(screen), [1000.0, 800.0]);
    assert!(!rig.function("ui", "locked", &[]).bool().expect("a flag"));
    rig.comp.external_set(22, "engaged", MirrorValue::Bool(true));
    rig.settle();
    assert!(rig.function("ui", "locked", &[]).bool().expect("a flag"));
    rig.comp.external_set(21, "pointer_x", MirrorValue::Double(12.0));
    rig.comp.external_set(21, "pointer_y", MirrorValue::Double(34.0));
    rig.settle();
    assert_eq!(numbers(rig.function("ui", "pointer", &[])), [12.0, 34.0]);
    assert_eq!(
        rig.function("ui", "seat_action", &[t("double_click")])
            .text()
            .expect("text"),
        "toggle-maximize",
        "the seat says nothing: the default stands"
    );
    rig.comp
        .external_set(21, "right_click", MirrorValue::Text("close".to_owned()));
    rig.settle();
    assert_eq!(
        rig.function("ui", "seat_action", &[t("right_click")])
            .text()
            .expect("text"),
        "close"
    );
    assert_eq!(
        rig.function("ui", "seat_action", &[t("no_such_click")])
            .text()
            .expect("text"),
        ""
    );

    // A frame hook runs once a frame, and stops when it is removed.
    let frames = Rc::new(Cell::new(0_u32));
    let count = Rc::clone(&frames);
    let hook = match rig.function(
        "ui",
        "on_frame",
        &[callback(move |_| {
            count.set(count.get() + 1);
            Ok(Val::None)
        })],
    ) {
        Val::Object { class, obj } => W {
            api: rig.api,
            class,
            obj,
        },
        other => panic!("a hook, got {other:?}"),
    };
    let canvas = rig.make("canvas", &[]);
    rig.settle();
    let ran = frames.get();
    assert!(ran >= 1, "the hook ran in the turn that made a canvas");
    canvas.call("invalidate", &[]);
    rig.settle();
    assert!(frames.get() > ran, "and again in the turn an invalidation asked for");
    hook.call("remove", &[]);
    let removed = frames.get();
    canvas.call("invalidate", &[]);
    rig.settle();
    assert_eq!(frames.get(), removed, "a hook that is removed runs no more");
    hook.call("remove", &[]);

    // A request of a window goes to the request hooks.
    let requests = Rc::new(RefCell::new(Vec::<(u64, String, f64, f64)>::new()));
    let seen = Rc::clone(&requests);
    rig.function(
        "ui",
        "on_request",
        &[callback(move |args| {
            let request = fields(args);
            seen.borrow_mut().push((
                request[0].uint()?,
                request[1].text()?.to_owned(),
                request[2].double()?,
                request[3].double()?,
            ));
            Ok(Val::None)
        })],
    );
    rig.comp.create_window(3, "win", "app");
    rig.settle();
    rig.comp.request(3, "menu", 10.0, 20.0);
    rig.settle();
    assert_eq!(*requests.borrow(), [(3, "menu".to_owned(), 10.0, 20.0)]);

    // The keyboard goes to a window as well as to a canvas, and is taken away with zero.
    rig.function("ui", "focus", &[Val::Uint(3)]);
    rig.settle();
    assert_eq!(rig.function("ui", "focused", &[]).uint().expect("a node"), 3);
    rig.function("ui", "focus", &[Val::Uint(0)]);
    rig.settle();
    assert_eq!(rig.function("ui", "focused", &[]).uint().expect("a node"), 0);
}

/// The numbers of a record of numbers.
fn numbers(record: Val) -> Vec<f64> {
    fields(&[record])
        .iter()
        .map(|field| field.double().expect("a number"))
        .collect()
}

#[test]
fn what_a_canvas_refuses() {
    let rig = scratch();
    let parent = rig.make("box", &[]);
    let child = rig.make("box", &[]);
    let grandchild = rig.make("box", &[]);
    parent.add(child);
    child.add(grandchild);
    let refused = |result: Result<Val, gnoblin_object::Fail>, what: &str| {
        let fail = result.expect_err(what);
        assert!(fail.message.contains(what), "{what}: {}", fail.message);
    };
    refused(parent.try_call("add", &[parent.val()]), "its own ancestor");
    refused(grandchild.try_call("add", &[parent.val()]), "its own ancestor");
    let stranger = rig.make("box", &[]);
    refused(parent.try_call("remove", &[stranger.val()]), "not a child");
    refused(
        rig.api.call(
            rig.api.class("ui").map(|_| parent.obj).expect("ui"),
            rig.api.member(parent.class, MEMBER_METHOD, "add").expect("add"),
            &[Val::Text("no".to_owned())],
        ),
        "canvas",
    );
    // A canvas the shell removed is gone, and cannot be put back.
    parent.call("remove", &[child.val()]);
    assert!(!child.flag("alive") && !grandchild.flag("alive"));
    refused(parent.try_call("add", &[child.val()]), "destroyed");
    // A key that makes no sense to an enum is refused by the protocol.
    assert!(parent.try_set("direction", d(1.0)).is_err());
    assert!(parent.try_set("placement", en(9)).is_err());
}

#[test]
fn a_property_of_a_canvas_reaches_its_node_when_the_toolkit_turns() {
    let mut rig = scratch();
    let canvas = rig.make(
        "box",
        &[
            ("width", d(40.0)),
            ("height", d(20.0)),
            ("placement", en(2)),
            ("secure", b(true)),
        ],
    );
    canvas.mount_top();
    rig.settle();
    let node = canvas.node();
    let comp = rig.comp.clone();
    let value = move |key: &str| comp.value(node, key);
    assert_eq!(value("placement"), Some(MirrorValue::Text("over".to_owned())));
    assert_eq!(value("secure"), Some(MirrorValue::Bool(true)));
    assert_eq!(value("width"), Some(MirrorValue::Double(40.0)));
    // A write that changes nothing is not sent: only what differs from what was last written goes out.
    let sent = rig.comp.sent_count();
    canvas.set("width", d(40.0));
    rig.settle();
    assert_eq!(rig.comp.sent_count(), sent);
    canvas.set("input", b(false));
    canvas.set("opacity", d(0.25));
    canvas.set("visible", b(false));
    canvas.set("placement", en(3));
    rig.settle();
    assert_eq!(value("input"), Some(MirrorValue::Bool(false)));
    assert_eq!(value("opacity"), Some(MirrorValue::Double(0.25)));
    assert_eq!(value("visible"), Some(MirrorValue::Bool(false)));
    assert_eq!(value("placement"), Some(MirrorValue::Text("under".to_owned())));
    assert_eq!(canvas.kids().len(), 0);
    // A box taller than its column leaves a negative share of space at zero.
    let column = rig.make(
        "box",
        &[("width", d(20.0)), ("height", d(10.0)), ("direction", en(COLUMN))],
    );
    column.add(rig.make("box", &[("height", d(30.0))]));
    column.add(rig.make("box", &[("grow", d(1.0))]));
    rig.settle();
    assert_eq!(column.kid(1).num("height"), 0.0);
    assert_eq!(column.kid(1).num("y"), 30.0);
}

#[test]
fn a_canvas_that_fails_to_paint_is_counted_each_turn_and_reported_once() {
    let mut rig = scratch();
    let bad = rig.make("canvas", &[("width", d(5.0)), ("height", d(5.0))]);
    bad.set(
        "behaviour",
        implement(&rig, &["paint"], |_, _| {
            Ok(Val::List(vec![Val::Record {
                class: 0,
                fields: {
                    let mut fields = vec![Val::None; 19];
                    fields[0] = en(1);
                    fields[6] = t("red");
                    fields
                },
            }]))
        }),
    );
    rig.settle();
    let first = bad.get("failures").uint().expect("a count");
    assert!(first >= 1);
    bad.call("invalidate", &[]);
    rig.settle();
    bad.call("invalidate", &[]);
    rig.settle();
    assert!(
        bad.get("failures").uint().expect("a count") > first,
        "every turn that fails is counted"
    );
    let lines = rig.reported();
    assert_eq!(lines.len(), 1, "and said once: {lines:?}");
    assert!(lines[0].contains("draw op 1: a colour is"), "{}", lines[0]);
    // The node never got the paint list the shell gave, and the canvas is still alive.
    assert_eq!(rig.comp.value(bad.node(), "content"), None);
    assert!(bad.flag("alive"));
}
