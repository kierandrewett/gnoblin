//! Named subscriptions on the host objects: `client.onTitleChanged(handler)`, `gnoblin.clients.onAdded(handler)`,
//! `output.onModeChanged(handler)` and `gnoblin.onClientCreated(handler)`, over the fake compositor of `tests/common`.
//!
//! A listener method is also a subscription: it attaches a listener that has that one method and returns the function
//! that detaches it, by the one `subscriber` of `gnoblin-core/subscriptions`. These tests hold three promises:
//!
//! - every `on…` method a listener class declares in the library's sources is a subscription on the objects that take the
//!   listener, and no subscription is there that no class declares (the names are read from the sources, so a method added
//!   to a class without its subscription fails here);
//! - each one hears its event and, once unsubscribed, hears nothing;
//! - `addListener` and a subscription are one list: they hear the same events in the order they were attached.
//!
//! The session's and the runtime's subscriptions, which the same implementation serves, are in `host_api.rs`.

use std::collections::BTreeSet;
use std::path::Path;

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

/// A shell on `gnoblin-core` alone, with a log the script keeps in `log`.
const CORE: &str =
    "import { gnoblin } from \"gnoblin-core\";\n(globalThis as any).g = globalThis;\n(globalThis as any).log = [];\n";

/// A shell that has `gnoblin-std` too, which adds `ClientListener.onRequest`.
const STD: &str = "import { gnoblin } from \"gnoblin-core\";\nimport \"gnoblin-std\";\n(globalThis as any).g = globalThis;\n(globalThis as any).log = [];\n";

/// A 1000 by 600 output `DP-1`, and the two windows `3` and `4` that were there at boot.
fn world(comp: &FakeCompositor) {
    comp.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("name", text("output-0")),
            ("connector", text("DP-1")),
            ("x", MirrorValue::Double(0.0)),
            ("y", MirrorValue::Double(0.0)),
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(600.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_window(3, "one", "app.one");
    comp.add_window(4, "two", "app.two");
}

fn rig(imports: &str, body: &str) -> Rig {
    let mut rig = rig_setup(imports, body, world);
    let told = rig.settle();
    assert!(told.is_empty(), "{told:?} {:?}", rig.load_diagnostics);
    rig
}

/// Runs a statement in the script.
fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

#[track_caller]
fn settle(rig: &mut Rig) {
    let told = rig.settle();
    assert!(told.is_empty(), "{told:?}");
}

// ---- the names the sources declare --------------------------------------------------------------------------------------

/// The text of a source of the library, by its path under `data/js`.
fn source(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js").join(path);
    std::fs::read_to_string(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()))
}

/// The `on…` methods the listener declared by the line that contains `header` (`class ClientListener`): the lines that
/// begin `onName?(` or `onName(` between it and its closing brace.
fn declared(text: &str, header: &str) -> BTreeSet<String> {
    let mut lines = text.lines();
    let opener = lines
        .by_ref()
        .find(|line| line.contains(header) && line.ends_with('{'))
        .unwrap_or_else(|| panic!("`{header}` is declared"));
    let indent = opener.len() - opener.trim_start().len();
    let mut found = BTreeSet::new();
    for line in lines {
        if line.len() - line.trim_start().len() == indent && line.trim_start().starts_with('}') {
            return found;
        }
        let name: String = line
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let after = &line.trim_start()[name.len()..];
        if name.starts_with("on")
            && name[2..].starts_with(|c: char| c.is_ascii_uppercase())
            && (after.starts_with("?(") || after.starts_with('('))
        {
            found.insert(name);
        }
    }
    panic!("`{header}` is never closed");
}

/// The methods of each listener class of the core library, as its sources declare them.
struct Declared {
    client: BTreeSet<String>,
    collection: BTreeSet<String>,
    output: BTreeSet<String>,
    session: BTreeSet<String>,
    runtime: BTreeSet<String>,
}

fn core_declared() -> Declared {
    let listeners = source("gnoblin-core/host/listeners.ts");
    let events = source("gnoblin-core/session/events.ts");
    Declared {
        client: declared(&listeners, "export class ClientListener {"),
        collection: declared(&listeners, "export class CollectionListener<"),
        output: declared(&listeners, "export class OutputListener {"),
        session: declared(&events, "export class SessionListener {"),
        runtime: declared(&events, "export class RuntimeListener {"),
    }
}

/// What `gnoblin-std` adds to `ClientListener` by declaration merging.
fn std_client_methods() -> BTreeSet<String> {
    declared(&source("gnoblin-std/host/members.ts"), "interface ClientListener {")
}

fn json(names: &BTreeSet<String>) -> String {
    serde_json::to_string(names).expect("names are JSON")
}

/// A script that lists the `on…` members an object has beside the names it should have, and returns every difference.
fn check_names(declared: &Declared, client_extra: &BTreeSet<String>) -> String {
    let mut client = declared.client.clone();
    client.extend(client_extra.iter().cloned());
    let mut every_member = client.clone();
    every_member.extend(declared.collection.iter().cloned());
    every_member.extend(declared.output.iter().cloned());
    let mut gnoblin: BTreeSet<String> = client
        .iter()
        .map(|name| format!("onClient{}", &name[2..]))
        .chain(declared.output.iter().map(|name| format!("onOutput{}", &name[2..])))
        .chain(declared.collection.iter().map(|name| format!("onOutput{}", &name[2..])))
        .collect();
    gnoblin.insert("onClientCreated".to_owned());
    format!(
        "(() => {{
            const ons = (object) => {{
                const found = new Set();
                for (let o = object; o !== null && o !== Object.prototype; o = Object.getPrototypeOf(o)) {{
                    for (const name of Object.getOwnPropertyNames(o)) {{
                        if (/^on[A-Z]/.test(name) && typeof object[name] === 'function') found.add(name);
                    }}
                }}
                return found;
            }};
            const problems = [];
            const check = (label, object, want) => {{
                const have = ons(object);
                for (const name of want) if (!have.has(name)) problems.push(label + ' lacks ' + name);
                for (const name of have) if (!want.includes(name)) problems.push(label + ' has ' + name + ' that no class declares');
            }};
            check('client', gnoblin.clients.get('3'), {client});
            check('clients', gnoblin.clients, {every});
            check('outputs', gnoblin.outputs, {every});
            check('where', gnoblin.clients.where({{ focused: true }}), {every});
            check('output', gnoblin.outputs.at(0), {output});
            check('gnoblin', gnoblin, {gnoblin});
            check('session', gnoblin.session, {session});
            check('runtime', gnoblin.runtime, {runtime});
            return problems.join('; ') || 'ok';
        }})()",
        client = json(&client),
        every = json(&every_member),
        output = json(&declared.output),
        gnoblin = json(&gnoblin),
        session = json(&declared.session),
        runtime = json(&declared.runtime),
    )
}

#[test]
fn every_method_a_listener_class_declares_is_a_subscription_of_the_objects_that_take_it_and_nothing_else_is() {
    let mut rig = rig(CORE, "");
    let declared = core_declared();
    assert_eq!(declared.client.len(), 12, "the client's events: {:?}", declared.client);
    assert_eq!(declared.collection.len(), 2, "{:?}", declared.collection);
    assert_eq!(declared.output.len(), 6, "{:?}", declared.output);
    assert_eq!(declared.session.len(), 4, "{:?}", declared.session);
    assert_eq!(declared.runtime.len(), 2, "{:?}", declared.runtime);
    assert_eq!(rig.eval(&check_names(&declared, &BTreeSet::new())), "ok");
}

#[test]
fn what_gnoblin_std_adds_to_a_listener_class_is_a_subscription_too_and_only_with_std() {
    let extra = std_client_methods();
    assert_eq!(
        extra,
        BTreeSet::from(["onRequest".to_owned()]),
        "std's additions to ClientListener"
    );
    let mut rig = rig(STD, "");
    assert_eq!(rig.eval(&check_names(&core_declared(), &extra)), "ok");
    // The core's registry is one per process: a rig is gone before the next is made.
    drop(rig);
    // A shell without std has neither the method nor its subscription.
    let mut core = self::rig(CORE, "");
    assert_eq!(
        core.eval(
            "[typeof gnoblin.clients.get('3').onRequest, typeof gnoblin.clients.onRequest, typeof gnoblin.onClientRequest].join()"
        ),
        "undefined,undefined,undefined"
    );
}

#[test]
fn a_subscription_takes_a_function_and_says_which_one_it_was_given_when_it_is_not() {
    let mut rig = rig(CORE, "");
    for (call, expected) in [
        (
            "gnoblin.clients.get('3').onTitleChanged(5)",
            "TypeError: onTitleChanged: the handler must be a function",
        ),
        (
            "gnoblin.clients.onAdded('no')",
            "TypeError: onAdded: the handler must be a function",
        ),
        (
            "gnoblin.onClientCreated(undefined)",
            "TypeError: onClientCreated: the handler must be a function",
        ),
        (
            "gnoblin.outputs.at(0).onModeChanged({})",
            "TypeError: onModeChanged: the handler must be a function",
        ),
    ] {
        assert_eq!(
            rig.eval(&format!(
                "try {{ {call}; 'none' }} catch (e) {{ e.constructor.name + ': ' + e.message }}"
            )),
            expected
        );
    }
}

// ---- each one fires, and stops when it is unsubscribed -----------------------------------------------------------------

/// What the compositor does to cause one event: the node, and which time it is done (so that a value changes each time).
type Cause = fn(&mut Rig, u64, u32);

/// A change the compositor publishes to a property, then a frame that raises the events.
fn set(rig: &mut Rig, node: u64, key: &str, value: MirrorValue) {
    rig.comp.external_set(node, key, value);
    settle(rig);
}

fn pointer(rig: &mut Rig, node: u64, kind: &str) {
    rig.comp.pointer(node, kind, 1.0, 2.0, 0);
    settle(rig);
}

/// The events of a window that happen again and again, with what the compositor does to raise each.
const REPEATING_CLIENT_EVENTS: &[(&str, Cause)] = &[
    ("onTitleChanged", |rig, node, n| {
        set(rig, node, "title", text(&format!("t{n}")))
    }),
    ("onAppIdChanged", |rig, node, n| {
        set(rig, node, "app_id", text(&format!("a{n}")))
    }),
    ("onDecorationChanged", |rig, node, n| {
        set(
            rig,
            node,
            "decoration",
            text(if n % 2 == 1 { "client" } else { "server" }),
        );
    }),
    ("onFocusChanged", |rig, node, n| {
        set(rig, node, "focused", MirrorValue::Bool(n % 2 == 1))
    }),
    ("onGeometryChanged", |rig, node, n| {
        set(rig, node, "width", MirrorValue::Double(300.0 + f64::from(n)));
    }),
    ("onPointerEnter", |rig, node, _| pointer(rig, node, "enter")),
    ("onPointerMotion", |rig, node, _| pointer(rig, node, "motion")),
    ("onPointerLeave", |rig, node, _| pointer(rig, node, "leave")),
    ("onPointerPress", |rig, node, _| pointer(rig, node, "press")),
    ("onPointerRelease", |rig, node, _| pointer(rig, node, "release")),
];

/// The facade's name for a method of the client listener: `onTitleChanged` is `onClientTitleChanged`.
fn on_client(method: &str) -> String {
    format!("onClient{}", &method[2..])
}

fn on_output(method: &str) -> String {
    format!("onOutput{}", &method[2..])
}

/// Subscribes `method` three ways (on `object`, on the collection that holds it, and on `gnoblin`), makes `cause` happen
/// to `node` and then again, with the three unsubscribed, and returns what the handlers logged each time. A handler logs
/// `where:method:id`, where `id` is that of the object the event is of.
fn heard_three_ways(
    rig: &mut Rig,
    object: &str,
    collection: &str,
    facade: &str,
    method: &str,
    cause: Cause,
    node: u64,
) -> (Vec<String>, Vec<String>) {
    exec(
        rig,
        &format!(
            "log.length = 0;
             g.offs = [
                {object}.{method}((o) => log.push('object:{method}:' + o.id)),
                {collection}.{method}((o) => log.push('collection:{method}:' + o.id)),
                gnoblin.{facade}((o) => log.push('facade:{method}:' + o.id)),
             ];"
        ),
    );
    settle(rig);
    exec(rig, "log.length = 0;");
    cause(rig, node, 1);
    let heard = log(rig);
    exec(rig, "g.offs.forEach((off) => { off(); off(); }); log.length = 0;");
    cause(rig, node, 2);
    (heard, log(rig))
}

fn log(rig: &mut Rig) -> Vec<String> {
    let text = rig.eval("log.join('|')");
    text.split('|')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn each_event_of_a_window_is_heard_on_the_window_its_collection_and_gnoblin_and_then_by_none_once_unsubscribed() {
    let mut rig = rig(CORE, "");
    for (method, cause) in REPEATING_CLIENT_EVENTS {
        let (heard, after) = heard_three_ways(
            &mut rig,
            "gnoblin.clients.get('3')",
            "gnoblin.clients",
            &on_client(method),
            method,
            *cause,
            3,
        );
        assert_eq!(
            heard,
            [
                format!("object:{method}:3"),
                format!("collection:{method}:3"),
                format!("facade:{method}:3")
            ],
            "{method}: the window's own listeners hear it first, then the collection's in the order they were attached"
        );
        assert!(after.is_empty(), "{method}: an unsubscribed handler heard {after:?}");
    }
}

#[test]
fn a_window_that_closes_is_heard_closing_and_leaving_the_collection_and_one_that_appears_is_heard_arriving() {
    let mut rig = rig(CORE, "");

    // `onClosed` and `onRemoved`: the window's own and the collection's, which hears the same window go.
    exec(
        &mut rig,
        "g.offs = [
            gnoblin.clients.get('3').onClosed((c) => log.push('object:onClosed:' + c.id)),
            gnoblin.clients.onClosed((c) => log.push('collection:onClosed:' + c.id)),
            gnoblin.onClientClosed((c) => log.push('facade:onClientClosed:' + c.id)),
            gnoblin.clients.onRemoved((c, from) => log.push('collection:onRemoved:' + c.id + ':' + (from === gnoblin.clients))),
         ];",
    );
    settle(&mut rig);
    rig.comp.destroy(3);
    settle(&mut rig);
    assert_eq!(
        log(&mut rig),
        [
            "object:onClosed:3",
            "collection:onClosed:3",
            "facade:onClientClosed:3",
            "collection:onRemoved:3:true"
        ]
    );
    exec(&mut rig, "g.offs.forEach((off) => off()); log.length = 0;");
    rig.comp.destroy(4);
    settle(&mut rig);
    assert!(log(&mut rig).is_empty(), "unsubscribed handlers hear nothing");

    // `onAdded`, `onClientCreated` and `onReady`: a window that appears. A subscription to additions is told of the
    // windows that are there already, as `addListener` of a `CollectionListener` is; the rest hear what happens after.
    exec(
        &mut rig,
        "g.offs = [
            gnoblin.clients.onAdded((c, to) => log.push('collection:onAdded:' + c.id + ':' + (to === gnoblin.clients))),
            gnoblin.onClientCreated((c) => log.push('facade:onClientCreated:' + c.id)),
            gnoblin.clients.onReady((c) => log.push('collection:onReady:' + c.id)),
            gnoblin.onClientReady((c) => log.push('facade:onClientReady:' + c.id)),
         ];",
    );
    settle(&mut rig);
    assert!(log(&mut rig).is_empty(), "no window is left to replay: both closed");
    rig.comp.create_window(100, "new", "app.new");
    settle(&mut rig);
    assert_eq!(
        log(&mut rig),
        [
            "collection:onAdded:100:true",
            "facade:onClientCreated:100",
            "collection:onReady:100",
            "facade:onClientReady:100"
        ]
    );
    exec(&mut rig, "g.offs.forEach((off) => off()); log.length = 0;");
    rig.comp.create_window(101, "newer", "app.newer");
    settle(&mut rig);
    assert!(log(&mut rig).is_empty(), "unsubscribed handlers hear nothing");
}

#[test]
fn a_subscription_to_additions_is_told_of_the_windows_that_are_there_and_of_each_that_comes() {
    // The spec's `gnoblin.onClientCreated(client => ...)`: made in the entry file, it runs for the windows of the boot
    // and for every window after.
    let mut rig = rig(
        CORE,
        "g.off = gnoblin.onClientCreated((client) => log.push('created ' + client.id));\n",
    );
    assert_eq!(log(&mut rig), ["created 3", "created 4"]);
    rig.comp.create_window(5, "three", "app.three");
    settle(&mut rig);
    assert_eq!(log(&mut rig), ["created 3", "created 4", "created 5"]);
    exec(&mut rig, "g.off();");
    rig.comp.create_window(6, "four", "app.four");
    settle(&mut rig);
    assert_eq!(log(&mut rig).len(), 3, "the detached handler hears no more");
}

#[test]
fn the_ready_of_the_windows_there_at_boot_reaches_every_subscription_once() {
    let mut rig = rig(
        CORE,
        "gnoblin.clients.get('3').onReady((c) => log.push('object:' + c.id));\n\
         gnoblin.clients.onReady((c) => log.push('collection:' + c.id));\n\
         gnoblin.onClientReady((c) => log.push('facade:' + c.id));\n",
    );
    let mut heard = log(&mut rig);
    heard.sort();
    assert_eq!(
        heard,
        ["collection:3", "collection:4", "facade:3", "facade:4", "object:3"],
        "each window that was there is ready once, to its own subscription and to the collection's"
    );
}

/// The events of an output that happen again and again, with what the compositor does to raise each.
const REPEATING_OUTPUT_EVENTS: &[(&str, Cause)] = &[
    ("onModeChanged", |rig, node, n| {
        set(rig, node, "mode", text(&format!("{}x600@60", 800 + n)))
    }),
    ("onScaleChanged", |rig, node, n| {
        set(rig, node, "output_scale", MirrorValue::Double(1.0 + f64::from(n)))
    }),
    ("onGeometryChanged", |rig, node, n| {
        set(rig, node, "width", MirrorValue::Double(1000.0 + f64::from(n)))
    }),
    ("onApplied", |rig, _, n| {
        exec(rig, &format!("gnoblin.outputs.at(0).apply({{ scale: 1.{n} }});"));
        settle(rig);
        let token = rig.comp.applies().last().expect("the apply went out").2;
        rig.comp.answer(token, "", "");
        settle(rig);
    }),
];

#[test]
fn each_event_of_an_output_is_heard_on_the_output_its_collection_and_gnoblin_and_then_by_none_once_unsubscribed() {
    let mut rig = rig(CORE, "");
    // The output's id is its label, which the compositor gives the node: read it, so the test names none.
    let id = rig.eval("gnoblin.outputs.at(0).id");
    for (method, cause) in REPEATING_OUTPUT_EVENTS {
        let (heard, after) = heard_three_ways(
            &mut rig,
            "gnoblin.outputs.at(0)",
            "gnoblin.outputs",
            &on_output(method),
            method,
            *cause,
            20,
        );
        assert_eq!(
            heard,
            [
                format!("object:{method}:{id}"),
                format!("collection:{method}:{id}"),
                format!("facade:{method}:{id}")
            ],
            "{method}"
        );
        assert!(after.is_empty(), "{method}: an unsubscribed handler heard {after:?}");
    }
}

#[test]
fn an_output_that_is_plugged_in_and_one_that_goes_are_heard_by_connection_and_by_membership() {
    let mut rig = rig(CORE, "");
    exec(
        &mut rig,
        "g.offs = [
            gnoblin.outputs.onConnected((o) => log.push('collection:onConnected:' + o.name)),
            gnoblin.onOutputConnected((o) => log.push('facade:onOutputConnected:' + o.name)),
            gnoblin.outputs.onAdded((o, to) => log.push('collection:onAdded:' + o.name + ':' + (to === gnoblin.outputs))),
            gnoblin.onOutputAdded((o) => log.push('facade:onOutputAdded:' + o.name)),
            gnoblin.outputs.onRemoved((o) => log.push('collection:onRemoved:' + o.name)),
            gnoblin.onOutputRemoved((o) => log.push('facade:onOutputRemoved:' + o.name)),
            gnoblin.outputs.onDisconnected((o) => log.push('collection:onDisconnected:' + o.name + ':' + o.connected)),
            gnoblin.onOutputDisconnected((o) => log.push('facade:onOutputDisconnected:' + o.name)),
         ];",
    );
    settle(&mut rig);
    // The two that report additions are told of the output that was there; the others hear only what happens.
    let mut replay = log(&mut rig);
    replay.sort();
    assert_eq!(
        replay,
        ["collection:onAdded:output-0:true", "facade:onOutputAdded:output-0"]
    );
    exec(&mut rig, "log.length = 0;");

    rig.comp
        .create_screen(21, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
    settle(&mut rig);
    let mut plugged = log(&mut rig);
    plugged.sort();
    assert_eq!(
        plugged,
        [
            "collection:onAdded:output-1:true",
            "collection:onConnected:output-1",
            "facade:onOutputAdded:output-1",
            "facade:onOutputConnected:output-1"
        ]
    );
    exec(&mut rig, "log.length = 0;");

    rig.comp.destroy(21);
    settle(&mut rig);
    let mut gone = log(&mut rig);
    gone.sort();
    assert_eq!(
        gone,
        [
            "collection:onDisconnected:output-1:false",
            "collection:onRemoved:output-1",
            "facade:onOutputDisconnected:output-1",
            "facade:onOutputRemoved:output-1"
        ]
    );

    exec(&mut rig, "g.offs.forEach((off) => off()); log.length = 0;");
    rig.comp
        .create_screen(22, "output-2", (1000.0, 0.0, 600.0, 400.0), false);
    settle(&mut rig);
    assert!(log(&mut rig).is_empty(), "unsubscribed handlers hear nothing");
}

// ---- addListener and a subscription are one list -----------------------------------------------------------------------

#[test]
fn addlistener_and_a_subscription_hear_the_same_events_in_the_order_they_were_attached() {
    let mut rig = rig(
        CORE,
        "const c = gnoblin.clients.get('3');
         const note = (who) => (...args) => log.push(who + ':' + args.length + ':' + args[0].id + ':' + args[1]);
         c.addListener({ onTitleChanged: note('A') });
         c.onTitleChanged(note('B'));
         c.addListener({ onTitleChanged: note('C'), onFocusChanged: note('c') });
         gnoblin.clients.addListener({ onTitleChanged: note('D') });
         gnoblin.clients.onTitleChanged(note('E'));
         gnoblin.onClientTitleChanged(note('F'));
         gnoblin.clients.addListener({ onTitleChanged: note('G') });
         gnoblin.clients.onFocusChanged(note('f'));
         c.onFocusChanged(note('b'));\n",
    );
    for n in 1..=2 {
        rig.comp.external_set(3, "title", text(&format!("t{n}")));
        settle(&mut rig);
        assert_eq!(
            log(&mut rig).split_off((n - 1) * 7),
            [
                format!("A:2:3:t{n}"),
                format!("B:2:3:t{n}"),
                format!("C:2:3:t{n}"),
                format!("D:2:3:t{n}"),
                format!("E:2:3:t{n}"),
                format!("F:2:3:t{n}"),
                format!("G:2:3:t{n}"),
            ],
            "the same arguments in the order of attaching, whichever way each was attached (round {n})"
        );
    }
    exec(&mut rig, "log.length = 0;");
    rig.comp.external_set(3, "focused", MirrorValue::Bool(true));
    settle(&mut rig);
    assert_eq!(
        log(&mut rig),
        ["c:2:3:true", "b:2:3:true", "f:2:3:true"],
        "the window's own listeners first, then the collection's, each list in the order of attaching"
    );
}

#[test]
fn a_listener_that_is_removed_and_a_subscription_that_is_undone_leave_the_others_in_place_in_order() {
    let mut rig = rig(
        CORE,
        "const c = gnoblin.clients.get('3');
         g.keep = { onTitleChanged() { log.push('keep'); } };
         g.gone = { onTitleChanged() { log.push('gone'); } };
         c.addListener(g.gone);
         g.off = c.onTitleChanged(() => log.push('sub'));
         c.addListener(g.keep);
         c.addListener(g.keep);\n",
    );
    set(&mut rig, 3, "title", text("first"));
    assert_eq!(
        log(&mut rig),
        ["gone", "sub", "keep"],
        "a listener added twice is heard once"
    );
    exec(
        &mut rig,
        "log.length = 0; gnoblin.clients.get('3').removeListener(g.gone); g.off();",
    );
    set(&mut rig, 3, "title", text("second"));
    assert_eq!(log(&mut rig), ["keep"]);
}
