//! The settings that have a home in the owner's specification, set the way the specification sets them: property
//! writes and method calls on live objects in an `init.ts`, with no config object.
//!
//! One case for each setting that has a spec object: workspace gaps and a layout's gap, keys and gestures as
//! `input.bind`, and the titlebar, window menu and not-responding alert as library helpers. The settings with a spec
//! object whose tests live elsewhere are named in `design/js-spec-native.md` (part 3.11), and the ones with none are
//! listed there as dropped. Over the fake compositor of `tests/common`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

const IMPORTS: &str = "import * as gCore from \"gnoblin-core\"; import * as gStd from \"gnoblin-std\"; const g = { ...gCore, ...gStd };\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

/// A shell over a screen of 1000 by 800, one seat, one lock and one server-decorated window (id 3) whose `init.ts` is
/// `body`.
fn world(body: &str) -> Rig {
    let mut rig = rig_setup(IMPORTS, body, |comp| {
        comp.add_node(
            20,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(22, ROOT, "lock", Vec::new());
        comp.add_window(3, "one", "foot");
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

#[test]
fn gaps_are_the_workspaces_and_the_layouts_own_properties() {
    // `gaps.outer` is `Workspace.gaps.outer` and `gaps.inner` a layout's `gap`: the window of a one-window master stack
    // is inset by the outer gap from every edge of the 1000 by 800 screen.
    let mut rig = world(
        "gnoblin.workspaces.add(new Workspace({ id: 'a', layout: new MasterStackLayout({ gap: 12 }), \
         gaps: { inner: 4, outer: 6 } }));",
    );
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.workspaces.get('a').gaps)"),
        r#"{"inner":4,"outer":6}"#
    );
    assert_eq!(rig.eval("String(gnoblin.workspaces.get('a').layout.gap)"), "12");
    assert_eq!(rig.comp.rect(3), "6,6,988,788", "the window sits inside the outer gap");
    // A gap is a property: a later write moves the window.
    rig.eval("gnoblin.workspaces.get('a').gaps = { inner: 4, outer: 20 }; 0");
    rig.settle();
    assert_eq!(rig.comp.rect(3), "20,20,960,760");
}

#[test]
fn a_key_is_input_bind_and_a_key_bound_twice_is_refused() {
    let mut rig = world(
        "globalThis.ran = 0;\n\
         gnoblin.input.primary.bind('Super+Return', () => { globalThis.ran += 1; });\n\
         globalThis.second = (() => { try { gnoblin.input.primary.bind('Super+Return', () => {}); return 'bound'; } \
         catch (error) { return error.constructor.name; } })();",
    );
    assert_eq!(rig.eval("globalThis.second"), "BindingConflictError");
    let id = rig.comp.claim_id("<Super>Return");
    rig.comp.chord(id, "<Super>Return", "press");
    rig.settle();
    assert_eq!(rig.eval("String(globalThis.ran)"), "1", "the handler ran once");
}

#[test]
fn a_touchpad_gesture_is_input_bind_on_a_gesture_trigger() {
    let mut rig = world(
        "globalThis.swiped = [];\n\
         gnoblin.input.primary.bind('Swipe4Up', () => { globalThis.swiped.push('up'); });",
    );
    let claim = rig.comp.gesture_claim("swipe", 4);
    for (phase, dy) in [("begin", 0.0), ("update", -100.0), ("end", -300.0)] {
        rig.comp.gesture(claim, "swipe", 4, phase, (0.0, dy), 1.0, 0.0);
    }
    rig.settle();
    assert_eq!(rig.eval("globalThis.swiped.join()"), "up");
}

#[test]
fn titlebars_the_window_menu_and_the_alert_are_started_by_helpers_and_only_once() {
    let mut rig = world("export {};");
    assert_eq!(rig.eval("String(windowMenu.started)"), "false");
    assert_eq!(rig.eval("String(notRespondingAlerts.started)"), "false");
    rig.comp.create_window(4, "two", "foot");
    rig.settle();
    assert_eq!(
        rig.eval("String(gnoblin.clients.at(1).frameInset.top)"),
        "0",
        "no titlebar until the helper runs"
    );
    rig.eval(
        "startDecorations(); startDecorations(); startWindowMenu(); startWindowMenu(); startNotRespondingAlert(); 0",
    );
    rig.comp.create_window(5, "three", "foot");
    rig.settle();
    assert_eq!(rig.eval("String(gnoblin.clients.at(2).frameInset.top)"), "28");
    assert_eq!(rig.eval("String(windowMenu.started)"), "true");
    assert_eq!(rig.eval("String(notRespondingAlerts.started)"), "true");
    assert_eq!(
        rig.comp.children(5).len(),
        1,
        "one bar under the window, not one for each call of the helper"
    );
}
