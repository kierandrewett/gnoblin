//! Focus, window rules and edge snapping of the stock library: `data/js/gnoblin-std/{focus,snap}.ts` and the
//! pieces of `gnoblin-core/host` and `gnoblin/bindings` they stand on.
//!
//! The cases are the retired shell's: `the retired retired corpus`. A retired case keeps its
//! inputs and its expected values; where the JavaScript API changed a word (`hover` is `sloppy`, a retired pattern is a
//! glob, `last matching rule wins` is `first wins unless all`), the case says so. A case with no retired original says
//! "new". The pure functions run in a bare generation (no host); everything that touches a window runs a real
//! `JsShell` over the shared fake compositor of `tests/common`, an in-memory compositor that records the calls and
//! writes the shell sends and plays back pointer edges, new windows and drags. Nothing here proves what the real
//! compositor does with those calls: that is `crates/gnoblin/tests/js_shell.rs`.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, JsManualClock, JsShell, stock_modules};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};
use tempfile::TempDir;

mod common;
use common::{FakeCompositor, ROOT, text, timed_config};

// -- the pure functions, run in a bare generation -----------------------------------------------------------

const HARNESS: &str = r##"
globalThis.gnoblin = {};
export const emit = (value) => { globalThis.out = String(value); };
export const thrown = (fn) => {
    try { fn(); } catch (e) { return `${e.name}: ${e.message}`; }
    return "did not throw";
};
"##;

const IMPORTS: &str = r#"
import { emit, thrown } from "harness";
import {
    focusSettings, seatFocusSettings, opensFocused, hoverFocus, REST_DELAY, restFocus,
    leaveClears, focusDecision, edgeTilingEnabled, edgeZone, tileRect,
    ClientResizeBinding, ClientMoveBinding,
} from "gnoblin-std";
"#;

/// Evaluates `body` (TypeScript) with the stock library imported and returns what it passed to `emit`.
fn run(body: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), format!("{IMPORTS}\n{body}\n")).expect("init.ts is written");
    let mut config = JsConfig::new(&root).embed("harness", HARNESS);
    for (name, source) in stock_modules() {
        config = config.embed(name, source);
    }
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
    generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, String>("out"))
        .expect("the case called emit")
}

fn lines(expected: &[&str]) -> String {
    expected.join("\n")
}

#[test]
fn with_no_options_the_documented_focus_defaults_apply() {
    // Before: "with no options, the documented defaults apply".
    let got = run(r#"
        const o = focusSettings({});
        emit([o.mode, o.newWindows, o.raiseOnClick, o.autoRaise, o.autoRaiseDelay, o.changeOnPointerRest].join(","));
    "#);
    assert_eq!(got, "click,prevent,true,false,500,false");
}

#[test]
fn a_user_option_overrides_only_the_key_it_names() {
    // Before: "a user option overrides only the key it names" (hover is sloppy).
    let got = run(r#"
        const o = focusSettings({ mode: "sloppy" });
        emit([o.mode, o.raiseOnClick].join(","));
    "#);
    assert_eq!(got, "sloppy,true");
}

#[test]
fn a_bad_focus_option_throws_naming_the_key_instead_of_falling_back() {
    // Before: "an unknown focus_mode raises", "focus_new_windows accepts only allow or prevent", "auto_raise_delay must be
    // a non-negative number of milliseconds", "an unknown option name raises". The the retired shell `sloppy` is invalid there and
    // valid here, so the unknown mode is `bogus`.
    let got = run(r#"
        const bad = (u: any) => thrown(() => focusSettings(u));
        emit([
            bad({ mode: "bogus" }),
            bad({ newWindows: "always" }),
            bad({ autoRaiseDelay: -1 }),
            bad({ autoRaiseDelay: "soon" }),
            bad({ mod: "sloppy" }),
            bad({ raiseOnClick: "yes" }),
            bad(null),
            thrown(() => focusSettings({ mode: "bogus" }, "config.focus")),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "TypeError: focus.mode must be click, sloppy or follow-mouse, got bogus",
            "TypeError: focus.newWindows must be allow or prevent, got always",
            "TypeError: focus.autoRaiseDelay must be a non-negative number of milliseconds, got -1",
            "TypeError: focus.autoRaiseDelay must be a non-negative number of milliseconds, got soon",
            "TypeError: focus has no \"mod\" (did you mean \"mode\"?); expected one of mode, newWindows, raiseOnClick, autoRaise, autoRaiseDelay, changeOnPointerRest",
            "TypeError: focus.raiseOnClick must be a boolean, got yes",
            "TypeError: focus must be an object",
            "TypeError: config.focus.mode must be click, sloppy or follow-mouse, got bogus",
        ])
    );
}

#[test]
fn a_window_from_a_launcher_takes_focus_only_under_allow() {
    // Before: "prevent keeps a window started from a launcher behind; allow focuses it" and "a window not started from a
    // launcher takes focus under either policy".
    let got = run(r#"
        const prevent = focusSettings({});
        const allow = focusSettings({ newWindows: "allow" });
        emit([
            opensFocused(prevent, true),
            opensFocused(allow, true),
            opensFocused(prevent, false),
            opensFocused(allow, false),
        ].join(","));
    "#);
    assert_eq!(got, "false,true,true,true");
}

#[test]
fn the_focus_modes_decide_what_the_pointer_over_a_window_does() {
    // Before: "click mode never moves focus on hover", "hover mode focuses the window under the pointer, on entry or
    // movement", "hover-strict focuses only when the pointer enters the window" (hover is sloppy, hover-strict is
    // follow-mouse).
    let got = run(r#"
        const click = focusSettings({});
        const sloppy = focusSettings({ mode: "sloppy" });
        const follow = focusSettings({ mode: "follow-mouse" });
        emit([
            hoverFocus(click, true, true),
            hoverFocus(sloppy, true, true),
            hoverFocus(sloppy, true, false),
            hoverFocus(sloppy, false, false),
            hoverFocus(follow, true, true),
            hoverFocus(follow, true, false),
        ].join(","));
    "#);
    assert_eq!(got, "false,true,true,false,true,false");
}

#[test]
fn the_auto_raise_delay_reaches_the_decision_in_milliseconds_as_the_setting_gives_it() {
    // Before: "auto_raise_delay converts to seconds for the frame clock"; every time is milliseconds now, so there is
    // nothing to convert.
    let got = run(r#"
        const enter = { kind: "enter", held: false };
        const after = (user: any) => focusDecision(focusSettings({ mode: "sloppy", autoRaise: true, ...user }), enter).raiseAfter;
        emit([after({}), after({ autoRaiseDelay: 1500 })].join(","));
    "#);
    assert_eq!(got, "500,1500");
}

#[test]
fn a_pointer_rest_focuses_only_in_the_hover_modes_and_only_when_asked() {
    // Before: "a pointer rest focuses only in the hover modes, and only when focus_change_on_pointer_rest is on".
    let got = run(r#"
        emit([
            REST_DELAY,
            restFocus(focusSettings({ changeOnPointerRest: true })),
            restFocus(focusSettings({ mode: "sloppy", changeOnPointerRest: true })),
            restFocus(focusSettings({ mode: "follow-mouse", changeOnPointerRest: true })),
            restFocus(focusSettings({ mode: "sloppy" })),
        ].join(","));
    "#);
    assert_eq!(got, "200,false,true,true,false");
}

#[test]
fn only_follow_mouse_clears_focus_when_the_pointer_leaves_the_focused_window() {
    // Before: "only hover-strict clears focus when the pointer leaves the focused window".
    let got = run(r#"
        emit([
            leaveClears(focusSettings({ mode: "follow-mouse" })),
            leaveClears(focusSettings({ mode: "sloppy" })),
            leaveClears(focusSettings({})),
        ].join(","));
    "#);
    assert_eq!(got, "true,false,false");
}

#[test]
fn the_seats_words_for_the_focus_modes_translate_and_an_unknown_one_throws() {
    // New: the retired shell read the seat's words directly.
    let got = run(r#"
        const seat = (focusMode: string) => ({
            edgeTiling: false, focusMode, focusNewWindows: "allow", raiseOnClick: false, autoRaise: true,
            autoRaiseDelay: 200, focusChangeOnPointerRest: true,
        });
        const s = seatFocusSettings(seat("hover-strict"));
        emit([
            seatFocusSettings(seat("click")).mode,
            seatFocusSettings(seat("hover")).mode,
            s.mode,
            [s.newWindows, s.raiseOnClick, s.autoRaise, s.autoRaiseDelay, s.changeOnPointerRest].join("/"),
            thrown(() => seatFocusSettings(seat("sloppy"))),
            focusSettings({ mode: "click" }, "config.focus", s).mode + ":" + focusSettings({}, "x", s).mode,
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "click",
            "sloppy",
            "follow-mouse",
            "allow/false/true/200/true",
            "TypeError: the seat's focus_mode must be click, hover or hover-strict, got sloppy",
            "click:follow-mouse",
        ])
    );
}

#[test]
fn the_focus_decision_is_a_pure_function_of_the_settings_and_one_event() {
    // New: the the retired shell `Window:on_*` handlers composed the same decisions inline.
    let got = run(r#"
        const d = (settings: any, event: any) => {
            const r = focusDecision(focusSettings(settings), event);
            return [r.focus, r.clear, r.raise, r.focusAfter, r.raiseAfter].map(String).join("/");
        };
        const enter = { kind: "enter", held: false };
        emit([
            "added " + d({}, { kind: "added" }),
            "added from a launcher, prevent " + d({}, { kind: "added", fromLauncher: true }),
            "added from a launcher, allow " + d({ newWindows: "allow" }, { kind: "added", fromLauncher: true }),
            "press " + d({}, { kind: "press" }),
            "press, no raise " + d({ raiseOnClick: false }, { kind: "press" }),
            "click enter " + d({}, enter),
            "sloppy enter " + d({ mode: "sloppy" }, enter),
            "sloppy motion " + d({ mode: "sloppy" }, { kind: "motion", held: false }),
            "follow motion " + d({ mode: "follow-mouse" }, { kind: "motion", held: false }),
            "sloppy enter, button held " + d({ mode: "sloppy" }, { kind: "enter", held: true }),
            "sloppy enter, auto raise " + d({ mode: "sloppy", autoRaise: true }, enter),
            "sloppy motion, auto raise " + d({ mode: "sloppy", autoRaise: true }, { kind: "motion", held: false }),
            "sloppy enter, rest " + d({ mode: "sloppy", changeOnPointerRest: true }, enter),
            "follow leave " + d({ mode: "follow-mouse" }, { kind: "leave", focused: true, heldHere: false }),
            "follow leave, not focused " + d({ mode: "follow-mouse" }, { kind: "leave", focused: false, heldHere: false }),
            "follow leave, pressed here " + d({ mode: "follow-mouse" }, { kind: "leave", focused: true, heldHere: true }),
            "sloppy leave " + d({ mode: "sloppy" }, { kind: "leave", focused: true, heldHere: false }),
            thrown(() => focusDecision(focusSettings({}), { kind: "scroll" } as any)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "added true/false/false/null/null",
            "added from a launcher, prevent false/false/false/null/null",
            "added from a launcher, allow true/false/false/null/null",
            "press true/false/true/null/null",
            "press, no raise true/false/false/null/null",
            "click enter false/false/false/null/null",
            "sloppy enter true/false/false/null/null",
            "sloppy motion true/false/false/null/null",
            "follow motion false/false/false/null/null",
            "sloppy enter, button held false/false/false/null/null",
            "sloppy enter, auto raise true/false/false/null/500",
            "sloppy motion, auto raise true/false/false/null/null",
            "sloppy enter, rest false/false/false/200/null",
            "follow leave false/true/false/null/null",
            "follow leave, not focused false/false/false/null/null",
            "follow leave, pressed here false/false/false/null/null",
            "sloppy leave false/false/false/null/null",
            "TypeError: unknown focus event {\"kind\":\"scroll\"}",
        ])
    );
}

// -- edge snapping, pure ------------------------------------------------------------------------------------

#[test]
fn edge_tiling_is_off_unless_it_is_the_boolean_true() {
    // Before: "edge tiling is off by default", "a non-boolean edge_tiling raises".
    let got = run(r#"
        emit([
            edgeTilingEnabled(undefined),
            edgeTilingEnabled(true),
            edgeTilingEnabled(false),
            thrown(() => edgeTilingEnabled("yes")),
        ].join(","));
    "#);
    assert_eq!(
        got,
        "false,true,false,TypeError: edge_tiling must be a boolean, got yes"
    );
}

#[test]
fn a_pointer_near_an_edge_names_the_zone_and_a_zone_names_a_rectangle() {
    // Before: "a pointer within the margin of an edge names that edge", "a pointer away from every edge names no zone",
    // "left and right tile to the matching half, top fills the area", "an unknown zone has no rectangle", in the the retired shell
    // work area of 1920 by 1048 starting at y 32. The retired rectangle was x, y, w, h; this one is x, y, width, height.
    let got = run(r#"
        const area = { x: 0, y: 32, width: 1920, height: 1048 };
        const text = (r: any) => (r === null ? "none" : [r.x, r.y, r.width, r.height].join(","));
        emit([
            edgeZone(2, 500, area, 4),
            edgeZone(1917, 500, area, 4),
            edgeZone(900, 33, area, 4),
            edgeZone(960, 500, area, 4),
            edgeZone(1, 33, area, 4),
            edgeZone(1919, 33, area, 4),
            edgeZone(6, 500, area),
            edgeZone(6, 500, area, 6),
            text(tileRect("left", area)),
            text(tileRect("right", area)),
            text(tileRect("top", area)),
            text(tileRect("bottom", area)),
            text(tileRect("right", { x: 0, y: 0, width: 101, height: 50 })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "left",
            "right",
            "top",
            "",
            "left",
            "right",
            "",
            "left",
            "0,32,960,1048",
            "960,32,960,1048",
            "0,32,1920,1048",
            "none",
            "50,0,51,50",
        ])
    );
}

#[test]
fn the_resize_binding_takes_nearest_as_an_edges_option() {
    // New: the owner spec's `edges: "nearest"`.
    let got = run(r#"
        emit([
            new ClientResizeBinding({ edges: "nearest" }).edges,
            new ClientResizeBinding().edges,
            thrown(() => new ClientResizeBinding({ edges: "middle" } as any)),
            new ClientMoveBinding().snap,
            new ClientMoveBinding({ snap: 12 }).snap,
            new ClientMoveBinding({ snap: false }).snap,
            thrown(() => new ClientMoveBinding({ snap: -1 })),
            thrown(() => new ClientMoveBinding({ snap: true as any })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "nearest",
            "quadrant",
            "TypeError: ClientResizeBinding edges must be quadrant, nearest or one of top-left, top-right, bottom-left, bottom-right, got middle",
            "4",
            "12",
            "false",
            "TypeError: ClientMoveBinding snap must be false or a distance of 0 or more, got -1",
            "TypeError: ClientMoveBinding snap must be false or a distance of 0 or more, got true",
        ])
    );
}

// -- the compositor's script --------------------------------------------------------------------------------

/// The shared fake compositor of `tests/common`, with the words these cases use for what the compositor does while
/// the shell runs: a pointer's edge on a window, a new window, a property the compositor itself writes.
#[derive(Clone)]
struct Comp(FakeCompositor);

fn short(value: &MirrorValue) -> String {
    match value {
        MirrorValue::Bool(flag) => flag.to_string(),
        MirrorValue::Double(number) => number.to_string(),
        MirrorValue::Node(id) => id.to_string(),
        MirrorValue::Text(words) => words.clone(),
        other => format!("{other:?}"),
    }
}

impl Comp {
    /// What the shell has sent since the last call, one line each, oldest first: `call:3:focus`, `insert:3->1`,
    /// `set:3.secure=true`. A seat claim is no line: it is read through [`Comp::claim_id`].
    fn ops(&self) -> Vec<String> {
        self.0
            .take_ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Set { node, key, value, .. } => Some(format!("set:{node}.{key}={}", short(value))),
                TxOp::Insert { node, parent, .. } => Some(format!("insert:{node}->{parent}")),
                TxOp::Call { node, method, .. } if !matches!(method.as_str(), "claim" | "unclaim") => {
                    Some(format!("call:{node}:{method}"))
                }
                _ => None,
            })
            .collect()
    }

    /// The pointer's edge on a window: `enter`, `motion`, `leave`, `press` or `release`.
    fn edge(&self, node: u64, kind: &str) {
        let button = if matches!(kind, "press" | "release") { 272 } else { 0 };
        self.0.pointer(node, kind, 5.0, 6.0, button);
    }

    /// A window the compositor makes while the shell runs.
    fn new_window(&self, id: u64, app_id: &str, title: &str) {
        self.0.create_bare_window(id, title, app_id);
    }

    /// A property the compositor itself changes on a node.
    fn external(&self, node: u64, key: &str, value: MirrorValue) {
        self.0.external_set(node, key, value);
    }

    fn set_pointer(&self, seat: u64, x: f64, y: f64) {
        self.0.set_pointer(seat, x, y);
    }

    fn chord(&self, claim: i64, chord: &str, state: &str) {
        self.0.drag(claim, chord, state);
    }

    fn claim_id(&self, chord: &str) -> i64 {
        self.0.claim_id(chord)
    }

    fn flag(&self, node: u64, key: &str) -> bool {
        self.0.flag(node, key) == Some(true)
    }

    fn rect(&self, node: u64) -> String {
        self.0.rect(node)
    }
}

/// A root (1), a workspace (2), window 3 at 10,20 sized 300x200 with the focus, window 4 (a browser) with no size,
/// seat 5 with `seat` published on it, and a 1000 by 800 primary screen (7).
fn world(seat: Vec<(&str, MirrorValue)>) -> Comp {
    let c = FakeCompositor::new();
    c.add_node(
        2,
        ROOT,
        "workspace",
        vec![("index", MirrorValue::Uint(0)), ("active", MirrorValue::Bool(true))],
    );
    c.add_node(
        3,
        ROOT,
        "window",
        vec![
            ("title", text("term")),
            ("app_id", text("foot")),
            ("focused", MirrorValue::Bool(true)),
            ("x", MirrorValue::Double(10.0)),
            ("y", MirrorValue::Double(20.0)),
            ("width", MirrorValue::Double(300.0)),
            ("height", MirrorValue::Double(200.0)),
        ],
    );
    c.add_node(
        4,
        ROOT,
        "window",
        vec![("title", text("web")), ("app_id", text("firefox"))],
    );
    let mut props = vec![
        ("name", text("seat0")),
        ("pointer_x", MirrorValue::Double(0.0)),
        ("pointer_y", MirrorValue::Double(0.0)),
        ("focus", MirrorValue::Node(3)),
    ];
    props.extend(seat);
    c.add_node(5, ROOT, "seat", props);
    c.add_node(
        7,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    Comp(c)
}

struct Rig {
    shell: JsShell,
    clock: JsManualClock,
    _dir: TempDir,
}

impl Rig {
    /// Moves the shell's clock forward by `ms` milliseconds: a delay is over when the test says so, not when the
    /// machine got there.
    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    fn frame(&mut self) -> Vec<String> {
        let diagnostics = self.shell.frame().expect("a frame").diagnostics;
        // A rule's `cornerRadius` is kept in the client's style and noticed once as not drawn.
        assert!(
            diagnostics.iter().all(|d| d.starts_with("not applied: style.")),
            "{diagnostics:?}"
        );
        diagnostics
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }
}

const PRELUDE: &str = r#"import { gnoblin } from "gnoblin-core";
import { Workspace, Layout, FloatingLayout, ClientMoveBinding, ClientResizeBinding, snapClient, unsnapClient, snappedZone } from "gnoblin-std";
(globalThis as any).lib = { snapClient, unsnapClient, snappedZone };
"#;

/// Boots a shell over `comp` with `init` after the prelude. The windows that are there at boot join the first frame.
fn boot(comp: &Comp, init: &str) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), format!("{PRELUDE}{init}")).expect("init.ts is written");
    let (config, clock) = timed_config(&root);
    let mut shell = JsShell::new(Box::new(comp.0.clone()), config).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut rig = Rig {
        shell,
        clock,
        _dir: dir,
    };
    rig.frame();
    rig.frame();
    comp.ops();
    rig
}

/// One workspace whose layout places nothing, so a window stays wherever the compositor put it. The shell has no stock
/// workspaces here: an `init.ts` declares what it wants.
const BASE: &str = "gnoblin.workspaces.add(new Workspace({ id: \"main\", layout: new Layout() }));\n";

/// One floating workspace: a window can be dragged and snapped on it.
const FLOATING: &str = "gnoblin.workspaces.add(new Workspace({ id: \"main\", layout: new FloatingLayout() }));\n";

/// The `init.ts` of a case: the one workspace of [`BASE`], then `statements`.
fn config(statements: &str) -> String {
    format!("{BASE}{statements}")
}

fn flag(value: bool) -> MirrorValue {
    MirrorValue::Bool(value)
}

fn seat(mode: &str) -> Vec<(&'static str, MirrorValue)> {
    vec![("focus_mode", text(mode))]
}

// -- focus, through the shell -------------------------------------------------------------------------------

#[test]
fn a_new_window_takes_focus_and_a_window_from_before_the_config_does_not() {
    // Before: `Window:on_ready` focuses the new window. New: windows already there at boot are left alone, so a reload
    // does not take the user's focus.
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config(""));
    comp.new_window(8, "kitty", "shell");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=8"]);
    rig.frame();
    assert!(
        comp.ops().is_empty(),
        "nothing else: windows 3 and 4 were there at boot"
    );
}

#[test]
fn a_click_focuses_the_window_and_raises_it_when_raise_on_click_is_on() {
    // Before: `Window:on_press` (retired corpus shell covers the wiring there). Window 4 is already on top, so it is focused
    // and not raised; window 3 is raised by being put last under the root.
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "press");
    comp.edge(4, "release");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=4"], "the topmost window needs no raise");
    comp.edge(3, "press");
    comp.edge(3, "release");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=3", "insert:3->1"]);
}

#[test]
fn a_click_only_focuses_when_the_seat_says_raise_on_click_is_off() {
    let comp = world(vec![("raise_on_click", flag(false))]);
    let mut rig = boot(&comp, BASE);
    comp.edge(3, "press");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=3"], "the seat's raise_on_click is false");
    comp.edge(3, "release");
    rig.frame();
}

#[test]
fn click_mode_ignores_the_pointer_over_a_window() {
    // Before: "click mode never moves focus on hover".
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "enter");
    comp.edge(4, "motion");
    rig.frame();
    assert!(comp.ops().is_empty());
}

#[test]
fn sloppy_focus_follows_the_pointer_on_entry_and_on_movement_and_keeps_focus_on_leave() {
    // Before: "hover mode focuses the window under the pointer, on entry or movement" and Window:on_leave for hover.
    let comp = world(seat("hover"));
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "enter");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=4"]);
    comp.edge(4, "motion");
    rig.frame();
    assert_eq!(
        comp.ops(),
        ["set:5.focus=4"],
        "movement inside the window focuses again"
    );
    comp.edge(3, "leave");
    rig.frame();
    assert!(comp.ops().is_empty(), "sloppy focus keeps the focus where it was");
}

#[test]
fn follow_mouse_focuses_on_entry_only_and_clears_focus_when_the_pointer_leaves_the_focused_window() {
    // Before: "hover-strict focuses only when the pointer enters the window" and "only hover-strict clears focus when
    // the pointer leaves the focused window".
    let comp = world(seat("hover-strict"));
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "enter");
    comp.edge(4, "motion");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=4"], "the motion does not focus again");
    comp.edge(3, "leave");
    rig.frame();
    assert!(
        comp.ops().is_empty(),
        "window 3 lost the focus to window 4 and is not the focused one"
    );
    comp.edge(4, "leave");
    rig.frame();
    assert_eq!(
        comp.ops(),
        ["set:5.focus=0"],
        "window 4 has the focus and the pointer left it"
    );
}

#[test]
fn a_window_pressed_holds_hover_focus_until_the_button_is_released() {
    // Before: `Window:on_enter` returns while `pressed` is set.
    let comp = world(seat("hover"));
    let mut rig = boot(&comp, &config(""));
    comp.edge(3, "press");
    rig.frame();
    comp.ops();
    comp.edge(4, "enter");
    rig.frame();
    assert!(comp.ops().is_empty(), "a button is held");
    comp.edge(3, "release");
    comp.edge(4, "leave");
    comp.edge(4, "enter");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=4"]);
}

#[test]
fn auto_raise_lifts_the_hovered_focused_window_after_the_delay_unless_the_pointer_left() {
    // Before: `Window:on_enter` starts the auto-raise timer and `Window:on_leave` cancels it. Window 3 has the focus.
    let comp = world(vec![
        ("focus_mode", text("hover")),
        ("auto_raise", flag(true)),
        ("auto_raise_delay", MirrorValue::Int(40)),
    ]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(3, "enter");
    rig.frame();
    assert_eq!(
        comp.ops(),
        ["set:5.focus=3"],
        "entering focuses at once; the raise waits"
    );
    rig.advance(80);
    rig.frame();
    assert_eq!(
        comp.ops(),
        ["insert:3->1"],
        "after the delay the focused window is raised"
    );
    rig.frame();
    assert!(comp.ops().is_empty(), "once");
    comp.edge(3, "enter");
    comp.edge(3, "leave");
    rig.frame();
    comp.ops();
    rig.advance(80);
    rig.frame();
    assert!(comp.ops().is_empty(), "a leave cancels the pending raise");
}

#[test]
fn auto_raise_is_off_by_default_and_a_window_that_lost_focus_is_not_raised() {
    let comp = world(vec![("focus_mode", text("hover"))]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(3, "enter");
    rig.frame();
    comp.ops();
    rig.advance(30);
    rig.frame();
    assert!(comp.ops().is_empty(), "auto_raise is false");
    let comp = world(vec![
        ("focus_mode", text("hover")),
        ("auto_raise", flag(true)),
        ("auto_raise_delay", MirrorValue::Int(40)),
    ]);
    let mut rig = boot(&comp, BASE);
    comp.edge(3, "enter");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=3"]);
    comp.external(5, "focus", MirrorValue::Node(4));
    rig.advance(80);
    rig.frame();
    assert!(
        comp.ops().is_empty(),
        "the compositor gave the focus to window 4 before the delay ran out, so window 3 is not raised"
    );
}

#[test]
fn a_pointer_rest_focuses_after_the_rest_time_and_a_leave_cancels_it() {
    // Before: `rest_focus_later`; the rest is 0.2 s.
    let comp = world(vec![
        ("focus_mode", text("hover")),
        ("focus_change_on_pointer_rest", flag(true)),
    ]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "enter");
    rig.frame();
    assert!(comp.ops().is_empty(), "a rest is not an entry");
    rig.advance(260);
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=4"]);
    comp.edge(4, "enter");
    comp.edge(4, "leave");
    rig.frame();
    rig.advance(260);
    rig.frame();
    assert!(comp.ops().is_empty(), "the pointer left before it rested");
}

// -- edge snapping, through the shell -----------------------------------------------------------------------

const SNAP_AREA: &str = "{ x: 0, y: 0, width: 800, height: 600 }";

#[test]
fn snapping_tiles_a_window_to_an_edge_and_unsnapping_restores_its_floating_rectangle() {
    // Before: "snapping tiles a window to an edge and unsnapping restores its floating rectangle".
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config(""));
    let floating = comp.rect(3);
    assert_eq!(
        rig.eval(&format!(
            "String(lib.snapClient(gnoblin.clients.get('3'), 'left', {{ edgeTiling: true, area: {SNAP_AREA} }}))"
        )),
        "true",
        "a snap with edge tiling on is taken"
    );
    rig.frame();
    assert_eq!(comp.rect(3), "0,0,400,600", "left tiles to the left half");
    assert_eq!(rig.eval("lib.snappedZone(gnoblin.clients.get('3'))"), "left");
    assert_eq!(rig.eval("String(lib.unsnapClient(gnoblin.clients.get('3')))"), "true");
    rig.frame();
    assert_eq!(comp.rect(3), floating, "unsnap restores the floating rectangle");
    assert_eq!(
        rig.eval("String(lib.unsnapClient(gnoblin.clients.get('3')))"),
        "false",
        "a second unsnap does nothing"
    );
    assert_eq!(rig.eval("String(lib.snappedZone(gnoblin.clients.get('3')))"), "null");
}

#[test]
fn snapping_a_maximised_window_floats_it_first_and_unsnapping_restores_the_floating_rectangle() {
    // Before: "snapping a maximised window floats it first, and unsnapping restores the floating rectangle".
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = boot(&comp, &config(""));
    rig.eval(
        "const c = gnoblin.clients.get('3'); c.floatingBounds = { x: 10, y: 20, width: 300, height: 200 };\n\
         c.maximised = true; c.bounds = { x: 0, y: 0, width: 800, height: 600 }; undefined",
    );
    rig.frame();
    assert!(comp.flag(3, "maximised"));
    rig.eval(&format!(
        "lib.snapClient(gnoblin.clients.get('3'), 'right', {{ edgeTiling: true, area: {SNAP_AREA} }})"
    ));
    rig.frame();
    assert!(!comp.flag(3, "maximised"), "snapping must float a maximised window");
    assert_eq!(comp.rect(3), "400,0,400,600");
    rig.eval("lib.unsnapClient(gnoblin.clients.get('3'))");
    rig.frame();
    assert_eq!(
        comp.rect(3),
        "10,20,300,200",
        "unsnap restores the rectangle the window had before it was maximised"
    );
}

#[test]
fn snapping_is_refused_when_edge_tiling_is_off_and_leaves_the_window_where_it_was() {
    // Before: "snapping is refused when edge tiling is off and leaves the window where it was".
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config(""));
    let before = comp.rect(3);
    assert_eq!(
        rig.eval(&format!(
            "String(lib.snapClient(gnoblin.clients.get('3'), 'left', {{ edgeTiling: false, area: {SNAP_AREA} }}))"
        )),
        "false"
    );
    assert_eq!(rig.eval("String(lib.snapClient(gnoblin.clients.get('3'), 'bottom', { edgeTiling: true, area: { x: 0, y: 0, width: 8, height: 6 } }))"), "false", "a zone that does not tile");
    rig.frame();
    assert_eq!(rig.eval("String(lib.snappedZone(gnoblin.clients.get('3')))"), "null");
    assert_eq!(comp.rect(3), before, "a refused snap must not move the window");
}

/// Super+left button drags window 3 from (50, 60) to `to` and releases there.
fn drag_to(comp: &Comp, rig: &mut Rig, to: (f64, f64)) {
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, to.0, to.1);
    rig.frame();
    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
}

#[test]
fn dropping_a_dragged_window_on_an_edge_tiles_it_when_the_seat_has_edge_tiling_on() {
    // Before: `drag` ends with `Snap.edge_zone(s.x, s.y, work_area(), EDGE_MARGIN)` and `w:snap(zone)`; a 1000 by 800
    // work area, the default margin of 4.
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = boot(&comp, FLOATING);
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(comp.rect(3), "0,0,500,800", "the left edge tiles to the left half");
    assert_eq!(rig.eval("lib.snappedZone(gnoblin.clients.get('3'))"), "left");
    drag_to(&comp, &mut rig, (400.0, 3.0));
    assert_eq!(comp.rect(3), "0,0,1000,800", "the top edge fills the work area");
    assert_eq!(rig.eval("lib.snappedZone(gnoblin.clients.get('3'))"), "top");
    drag_to(&comp, &mut rig, (999.0, 300.0));
    assert_eq!(comp.rect(3), "500,0,500,800", "the right edge tiles to the right half");
}

#[test]
fn a_drop_on_an_edge_is_only_a_move_when_edge_tiling_is_off() {
    // Before: "edge tiling is off by default".
    let comp = world(vec![]);
    let mut rig = boot(&comp, FLOATING);
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(
        comp.rect(3),
        "0,260,300,200",
        "moved by the pointer's delta and kept in the work area"
    );
    assert_eq!(rig.eval("String(lib.snappedZone(gnoblin.clients.get('3')))"), "null");
}

/// A floating workspace whose only binding is `binding`, so a case chooses the move binding's options.
fn rig_with_move(comp: &Comp, binding: &str) -> Rig {
    boot(
        comp,
        &format!(
            "const layout = new FloatingLayout({{ triggers: {{ move: null, resize: null }} }});\n\
             layout.bindings.add({binding});\n\
             gnoblin.workspaces.add(new Workspace({{ id: \"main\", layout }}));\n"
        ),
    )
}

#[test]
fn the_move_bindings_snap_option_sets_the_margin_or_turns_snapping_off() {
    // New: `ClientMoveBinding({ snap })`.
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "new ClientMoveBinding({ id: \"move\" })");
    drag_to(&comp, &mut rig, (10.0, 300.0));
    assert_eq!(
        comp.rect(3),
        "0,260,300,200",
        "10 pixels from the edge is outside the default margin of 4"
    );
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "new ClientMoveBinding({ id: \"move\", snap: 12 })");
    drag_to(&comp, &mut rig, (10.0, 300.0));
    assert_eq!(comp.rect(3), "0,0,500,800", "10 pixels is inside a margin of 12");
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "new ClientMoveBinding({ id: \"move\", snap: false })");
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(comp.rect(3), "0,260,300,200", "snap: false never tiles");
}

#[test]
fn a_snapped_window_the_user_drags_away_is_no_longer_snapped() {
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = boot(&comp, FLOATING);
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(rig.eval("lib.snappedZone(gnoblin.clients.get('3'))"), "left");
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 100.0, 100.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 400.0, 400.0);
    rig.frame();
    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
    assert_eq!(rig.eval("String(lib.snappedZone(gnoblin.clients.get('3')))"), "null");
    assert_eq!(
        comp.rect(3),
        "300,0,500,800",
        "moved by the pointer's delta, and not tiled again"
    );
}

fn rig_with_resize(comp: &Comp, edges: &str) -> Rig {
    boot(
        comp,
        &format!(
            "const layout = new FloatingLayout({{ triggers: {{ move: null, resize: null }} }});\n\
             layout.bindings.add(new ClientResizeBinding({{ id: \"resize\", edges: \"{edges}\" }}));\n\
             gnoblin.workspaces.add(new Workspace({{ id: \"main\", layout }}));\n"
        ),
    )
}

/// Super+right button drags from `from` to `to` and releases.
fn resize_from(comp: &Comp, rig: &mut Rig, from: (f64, f64), to: (f64, f64)) {
    let id = comp.claim_id("<Super>Button3");
    comp.set_pointer(5, from.0, from.1);
    comp.chord(id, "<Super>Button3", "press");
    rig.frame();
    comp.set_pointer(5, to.0, to.1);
    rig.frame();
    comp.chord(id, "<Super>Button3", "release");
    rig.frame();
}

#[test]
fn a_nearest_resize_moves_the_one_side_the_press_is_nearest_to() {
    // New: `edges: "nearest"`. Window 3 is 300 by 200 at 10,20.
    let comp = world(vec![]);
    let mut rig = rig_with_resize(&comp, "nearest");
    resize_from(&comp, &mut rig, (305.0, 100.0), (355.0, 130.0));
    assert_eq!(
        comp.rect(3),
        "10,20,350,200",
        "the right side moved with the pointer's x and nothing else"
    );
    resize_from(&comp, &mut rig, (200.0, 215.0), (220.0, 255.0));
    assert_eq!(
        comp.rect(3),
        "10,20,350,240",
        "the bottom side moved with the pointer's y"
    );
    resize_from(&comp, &mut rig, (13.0, 100.0), (33.0, 100.0));
    assert_eq!(
        comp.rect(3),
        "30,20,330,240",
        "the left side moved and the right stayed"
    );
    resize_from(&comp, &mut rig, (200.0, 22.0), (200.0, 42.0));
    assert_eq!(
        comp.rect(3),
        "30,40,330,220",
        "the top side moved and the bottom stayed"
    );
}

#[test]
fn a_quadrant_resize_still_moves_the_nearest_corner() {
    let comp = world(vec![]);
    let mut rig = rig_with_resize(&comp, "quadrant");
    resize_from(&comp, &mut rig, (305.0, 215.0), (325.0, 235.0));
    assert_eq!(comp.rect(3), "10,20,320,220");
}
