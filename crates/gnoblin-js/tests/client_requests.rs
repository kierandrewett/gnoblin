//! What the stock shell answers when a client or a taskbar asks for a window state change: `data/js/gnoblin/window_state.ts`
//! and the pieces of `gnoblin.ts` it stands on, over the fake compositor of `tests/common`.
//!
//! The cases are the retired shell's: `the retired retired corpus` (12 cases) and the `retired corpus shell`
//! cases for raising with a fullscreen sibling, activation and a repeat fullscreen. A case keeps its inputs and its
//! expected values; where the JavaScript shell differs the case says why. The differences are the model's, not the
//! policy's: a JavaScript window is placed in screen coordinates, so the retired rectangle `{ x = 0, y = -32, ... }` (the
//! workspace's own space, which a bar pushes down) is `{ x = 0, y = 0, ... }` here; and there is no workspace clip, so
//! the the retired shell `space.clip` check has no counterpart. A case with no retired original says "new". What the real compositor does
//! with the writes is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

const IMPORTS: &str = "import * as g from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

const SCREEN: u64 = 20;
const SEAT: u64 = 21;

/// A shell over a screen of 1000 by 800, one seat and `n` windows (ids 3 on, 300 by 200 at 10 n, 20 n), on the
/// workspaces `main` and `other`, whose layouts place nothing, with `keys` after them in the config.
fn world_with(n: u64, keys: &str) -> Rig {
    let init = format!(
        "defineConfig({{ workspaces: [{{ id: \"main\", layout: new Layout() }}, {{ id: \"other\", layout: new Layout() }}], decorations: false, {keys} }});\n"
    );
    let mut rig = rig_setup(IMPORTS, &init, move |comp| {
        comp.add_node(
            SCREEN,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        comp.add_node(
            SEAT,
            ROOT,
            "seat",
            vec![
                ("name", text("seat0")),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
            ],
        );
        for i in 1..=n {
            comp.add_node(
                2 + i,
                ROOT,
                "window",
                vec![
                    ("title", text(&format!("win{i}"))),
                    ("app_id", text("foot")),
                    ("x", MirrorValue::Double(10.0 * i as f64)),
                    ("y", MirrorValue::Double(20.0 * i as f64)),
                    ("width", MirrorValue::Double(300.0)),
                    ("height", MirrorValue::Double(200.0)),
                ],
            );
        }
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// Runs a statement in the script. `Rig::eval` wants a value back; a statement has none.
fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

fn world(n: u64) -> Rig {
    world_with(n, "")
}

/// The client's request for window `node`, and the frames the shell takes to answer it.
fn request(rig: &mut Rig, node: u64, name: &str) {
    rig.comp.request(node, name, 0.0, 0.0);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{name}: {diagnostics:?}");
}

fn flag(rig: &Rig, node: u64, key: &str) -> bool {
    rig.comp.flag(node, key) == Some(true)
}

/// The seat's focus: a window's node, or 0 for none.
fn focus(rig: &Rig) -> u64 {
    match rig.comp.value(SEAT, "focus") {
        Some(MirrorValue::Node(id)) => id,
        _ => 0,
    }
}

fn give_focus(rig: &mut Rig, node: u64) {
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(node));
    rig.settle();
}

/// The windows back to front, as the tree stacks them.
fn stack(rig: &Rig, n: u64) -> Vec<u64> {
    rig.comp
        .children(ROOT)
        .into_iter()
        .filter(|id| (3..3 + n).contains(id))
        .collect()
}

fn rect(rig: &Rig, node: u64) -> String {
    rig.comp.rect(node)
}

// ---- the cases of retired corpus states ----------------------------------------------------------------------------------

#[test]
fn a_fullscreen_request_covers_the_whole_output_and_the_window_goes_last_in_its_workspace() {
    // Before: "a fullscreen request covers the whole output and the window goes last in its workspace".
    let mut rig = world(2);
    request(&mut rig, 3, "fullscreen");
    assert!(flag(&rig, 3, "fullscreen"), "a fullscreen request must be granted");
    assert_eq!(rect(&rig, 3), "0,0,1000,800", "fullscreen covers the output");
    assert_eq!(
        stack(&rig, 2).last(),
        Some(&3),
        "the fullscreen window must be last in tree order"
    );
    exec(&mut rig, "gnoblin.clients.at(1).raise()");
    rig.settle();
    assert_eq!(
        stack(&rig, 2).last(),
        Some(&3),
        "a raised sibling must not cover a fullscreen window"
    );
}

#[test]
fn an_unfullscreen_request_returns_the_rectangle_the_window_had() {
    // Before: "an unfullscreen request returns the rectangle the window had".
    let mut rig = world(1);
    let before = rect(&rig, 3);
    request(&mut rig, 3, "fullscreen");
    request(&mut rig, 3, "unfullscreen");
    assert!(!flag(&rig, 3, "fullscreen"), "an unfullscreen request must be granted");
    assert_eq!(
        rect(&rig, 3),
        before,
        "leaving fullscreen returns the rectangle the window had"
    );
}

#[test]
fn a_fullscreen_window_covers_the_bars_which_come_back_when_it_leaves() {
    // Before: "a fullscreen window covers the bars, which come back when it leaves". The retired window starts at the
    // screen's top in the workspace's own space (y -32); in screen coordinates that is y 0.
    let mut rig = world(1);
    exec(&mut rig, "globalThis.bar = new Bar({ edge: 'top', height: 32 })");
    rig.settle();
    assert_eq!(
        rig.eval("String(barInsets().top)"),
        "32",
        "the bar must reserve the top"
    );
    request(&mut rig, 3, "fullscreen");
    assert_eq!(rect(&rig, 3), "0,0,1000,800", "the window starts at the screen's top");
    assert_eq!(
        rig.eval("String(bar.visible)"),
        "false",
        "the bar must give way to a fullscreen window"
    );
    assert_eq!(
        rig.eval("String(barInsets().top)"),
        "0",
        "a bar that gave way takes no room"
    );
    request(&mut rig, 3, "unfullscreen");
    assert_eq!(rig.eval("String(bar.visible)"), "true", "the bar must come back");
    assert_eq!(rig.eval("String(barInsets().top)"), "32");
}

#[test]
fn a_fullscreen_window_keeps_covering_the_output_when_a_bar_reserves_space() {
    // Before: "a fullscreen window keeps covering the output when a bar reserves space".
    let mut rig = world(1);
    request(&mut rig, 3, "fullscreen");
    exec(&mut rig, "globalThis.bar = new Bar({ edge: 'top', height: 32 })");
    rig.settle();
    assert_eq!(
        rect(&rig, 3),
        "0,0,1000,800",
        "the fullscreen window must still cover the whole output"
    );
}

#[test]
fn fullscreen_over_a_maximised_window_keeps_it_maximised_and_leaving_returns_the_maximised_rectangle() {
    // Before: "fullscreen over a maximised window keeps it maximised, and leaving returns the maximised rectangle".
    let mut rig = world(1);
    let floating = rect(&rig, 3);
    request(&mut rig, 3, "maximize");
    let maximised = rect(&rig, 3);
    request(&mut rig, 3, "fullscreen");
    assert!(flag(&rig, 3, "maximised"), "fullscreen must not clear maximise");
    request(&mut rig, 3, "unfullscreen");
    assert_eq!(
        rect(&rig, 3),
        maximised,
        "leaving fullscreen returns the maximised rectangle"
    );
    request(&mut rig, 3, "unmaximize");
    assert_eq!(rect(&rig, 3), floating, "unmaximise returns the floating rectangle");
}

#[test]
fn maximising_fills_the_area_the_bars_leave_and_fullscreen_over_it_fills_the_whole_output() {
    // New: the retired case has no bar, so its maximised and fullscreen rectangles are the same. With a 32 pixel bar the
    // maximised window fills what the bar leaves and fullscreen still covers the whole output.
    let mut rig = world(1);
    exec(&mut rig, "globalThis.bar = new Bar({ edge: 'top', height: 32 })");
    rig.settle();
    let floating = rect(&rig, 3);
    request(&mut rig, 3, "maximize");
    assert_eq!(rect(&rig, 3), "0,32,1000,768");
    request(&mut rig, 3, "fullscreen");
    assert_eq!(rect(&rig, 3), "0,0,1000,800");
    request(&mut rig, 3, "unfullscreen");
    assert_eq!(
        rect(&rig, 3),
        "0,32,1000,768",
        "leaving fullscreen returns the maximised rectangle"
    );
    request(&mut rig, 3, "unmaximize");
    assert_eq!(rect(&rig, 3), floating);
}

#[test]
fn closing_a_fullscreen_window_brings_the_bars_back() {
    // Before: "closing a fullscreen window brings the bars back".
    let mut rig = world(1);
    exec(&mut rig, "globalThis.bar = new Bar({ edge: 'top', height: 32 })");
    rig.settle();
    request(&mut rig, 3, "fullscreen");
    assert_eq!(rig.eval("String(bar.visible)"), "false", "the bar gives way");
    rig.comp.destroy(3);
    rig.settle();
    assert_eq!(
        rig.eval("String(bar.visible)"),
        "true",
        "the bar must come back when the fullscreen window closes"
    );
}

#[test]
fn a_minimise_request_hides_the_window_and_hands_focus_to_the_topmost_window_left() {
    // Before: "a minimise request hides the window and hands focus to the topmost window left". The compositor hides a
    // minimised window (`minimised` is the whole answer), so the the retired shell `visible = false` has no JavaScript write.
    let mut rig = world(2);
    give_focus(&mut rig, 3);
    request(&mut rig, 3, "minimize");
    assert!(flag(&rig, 3, "minimised"), "a minimise request must be granted");
    assert_eq!(focus(&rig), 4, "focus must pass to the window that stays");
}

#[test]
fn minimising_the_last_window_leaves_the_seat_with_no_focus_and_minimising_an_unfocused_window_leaves_it_alone() {
    // Before: "minimising the last window leaves the seat with no focus, and minimising an unfocused window leaves it alone".
    let mut rig = world(2);
    give_focus(&mut rig, 4);
    request(&mut rig, 3, "minimize");
    assert_eq!(focus(&rig), 4, "minimising another window must not move focus");
    request(&mut rig, 4, "minimize");
    assert_eq!(focus(&rig), 0, "no window is left to take focus");
}

#[test]
fn an_unminimise_request_shows_the_window_and_focuses_it_only_when_nothing_holds_focus() {
    // Before: "an unminimise request shows the window, and focuses it only when nothing holds focus".
    let mut rig = world(2);
    give_focus(&mut rig, 4);
    request(&mut rig, 3, "minimize");
    request(&mut rig, 3, "unminimize");
    assert!(!flag(&rig, 3, "minimised"), "the window must be shown again");
    assert_eq!(focus(&rig), 4, "focus stays where it was");
    request(&mut rig, 4, "minimize");
    request(&mut rig, 3, "minimize");
    assert_eq!(focus(&rig), 0, "both windows are hidden");
    request(&mut rig, 4, "unminimize");
    assert_eq!(focus(&rig), 4, "a restored window takes focus when nothing holds it");
}

#[test]
fn an_activate_request_brings_a_minimised_window_back_focused_and_on_top() {
    // Before: "an activate request brings a minimised window back, focused and on top".
    let mut rig = world(2);
    request(&mut rig, 3, "minimize");
    request(&mut rig, 3, "activate");
    assert!(!flag(&rig, 3, "minimised"), "activating shows the window");
    assert_eq!(focus(&rig), 3, "the window must be focused");
    assert_eq!(stack(&rig, 2).last(), Some(&3), "the window must be raised");
}

#[test]
fn a_close_request_calls_the_windows_close_which_the_compositor_turns_into_the_clients_close() {
    // Before: "a close request calls the window's close, which the compositor turns into the client's close".
    let mut rig = world(2);
    request(&mut rig, 4, "close");
    let closes: Vec<u64> = rig
        .comp
        .calls()
        .into_iter()
        .filter(|(_, method)| method == "close")
        .map(|(node, _)| node)
        .collect();
    assert_eq!(
        closes,
        vec![4],
        "one close call, naming the window that was asked to close"
    );
    assert_eq!(
        rig.eval("String(gnoblin.clients.at(1).alive)"),
        "true",
        "the window stays until the client destroys it"
    );
}

#[test]
fn raise_and_lower_requests_restack_the_window_among_its_siblings() {
    // Before: "raise and lower requests restack the window among its siblings".
    let mut rig = world(2);
    assert_eq!(stack(&rig, 2).last(), Some(&4), "the second window starts on top");
    request(&mut rig, 3, "raise");
    assert_eq!(
        stack(&rig, 2).last(),
        Some(&3),
        "a raise request puts the window on top"
    );
    request(&mut rig, 3, "lower");
    assert_eq!(
        stack(&rig, 2).first(),
        Some(&3),
        "a lower request puts the window at the bottom"
    );
}

// ---- the cases of retired corpus shell -----------------------------------------------------------------------------------

#[test]
fn raising_a_window_keeps_a_fullscreen_sibling_above_it_and_a_fullscreen_window_stays_on_top_when_raised() {
    // Before: "raising a window keeps a fullscreen sibling above it, and a fullscreen window stays on top when raised",
    // with a, b, c as windows 3, 4, 5 and `b.fullscreen = true` the plain write of the flag.
    let mut rig = world(3);
    for at in [0, 1, 2] {
        exec(&mut rig, &format!("gnoblin.clients.at({at}).raise()"));
    }
    rig.settle();
    assert_eq!(stack(&rig, 3), vec![3, 4, 5]);
    exec(&mut rig, "gnoblin.clients.at(1).fullscreen = true");
    exec(&mut rig, "gnoblin.clients.at(0).raise()");
    rig.settle();
    assert_eq!(stack(&rig, 3), vec![5, 3, 4]);
    exec(&mut rig, "gnoblin.clients.at(2).raise()");
    rig.settle();
    assert_eq!(stack(&rig, 3), vec![3, 5, 4]);
    exec(&mut rig, "gnoblin.clients.at(1).raise()");
    rig.settle();
    assert_eq!(stack(&rig, 3), vec![3, 5, 4]);
}

#[test]
fn an_activate_request_focuses_a_window_on_the_active_workspace() {
    // Before: "a client's activate request is decided by the shell as an activation".
    let mut rig = world(1);
    request(&mut rig, 3, "activate");
    assert_eq!(focus(&rig), 3, "an activate request must focus");
}

#[test]
fn activating_a_window_on_a_hidden_workspace_is_refused_and_leaves_focus_alone() {
    // Before: "activating a window on a hidden workspace is refused and leaves focus alone".
    let mut rig = world(1);
    exec(&mut rig, "gnoblin.show('other')");
    rig.settle();
    request(&mut rig, 3, "activate");
    assert_eq!(focus(&rig), 0, "a hidden window must not take focus");
}

#[test]
fn activating_the_focused_window_again_changes_nothing() {
    // Before: "activating the focused window again changes nothing".
    let mut rig = world(1);
    request(&mut rig, 3, "activate");
    request(&mut rig, 3, "activate");
    assert_eq!(focus(&rig), 3, "repeat activation keeps the focus on the window");
}

#[test]
fn a_repeat_fullscreen_request_is_refused_so_the_windowed_rectangle_is_not_overwritten() {
    // Before: `enter_fullscreen` returns false when the window is fullscreen already and keeps `data.windowed`.
    let mut rig = world(1);
    let before = rect(&rig, 3);
    request(&mut rig, 3, "fullscreen");
    request(&mut rig, 3, "fullscreen");
    request(&mut rig, 3, "unfullscreen");
    assert_eq!(
        rect(&rig, 3),
        before,
        "the rectangle kept is the windowed one, not the fullscreen one"
    );
    assert!(!flag(&rig, 3, "fullscreen"));
}

#[test]
fn a_maximise_request_while_fullscreen_is_refused() {
    // Before: `Window:maximise` returns false when the window is fullscreen.
    let mut rig = world(1);
    request(&mut rig, 3, "fullscreen");
    request(&mut rig, 3, "maximize");
    assert!(!flag(&rig, 3, "maximised"));
    assert_eq!(rect(&rig, 3), "0,0,1000,800");
}

#[test]
fn an_unminimise_request_for_a_window_that_is_not_minimised_changes_nothing() {
    // Before: `Window:unminimise` returns false when the window is not minimised, so it does not take focus either.
    let mut rig = world(2);
    request(&mut rig, 3, "unminimize");
    assert_eq!(focus(&rig), 0);
}

// ---- the listener hook: new ----------------------------------------------------------------------------------------

const LISTENERS: &str = r#"
globalThis.log = [];
class Deny extends ClientListener {
    onRequest(client, request) {
        log.push(`deny:${request.kind}:${client.title}:${request.handled}`);
        if (request.kind === "fullscreen") {
            request.handle();
        }
    }
}
class Watch extends ClientListener {
    onRequest(client, request) {
        log.push(`watch:${request.kind}:${request.handled}`);
    }
}
gnoblin.clients.addListener(new Deny());
gnoblin.clients.addListener(new Watch());
"#;

#[test]
fn a_listener_that_handles_a_request_keeps_the_built_in_policy_from_running_and_later_listeners_from_hearing_it() {
    // New: `ClientListener.onRequest`. The first listener that calls `request.handle()` wins; the rest are not asked.
    let mut rig = world_with(1, "setup: () => {}");
    exec(&mut rig, LISTENERS);
    request(&mut rig, 3, "fullscreen");
    assert!(
        !flag(&rig, 3, "fullscreen"),
        "the listener answered, so the built-in policy did not"
    );
    assert_eq!(rig.eval("log.join(',')"), "deny:fullscreen:win1:false");
}

#[test]
fn a_request_no_listener_handles_reaches_the_built_in_policy_after_every_listener_heard_it() {
    // New: letting the request through is just returning.
    let mut rig = world_with(1, "setup: () => {}");
    exec(&mut rig, LISTENERS);
    request(&mut rig, 3, "maximize");
    assert!(flag(&rig, 3, "maximised"));
    assert_eq!(
        rig.eval("log.join(',')"),
        "deny:maximize:win1:false,watch:maximize:false"
    );
}

#[test]
fn a_listener_on_one_client_hears_the_requests_of_that_client_before_the_collections_listeners() {
    // New: the client's own listeners go first, then those of the collections that hold it, each in the order attached.
    let mut rig = world_with(2, "setup: () => {}");
    exec(
        &mut rig,
        r#"
        globalThis.log = [];
        class Own extends ClientListener {
            onRequest(client, request) { log.push(`own:${client.title}:${request.kind}`); }
        }
        class All extends ClientListener {
            onRequest(client, request) { log.push(`all:${client.title}:${request.kind}`); }
        }
        gnoblin.clients.addListener(new All());
        gnoblin.clients.at(0).addListener(new Own());
        "#,
    );
    request(&mut rig, 3, "lower");
    request(&mut rig, 4, "lower");
    assert_eq!(
        rig.eval("log.join(',')"),
        "own:win1:lower,all:win1:lower,all:win2:lower"
    );
}

#[test]
fn a_listener_that_throws_is_reported_and_the_built_in_policy_still_runs() {
    // New: a throw is the listener's failure, not the request's.
    let mut rig = world_with(1, "setup: () => {}");
    exec(
        &mut rig,
        r#"
        class Bad extends ClientListener {
            onRequest() { throw new Error("no"); }
        }
        gnoblin.clients.addListener(new Bad());
        "#,
    );
    rig.comp.request(3, "fullscreen", 0.0, 0.0);
    let diagnostics = rig.settle();
    assert!(
        diagnostics.iter().any(|d| d.contains("onRequest") && d.contains("no")),
        "{diagnostics:?}"
    );
    assert!(flag(&rig, 3, "fullscreen"));
}

#[test]
fn a_script_that_handles_a_request_can_run_the_stock_answer_for_another_kind() {
    // New: `windowState` is the policy the built-in answer calls, so a listener can answer a maximise as a fullscreen.
    let mut rig = world_with(1, "setup: () => {}");
    exec(
        &mut rig,
        r#"
        class Redirect extends ClientListener {
            onRequest(client, request) {
                if (request.kind === "maximize") {
                    request.handle();
                    windowState.enterFullscreen(client);
                }
            }
        }
        gnoblin.clients.addListener(new Redirect());
        "#,
    );
    request(&mut rig, 3, "maximize");
    assert!(flag(&rig, 3, "fullscreen"));
    assert!(!flag(&rig, 3, "maximised"));
    assert_eq!(rect(&rig, 3), "0,0,1000,800");
}

#[test]
fn a_request_for_a_window_the_shell_does_not_know_is_ignored() {
    // New: the compositor may name a node that went in the same frame.
    let mut rig = world(1);
    rig.comp.request(99, "fullscreen", 0.0, 0.0);
    assert!(rig.settle().is_empty());
}

// ---- move and resize requests: new ---------------------------------------------------------------------------------

fn pointer_to(rig: &mut Rig, x: f64, y: f64) {
    rig.comp.external_set(SEAT, "pointer_x", MirrorValue::Double(x));
    rig.comp.external_set(SEAT, "pointer_y", MirrorValue::Double(y));
    rig.settle();
}

#[test]
fn a_move_request_on_a_floating_window_follows_the_pointer_until_the_button_is_released() {
    // New. Window 3 is 300 by 200 at 10,20. The request carries no pointer: the shell reads the seat's.
    let mut rig = world(1);
    pointer_to(&mut rig, 100.0, 100.0);
    request(&mut rig, 3, "move");
    pointer_to(&mut rig, 130.0, 150.0);
    assert_eq!(
        rect(&rig, 3),
        "40,70,300,200",
        "the window follows the pointer by the distance it moved"
    );
    rig.comp.pointer(3, "release", 0.0, 0.0, LEFT);
    rig.settle();
    pointer_to(&mut rig, 500.0, 500.0);
    assert_eq!(
        rect(&rig, 3),
        "40,70,300,200",
        "after the release the pointer moves the window no more"
    );
}

#[test]
fn a_move_request_keeps_the_window_in_the_work_area_as_the_pointer_drag_does() {
    // New: the same `constrainRect` as `ClientMoveBinding`, so a window cannot be dragged off the screen.
    let mut rig = world(1);
    pointer_to(&mut rig, 100.0, 100.0);
    request(&mut rig, 3, "move");
    pointer_to(&mut rig, 2000.0, 2000.0);
    assert_eq!(rect(&rig, 3), "700,600,300,200");
}

#[test]
fn a_resize_request_moves_the_corner_nearest_the_pointer() {
    // New. Window 3 spans 10,20 to 310,220; the pointer is by its bottom right corner.
    let mut rig = world(1);
    pointer_to(&mut rig, 305.0, 215.0);
    request(&mut rig, 3, "resize");
    pointer_to(&mut rig, 325.0, 235.0);
    assert_eq!(rect(&rig, 3), "10,20,320,220");
    rig.comp.pointer(3, "release", 0.0, 0.0, LEFT);
    rig.settle();
}

#[test]
fn a_resize_request_by_an_edge_moves_that_edge_alone() {
    // New. The pointer is at the middle of the right edge, so only the width changes.
    let mut rig = world(1);
    pointer_to(&mut rig, 308.0, 120.0);
    request(&mut rig, 3, "resize");
    pointer_to(&mut rig, 338.0, 160.0);
    assert_eq!(rect(&rig, 3), "10,20,330,200");
}

#[test]
fn a_move_request_is_ignored_for_a_window_a_layout_tiles() {
    // New: a tiled window is not dragged; the layout places it.
    let mut rig = world_with(1, "setup: () => {}");
    exec(&mut rig, "gnoblin.workspaces.at(0).layout = new MasterStackLayout()");
    rig.settle();
    let tiled = rect(&rig, 3);
    assert_eq!(rig.eval("String(gnoblin.clients.at(0).tiled)"), "true");
    pointer_to(&mut rig, 100.0, 100.0);
    request(&mut rig, 3, "move");
    pointer_to(&mut rig, 130.0, 150.0);
    assert_eq!(rect(&rig, 3), tiled);
}

#[test]
fn a_move_request_is_ignored_for_a_fullscreen_or_maximised_window() {
    // New: `ClientMoveBinding.onStart` declines them.
    let mut rig = world(1);
    request(&mut rig, 3, "fullscreen");
    pointer_to(&mut rig, 100.0, 100.0);
    request(&mut rig, 3, "move");
    pointer_to(&mut rig, 130.0, 150.0);
    assert_eq!(rect(&rig, 3), "0,0,1000,800");
}

#[test]
fn a_drag_the_request_started_ends_when_its_window_goes() {
    // New: nothing is left following the pointer for a window that is gone.
    let mut rig = world(2);
    pointer_to(&mut rig, 100.0, 100.0);
    request(&mut rig, 3, "move");
    rig.comp.destroy(3);
    rig.settle();
    pointer_to(&mut rig, 130.0, 150.0);
    assert_eq!(rect(&rig, 4), "20,40,300,200", "the other window was not touched");
}

// ---- a fullscreen window and the layout: new -----------------------------------------------------------------------

#[test]
fn a_tiling_layout_leaves_a_fullscreen_window_alone_and_places_it_again_when_it_leaves() {
    // New: the layout would otherwise re-place a fullscreen window on its next pass.
    let mut rig = world_with(2, "setup: () => {}");
    exec(&mut rig, "gnoblin.workspaces.at(0).layout = new MasterStackLayout()");
    rig.settle();
    let tiled = rect(&rig, 3);
    request(&mut rig, 3, "fullscreen");
    exec(&mut rig, "gnoblin.workspaces.at(0).layout.ratio = 0.3");
    rig.settle();
    assert_eq!(
        rect(&rig, 3),
        "0,0,1000,800",
        "the layout does not move a fullscreen window"
    );
    request(&mut rig, 3, "unfullscreen");
    rig.settle();
    assert_ne!(rect(&rig, 3), "0,0,1000,800", "the window is tiled again");
    assert_ne!(rect(&rig, 3), tiled, "the layout placed it with the ratio it has now");
}
