#![allow(unsafe_code)]
//! A member of `gnoblin.std` that takes a window takes the core's `Client` (`object:gnoblin.model.client`), as the TypeScript library took
//! a `NativeClient`: what a shell passes is checked, and never trusted.
//!
//! The failure modes, and the test that covers each:
//!
//! - the shell passes the window as text, the node number it used to be: `every_member_that_takes_a_client_refuses_what_is_not_one`;
//! - the shell passes a handle it released: `a_released_client_is_stale_everywhere`;
//! - the shell passes an object of another class of the core, or a handle that names a window of another generation's table by number
//!   only: `an_object_of_another_class_is_a_type_error_everywhere`;
//! - the window is gone but the shell still holds its `Client` (the core keeps it, frozen, and does not call it stale):
//!   `a_destroyed_client_is_gone_everywhere`, and the message has the shape the core gives a write to the same client;
//! - the window is there: `a_live_client_is_the_window_it_names` for the members whose effect is visible here, and
//!   `a_live_client_is_never_refused_by_the_check` for every member, so a member that asks the resolver for the wrong thing is found;
//! - a refused call changes nothing: `a_refused_client_changes_nothing`.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::{Fail, Val};
use rig::{Rig, W, b, d, en, t};

/// The receivers of every member that takes a client, with the desktop they run over: two windows (nodes 3 and 4) and one workspace.
struct World {
    rig: Rig,
    decorations: W,
    menus: W,
    state: W,
    reposition: W,
    tracker: W,
    alerts: W,
    clients: W,
    workspace: W,
    list: W,
    desks: W,
    view: W,
}

impl World {
    fn new() -> World {
        let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 2);
        let list = rig.make("workspaces", &[]);
        let workspace = rig.make("workspace", &[("id", t("main"))]);
        list.call("add", &[workspace.val()]);
        let decorations = rig.make("decorations", &[]);
        let menus = rig.make("window_menus", &[]);
        menus.call("start", &[]);
        let state = rig.make("window_state", &[]);
        let reposition = rig.make("repositioning", &[]);
        let tracker = rig.make("alert_tracker", &[]);
        let alerts = rig.make("not_responding_alerts", &[]);
        let desks = rig.make("desks", &[]);
        let view = rig.make("view", &[("id", t("panel"))]);
        let clients = workspace.obj("clients");
        rig.settle();
        World {
            rig,
            decorations,
            menus,
            state,
            reposition,
            tracker,
            alerts,
            clients,
            workspace,
            list,
            desks,
            view,
        }
    }
}

/// One member that takes a client: what it is called on, what it is called, and the arguments around the client.
struct Member {
    label: &'static str,
    call: fn(&World, &Val) -> Result<Val, Fail>,
}

/// Calls the member `name` of `on` with `args`.
fn call(on: &W, name: &str, args: &[Val]) -> Result<Val, Fail> {
    on.try_call(name, args)
}

/// Every member of the catalog that takes `object:gnoblin.model.client`, in the order of the schema files. A member added to the catalog
/// with that type and left out of this list is found by `the_list_names_every_member_the_catalog_gives_a_client`.
fn members() -> Vec<Member> {
    macro_rules! m {
        ($label:literal, |$w:ident, $c:ident| $body:expr) => {
            Member {
                label: $label,
                call: |$w: &World, $c: &Val| $body,
            }
        };
    }
    vec![
        m!("decorations.frame_of", |w, c| call(
            &w.decorations,
            "frame_of",
            std::slice::from_ref(c)
        )),
        m!("decorations.inset_of", |w, c| call(
            &w.decorations,
            "inset_of",
            std::slice::from_ref(c)
        )),
        m!("decorations.reserved_of", |w, c| call(
            &w.decorations,
            "reserved_of",
            std::slice::from_ref(c)
        )),
        m!("decorations.reframe", |w, c| call(
            &w.decorations,
            "reframe",
            std::slice::from_ref(c)
        )),
        m!("window_menus.default_items", |w, c| call(
            &w.menus,
            "default_items",
            std::slice::from_ref(c)
        )),
        m!("window_menus.items", |w, c| call(
            &w.menus,
            "items",
            std::slice::from_ref(c)
        )),
        m!("window_menus.show", |w, c| call(
            &w.menus,
            "show",
            &[c.clone(), d(10.0), d(10.0)]
        )),
        m!("window_menus.open", |w, c| call(
            &w.menus,
            "open",
            &[c.clone(), d(10.0), d(10.0)]
        )),
        m!("window_menus.is_above", |w, c| call(
            &w.menus,
            "is_above",
            std::slice::from_ref(c)
        )),
        m!("window_menus.set_above", |w, c| call(
            &w.menus,
            "set_above",
            &[c.clone(), b(false)]
        )),
        m!("window_state.add_listener", |w, c| {
            let listener = w
                .rig
                .implement("request_listener", &["on_request"], |_, _| Ok(Val::None));
            call(&w.state, "add_listener", &[listener, c.clone()])
        }),
        m!("window_state.answer", |w, c| call(
            &w.state,
            "answer",
            &[c.clone(), en(1)]
        )),
        m!("window_state.maximise", |w, c| call(
            &w.state,
            "maximise",
            std::slice::from_ref(c)
        )),
        m!("window_state.unmaximise", |w, c| call(
            &w.state,
            "unmaximise",
            std::slice::from_ref(c)
        )),
        m!("window_state.enter_fullscreen", |w, c| call(
            &w.state,
            "enter_fullscreen",
            std::slice::from_ref(c)
        )),
        m!("window_state.leave_fullscreen", |w, c| call(
            &w.state,
            "leave_fullscreen",
            std::slice::from_ref(c)
        )),
        m!("window_state.minimise", |w, c| call(
            &w.state,
            "minimise",
            std::slice::from_ref(c)
        )),
        m!("window_state.unminimise", |w, c| call(
            &w.state,
            "unminimise",
            std::slice::from_ref(c)
        )),
        m!("window_state.present", |w, c| call(
            &w.state,
            "present",
            std::slice::from_ref(c)
        )),
        m!("window_state.raise", |w, c| call(
            &w.state,
            "raise",
            std::slice::from_ref(c)
        )),
        m!("window_state.lower", |w, c| call(
            &w.state,
            "lower",
            std::slice::from_ref(c)
        )),
        m!("window_state.begin_move", |w, c| call(
            &w.state,
            "begin_move",
            std::slice::from_ref(c)
        )),
        m!("window_state.begin_resize", |w, c| call(
            &w.state,
            "begin_resize",
            std::slice::from_ref(c)
        )),
        m!("window_state.kept", |w, c| call(
            &w.state,
            "kept",
            std::slice::from_ref(c)
        )),
        m!("window_state.keep", |w, c| call(
            &w.state,
            "keep",
            &[
                c.clone(),
                Val::Record {
                    class: 0,
                    fields: vec![]
                }
            ]
        )),
        m!("repositioning.begin", |w, c| call(
            &w.reposition,
            "begin",
            &[c.clone(), en(1)]
        )),
        m!("view.show_client", |w, c| call(
            &w.view,
            "show_client",
            std::slice::from_ref(c)
        )),
        m!("alert_tracker.is_open", |w, c| call(
            &w.tracker,
            "is_open",
            std::slice::from_ref(c)
        )),
        m!("alert_tracker.is_waiting", |w, c| call(
            &w.tracker,
            "is_waiting",
            std::slice::from_ref(c)
        )),
        m!("alert_tracker.answered", |w, c| call(
            &w.tracker,
            "answered",
            &[c.clone(), en(1)]
        )),
        m!("not_responding_alerts.card_of", |w, c| call(
            &w.alerts,
            "card_of",
            std::slice::from_ref(c)
        )),
        m!("client_collection.contains", |w, c| call(
            &w.clients,
            "contains",
            std::slice::from_ref(c)
        )),
        m!("client_collection.add", |w, c| call(
            &w.clients,
            "add",
            std::slice::from_ref(c)
        )),
        m!("client_collection.remove", |w, c| call(
            &w.clients,
            "remove",
            std::slice::from_ref(c)
        )),
        m!("client_collection.swap", |w, c| call(
            &w.clients,
            "swap",
            &[c.clone(), c.clone()]
        )),
        m!("client_state.of", |w, c| {
            let class = w.rig.api.class("client_state").expect("the class");
            let member = w.rig.api.member(class, abi::MEMBER_METHOD, "of").expect("the function");
            w.rig.api.call_static(class, member, std::slice::from_ref(c))
        }),
        m!("workspace.find_split", |w, c| call(
            &w.workspace,
            "find_split",
            &[c.clone(), d(1.0), d(1.0)]
        )),
        m!("workspaces.of_client", |w, c| call(
            &w.list,
            "of_client",
            std::slice::from_ref(c)
        )),
        m!("desks.areas", |w, c| call(&w.desks, "areas", std::slice::from_ref(c))),
        m!("desks.restore_client", |w, c| call(
            &w.desks,
            "restore_client",
            &[w.workspace.val(), c.clone(), b(false)]
        )),
        m!("desks.restore_snap", |w, c| call(
            &w.desks,
            "restore_snap",
            &[c.clone(), en(1), rect(0.0, 0.0, 10.0, 10.0)]
        )),
    ]
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![d(x), d(y), d(width), d(height)],
    }
}

/// Runs every member with the value `bad` for its client and checks each refuses it with `code` and a message that has all of `words`.
#[track_caller]
fn every_member_refuses(world: &World, bad: &Val, code: i32, words: &[&str]) {
    let mut wrong = Vec::new();
    for member in members() {
        match (member.call)(world, bad) {
            Ok(value) => wrong.push(format!("{}: answered {value:?}", member.label)),
            Err(fail) if fail.code != code || !words.iter().all(|word| fail.message.contains(word)) => {
                wrong.push(format!("{}: {} ({})", member.label, fail.message, fail.code));
            }
            Err(_) => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "every member must refuse with {code} and {words:?}:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_member_that_takes_a_client_refuses_what_is_not_one() {
    let world = World::new();
    // The node number as text is what the members took before the catalog named the core's class. It is refused by name now, not
    // misread: a window's number is not a window.
    every_member_refuses(&world, &t("3"), abi::E_TYPE, &["gnoblin.model.client"]);
    every_member_refuses(&world, &Val::Uint(3), abi::E_TYPE, &["gnoblin.model.client"]);
    every_member_refuses(&world, &Val::Bool(true), abi::E_TYPE, &["gnoblin.model.client"]);
}

#[test]
fn a_released_client_is_stale_everywhere() {
    let mut world = World::new();
    let client = world.rig.client(4);
    let Val::Object { obj, .. } = client else {
        panic!("a client is an object: {client:?}");
    };
    // The core owns the object of a window that is there: a shell that lets go of its reference to a live window still has a handle that
    // names it, and the members still take it.
    world.rig.core.release(obj).expect("the shell lets go of its reference");
    let kept = world.menus.try_call("is_above", std::slice::from_ref(&client));
    assert!(
        kept.is_ok(),
        "a live window's handle outlives the shell's reference: {kept:?}"
    );
    // Once the window is gone and the shell has let go of the last reference, the handle names nothing at all: a released handle is
    // stale, as a call on any released handle says, and is not a window that is gone.
    let again = world.rig.client(4);
    world.rig.external_destroy(4);
    world.rig.settle();
    let Val::Object { obj: held, .. } = again else {
        panic!("a client is an object: {again:?}");
    };
    assert_eq!(held, obj, "the core has one object for a window");
    world
        .rig
        .core
        .release(held)
        .expect("the shell lets go of the last reference");
    world.rig.settle();
    every_member_refuses(&world, &client, abi::E_STALE, &[]);
}

#[test]
fn an_object_of_another_class_is_a_type_error_everywhere() {
    let world = World::new();
    // The core's root object is an object of the core, and is not a window.
    let service = world.rig.core.class("gnoblin").expect("the core's root class");
    let root = world.rig.core.create(service, &[]).expect("the root object");
    let other = Val::Object {
        class: service,
        obj: root,
    };
    every_member_refuses(&world, &other, abi::E_TYPE, &["Client", "another class"]);
    // The class the sender names is not what decides: a handle of the root object that claims to be a client is still the root object.
    let claims = Val::Object {
        class: world.rig.core.class("client").expect("the client class"),
        obj: root,
    };
    every_member_refuses(&world, &claims, abi::E_TYPE, &["Client", "another class"]);
}

#[test]
fn a_destroyed_client_is_gone_everywhere() {
    let mut world = World::new();
    let client = world.rig.client(4);
    world.rig.external_destroy(4);
    world.rig.settle();
    // The core still holds the client as a frozen object: it is not stale, and it is not alive.
    let Val::Object { obj, class } = client else {
        panic!("a client is an object: {client:?}");
    };
    let alive = world
        .rig
        .core
        .member(class, abi::MEMBER_PROPERTY, "alive")
        .and_then(|member| world.rig.core.get(obj, member))
        .expect("a destroyed client can still be read");
    assert!(matches!(alive, Val::Bool(false)), "the client is not alive: {alive:?}");
    // The same code and the same words the core answers for a write to this client, so a shell sees one error for "this window is gone".
    every_member_refuses(&world, &client, abi::E_NO_SUCH_NODE, &["Client#", "the client is gone"]);
    let fullscreen = world
        .rig
        .core
        .member(class, abi::MEMBER_PROPERTY, "fullscreen")
        .and_then(|member| world.rig.core.set(obj, member, &Val::Bool(true)));
    let core_says = fullscreen.expect_err("the core refuses a write to a destroyed client");
    assert_eq!(core_says.code, abi::E_NO_SUCH_NODE, "{}", core_says.message);
    assert!(
        core_says.message.contains("the client is gone"),
        "{}",
        core_says.message
    );
}

#[test]
fn a_destroyed_client_names_itself_in_the_message() {
    let mut world = World::new();
    let client = world.rig.client(4);
    let Val::Object { class, obj } = client else {
        panic!("a client is an object: {client:?}");
    };
    let id = world
        .rig
        .core
        .member(class, abi::MEMBER_PROPERTY, "id")
        .and_then(|member| world.rig.core.get(obj, member))
        .expect("the id of a live client")
        .text()
        .expect("text")
        .to_owned();
    world.rig.external_destroy(4);
    world.rig.settle();
    let fail = world
        .decorations
        .try_call("inset_of", &[client])
        .expect_err("a destroyed client is refused");
    // The message says which window is gone and which member was asked, in the shape of the core's own.
    assert!(
        fail.message
            .contains(&format!("Client#{id}.inset_of: the client is gone")),
        "{}",
        fail.message
    );
}

#[test]
fn a_live_client_is_the_window_it_names() {
    let world = World::new();
    let three = world.rig.client(3);
    let four = world.rig.client(4);
    // `is_above` and `set_above` are per window: the one that was set is the one that answers.
    assert!(matches!(
        world.menus.call("is_above", std::slice::from_ref(&three)),
        Val::Bool(false)
    ));
    world.menus.call("set_above", &[three.clone(), b(true)]);
    assert!(matches!(
        world.menus.call("is_above", std::slice::from_ref(&three)),
        Val::Bool(true)
    ));
    assert!(matches!(
        world.menus.call("is_above", std::slice::from_ref(&four)),
        Val::Bool(false)
    ));
    // A workspace holds the window it was given, and not the other: adding one leaves the other as it was.
    let holds = |client: &Val| {
        matches!(
            world.clients.call("contains", std::slice::from_ref(client)),
            Val::Bool(true)
        )
    };
    let three_held = holds(&three);
    world.clients.call("remove", std::slice::from_ref(&four));
    assert!(!holds(&four));
    world.clients.call("add", std::slice::from_ref(&four));
    assert!(holds(&four));
    assert_eq!(holds(&three), three_held, "the other window is as it was");
    assert!(matches!(
        world.clients.call("remove", std::slice::from_ref(&four)),
        Val::Bool(true)
    ));
    assert!(!holds(&four));
    // The window list answers the workspace of the window.
    world.clients.call("add", std::slice::from_ref(&three));
    let found = world.list.call("of_client", &[three]);
    let found = match found {
        Val::Object { class, obj } => W {
            api: world.list.api,
            class,
            obj,
        },
        other => panic!("the window is in a workspace: {other:?}"),
    };
    assert_eq!(found.text("id"), "main");
}

#[test]
fn a_live_client_is_never_refused_by_the_check() {
    let world = World::new();
    let client = world.rig.client(3);
    let mut refused = Vec::new();
    for member in members() {
        if let Err(fail) = (member.call)(&world, &client)
            && (fail.message.contains("takes a Client") || fail.message.contains("the client is gone"))
        {
            refused.push(format!("{}: {}", member.label, fail.message));
        }
    }
    assert!(
        refused.is_empty(),
        "a live client is what every member takes:\n{}",
        refused.join("\n")
    );
}

#[test]
fn swap_checks_both_clients() {
    let world = World::new();
    let live = world.rig.client(3);
    for (first, second) in [(&live, &t("4")), (&t("4"), &live)] {
        let refused = world.clients.try_call("swap", &[first.clone(), second.clone()]);
        assert_eq!(refused.expect_err("one of the two is not a client").code, abi::E_TYPE);
    }
}

#[test]
fn a_refused_client_changes_nothing() {
    let mut world = World::new();
    let client = world.rig.client(4);
    world.rig.external_destroy(4);
    world.rig.settle();
    let before = world.rig.comp.flag(3, "maximised");
    let calls = Rc::new(Cell::new(0));
    for member in members() {
        let _ = (member.call)(&world, &client);
        calls.set(calls.get() + 1);
    }
    world.rig.settle();
    assert!(calls.get() > 35, "every member was tried");
    assert_eq!(
        world.rig.comp.flag(3, "maximised"),
        before,
        "a call refused for a window that is gone changes no other window"
    );
}

#[test]
fn an_optional_client_left_out_is_not_a_bad_one() {
    let world = World::new();
    // `desks.areas` answers the primary output's room for no client at all.
    let none = world.desks.try_call("areas", &[Val::None]);
    assert!(none.is_ok(), "{none:?}");
    // A listener taking one window: absent is every window.
    let listener = world
        .rig
        .implement("request_listener", &["on_request"], |_, _| Ok(Val::None));
    assert!(
        world
            .state
            .try_call("add_listener", std::slice::from_ref(&listener))
            .is_ok(),
        "a listener for every window"
    );
    assert!(
        world
            .state
            .try_call("add_listener", &[listener.clone(), world.rig.client(3)])
            .is_ok(),
        "a listener for one window"
    );
    let bad = world.state.try_call("add_listener", &[listener, t("3")]);
    assert_eq!(bad.expect_err("text is not a client").code, abi::E_TYPE);
}

/// The members of the schema files that take a client from the shell, as `class.member`: read from the text of the files, so a member
/// added with the type and left out of [`members`] fails here and is not left untested.
fn catalog_members() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../gnoblin-std-abi/schema");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).expect("the schema directory") {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a schema file");
        let (mut class, mut method, mut interface) = (String::new(), String::new(), false);
        let mut last = "";
        for line in text.lines() {
            if line.starts_with("[[") {
                last = if line == "[[object]]" {
                    "object"
                } else if line == "[[object.method]]" {
                    "method"
                } else {
                    ""
                };
                continue;
            }
            let Some((key, value)) = line.split_once(" = ") else {
                continue;
            };
            let value = value.trim_matches('"');
            match (last, key) {
                ("object", "name") => {
                    class = value.to_owned();
                    interface = false;
                }
                ("object", "kind") => interface = value == "interface",
                ("method", "name") => method = value.to_owned(),
                _ => {}
            }
            if key == "type" && value == "object:gnoblin.model.client" && !interface {
                let label = format!("{class}.{method}");
                if !found.contains(&label) {
                    found.push(label);
                }
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_list_names_every_member_the_catalog_gives_a_client() {
    let mut tested: Vec<String> = members().iter().map(|member| member.label.to_owned()).collect();
    tested.sort();
    assert_eq!(tested, catalog_members());
}
