//! The order of a frame, as the script sees it.
//!
//! A frame is the table of stages in `shell/frame.rs`, and several stages are in the order they are in because of what an
//! earlier one leaves for a later one: `plan` before `pointer` and `input` (a client made in the frame has its listeners,
//! and a client that died has cancelled its drags), the listeners before the layout (what a handler wrote is laid out in
//! the same frame), the layout before the toolkit (a titlebar reads the size the layout gave its window), the timers
//! before the layout (a timer's writes are laid out in the frame they fire), the document after the listeners, and the
//! transaction last. None of that is visible to a test of one behaviour, so this one records every call a frame makes
//! into the entry and pins the sequence.

mod common;
use common::*;

const IMPORTS: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n";

/// Wraps every function of the bridge's entry so that each call is appended to `__calls` and then made.
const RECORD: &str = "(() => {\n\
    \x20   const entry = globalThis.__gnoblinHost.entry;\n\
    \x20   globalThis.__calls = [];\n\
    \x20   for (const name of Object.keys(entry)) {\n\
    \x20       const call = entry[name];\n\
    \x20       if (typeof call !== 'function') continue;\n\
    \x20       entry[name] = function (...args) { globalThis.__calls.push(name); return call.apply(entry, args); };\n\
    \x20   }\n\
    \x20   return 'recording';\n\
    })()";

const INIT: &str = "class Titles extends ClientListener { onTitleChanged() {} }\n\
    gnoblin.clients.addListener(new Titles());\n";

/// The calls the next frame makes into the entry, in order.
fn calls_of_a_frame(rig: &mut Rig) -> Vec<String> {
    rig.eval("(globalThis.__calls.length = 0, 'cleared')");
    rig.frame();
    rig.eval("__calls.join(' ')")
        .split(' ')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The names of `calls` that are one of `stages`, in order and once each in a row: what a frame's stages did, without the
/// calls of the gate that decides whether the next frame is due.
fn of(calls: &[String], stages: &[&str]) -> Vec<String> {
    calls
        .iter()
        .filter(|call| stages.contains(&call.as_str()))
        .cloned()
        .collect()
}

/// The entry calls each stage makes, in the order of the stages. A stage that makes none is not in the list.
const STAGE_CALLS: [&str; 17] = [
    "ready",
    "plan",
    "pointer",
    "input",
    "next",
    "apiFrame",
    "documentRevision",
    "layout",
    "ui",
    "takeVerdicts",
    "dialogsWanted",
    "wants",
    "takeErrors",
    "takeNotices",
    "wake",
    "pending",
    "document",
];

fn rig() -> Rig {
    let mut rig = rig_setup(IMPORTS, INIT, |comp| comp.add_window(3, "one", "foot"));
    assert_eq!(rig.eval(RECORD), "recording");
    rig
}

#[test]
#[ignore = "vrnw: TS internal: the frame's calls into `__gnoblinHost.entry` (ready, plan, pointer, input, layout, ui, ...) are the TS library's bridge; a shell on the generated classes has no `__gnoblinHost`"]
fn the_first_frame_announces_the_snapshot_before_it_plans_and_runs_every_stage_in_order() {
    let mut rig = rig();
    let calls = calls_of_a_frame(&mut rig);
    let stages = of(&calls, &STAGE_CALLS);
    let expected = [
        "ready",
        "plan",
        "pointer",
        "input",
        "apiFrame",
        "documentRevision",
        "layout",
        "ui",
        "takeVerdicts",
        "dialogsWanted",
        "wants",
        "takeErrors",
        "takeNotices",
        "wake",
    ];
    assert_eq!(stages, expected, "the whole frame: {calls:?}");
}

#[test]
#[ignore = "vrnw: TS internal: the frame's calls into `__gnoblinHost.entry` are the TS library's bridge; a shell on the generated classes has no `__gnoblinHost`"]
fn a_frame_after_the_first_does_not_announce_the_snapshot_again() {
    let mut rig = rig();
    calls_of_a_frame(&mut rig);
    rig.comp.external_set(3, "title", text("two"));
    let calls = calls_of_a_frame(&mut rig);
    assert!(!calls.contains(&"ready".to_owned()), "{calls:?}");
}

#[test]
#[ignore = "vrnw: TS internal: the order of a frame's calls into `__gnoblinHost.entry` is the TS library's bridge; a shell on the generated classes has no `__gnoblinHost`"]
fn a_listener_a_change_queued_runs_after_the_input_stage_and_before_the_timers_and_the_layout() {
    let mut rig = rig();
    calls_of_a_frame(&mut rig);
    rig.comp.external_set(3, "title", text("two"));
    let calls = calls_of_a_frame(&mut rig);
    let stages = of(&calls, &STAGE_CALLS);
    let at = |name: &str| {
        stages
            .iter()
            .position(|call| call == name)
            .unwrap_or_else(|| panic!("{name} was not called: {calls:?}"))
    };
    assert!(at("plan") < at("pointer"), "{calls:?}");
    assert!(at("pointer") < at("input"), "{calls:?}");
    assert!(
        at("input") < at("next"),
        "the queued listener runs after the input stage: {calls:?}"
    );
    assert!(at("next") < at("apiFrame"), "and before the timers: {calls:?}");
    assert!(
        at("apiFrame") < at("layout"),
        "so a timer's writes are laid out in the frame they fire: {calls:?}"
    );
    assert!(
        at("layout") < at("ui"),
        "a titlebar reads the size the layout gave its window: {calls:?}"
    );
    assert!(at("documentRevision") < at("layout"), "{calls:?}");
}
