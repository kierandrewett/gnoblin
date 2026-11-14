//! Window policy of the stock shell that the Lua shell had and the TypeScript shell gained: the touchpad gesture actions
//! and reduced motion (`data/js/gnoblin/gesture_actions.ts`), over the fake compositor of `tests/common`.
//!
//! The cases are the Lua shell's: the last seven cases and the reduced-motion cases of
//! `crates/gnoblin-lua/tests/lua/gestures_test.lua`, with the same inputs and the same expected values. A case with no Lua
//! original says "new". What the real compositor does with a gesture or a key is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::policy::*;
use common::*;

fn active(rig: &mut Rig) -> String {
    rig.eval("gnoblin.workspaces.where({ active: true }).at(0)?.id ?? 'none'")
}

fn has_call(rig: &Rig, node: u64, method: &str) -> bool {
    rig.comp.calls().iter().any(|(n, m)| *n == node && m == method)
}

// ---- Gestures.actions -------------------------------------------------------------------------------------------------

#[test]
fn workspace_next_and_workspace_previous_step_the_shown_workspace_and_stop_at_the_ends() {
    // Lua: "workspace_next and workspace_previous step the desk".
    let mut rig = world(0, false, "new Layout()", "");
    assert_eq!(active(&mut rig), "1");
    exec(&mut rig, "gnoblin.gestures.actions.workspacePrevious()");
    settle(&mut rig);
    assert_eq!(active(&mut rig), "1", "there is nothing before the first workspace");
    exec(&mut rig, "gnoblin.gestures.actions.workspaceNext()");
    settle(&mut rig);
    assert_eq!(active(&mut rig), "2", "workspaceNext shows the second workspace");
    exec(&mut rig, "gnoblin.gestures.actions.workspaceNext()");
    settle(&mut rig);
    assert_eq!(active(&mut rig), "3");
    exec(&mut rig, "gnoblin.gestures.actions.workspaceNext()");
    settle(&mut rig);
    assert_eq!(active(&mut rig), "3", "nor after the last");
    exec(&mut rig, "gnoblin.gestures.actions.workspacePrevious()");
    settle(&mut rig);
    assert_eq!(active(&mut rig), "2", "workspacePrevious shows the one before");
}

#[test]
fn window_close_closes_the_focused_window_and_does_nothing_without_one() {
    // Lua: "window_close closes the focused window and does nothing without one".
    let mut rig = world(1, false, "new Layout()", "");
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(0));
    settle(&mut rig);
    exec(&mut rig, "gnoblin.gestures.actions.windowClose()");
    settle(&mut rig);
    assert!(!has_call(&rig, 3, "close"), "no focused window: nothing to close");
    give_focus(&mut rig, 3);
    exec(&mut rig, "gnoblin.gestures.actions.windowClose()");
    settle(&mut rig);
    assert!(has_call(&rig, 3, "close"), "the focused window must be asked to close");
}

#[test]
fn window_minimise_hides_the_focused_window_and_hands_the_keyboard_on() {
    // Lua: "window_minimise hides the focused window".
    let mut rig = world(2, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    exec(&mut rig, "gnoblin.gestures.actions.windowMinimise()");
    settle(&mut rig);
    assert_eq!(
        rig.comp.flag(3, "minimised"),
        Some(true),
        "the focused window is minimised"
    );
    assert_eq!(rig.comp.flag(4, "minimised"), None, "the other window is left alone");
    assert_eq!(focus(&rig), 4, "the keyboard goes to the window that is left");
}

#[test]
fn window_toggle_maximise_maximises_the_focused_window_and_then_restores_it() {
    // Lua: "window_toggle_maximise maximises the focused window and then restores it".
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    let floating = rect(&rig, 3);
    exec(&mut rig, "gnoblin.gestures.actions.windowToggleMaximise()");
    settle(&mut rig);
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(true),
        "the first toggle must maximise"
    );
    exec(&mut rig, "gnoblin.gestures.actions.windowToggleMaximise()");
    settle(&mut rig);
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(false),
        "the second toggle must restore"
    );
    assert_eq!(rect(&rig, 3), floating, "and the window is back where it floated");
}

// ---- direction, onSwipe, onPinch -----------------------------------------------------------------------------------

#[test]
fn direction_names_the_longer_axis_once_the_swipe_has_gone_the_threshold() {
    // Lua: `Gestures.direction`, the part of "on_swipe runs the fn for the direction a swipe ended in".
    let mut rig = world(0, false, "new Layout()", "");
    let got = rig.eval(
        "[[10, -120], [90, 30], [20, -20], [-200, 0], [0, 200], [48, 0], [47, 0], [30, 30]] \
         .map(([dx, dy]) => String(gestureDirection({ dx, dy }))).join(',')",
    );
    assert_eq!(
        got, "up,right,null,left,down,right,null,null",
        "the longer axis; under the 48 units nothing; a tie goes to the horizontal axis"
    );
    assert_eq!(
        rig.eval("[gestureDirection({ dx: 20, dy: 0 }, 10), gestureDirection({ dx: 60, dy: 0 }, 100)].join(',')"),
        "right,",
        "a threshold of the caller's own"
    );
    assert_eq!(
        rig.eval("gnoblin.gestures.direction === gestureDirection ? 'same' : 'other'"),
        "same"
    );
}

#[test]
fn on_swipe_runs_the_function_for_the_direction_a_swipe_ended_in_past_the_threshold_only() {
    // Lua: "on_swipe runs the fn for the direction a swipe ended in, past the threshold only".
    let mut rig = world(0, false, "new Layout()", "");
    exec(
        &mut rig,
        "globalThis.ran = []; \
         globalThis.swipe = onSwipe({ up: () => ran.push('up'), right: () => ran.push('right') }); \
         const state = swipe.onStart({}); \
         swipe.onEnd({ phase: 'end', dx: 10, dy: -120 }, state); \
         swipe.onEnd({ phase: 'end', dx: 90, dy: 30 }, state); \
         swipe.onEnd({ phase: 'end', dx: 20, dy: -20 }, state); \
         swipe.onEnd({ phase: 'end', dx: -200, dy: 0 }, state); \
         swipe.onUpdate({ phase: 'update', dx: 0, dy: -300 }, state); \
         swipe.onCancel({ phase: 'cancel', dx: 0, dy: -300 }, state)",
    );
    assert_eq!(
        rig.eval("ran.join(',')"),
        "up,right",
        "under the 48 units and an unmapped way run nothing; only the end acts"
    );
    assert_eq!(
        rig.eval(
            "(() => { const out = []; const s = onSwipe({ right: () => out.push('r') }, 10); \
             s.onEnd({ dx: 12, dy: 0 }, {}); return out.join(); })()"
        ),
        "r",
        "a threshold the caller gave"
    );
}

#[test]
fn on_pinch_runs_out_for_a_spread_and_in_for_a_gather_past_the_threshold_only() {
    // Lua: "on_pinch runs out for a spread and in for a gather, past the threshold only".
    let mut rig = world(0, false, "new Layout()", "");
    exec(
        &mut rig,
        "globalThis.ran = []; \
         globalThis.pinch = onPinch({ out: () => ran.push('out'), in: () => ran.push('in') }); \
         const state = pinch.onStart({}); \
         pinch.onEnd({ phase: 'end', scale: 1.5 }, state); \
         pinch.onEnd({ phase: 'end', scale: 0.6 }, state); \
         pinch.onEnd({ phase: 'end', scale: 1.05 }, state); \
         pinch.onEnd({ phase: 'end', scale: 1.2 }, state); \
         pinch.onEnd({ phase: 'end', scale: 0.8 }, state)",
    );
    assert_eq!(
        rig.eval("ran.join(',')"),
        "out,in,out,in",
        "1.5 spreads, 0.6 gathers, 1.05 is inside the 0.12 threshold, and 1.2 and 0.8 are past it"
    );
}

#[test]
fn a_map_names_only_the_ways_its_gesture_can_go_and_actions_that_exist() {
    // New: the Lua `map[direction]` took any key and ran any value; a typo here is reported with its path.
    let mut rig = world(0, false, "new Layout()", "");
    let thrown = |rig: &mut Rig, source: &str| {
        rig.eval(&format!(
            "try {{ {source}; 'ok' }} catch (e) {{ `${{e.name}}: ${{e.message}}` }}"
        ))
    };
    assert_eq!(
        thrown(&mut rig, "onSwipe({ forwards: () => {} })"),
        "TypeError: onSwipe: \"forwards\" is not a way for this gesture to go; use left, right, up, down"
    );
    assert_eq!(
        thrown(&mut rig, "onPinch({ up: () => {} })"),
        "TypeError: onPinch: \"up\" is not a way for this gesture to go; use in, out"
    );
    assert!(
        thrown(&mut rig, "onSwipe({ up: 'window.explode' })")
            .starts_with("TypeError: onSwipe.up: an action is a function or one of workspace.next,"),
    );
    assert!(thrown(&mut rig, "onSwipe({ up: () => {} }, -1)").starts_with("RangeError: a gesture threshold"));
}

#[test]
fn a_swipe_bound_through_input_runs_a_named_action_when_it_ends_and_a_cancel_runs_nothing() {
    // Lua: "a user binds a swipe to an action: four fingers up toggles maximise through the whole chain".
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    exec(
        &mut rig,
        "gnoblin.input.primary.bind('Swipe4', gnoblin.gestures.onSwipe({ up: 'window.toggle-maximise' }))",
    );
    settle(&mut rig);
    let id = rig.comp.gesture_claim("swipe", 4);
    rig.comp.gesture(id, "swipe", 4, "begin", (0.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    rig.comp.gesture(id, "swipe", 4, "update", (0.0, -80.0), 1.0, 0.0);
    settle(&mut rig);
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        None,
        "nothing runs before the swipe ends"
    );
    rig.comp.gesture(id, "swipe", 4, "end", (0.0, -120.0), 1.0, 0.0);
    settle(&mut rig);
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(true),
        "the swipe up must maximise the focused window"
    );

    rig.comp.gesture(id, "swipe", 4, "begin", (0.0, 0.0), 1.0, 0.0);
    rig.comp.gesture(id, "swipe", 4, "cancel", (0.0, 300.0), 1.0, 0.0);
    settle(&mut rig);
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(true),
        "a cancelled swipe runs nothing"
    );
}

#[test]
fn gesture_actions_in_the_config_bind_swipes_and_pinches_and_replace_the_stock_slide() {
    // New: the Lua configuration named these five actions; `gestureActions` is the TypeScript spelling of that.
    let config = "gestureActions: { \
                      Swipe4: { up: 'window.toggle-maximise', left: 'workspace.next' }, \
                      Swipe3: { right: 'window.close' }, \
                      Pinch2: { in: 'window.minimize', out: (event) => globalThis.pinched = event.scale } },";
    let mut rig = world(2, false, "new Layout()", config);
    give_focus(&mut rig, 3);
    // The stock three-finger slide is replaced by the config's own Swipe3, and its claim is the one that is held.
    assert_eq!(
        rig.eval("gnoblin.input.primary.describeBindings().filter((b) => b.trigger === 'Swipe3').length"),
        "1"
    );
    let swipe3 = rig.comp.gesture_claim("swipe", 3);
    for (phase, at) in [("begin", 0.0), ("update", 200.0), ("end", 200.0)] {
        rig.comp.gesture(swipe3, "swipe", 3, phase, (at, 0.0), 1.0, 0.0);
        settle(&mut rig);
    }
    assert!(
        has_call(&rig, 3, "close"),
        "right runs window.close on the focused window"
    );
    let swipe4 = rig.comp.gesture_claim("swipe", 4);
    for (phase, at) in [("begin", 0.0), ("update", -200.0), ("end", -200.0)] {
        rig.comp.gesture(swipe4, "swipe", 4, phase, (at, 0.0), 1.0, 0.0);
        settle(&mut rig);
    }
    assert_eq!(active(&mut rig), "2", "left runs workspace.next");
    let pinch = rig.comp.gesture_claim("pinch", 2);
    for (phase, scale) in [("begin", 1.0), ("update", 1.4), ("end", 1.4)] {
        rig.comp.gesture(pinch, "pinch", 2, phase, (0.0, 0.0), scale, 0.0);
        settle(&mut rig);
    }
    assert_eq!(
        rig.eval("String(globalThis.pinched)"),
        "1.4",
        "out runs the function with the event"
    );
}

#[test]
fn a_config_that_gives_one_gesture_to_keys_and_to_gesture_actions_is_refused_with_both_names() {
    // New: two bindings of one gesture could not both be claimed.
    let both = "try { defineConfig({ keys: { Swipe4: binding(() => new WorkspaceSlideBinding({ trigger: 'Swipe4' })) }, \
                gestureActions: { Swipe4: { up: 'window.close' } } }); (globalThis as any).out = 'accepted'; } \
                catch (e: any) { (globalThis as any).out = e.message; }\n";
    let mut rig = rig_with(IMPORTS, both);
    assert_eq!(
        rig.eval("String(globalThis.out)"),
        "gestureActions[\"Swipe4\"]: config.keys binds Swipe4 too; give the gesture to one of them"
    );
    let sideways = "try { defineConfig({ gestureActions: { Swipe4: { sideways: 'window.close' } } }); \
                    (globalThis as any).out = 'accepted'; } catch (e: any) { (globalThis as any).out = e.message; }\n";
    let mut rig = rig_with(IMPORTS, sideways);
    assert_eq!(
        rig.eval("String(globalThis.out)"),
        "onSwipe: \"sideways\" is not a way for this gesture to go; use left, right, up, down",
        "a way the gesture cannot go is reported when the config is checked"
    );
}

// ---- the gesture event ----------------------------------------------------------------------------------------------

#[test]
fn a_gesture_binding_hears_the_raw_dx_dy_scale_and_rotation_next_to_progress() {
    // Lua: `seat:bind({ Swipe4 = fn })` got `e.phase`, `e.dx`, `e.dy`, `e.scale` and `e.rotation`; the audit row said the
    // TypeScript event carried `progress` only.
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "globalThis.seen = []; \
         class Probe extends GestureBinding { \
             onStart(e) { seen.push(['begin', e.phase, e.fingers, e.dx, e.dy, e.scale, e.rotation, e.progress].join(' ')); return {}; } \
             onUpdate(e) { seen.push(['update', e.phase, e.fingers, e.dx, e.dy, e.scale, e.rotation, e.progress].join(' ')); } \
             onEnd(e) { seen.push(['end', e.phase, e.fingers, e.dx, e.dy, e.scale, e.rotation, e.progress].join(' ')); } \
         } \
         gnoblin.input.primary.bind('Pinch3', new Probe({ id: 'probe', trigger: 'Pinch3' }))",
    );
    settle(&mut rig);
    let id = rig.comp.gesture_claim("pinch", 3);
    rig.comp.gesture(id, "pinch", 3, "begin", (0.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    rig.comp.gesture(id, "pinch", 3, "update", (4.0, -6.0), 1.25, 0.5);
    settle(&mut rig);
    rig.comp.gesture(id, "pinch", 3, "end", (4.0, -6.0), 1.5, 0.75);
    settle(&mut rig);
    assert_eq!(
        rig.eval("seen.join('|')"),
        "begin begin 3 0 0 1 0 0|update update 3 4 -6 1.25 0.5 0.5|end end 3 4 -6 1.5 0.75 1",
        "the phase, the finger count, dx, dy, scale, rotation and then progress"
    );
}

// ---- reduced motion ------------------------------------------------------------------------------------------------

#[test]
fn motion_is_the_given_seconds_and_zero_while_the_seat_asks_for_reduced_motion() {
    // Lua: `motion(seconds)` of init.lua, "with reduced motion on, ...".
    let mut rig = world(0, false, "new Layout()", "");
    assert_eq!(rig.eval("String(gnoblin.motion(0.25))"), "0.25");
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.motion(0.25))"),
        "0",
        "reduced motion zeroes it"
    );
    assert_eq!(rig.eval("String(gnoblin.input.primary.settings.reducedMotion)"), "true");
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(false));
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.motion(0.25))"),
        "0.25",
        "and it follows the seat back"
    );
}

/// Shows workspace 2 from 1 over a quarter of a second and returns whether the windows of 1 are still drawn a frame later.
fn leaving_windows_stay_drawn(rig: &mut Rig) -> bool {
    exec(rig, "gnoblin.workspaces.show('2', { duration: 0.25 })");
    rig.frame();
    rig.frame();
    rig.eval("String(gnoblin.clients.at(0).visible)") == "true"
}

#[test]
fn a_workspace_show_slides_unless_the_seat_asks_for_reduced_motion() {
    // Lua: "with reduced motion on, a committed swipe writes no tween and still ends in place".
    let mut rig = world(2, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    settle(&mut rig);
    assert!(
        leaving_windows_stay_drawn(&mut rig),
        "without reduced motion the leaving windows stay drawn while they slide"
    );

    let mut rig = world(2, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    assert!(
        !leaving_windows_stay_drawn(&mut rig),
        "with it the switch is at once: nothing is held over"
    );
    assert_eq!(active(&mut rig), "2");
    assert_eq!(rig.comp.timed_sets(), 0, "no write may carry a duration");
}

#[test]
fn a_committed_swipe_with_reduced_motion_writes_no_tween_and_ends_in_place() {
    // Lua: "with reduced motion on, a committed swipe writes no tween and still ends in place".
    let mut rig = world(2, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    let (rest_a, rest_b) = (rig.comp.number(3, "x"), rig.comp.number(4, "x"));
    let id = rig.comp.gesture_claim("swipe", 3);
    rig.comp.gesture(id, "swipe", 3, "begin", (0.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    rig.comp.gesture(id, "swipe", 3, "update", (-300.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    rig.comp.gesture(id, "swipe", 3, "end", (-300.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    assert_eq!(rig.comp.timed_sets(), 0, "no write may carry a duration");
    assert_eq!(active(&mut rig), "2", "the next workspace must be shown");
    assert_eq!(rig.comp.number(4, "x"), rest_b, "its window is in place");
    assert_eq!(rig.comp.number(3, "x"), rest_a, "and the old one is put back");
    assert_eq!(rig.eval("String(gnoblin.clients.get('3').visible)"), "false");
}

#[test]
fn a_short_swipe_with_reduced_motion_goes_back_at_once() {
    // New: the slide back of the Lua shell used `motion(0.2)` too.
    let mut rig = world(2, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    let id = rig.comp.gesture_claim("swipe", 3);
    rig.comp.gesture(id, "swipe", 3, "begin", (0.0, 0.0), 1.0, 0.0);
    rig.comp.gesture(id, "swipe", 3, "update", (-40.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    rig.comp.gesture(id, "swipe", 3, "end", (-40.0, 0.0), 1.0, 0.0);
    settle(&mut rig);
    assert_eq!(rig.comp.timed_sets(), 0, "no write may carry a duration");
    assert_eq!(active(&mut rig), "1", "a short swipe leaves the workspace");
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('4').visible)"),
        "false",
        "the pulled-in window is hidden again"
    );
}

#[test]
fn client_animate_lands_at_once_under_reduced_motion_and_takes_its_time_otherwise() {
    // New: `Client.animate` is where a script's own animation goes; the Lua `node:to` took `motion(seconds)`.
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').animate({ opacity: 0.5 }, { duration: 1 })",
    );
    settle(&mut rig);
    assert_eq!(rig.comp.timed_sets(), 1, "a timed write when motion is on");
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    exec(
        &mut rig,
        "gnoblin.clients.get('3').animate({ opacity: 0.25 }, { duration: 1 })",
    );
    settle(&mut rig);
    assert_eq!(rig.comp.timed_sets(), 1, "no second timed write");
    assert_eq!(rig.comp.number(3, "opacity"), 0.25, "the value landed at once");
}

#[test]
fn the_titlebar_close_fades_the_window_out_unless_the_seat_asks_for_reduced_motion() {
    // New: the Lua fade (`w:to(motion(0.18), ...)`) of decorations.lua, ported with its reduced-motion rule.
    let close_button = |rig: &Rig| {
        let bar = rig.comp.children(3)[0];
        *rig.comp.children(bar).last().expect("the bar has buttons")
    };
    let mut rig = world(1, false, "new Layout()", "decorations: true,");
    rig.comp.pointer(close_button(&rig), "press", 4.0, 4.0, LEFT);
    settle(&mut rig);
    assert!(rig.comp.timed_sets() > 0, "the fade is a timed write");

    let mut rig = world(1, false, "new Layout()", "decorations: true,");
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    rig.comp.pointer(close_button(&rig), "press", 4.0, 4.0, LEFT);
    settle(&mut rig);
    assert_eq!(rig.comp.timed_sets(), 0, "no fade under reduced motion");
    assert!(has_call(&rig, 3, "close"), "the window is asked to close at once");
    assert_eq!(
        rig.comp.number(3, "opacity"),
        0.0,
        "and the fade's end value is already there"
    );
}

// ---- the keyboard goes to the neighbour of a window that closes -----------------------------------------------------

#[test]
fn closing_the_focused_window_hands_the_keyboard_to_its_neighbour_in_every_layout() {
    // Lua: "closing the focused column moves focus to its neighbour, not to the closed one" (strip.lua:110), for all layouts.
    for layout in [
        "new Layout()",
        "'master-stack'",
        "'monocle'",
        "'grid'",
        "'dwindle'",
        "'floating'",
    ] {
        let mut rig = world(3, false, layout, "");
        give_focus(&mut rig, 4);
        close_window(&mut rig, 4);
        assert_eq!(
            focus(&rig),
            5,
            "{layout}: the window after the closed one takes the keyboard"
        );
        close_window(&mut rig, 5);
        assert_eq!(
            focus(&rig),
            3,
            "{layout}: the last window closing hands it to the one before"
        );
        close_window(&mut rig, 3);
        assert_eq!(focus(&rig), 0, "{layout}: the last window closing leaves no focus");
    }
}

#[test]
fn closing_a_window_that_is_not_focused_changes_no_focus() {
    // Lua: the `Strip:on_closed` hand-off runs for the focused window only.
    let mut rig = world(3, false, "new Layout()", "");
    give_focus(&mut rig, 5);
    close_window(&mut rig, 4);
    assert_eq!(focus(&rig), 5, "the focused window keeps the keyboard");
}

#[test]
fn a_window_the_compositor_already_gave_the_keyboard_to_is_left_alone() {
    // New: the compositor may pick the next window itself; the shell hands the keyboard on only to nobody.
    let mut rig = world(3, false, "new Layout()", "");
    give_focus(&mut rig, 4);
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(3));
    rig.comp.destroy(4);
    settle(&mut rig);
    assert_eq!(focus(&rig), 3);
}

#[test]
fn closing_a_focused_dialog_hands_the_keyboard_to_its_parent_and_a_minimised_neighbour_is_skipped() {
    // New: the Lua strip took the next column whatever it was; a dialog's natural neighbour is its parent.
    let mut rig = world(3, false, "new Layout()", "");
    create_dialog(&mut rig, 30, 4);
    give_focus(&mut rig, 30);
    close_window(&mut rig, 30);
    assert_eq!(
        focus(&rig),
        4,
        "the dialog's parent, though the dialog sat last in the list"
    );

    rig.comp.external_set(5, "minimised", MirrorValue::Bool(true));
    settle(&mut rig);
    give_focus(&mut rig, 4);
    close_window(&mut rig, 4);
    assert_eq!(
        focus(&rig),
        3,
        "a minimised window is skipped: the window before takes it"
    );
}

#[test]
fn a_layout_with_its_own_removal_hook_keeps_the_hand_off_to_itself() {
    // Lua: `Strip` hands the focus on in its own column order; every other layout has the shell's order.
    let mut rig = world(
        3,
        false,
        "new (class extends Layout { onClientRemoved() { globalThis.hooked = (globalThis.hooked ?? 0) + 1; } })()",
        "",
    );
    give_focus(&mut rig, 4);
    close_window(&mut rig, 4);
    assert_eq!(rig.eval("String(globalThis.hooked)"), "1", "the layout heard it");
    assert_eq!(focus(&rig), 0, "and the shell left the hand-off to it");
}

#[test]
fn workspace_walk_moves_the_focus_along_the_workspaces_windows() {
    // Lua: `Workspace:walk(step)`: focus the next window in tree order; no window focused or a step past the end does nothing.
    let mut rig = world(3, false, "new Layout()", "");
    exec(&mut rig, "globalThis.ws = gnoblin.workspaces.get('1')");
    exec(&mut rig, "ws.walk(1)");
    settle(&mut rig);
    assert_eq!(focus(&rig), 0, "no focused window: nothing to walk from");
    give_focus(&mut rig, 4);
    exec(&mut rig, "ws.walk(1)");
    settle(&mut rig);
    assert_eq!(focus(&rig), 5);
    exec(&mut rig, "ws.walk(1)");
    settle(&mut rig);
    assert_eq!(focus(&rig), 5, "past the last window the focus stays");
    exec(&mut rig, "ws.walk(-1)");
    settle(&mut rig);
    assert_eq!(focus(&rig), 4, "a step back");
    exec(&mut rig, "ws.walk(-1)");
    settle(&mut rig);
    assert_eq!(focus(&rig), 3, "and another");
    exec(&mut rig, "ws.walk(-1)");
    settle(&mut rig);
    assert_eq!(focus(&rig), 3, "past the first window the focus stays");
    assert_eq!(
        rig.eval("try { ws.walk(1.5); 'walked' } catch (e) { e.message }"),
        "Workspace#1.walk: step must be a whole number, got 1.5"
    );
}

// ---- removing a workspace --------------------------------------------------------------------------------------------

#[test]
fn removing_a_workspace_moves_its_windows_to_the_neighbour_before_it() {
    // Lua: "removing a workspace moves its windows to the neighbour before it".
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('3'))",
    );
    settle(&mut rig);
    assert_eq!(rig.eval("String(gnoblin.workspaces.remove('3'))"), "true");
    settle(&mut rig);
    assert_eq!(
        workspace_of(&mut rig, 3),
        "2",
        "the window moves to the neighbour before the removed workspace"
    );
    assert_eq!(
        active(&mut rig),
        "1",
        "removing an inactive workspace keeps the active one"
    );
    assert_eq!(rig.eval("String(gnoblin.workspaces.length)"), "2");

    // The first workspace has none before it: its windows go to the one after.
    let mut rig = world(1, false, "new Layout()", "");
    exec(&mut rig, "gnoblin.workspaces.remove('1')");
    settle(&mut rig);
    assert_eq!(workspace_of(&mut rig, 3), "2", "the one after it, for the first");
}

#[test]
fn removing_the_active_workspace_shows_its_neighbour_and_the_moved_window_keeps_the_focus() {
    // Lua: "removing the active workspace shows its neighbour and refocuses the moved window".
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    exec(&mut rig, "gnoblin.workspaces.remove(gnoblin.workspaces.get('1'))");
    settle(&mut rig);
    assert_eq!(active(&mut rig), "2", "the neighbour must become active");
    assert_eq!(workspace_of(&mut rig, 3), "2", "the window must move to the neighbour");
    assert_eq!(rig.eval("String(gnoblin.clients.get('3').visible)"), "true");
    assert_eq!(focus(&rig), 3, "the moved window must keep focus");
}

#[test]
fn removing_an_inactive_workspace_keeps_the_focus_and_moves_the_stray_window_beside_it() {
    // Lua: "removing an inactive workspace keeps focus on the focused window and moves the stray window beside it".
    let mut rig = world(2, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    give_focus(&mut rig, 3);
    exec(&mut rig, "gnoblin.workspaces.remove('2')");
    settle(&mut rig);
    assert_eq!(
        workspace_of(&mut rig, 4),
        "1",
        "the stray window must move to the neighbour before it"
    );
    assert_eq!(active(&mut rig), "1", "the active workspace must stay");
    assert_eq!(
        focus(&rig),
        3,
        "the focused window must keep focus when another workspace goes"
    );
}

#[test]
fn the_last_workspace_cannot_be_removed() {
    // Lua: "the last workspace of a desk cannot be removed".
    let mut rig = world(1, false, "new Layout()", "");
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.remove('3'))"),
        "true",
        "a spare workspace must go"
    );
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.remove('2'))"),
        "true",
        "a spare workspace must go"
    );
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.remove(gnoblin.workspaces.get('1')))"),
        "false",
        "the last workspace must be refused"
    );
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.length)"),
        "1",
        "the refused workspace must stay"
    );
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.remove('nope'))"),
        "false",
        "an unknown id removes nothing"
    );
    assert_eq!(workspace_of(&mut rig, 3), "1");
}

// ---- a press on a titlebar -------------------------------------------------------------------------------------------

#[test]
fn a_press_on_a_titlebar_focuses_and_raises_its_window() {
    // The audit row of decorations.lua:74-92: the stock policy of a press on a window (`seat.focus = w`, then `raise`) holds
    // for the press on its bar too.
    let mut rig = world(2, false, "new Layout()", "decorations: true,");
    give_focus(&mut rig, 4);
    let bar = rig.comp.children(3)[0];
    assert_eq!(
        rig.comp.children(ROOT).last(),
        Some(&4),
        "window 4 is on top to begin with"
    );
    rig.comp.pointer(bar, "press", 5.0, 5.0, LEFT);
    settle(&mut rig);
    assert_eq!(focus(&rig), 3, "the press focuses the window");
    assert_eq!(
        rig.comp.children(ROOT).last(),
        Some(&3),
        "and raises it over the other window"
    );
    rig.comp.pointer(bar, "release", 5.0, 5.0, LEFT);
    settle(&mut rig);

    // The same press on a window whose seat keeps the stacking (raise_on_click off) focuses and does not raise.
    let mut rig = world(
        2,
        false,
        "new Layout()",
        "decorations: true, focus: { raiseOnClick: false },",
    );
    give_focus(&mut rig, 4);
    let bar = rig.comp.children(3)[0];
    rig.comp.pointer(bar, "press", 5.0, 5.0, LEFT);
    settle(&mut rig);
    assert_eq!(focus(&rig), 3, "the press focuses the window");
    assert_eq!(
        rig.comp.children(ROOT).last(),
        Some(&4),
        "raise_on_click off: the stacking is left alone"
    );
}

// ---- the bars change under maximised and snapped windows -----------------------------------------------------------

#[test]
fn a_maximised_window_refits_when_a_bar_reserves_space_at_the_top() {
    // Lua: "a maximised window refits when a bar reserves space at the top (failure mode 9, maximised)".
    let mut rig = world(1, false, "new Layout()", "");
    exec(&mut rig, "windowState.maximise(gnoblin.clients.get('3'))");
    settle(&mut rig);
    assert_eq!(rect(&rig, 3), "0,0,1000,800", "maximised fills the screen");
    exec(&mut rig, "g.bar = new Bar({ edge: 'top', height: 32 })");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 3),
        "0,32,1000,768",
        "the window must move below the bar and refit"
    );
    exec(&mut rig, "g.bar.destroy()");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 3),
        "0,0,1000,800",
        "and fill the screen again when the bar goes"
    );
}

#[test]
fn a_snapped_window_refits_when_a_bar_reserves_space_at_the_top() {
    // Lua: "a snapped window refits when a bar reserves space at the top (failure mode 9, snapped)".
    let mut rig = world(1, false, "new Layout()", "");
    assert_eq!(
        rig.eval(
            "String(snapClient(gnoblin.clients.get('3'), 'right', { edgeTiling: true, area: { x: 0, y: 0, width: 1000, height: 800 } }))"
        ),
        "true"
    );
    settle(&mut rig);
    assert_eq!(rect(&rig, 3), "500,0,500,800", "the right half must be tiled");
    exec(&mut rig, "g.bar = new Bar({ edge: 'top', height: 32 })");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 3),
        "500,32,500,768",
        "the snapped window must move below the bar and refit"
    );
    assert_eq!(
        rig.eval("snappedZone(gnoblin.clients.get('3'))"),
        "right",
        "and is still snapped"
    );
}

#[test]
fn a_floating_window_is_brought_inside_the_smaller_work_area() {
    // Lua: `Desk:arrange` refits every window, a floating one included: `w:refit(self:work_area())`.
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').bounds = { x: 100, y: 0, width: 300, height: 200 }",
    );
    settle(&mut rig);
    exec(&mut rig, "g.bar = new Bar({ edge: 'top', height: 32 })");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 3),
        "100,32,300,200",
        "a window under the new bar moves below it"
    );
}

// ---- a reload ------------------------------------------------------------------------------------------------------

#[test]
fn a_reload_keeps_the_rectangles_a_maximised_fullscreen_or_snapped_window_returns_to() {
    // New: JsClientRequests left it undone: "the kept rectangles across a reload live in the generation".
    let mut rig = world(3, false, "new Layout()", "");
    exec(
        &mut rig,
        "windowState.maximise(gnoblin.clients.get('3')); \
         windowState.enterFullscreen(gnoblin.clients.get('4')); \
         snapClient(gnoblin.clients.get('5'), 'left', { edgeTiling: true, area: { x: 0, y: 0, width: 1000, height: 800 } })",
    );
    settle(&mut rig);
    let snapshot = rig.shell.workspace_snapshot().expect("a snapshot");
    let kept = |node: u64| snapshot.kept.iter().find(|window| window.node == node).cloned();
    assert_eq!(
        kept(3).and_then(|window| window.floating),
        Some([10.0, 20.0, 300.0, 200.0])
    );
    assert_eq!(
        kept(4).and_then(|window| window.windowed),
        Some([20.0, 40.0, 300.0, 200.0])
    );
    assert_eq!(
        kept(5).and_then(|window| window.snapped),
        Some(("left".to_owned(), [30.0, 60.0, 300.0, 200.0]))
    );

    let mut second = next_generation(&rig, "");
    second.shell.reconcile(&snapshot).expect("reconciles");
    settle(&mut second);
    exec(
        &mut second,
        "windowState.unmaximise(gnoblin.clients.get('3')); \
         windowState.leaveFullscreen(gnoblin.clients.get('4')); \
         unsnapClient(gnoblin.clients.get('5'))",
    );
    settle(&mut second);
    assert_eq!(
        rect(&second, 3),
        "10,20,300,200",
        "the maximised window goes back to where it floated"
    );
    assert_eq!(
        rect(&second, 4),
        "20,40,300,200",
        "the fullscreen window goes back to its windowed rectangle"
    );
    assert_eq!(
        rect(&second, 5),
        "30,60,300,200",
        "and the snapped one to the rectangle it was snapped from"
    );
}
