//! Focus, window rules and edge snapping of the stock library: `data/js/gnoblin/{focus,rules,snap}.ts` and the
//! pieces of `gnoblin.ts` and `bindings.ts` they stand on.
//!
//! The cases are the Lua shell's: `crates/gnoblin-lua/tests/lua/{focus,rules,snap}_test.lua`. A Lua case keeps its
//! inputs and its expected values; where the JavaScript API changed a word (`hover` is `sloppy`, a Lua pattern is a
//! glob, `last matching rule wins` is `first wins unless all`), the case says so. A case with no Lua original says
//! "new". The pure functions run in a bare generation (no host); everything that touches a window runs a real
//! `JsShell` over `FakeLink`, an in-memory compositor that records the calls and writes the shell sends and plays back
//! pointer edges, new windows and drags. Nothing here proves what the real compositor does with those calls: that is
//! `crates/gnoblin/tests/js_shell.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::thread::sleep;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, JsShell, JsShellConfig, stock_modules};
use gnoblin_tree_client::tree_mirror::{
    Attached, EventFields, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeEvent,
    TreeIncoming, TreeLink, TreePointerChord, TreeTx, TxOp, TxReply,
};
use tempfile::TempDir;

// -- the pure functions, run in a bare generation -----------------------------------------------------------

const HARNESS: &str = r##"
globalThis.gnoblin = { __register() {} };
export const emit = (value) => { globalThis.out = String(value); };
export const thrown = (fn) => {
    try { fn(); } catch (e) { return `${e.name}: ${e.message}`; }
    return "did not throw";
};
"##;

const IMPORTS: &str = r#"
import { emit, thrown } from "harness";
import {
    focusSettings, seatFocusSettings, opensFocused, hoverFocus, autoRaiseSeconds, REST_SECONDS, restFocus,
    leaveClears, focusDecision, compileRules, effectsFor, matchingRules, edgeTilingEnabled, edgeZone, tileRect,
    desugarConfig, ClientResizeBinding, ClientMoveBinding,
} from "gnoblin";
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
    // Lua: "with no options, the documented defaults apply".
    let got = run(r#"
        const o = focusSettings({});
        emit([o.mode, o.newWindows, o.raiseOnClick, o.autoRaise, o.autoRaiseDelay, o.changeOnPointerRest].join(","));
    "#);
    assert_eq!(got, "click,prevent,true,false,500,false");
}

#[test]
fn a_user_option_overrides_only_the_key_it_names() {
    // Lua: "a user option overrides only the key it names" (hover is sloppy).
    let got = run(r#"
        const o = focusSettings({ mode: "sloppy" });
        emit([o.mode, o.raiseOnClick].join(","));
    "#);
    assert_eq!(got, "sloppy,true");
}

#[test]
fn a_bad_focus_option_throws_naming_the_key_instead_of_falling_back() {
    // Lua: "an unknown focus_mode raises", "focus_new_windows accepts only allow or prevent", "auto_raise_delay must be
    // a non-negative number of milliseconds", "an unknown option name raises". The Lua `sloppy` is invalid there and
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
    // Lua: "prevent keeps a window started from a launcher behind; allow focuses it" and "a window not started from a
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
    // Lua: "click mode never moves focus on hover", "hover mode focuses the window under the pointer, on entry or
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
fn the_auto_raise_delay_is_in_seconds_for_the_frame_clock() {
    // Lua: "auto_raise_delay converts to seconds for the frame clock".
    let got = run(r#"
        emit([autoRaiseSeconds(focusSettings({})), autoRaiseSeconds(focusSettings({ autoRaiseDelay: 1500 }))].join(","));
    "#);
    assert_eq!(got, "0.5,1.5");
}

#[test]
fn a_pointer_rest_focuses_only_in_the_hover_modes_and_only_when_asked() {
    // Lua: "a pointer rest focuses only in the hover modes, and only when focus_change_on_pointer_rest is on".
    let got = run(r#"
        emit([
            REST_SECONDS,
            restFocus(focusSettings({ changeOnPointerRest: true })),
            restFocus(focusSettings({ mode: "sloppy", changeOnPointerRest: true })),
            restFocus(focusSettings({ mode: "follow-mouse", changeOnPointerRest: true })),
            restFocus(focusSettings({ mode: "sloppy" })),
        ].join(","));
    "#);
    assert_eq!(got, "0.2,false,true,true,false");
}

#[test]
fn only_follow_mouse_clears_focus_when_the_pointer_leaves_the_focused_window() {
    // Lua: "only hover-strict clears focus when the pointer leaves the focused window".
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
    // New: the Lua shell read the seat's words directly.
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
    // New: the Lua `Window:on_*` handlers composed the same decisions inline.
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
            "added, skipFocus " + d({}, { kind: "added", skipFocus: true }),
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
            "added, skipFocus false/false/false/null/null",
            "press true/false/true/null/null",
            "press, no raise true/false/false/null/null",
            "click enter false/false/false/null/null",
            "sloppy enter true/false/false/null/null",
            "sloppy motion true/false/false/null/null",
            "follow motion false/false/false/null/null",
            "sloppy enter, button held false/false/false/null/null",
            "sloppy enter, auto raise true/false/false/null/0.5",
            "sloppy motion, auto raise true/false/false/null/null",
            "sloppy enter, rest false/false/false/0.2/null",
            "follow leave false/true/false/null/null",
            "follow leave, not focused false/false/false/null/null",
            "follow leave, pressed here false/false/false/null/null",
            "sloppy leave false/false/false/null/null",
            "TypeError: unknown focus event {\"kind\":\"scroll\"}",
        ])
    );
}

// -- window rules, pure -------------------------------------------------------------------------------------

const VIEW: &str = r#"
        const effects = (list: any[], view: any = {}) =>
            effectsFor(compileRules(list), { appId: "", title: "", focused: false, workspace: null, ...view });
"#;

#[test]
fn a_glob_matches_the_whole_text_and_a_regexp_anywhere_in_it() {
    // Lua: "a pattern matches anywhere in the app_id unless anchored". A Lua pattern `fire` is the glob `*fire*`; the
    // Lua `^fire$` is the plain text `fire`, which matches the whole id only.
    let got = run(&format!(
        r#"{VIEW}
        const fs = (pattern: any, appId: string) =>
            String(effects([{{ appId: pattern, fullscreen: true }}], {{ appId }}).fullscreen);
        const g = /fire/g;
        emit([
            fs("*fire*", "org.mozilla.firefox"),
            fs("fire", "firefox"),
            fs("fire", "fire"),
            fs("fire*", "firefox"),
            fs("fire*", "xfire"),
            fs("org.?ozilla.*", "org.mozilla.firefox"),
            fs("org.?ozilla.*", "orgxmozilla.firefox"),
            fs("a+b", "a+b"),
            fs("a+b", "aab"),
            fs(/^org\.mozilla\./, "org.mozilla.firefox"),
            fs(/fire/, "org.mozilla.firefox"),
            fs(/^fire$/, "firefox"),
            fs(g, "fire") + fs(g, "fire"),
            String(effects([{{ title: "a*b", fullscreen: true }}], {{ title: "a\nb" }}).fullscreen),
        ].join(","));
    "#
    ));
    assert_eq!(
        got,
        "true,undefined,true,true,undefined,true,undefined,true,undefined,true,true,undefined,truetrue,true"
    );
}

#[test]
fn a_malformed_rule_throws_at_compile_time_naming_its_index_and_field() {
    // Lua: "a malformed pattern is a compile error", "an unknown rule field raises", "a rule's opacity must be a
    // number in [0, 1]", "corners is refused at load", "maximised is a boolean rule field". New: the sets-nothing,
    // needs-a-query and role refusals, which the Lua rule could not express.
    let got = run(r#"
        const bad = (list: any) => thrown(() => compileRules(list));
        const ok = { appId: "x", secure: true };
        emit([
            bad([{ appId: 5, fullscreen: true }]),
            bad([{ title: { not: "a pattern" }, secure: true }]),
            bad([{ title: "x", fullscren: true }]),
            bad([ok, { appId: "x", style: { opacity: 1.5 } }]),
            bad([{ appId: "x", style: { opacity: "half" } }]),
            bad([{ appId: "x", style: { shine: 1 } }]),
            bad([{ appId: "x", corners: { radius: 8 } }]),
            bad([{ appId: "x", maximised: "yes" }]),
            bad([{ appId: "x", role: "dialog", secure: true }]),
            bad([{ appId: "x" }]),
            bad([{ secure: true }]),
            bad([{ appId: "x", size: { width: 0, height: 5 } }]),
            bad([{ appId: "x", position: { x: 1 } }]),
            bad([{ appId: "x", workspace: "" }]),
            bad(5),
            bad([null]),
            bad([ok, { appId: "y", secure: true, all: 1 }]),
            bad([ok, { appId: "y", style: { opacity: 0.5 }, maximised: true }]),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "TypeError: rules[0].appId: must be a glob string or a RegExp, got 5",
            "TypeError: rules[0].title: must be a glob string or a RegExp, got [object Object]",
            "TypeError: rules[0] has no \"fullscren\" (did you mean \"fullscreen\"?); expected one of appId, title, role, focused, onWorkspace, workspace, floating, size, position, style, fullscreen, maximised, secure, skipFocus, all, reapply",
            "TypeError: rules[1].style.opacity: must be a number from 0 to 1, got 1.5",
            "TypeError: rules[0].style.opacity: must be a number from 0 to 1, got \"half\"",
            "TypeError: rules[0].style has no \"shine\"; expected one of cornerRadius, opacity, shadows",
            "TypeError: rules[0] has no \"corners\"; expected one of appId, title, role, focused, onWorkspace, workspace, floating, size, position, style, fullscreen, maximised, secure, skipFocus, all, reapply",
            "TypeError: rules[0].maximised: must be true or false, got \"yes\"",
            "TypeError: rules[0].role: the compositor publishes no window role, so a rule cannot match one; use appId or title",
            "TypeError: rules[0]: sets nothing; give it one of workspace, floating, size, position, style, fullscreen, maximised, secure, skipFocus",
            "TypeError: rules[0]: needs a query: appId, title, focused or onWorkspace",
            "TypeError: rules[0].size.width: must be a positive finite number, got 0",
            "TypeError: rules[0].position.y: must be a finite number, got undefined",
            "TypeError: rules[0].workspace: must be a workspace id, got \"\"",
            "TypeError: rules must be an array",
            "TypeError: rules[0]: must be an object",
            "TypeError: rules[1].all: must be true or false, got 1",
            "did not throw",
        ])
    );
}

#[test]
fn the_first_matching_rule_wins_unless_a_rule_says_all() {
    // Lua: "the last matching rule wins for a field", "a later rule that does not set a field leaves the earlier
    // value", "rules merge in list order". The Lua list merged every matching rule; the JavaScript one stops at the
    // first match unless the rule has `all: true`, so a Lua list translates by adding `all: true` to each rule.
    let got = run(&format!(
        r#"{VIEW}
        const show = (e: any) => [e.fullscreen, e.secure, e.maximised].map(String).join("/");
        const foo = {{ appId: "foo" }};
        emit([
            show(effects([{{ ...foo, fullscreen: true }}, {{ ...foo, fullscreen: false, secure: true }}], foo)),
            show(effects([{{ ...foo, fullscreen: true, all: true }}, {{ ...foo, fullscreen: false, secure: true }}], foo)),
            show(effects([{{ ...foo, secure: true, all: true }}, {{ ...foo, fullscreen: true }}], foo)),
            show(effects([
                {{ ...foo, fullscreen: true, all: true }},
                {{ ...foo, secure: true, all: true }},
                {{ ...foo, fullscreen: false, all: true }},
            ], foo)),
            show(effects([{{ appId: "bar", secure: true }}, {{ ...foo, maximised: true }}], foo)),
            show(effects([{{ ...foo, secure: true }}], {{ appId: "bar" }})),
            matchingRules(compileRules([{{ ...foo, secure: true, all: true }}, {{ ...foo, fullscreen: true }}, {{ ...foo, maximised: true }}]), {{ appId: "foo", title: "", focused: false, workspace: null }}).map((r) => r.index).join(","),
        ].join("\n"));
    "#
    ));
    assert_eq!(
        got,
        lines(&[
            "true/undefined/undefined",
            "false/true/undefined",
            "true/true/undefined",
            "false/true/undefined",
            "undefined/undefined/true",
            "undefined/undefined/undefined",
            "0,1",
        ])
    );
}

#[test]
fn a_query_can_read_the_focus_and_the_workspace_of_the_window() {
    // Lua: "the focused condition follows the window's focus", "a workspace rule carries its workspace id". The Lua
    // `match.workspace_id` is `onWorkspace`, because `workspace` is where the rule sends the window.
    let got = run(&format!(
        r#"{VIEW}
        const focused = [{{ focused: true, secure: true }}];
        const sent = [{{ appId: "term", workspace: "2" }}];
        const here = [{{ appId: "x", onWorkspace: "2", secure: true }}];
        emit([
            effects(focused, {{ focused: true }}).secure,
            effects(focused, {{ focused: false }}).secure,
            effects(sent, {{ appId: "term" }}).workspace,
            effects(here, {{ appId: "x", workspace: "2" }}).secure,
            effects(here, {{ appId: "x", workspace: "1" }}).secure,
            effects(here, {{ appId: "x", workspace: null }}).secure,
            effects([{{ appId: "x", size: {{ width: 3, height: 4 }}, position: {{ x: 5, y: 6 }}, style: {{ opacity: 0.5, cornerRadius: 9 }} }}], {{ appId: "x" }}).opacity,
        ].map(String).join(","));
    "#
    ));
    assert_eq!(got, "true,undefined,2,true,undefined,undefined,0.5");
}

#[test]
fn desugaring_a_config_checks_the_rules_the_focus_keys_and_the_seat_overrides() {
    // New: the Lua shell read these from the seat and compiled the rules on first use.
    let got = run(r#"
        const bad = (config: any) => thrown(() => desugarConfig(config));
        const plan = desugarConfig({
            rules: [{ appId: /^org\./, workspace: "dev", reapply: true }],
            workspaces: [{ id: "dev" }],
            focus: { mode: "sloppy", autoRaise: undefined },
            edgeTiling: true,
            dragConstraint: "minimum_strip",
        });
        const bare = desugarConfig({});
        emit([
            plan.rules.length,
            String(plan.rules[0].appId),
            JSON.stringify(plan.focus),
            plan.edgeTiling,
            plan.dragConstraint,
            JSON.stringify(bare.focus) + " " + bare.edgeTiling + " " + bare.dragConstraint,
            bad({ workspaces: [{ id: "a" }], rules: [{ appId: "x", workspace: "b" }] }),
            bad({ focus: { mode: "bogus" } }),
            bad({ focus: { raiseOnClick: 1 } }),
            bad({ edgeTiling: "yes" }),
            bad({ dragConstraint: "loose" }),
            bad({ rules: [{ appId: "x", style: { opacity: 2 } }] }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "1",
            "/^org\\./",
            "{\"mode\":\"sloppy\"}",
            "true",
            "minimum_strip",
            "{} null null",
            "Error: rules[0].workspace: moves to workspace \"b\", which the config does not declare",
            "TypeError: config.focus.mode must be click, sloppy or follow-mouse, got bogus",
            "TypeError: config.focus.raiseOnClick must be a boolean, got 1",
            "TypeError: config.edgeTiling: must be true or false",
            "TypeError: config.dragConstraint must be work_area or minimum_strip, got loose",
            "TypeError: rules[0].style.opacity: must be a number from 0 to 1, got 2",
        ])
    );
}

// -- edge snapping, pure ------------------------------------------------------------------------------------

#[test]
fn edge_tiling_is_off_unless_it_is_the_boolean_true() {
    // Lua: "edge tiling is off by default", "a non-boolean edge_tiling raises".
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
    // Lua: "a pointer within the margin of an edge names that edge", "a pointer away from every edge names no zone",
    // "left and right tile to the matching half, top fills the area", "an unknown zone has no rectangle", in the Lua
    // work area of 1920 by 1048 starting at y 32. The Lua rectangle was x, y, w, h; this one is x, y, width, height.
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

// -- the fake compositor ------------------------------------------------------------------------------------

struct Node {
    class: String,
    parent: u64,
    props: BTreeMap<String, MirrorValue>,
}

#[derive(Default)]
struct Compositor {
    nodes: BTreeMap<u64, Node>,
    serial: u64,
    queue: Vec<TreeIncoming>,
    /// What the shell sent, one line each, in order: `call:3:focus`, `insert:3->1`, `set:3.secure=true`.
    ops: Vec<String>,
    /// The seat claims the shell holds: id to the chord.
    claims: BTreeMap<i64, String>,
    claim_seats: BTreeMap<i64, u64>,
    /// The pointer chord drags in flight, by claim id: the seat and where the drag began.
    drags: BTreeMap<i64, (u64, (f64, f64), i64, String)>,
}

#[derive(Clone)]
struct FakeLink(Rc<RefCell<Compositor>>);

type Comp = Rc<RefCell<Compositor>>;

fn change(kind: MirrorChangeKind, node: u64) -> MirrorChange {
    MirrorChange {
        kind,
        node,
        class: String::new(),
        parent: 0,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

fn set_change(node: u64, key: &str, value: MirrorValue) -> MirrorChange {
    MirrorChange {
        key: key.into(),
        value: Some(value),
        ..change(MirrorChangeKind::Set, node)
    }
}

fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.into())
}

fn le(bytes: &[u8]) -> i64 {
    i64::from_le_bytes(bytes.try_into().expect("eight bytes"))
}

fn short(value: &MirrorValue) -> String {
    match value {
        MirrorValue::Bool(flag) => flag.to_string(),
        MirrorValue::Double(number) => number.to_string(),
        MirrorValue::Node(id) => id.to_string(),
        MirrorValue::Text(words) => words.clone(),
        other => format!("{other:?}"),
    }
}

impl FakeLink {
    /// Applies a transaction: writes go to the node map and the log, seat claims to the claim table.
    fn land(&self, tx: &TreeTx) {
        let mut c = self.0.borrow_mut();
        let mut changes = Vec::new();
        for op in &tx.ops {
            match op {
                TxOp::Set { node, key, value, .. } => {
                    c.ops.push(format!("set:{node}.{key}={}", short(value)));
                    if let Some(entry) = c.nodes.get_mut(node) {
                        entry.props.insert(key.clone(), value.clone());
                    }
                    changes.push(set_change(*node, key, value.clone()));
                }
                TxOp::Insert { node, parent, .. } => {
                    c.ops.push(format!("insert:{node}->{parent}"));
                    if let Some(entry) = c.nodes.get_mut(node) {
                        entry.parent = *parent;
                    }
                    changes.push(MirrorChange {
                        parent: *parent,
                        index: 99,
                        ..change(MirrorChangeKind::Parented, *node)
                    });
                }
                TxOp::Call {
                    node,
                    method,
                    args,
                    text,
                    ..
                } => match method.as_str() {
                    "claim" => {
                        let id = le(&args[0..8]);
                        c.claim_seats.insert(id, *node);
                        c.claims.insert(id, text.expose().to_owned());
                    }
                    "unclaim" => {
                        let id = le(&args[0..8]);
                        c.claims.remove(&id);
                        c.claim_seats.remove(&id);
                        c.drags.remove(&id);
                    }
                    other => c.ops.push(format!("call:{node}:{other}")),
                },
                _ => {}
            }
        }
        c.serial = tx.serial;
        c.queue.push(TreeIncoming::Changes {
            serial: tx.serial,
            changes,
        });
        c.queue.push(TreeIncoming::Reply(TxReply {
            serial: tx.serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
    }

    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }
}

impl TreeLink for FakeLink {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let mut c = self.0.borrow_mut();
        let attached = Attached {
            epoch: 1,
            root: 1,
            serial: c.serial,
        };
        let nodes = c
            .nodes
            .iter()
            .map(|(id, node)| SnapshotNode {
                id: *id,
                parent: node.parent,
                index: 0,
                class: node.class.clone(),
                title: String::new(),
                app_id: String::new(),
                props: node.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            })
            .collect();
        let snapshot = TreeIncoming::Snapshot {
            serial: c.serial,
            chunk: 0,
            last: true,
            nodes,
        };
        c.queue.push(TreeIncoming::Attached(attached));
        c.queue.push(snapshot);
        Ok(())
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        self.land(tx);
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

/// The compositor's side of a scripted session.
trait Script {
    /// What the shell has sent since the last call, oldest first.
    fn ops(&self) -> Vec<String>;
    /// The pointer's edge on a window: `enter`, `motion`, `leave`, `press` or `release`.
    fn edge(&self, node: u64, kind: &str);
    /// A window the compositor makes while the shell runs.
    fn new_window(&self, id: u64, app_id: &str, title: &str);
    /// A property the compositor itself changes on a node.
    fn external(&self, node: u64, key: &str, value: MirrorValue);
    fn set_pointer(&self, seat: u64, x: f64, y: f64);
    fn chord(&self, claim: i64, chord: &str, state: &str);
    fn claim_id(&self, chord: &str) -> i64;
    fn number(&self, node: u64, key: &str) -> f64;
    fn flag(&self, node: u64, key: &str) -> bool;
    fn rect(&self, node: u64) -> String;
}

impl Script for Comp {
    fn ops(&self) -> Vec<String> {
        std::mem::take(&mut self.borrow_mut().ops)
    }

    fn edge(&self, node: u64, kind: &str) {
        let fields = EventFields {
            x: 5.0,
            y: 6.0,
            button: if matches!(kind, "press" | "release") { 272 } else { 0 },
            ..EventFields::default()
        };
        self.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: kind.into(),
            fields,
        }));
    }

    fn new_window(&self, id: u64, app_id: &str, title: &str) {
        self.borrow_mut().nodes.insert(
            id,
            Node {
                class: "window".into(),
                parent: 1,
                props: BTreeMap::from([("title".to_owned(), text(title)), ("app_id".to_owned(), text(app_id))]),
            },
        );
        FakeLink(self.clone()).publish(vec![
            MirrorChange {
                class: "window".into(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: 1,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "title", text(title)),
            set_change(id, "app_id", text(app_id)),
        ]);
    }

    fn external(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.into(), value.clone());
        }
        FakeLink(self.clone()).publish(vec![set_change(node, key, value)]);
    }

    fn set_pointer(&self, seat: u64, x: f64, y: f64) {
        {
            let mut c = self.borrow_mut();
            let node = c.nodes.get_mut(&seat).expect("the seat exists");
            node.props.insert("pointer_x".into(), MirrorValue::Double(x));
            node.props.insert("pointer_y".into(), MirrorValue::Double(y));
        }
        FakeLink(self.clone()).publish(vec![
            set_change(seat, "pointer_x", MirrorValue::Double(x)),
            set_change(seat, "pointer_y", MirrorValue::Double(y)),
        ]);
        let mut c = self.borrow_mut();
        let updates: Vec<TreePointerChord> = c
            .drags
            .iter()
            .filter(|(_, drag)| drag.0 == seat)
            .map(|(claim, drag)| drag_message(*claim, drag, "update", (x, y)))
            .collect();
        c.queue.extend(updates.into_iter().map(TreeIncoming::PointerChord));
    }

    fn chord(&self, claim: i64, chord: &str, state: &str) {
        let (modifiers, button) = chord.rsplit_once("Button").expect("a pointer chord");
        let mut c = self.borrow_mut();
        let seat = *c.claim_seats.get(&claim).expect("a claim on a seat");
        let at = pointer_at(&c, seat);
        let message = if state == "press" {
            let drag = (seat, at, button.parse().expect("a button number"), modifiers.to_owned());
            let message = drag_message(claim, &drag, "start", at);
            c.drags.insert(claim, drag);
            message
        } else {
            let drag = c.drags.remove(&claim).expect("a drag is live");
            drag_message(claim, &drag, "end", at)
        };
        c.queue.push(TreeIncoming::PointerChord(message));
    }

    fn claim_id(&self, chord: &str) -> i64 {
        let c = self.borrow();
        *c.claims
            .iter()
            .find(|(_, held)| held.as_str() == chord)
            .unwrap_or_else(|| panic!("no live claim for {chord:?}; live: {:?}", c.claims))
            .0
    }

    fn number(&self, node: u64, key: &str) -> f64 {
        match self.borrow().nodes[&node].props.get(key) {
            Some(MirrorValue::Double(value)) => *value,
            None => 0.0,
            other => panic!("node {node} has no double {key}: {other:?}"),
        }
    }

    fn flag(&self, node: u64, key: &str) -> bool {
        matches!(self.borrow().nodes[&node].props.get(key), Some(MirrorValue::Bool(true)))
    }

    fn rect(&self, node: u64) -> String {
        ["x", "y", "width", "height"]
            .iter()
            .map(|key| self.number(node, key).to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn drag_message(claim: i64, drag: &(u64, (f64, f64), i64, String), phase: &str, at: (f64, f64)) -> TreePointerChord {
    TreePointerChord {
        claim,
        phase: phase.to_owned(),
        x: at.0,
        y: at.1,
        dx: at.0 - drag.1.0,
        dy: at.1 - drag.1.1,
        button: drag.2,
        modifiers: drag.3.clone(),
        time: 0,
    }
}

fn pointer_at(c: &Compositor, seat: u64) -> (f64, f64) {
    let props = &c.nodes[&seat].props;
    let axis = |key: &str| match props.get(key) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    };
    (axis("pointer_x"), axis("pointer_y"))
}

fn node(class: &str, parent: u64, props: Vec<(&str, MirrorValue)>) -> Node {
    Node {
        class: class.into(),
        parent,
        props: props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    }
}

/// A root (1), a workspace (2), window 3 at 10,20 sized 300x200 with the focus, window 4 (a browser) with no size,
/// seat 5 with `seat` published on it, and a 1000 by 800 primary screen (7).
fn world(seat: Vec<(&str, MirrorValue)>) -> Comp {
    let mut c = Compositor::default();
    c.nodes.insert(1, node("node", 0, vec![]));
    c.nodes.insert(
        2,
        node(
            "workspace",
            1,
            vec![("index", MirrorValue::Uint(0)), ("active", MirrorValue::Bool(true))],
        ),
    );
    c.nodes.insert(
        3,
        node(
            "window",
            1,
            vec![
                ("title", text("term")),
                ("app_id", text("foot")),
                ("focused", MirrorValue::Bool(true)),
                ("x", MirrorValue::Double(10.0)),
                ("y", MirrorValue::Double(20.0)),
                ("width", MirrorValue::Double(300.0)),
                ("height", MirrorValue::Double(200.0)),
            ],
        ),
    );
    c.nodes.insert(
        4,
        node("window", 1, vec![("title", text("web")), ("app_id", text("firefox"))]),
    );
    let mut props = vec![
        ("name", text("seat0")),
        ("pointer_x", MirrorValue::Double(0.0)),
        ("pointer_y", MirrorValue::Double(0.0)),
        ("focus", MirrorValue::Node(3)),
    ];
    props.extend(seat);
    c.nodes.insert(5, node("seat", 1, props));
    c.nodes.insert(
        7,
        node(
            "screen",
            1,
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        ),
    );
    c.serial = 1;
    Rc::new(RefCell::new(c))
}

struct Rig {
    shell: JsShell,
    _dir: TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        let diagnostics = self.shell.frame().expect("a frame").diagnostics;
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        diagnostics
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }
}

const PRELUDE: &str = r#"import {
    gnoblin, defineConfig, Layout, FloatingLayout, MasterStackLayout, ClientMoveBinding, ClientResizeBinding,
    snapClient, unsnapClient, snappedZone,
} from "gnoblin";
(globalThis as any).lib = { snapClient, unsnapClient, snappedZone };
"#;

/// Boots a shell over `comp` with `init` after the prelude. The windows that are there at boot join the first frame.
fn boot(comp: &Comp, init: &str) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), format!("{PRELUDE}{init}")).expect("init.ts is written");
    let mut shell = JsShell::new(Box::new(FakeLink(comp.clone())), JsShellConfig::new(&root)).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut rig = Rig { shell, _dir: dir };
    rig.frame();
    rig.frame();
    comp.ops();
    rig
}

/// One workspace whose layout places nothing, so a window stays wherever a rule or the compositor put it.
const BASE: &str = "workspaces: [{ id: \"main\", layout: new Layout() }]";

fn config(keys: &str) -> String {
    format!("defineConfig({{ {BASE}, {keys} }});\n")
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
    // Lua: `Window:on_ready` focuses the new window. New: windows already there at boot are left alone, so a reload
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
fn a_rule_can_keep_a_new_window_from_taking_focus() {
    // New: `skipFocus`. The rule runs before the focus policy hears of the window.
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config("rules: [{ appId: \"pinentry*\", skipFocus: true }]"));
    comp.new_window(8, "pinentry-gnome3", "passphrase");
    comp.new_window(9, "kitty", "shell");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=9"]);
}

#[test]
fn a_click_focuses_the_window_and_raises_it_when_raise_on_click_is_on() {
    // Lua: `Window:on_press` (shell_test.lua covers the wiring there). Window 4 is already on top, so it is focused
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
fn a_click_only_focuses_when_the_seat_says_raise_on_click_is_off_and_the_config_can_override_it() {
    let comp = world(vec![("raise_on_click", flag(false))]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(3, "press");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=3"], "the seat's raise_on_click is false");
    comp.edge(3, "release");
    rig.frame();
    let comp = world(vec![("raise_on_click", flag(false))]);
    let mut rig = rig_over(&comp, "focus: { raiseOnClick: true }");
    comp.edge(3, "press");
    rig.frame();
    assert_eq!(
        comp.ops(),
        ["set:5.focus=3", "insert:3->1"],
        "the config's key wins over the seat's"
    );
}

fn rig_over(comp: &Comp, keys: &str) -> Rig {
    boot(comp, &config(keys))
}

#[test]
fn click_mode_ignores_the_pointer_over_a_window() {
    // Lua: "click mode never moves focus on hover".
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "enter");
    comp.edge(4, "motion");
    rig.frame();
    assert!(comp.ops().is_empty());
}

#[test]
fn sloppy_focus_follows_the_pointer_on_entry_and_on_movement_and_keeps_focus_on_leave() {
    // Lua: "hover mode focuses the window under the pointer, on entry or movement" and Window:on_leave for hover.
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
    // Lua: "hover-strict focuses only when the pointer enters the window" and "only hover-strict clears focus when
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
fn the_configs_focus_mode_wins_over_the_seats() {
    // New: `focus` in the config overrides what the seat publishes, key by key.
    let comp = world(seat("hover"));
    let mut rig = boot(&comp, &config("focus: { mode: \"click\" }"));
    comp.edge(4, "enter");
    rig.frame();
    assert!(comp.ops().is_empty(), "the config says click");
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config("focus: { mode: \"sloppy\" }"));
    comp.edge(4, "enter");
    rig.frame();
    assert_eq!(
        comp.ops(),
        ["set:5.focus=4"],
        "the seat says click and the config says sloppy"
    );
}

#[test]
fn a_window_pressed_holds_hover_focus_until_the_button_is_released() {
    // Lua: `Window:on_enter` returns while `pressed` is set.
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
    // Lua: `Window:on_enter` starts the auto-raise timer and `Window:on_leave` cancels it. Window 3 has the focus.
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
    sleep(Duration::from_millis(80));
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
    sleep(Duration::from_millis(80));
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
    sleep(Duration::from_millis(30));
    rig.frame();
    assert!(comp.ops().is_empty(), "auto_raise is false");
    let comp = world(vec![
        ("auto_raise", flag(true)),
        ("auto_raise_delay", MirrorValue::Int(40)),
    ]);
    let mut rig = boot(&comp, &config("focus: { mode: \"sloppy\" }"));
    comp.edge(3, "enter");
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=3"]);
    comp.external(5, "focus", MirrorValue::Node(4));
    sleep(Duration::from_millis(80));
    rig.frame();
    assert!(
        comp.ops().is_empty(),
        "the compositor gave the focus to window 4 before the delay ran out, so window 3 is not raised"
    );
}

#[test]
fn a_pointer_rest_focuses_after_the_rest_time_and_a_leave_cancels_it() {
    // Lua: `rest_focus_later`; the rest is 0.2 s.
    let comp = world(vec![
        ("focus_mode", text("hover")),
        ("focus_change_on_pointer_rest", flag(true)),
    ]);
    let mut rig = boot(&comp, &config(""));
    comp.edge(4, "enter");
    rig.frame();
    assert!(comp.ops().is_empty(), "a rest is not an entry");
    sleep(Duration::from_millis(260));
    rig.frame();
    assert_eq!(comp.ops(), ["set:5.focus=4"]);
    comp.edge(4, "enter");
    comp.edge(4, "leave");
    rig.frame();
    sleep(Duration::from_millis(260));
    rig.frame();
    assert!(comp.ops().is_empty(), "the pointer left before it rested");
}

// -- window rules, through the shell ------------------------------------------------------------------------

#[test]
fn a_rule_places_a_new_window_of_one_app_and_leaves_another_alone() {
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        &config("rules: [{ appId: \"term*\", position: { x: 300, y: 200 }, size: { width: 400, height: 300 } }]"),
    );
    comp.new_window(8, "term-a", "one");
    comp.new_window(9, "kitty", "two");
    rig.frame();
    assert_eq!(comp.rect(8), "300,200,400,300");
    assert_eq!(
        comp.rect(9),
        "0,0,0,0",
        "the other window is where the compositor put it"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.get('8').floatingBounds.x"),
        "300",
        "the layout keeps it there"
    );
}

#[test]
fn a_rule_does_not_touch_a_window_the_shell_found_at_boot() {
    // New: a reload does not move the windows the user has placed.
    let comp = world(vec![]);
    let _rig = boot(
        &comp,
        &config("rules: [{ appId: \"foot\", secure: true, position: { x: 1, y: 2 } }]"),
    );
    assert!(!comp.flag(3, "secure"));
    assert_eq!(comp.rect(3), "10,20,300,200");
}

#[test]
fn a_workspace_rule_moves_a_new_window_to_the_workspace_it_names() {
    // Lua: "a workspace rule moves the window to the workspace it names".
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        "defineConfig({ workspaces: [{ id: \"a\", layout: new Layout() }, { id: \"b\", layout: new Layout() }],\n\
         rules: [{ appId: \"term\", workspace: \"b\" }] });\n",
    );
    comp.new_window(8, "term", "shell");
    comp.new_window(9, "other", "shell");
    rig.frame();
    assert_eq!(rig.eval("gnoblin.clients.get('8').workspace.id"), "b");
    assert_eq!(
        rig.eval("gnoblin.clients.get('9').workspace.id"),
        "a",
        "the first workspace is where a window opens"
    );
}

#[test]
fn a_rule_that_names_a_workspace_the_script_removed_is_reported_once_and_leaves_the_window() {
    // Lua: "a rule naming a missing workspace is reported once and the window stays where it is". The config refuses
    // a workspace it does not declare, so this one is declared and then removed by the script.
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        "defineConfig({ workspaces: [{ id: \"a\", layout: new Layout() }, { id: \"b\", layout: new Layout() }],\n\
         rules: [{ appId: \"term\", workspace: \"b\" }] });\n\
         gnoblin.workspaces.remove(\"b\");\n",
    );
    comp.new_window(8, "term", "shell");
    let diagnostics = rig.shell.frame().expect("a frame").diagnostics;
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("workspace \"b\""), "{}", diagnostics[0]);
    assert_eq!(rig.eval("gnoblin.clients.get('8').workspace.id"), "a");
    let diagnostics = rig.shell.frame().expect("a frame").diagnostics;
    assert!(
        diagnostics.is_empty(),
        "a frame with no change does not report again: {diagnostics:?}"
    );
}

#[test]
fn a_rule_with_reapply_follows_the_app_id_and_undoes_itself_when_it_stops_matching() {
    // Lua: "a rule that stops matching clears secure", "an app_id that newly matches a rule applies it".
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        &config("rules: [{ appId: \"bank\", secure: true, reapply: true }]"),
    );
    comp.new_window(8, "notes", "x");
    comp.new_window(9, "bank", "x");
    rig.frame();
    assert!(!comp.flag(8, "secure"), "no rule matches the first app id");
    assert!(
        comp.flag(9, "secure"),
        "the rule sets secure on a window that opens as the bank"
    );
    comp.external(8, "app_id", text("bank"));
    rig.frame();
    assert!(comp.flag(8, "secure"), "the new app id matches the rule");
    comp.external(9, "app_id", text("notes"));
    rig.frame();
    assert!(!comp.flag(9, "secure"), "secure must clear once no rule matches");
    assert_eq!(
        comp.ops().iter().filter(|op| op.starts_with("set:9.secure")).count(),
        2,
        "set, then cleared"
    );
}

#[test]
fn a_rule_with_reapply_follows_the_title() {
    // Lua: "a title change re-evaluates the rules: secure follows the new title".
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        &config("rules: [{ title: \"Secret*\", secure: true, reapply: true }]"),
    );
    comp.new_window(8, "mail", "Inbox");
    rig.frame();
    assert!(!comp.flag(8, "secure"), "no rule matches the first title");
    comp.external(8, "title", text("Secret plans"));
    rig.frame();
    assert!(comp.flag(8, "secure"), "the new title must match the rule");
}

#[test]
fn a_rule_without_reapply_applies_once_when_the_window_appears() {
    // New: `reapply` is what the Lua rules always did.
    let comp = world(vec![]);
    let mut rig = boot(&comp, &config("rules: [{ title: \"Secret*\", secure: true }]"));
    comp.new_window(8, "mail", "Inbox");
    rig.frame();
    comp.external(8, "title", text("Secret plans"));
    rig.frame();
    assert!(!comp.flag(8, "secure"), "the title changed after the rule was read");
    comp.new_window(9, "mail", "Secret plans");
    rig.frame();
    assert!(comp.flag(9, "secure"), "a window that opens with the title matches");
}

#[test]
fn re_applying_rules_that_still_match_writes_nothing() {
    // Lua: "re-applying unchanged rules writes nothing to the core".
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        &config("rules: [{ title: \"Secret*\", secure: true, reapply: true }]"),
    );
    comp.new_window(8, "mail", "Secret one");
    rig.frame();
    rig.frame();
    comp.ops();
    comp.external(8, "title", text("Secret two"));
    rig.frame();
    rig.frame();
    let ops = comp.ops();
    assert!(ops.is_empty(), "the rule matched before and after: {ops:?}");
}

#[test]
fn a_rule_can_maximise_fullscreen_and_dim_a_window() {
    // Lua: "a maximised rule fills the work area, not only sets the flag", "an opacity rule sets the window's
    // opacity", over the 1000 by 800 screen. New: fullscreen goes first and a fullscreen window does not maximise.
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        &config(
            "rules: [\n\
               { appId: \"big\", maximised: true },\n\
               { appId: \"dim\", style: { opacity: 0.5, cornerRadius: 12 } },\n\
               { appId: \"full\", fullscreen: true, maximised: true },\n\
             ]",
        ),
    );
    comp.new_window(8, "big", "x");
    comp.new_window(9, "dim", "x");
    comp.new_window(10, "full", "x");
    rig.frame();
    assert!(comp.flag(8, "maximised"));
    assert_eq!(comp.rect(8), "0,0,1000,800", "the window fills the work area");
    assert_eq!(comp.number(9, "opacity"), 0.5);
    assert!(!comp.flag(9, "maximised"));
    assert!(comp.flag(10, "fullscreen"));
    assert!(!comp.flag(10, "maximised"), "a fullscreen window does not maximise");
    assert_eq!(comp.rect(10), "0,0,1000,800");
}

#[test]
fn the_first_matching_rule_applies_and_all_lets_the_next_one_apply_too() {
    // Lua: "rules merge in list order: the last matching rule wins for each field, the others are kept".
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        &config(
            "rules: [\n\
               { appId: \"foo\", fullscreen: true, all: true },\n\
               { appId: \"foo\", secure: true, all: true },\n\
               { appId: \"foo\", fullscreen: false },\n\
               { appId: \"foo\", maximised: true },\n\
             ]",
        ),
    );
    comp.new_window(8, "foo", "x");
    rig.frame();
    assert!(comp.flag(8, "secure"), "secure from the middle rule is kept");
    assert!(
        !comp.flag(8, "fullscreen"),
        "the later rule's fullscreen wins over the earlier true"
    );
    assert!(
        !comp.flag(8, "maximised"),
        "the rule after the one without all never ran"
    );
}

#[test]
fn a_floating_rule_keeps_a_window_out_of_the_tiling() {
    // New: `floating`. Two windows tile in the master-stack layout; the pinned one keeps its place.
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        "defineConfig({ workspaces: [{ id: \"tiles\", layout: new MasterStackLayout() }],\n\
         rules: [{ appId: \"pin\", floating: true, position: { x: 700, y: 500 }, size: { width: 200, height: 100 } }] });\n",
    );
    assert_eq!(comp.rect(3), "0,0,500,800");
    assert_eq!(comp.rect(4), "500,0,500,800");
    comp.new_window(8, "pin", "x");
    rig.frame();
    rig.frame();
    assert_eq!(
        comp.rect(8),
        "700,500,200,100",
        "the layout does not place a floating window"
    );
    assert_eq!(comp.rect(3), "0,0,500,800", "the tiles are as they were");
    assert_eq!(comp.rect(4), "500,0,500,800");
    assert_eq!(rig.eval("String(gnoblin.clients.get('8').floating)"), "true");
}

// -- edge snapping, through the shell -----------------------------------------------------------------------

const SNAP_AREA: &str = "{ x: 0, y: 0, width: 800, height: 600 }";

#[test]
fn snapping_tiles_a_window_to_an_edge_and_unsnapping_restores_its_floating_rectangle() {
    // Lua: "snapping tiles a window to an edge and unsnapping restores its floating rectangle".
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
    // Lua: "snapping a maximised window floats it first, and unsnapping restores the floating rectangle".
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
    // Lua: "snapping is refused when edge tiling is off and leaves the window where it was".
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
    // Lua: `drag` ends with `Snap.edge_zone(s.x, s.y, work_area(), EDGE_MARGIN)` and `w:snap(zone)`; a 1000 by 800
    // work area, the default margin of 4.
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = boot(
        &comp,
        "defineConfig({ workspaces: [{ id: \"main\", layout: new FloatingLayout() }] });\n",
    );
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
    // Lua: "edge tiling is off by default".
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        "defineConfig({ workspaces: [{ id: \"main\", layout: new FloatingLayout() }] });\n",
    );
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(
        comp.rect(3),
        "0,260,300,200",
        "moved by the pointer's delta and kept in the work area"
    );
    assert_eq!(rig.eval("String(lib.snappedZone(gnoblin.clients.get('3')))"), "null");
}

#[test]
fn the_configs_edge_tiling_overrides_the_seats() {
    // New: `defineConfig({ edgeTiling })`.
    let comp = world(vec![]);
    let mut rig = boot(
        &comp,
        "defineConfig({ edgeTiling: true, workspaces: [{ id: \"main\", layout: new FloatingLayout() }] });\n",
    );
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(comp.rect(3), "0,0,500,800");
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "edgeTiling: false,", "new ClientMoveBinding({ id: \"move\" })");
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(
        comp.rect(3),
        "0,260,300,200",
        "the config says off and the seat says on"
    );
}

/// A floating workspace whose only binding is `binding`, so a case chooses the move binding's options.
fn rig_with_move(comp: &Comp, keys: &str, binding: &str) -> Rig {
    boot(
        comp,
        &format!(
            "const layout = new FloatingLayout({{ triggers: {{ move: null, resize: null }} }});\n\
             layout.bindings.add({binding});\n\
             defineConfig({{ {keys} workspaces: [{{ id: \"main\", layout }}] }});\n"
        ),
    )
}

#[test]
fn the_move_bindings_snap_option_sets_the_margin_or_turns_snapping_off() {
    // New: `ClientMoveBinding({ snap })`.
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "", "new ClientMoveBinding({ id: \"move\" })");
    drag_to(&comp, &mut rig, (10.0, 300.0));
    assert_eq!(
        comp.rect(3),
        "0,260,300,200",
        "10 pixels from the edge is outside the default margin of 4"
    );
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "", "new ClientMoveBinding({ id: \"move\", snap: 12 })");
    drag_to(&comp, &mut rig, (10.0, 300.0));
    assert_eq!(comp.rect(3), "0,0,500,800", "10 pixels is inside a margin of 12");
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = rig_with_move(&comp, "", "new ClientMoveBinding({ id: \"move\", snap: false })");
    drag_to(&comp, &mut rig, (2.0, 300.0));
    assert_eq!(comp.rect(3), "0,260,300,200", "snap: false never tiles");
}

#[test]
fn a_snapped_window_the_user_drags_away_is_no_longer_snapped() {
    let comp = world(vec![("edge_tiling", flag(true))]);
    let mut rig = boot(
        &comp,
        "defineConfig({ workspaces: [{ id: \"main\", layout: new FloatingLayout() }] });\n",
    );
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
             defineConfig({{ workspaces: [{{ id: \"main\", layout }}] }});\n"
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
