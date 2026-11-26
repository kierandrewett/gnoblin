//! The settings that have a home in the owner's specification, set the way the specification sets them: property
//! writes and method calls on live objects in a `shell.ts`, with no config object.
//!
//! One case for each setting that has a spec object: workspace gaps and a layout's gap, keys and gestures as
//! `input.bind`, and the titlebar, window menu and not-responding alert as library helpers. Over the fake compositor of
//! `tests/common`.

mod common;
use common::*;

/// A shell over a screen of 1000 by 800, one seat, one lock and one server-decorated window (id 3) whose `shell.ts` is
/// `body`.
fn world(body: &str) -> Rig {
    let mut rig = rig_setup(LIBRARY_GLOBALS, body, |comp| {
        comp.add_desktop(1000.0, 800.0);
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
        "gnoblin.workspaces.add(new Workspace('a', { layout: new MasterStackLayout({ gap: 12 }), \
         gaps: { inner: 4, outer: 6 } }));",
    );
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.workspaces.byId('a').gaps)"),
        r#"{"inner":4,"outer":6}"#
    );
    assert_eq!(rig.eval("String(gnoblin.workspaces.byId('a').layout.gap)"), "12");
    assert_eq!(rig.comp.rect(3), "6,6,988,788", "the window sits inside the outer gap");
    // A gap is a property: a later write moves the window.
    rig.eval("gnoblin.workspaces.byId('a').gaps = { inner: 4, outer: 20 }; 0");
    rig.settle();
    assert_eq!(rig.comp.rect(3), "20,20,960,760");
}

#[test]
fn a_key_is_input_bind_and_a_key_bound_twice_is_refused() {
    let mut rig = world(
        "globalThis.ran = 0;\n\
         gnoblin.input.primary.bind('Super+Return', () => { globalThis.ran += 1; });\n\
         globalThis.second = (() => { try { gnoblin.input.primary.bind('Super+Return', () => {}); return 'bound'; } \
         catch (error) { return error.constructor.name + ':' + error.code; } })();",
    );
    assert_eq!(rig.eval("globalThis.second"), "GnoblinError:already_exists");
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
    assert_eq!(rig.eval("String(WindowMenus.get().started)"), "false");
    assert_eq!(rig.eval("String(NotRespondingAlerts.get().started)"), "false");
    rig.comp.create_window(4, "two", "foot");
    rig.settle();
    assert_eq!(
        rig.eval("String(Decorations.get().insetOf(gnoblin.clients.at(1)).top)"),
        "0",
        "no titlebar until the helper runs"
    );
    rig.eval(
        "Decorations.get().start(); Decorations.get().start(); WindowMenus.get().start(); WindowMenus.get().start(); NotRespondingAlerts.get().start(); 0",
    );
    rig.comp.create_window(5, "three", "foot");
    rig.settle();
    assert_eq!(
        rig.eval("String(Decorations.get().insetOf(gnoblin.clients.at(2)).top)"),
        "28"
    );
    assert_eq!(rig.eval("String(WindowMenus.get().started)"), "true");
    assert_eq!(rig.eval("String(NotRespondingAlerts.get().started)"), "true");
    assert_eq!(
        rig.comp.children(5).len(),
        1,
        "one bar under the window, not one for each call of the helper"
    );
}
