//! The second set of stock TypeScript layouts (`data/js/gnoblin/`): Dwindle, Kiosk, Strip and Carousel3d, the
//! workspace slide gesture, workspace sets and the launcher and workspace keys of the stock config, run in a real
//! generation against a fake `gnoblin` global.
//!
//! The cases are the Lua shell's: `crates/gnoblin-lua/tests/lua/tiling_test.lua` (Dwindle, Kiosk),
//! `crates/gnoblin-lua/tests/lua/strip_test.lua` (Strip, Carousel3d, the ring), `sets_test.lua` (the pure set cases)
//! and the gesture cases of `crates/gnoblin-compositor/tests/shell_vm.rs` (swipe left slides on, a short swipe
//! slides back). A case taken from that corpus says so; a case marked "derived" is not in it (the Lua class has no
//! such option, or the case was a property of the Lua node tree) and its expectation was worked out by hand from the
//! same algorithm. Rectangles are written `x,y,width,height`, in the Lua tests' 1000 by 800 workspace.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, stock_modules};

/// The fake host and the helpers every case shares. Plain JavaScript: embedded modules are not stripped.
const HARNESS: &str = r##"
globalThis.__registered = [];
globalThis.gnoblin = {
    __register(config) {
        globalThis.__registered.push(config);
    },
};
export const area = (width = 1000, height = 800) => ({ x: 0, y: 0, width, height });
export const nodes = (n) => Array.from({ length: n }, (_, i) => ({ name: `w${i + 1}` }));
const num = (v) => String(Math.round(v * 1000) / 1000);
export const rect = (r) => [r.x, r.y, r.width, r.height].map(num).join(",");
// `n` nodes where one holds the focus: `focused` and `focus()` are what a client has.
export function focusables(n) {
    const state = { current: null };
    const list = Array.from({ length: n }, (_, i) => ({
        name: `w${i + 1}`,
        get focused() {
            return state.current === this;
        },
        focus() {
            state.current = this;
        },
    }));
    return { list, state };
}
// The placements as "x,y,w,h" per node in `list` order, after checking each is for its node, tiled and whole pixels.
export function tiled(placements, list) {
    return placements
        .map((p, i) => {
            if (p.node !== list[i]) throw new Error(`placement ${i} is for the wrong node`);
            if (p.tiled !== true) throw new Error(`placement ${i} is not tiled`);
            for (const v of [p.bounds.x, p.bounds.y, p.bounds.width, p.bounds.height]) {
                if (!Number.isInteger(v)) throw new Error(`placement ${i} is not whole pixels: ${rect(p.bounds)}`);
            }
            return rect(p.bounds);
        })
        .join(" ");
}
export function lay(layout, list, ctx = {}) {
    return tiled(layout.compute({ children: list, area: area(), ...ctx }), list);
}
// The placements as "name:x,y,w,h" in placement order, none of them tiled (the strip and the ring do not tile).
export function named(placements) {
    return placements
        .map((p) => {
            if (p.tiled !== undefined) throw new Error(`${p.node.name} is marked tiled`);
            return `${p.node.name}:${rect(p.bounds)}`;
        })
        .join(" ");
}
export function thrown(fn) {
    try {
        fn();
    } catch (e) {
        return `${e.name}: ${e.message}`;
    }
    return "did not throw";
}
export function emit(value) {
    globalThis.out = String(value);
}
"##;

const IMPORTS: &str = r#"
import { area, nodes, focusables, tiled, lay, named, thrown, rect, emit } from "harness";
import {
    DwindleLayout, KioskLayout, StripLayout, Carousel3dLayout, ring, clampColumn,
    WorkspaceSlideBinding, defineSets, activateSet, WorkspaceSets, WorkspaceSetError,
    defineConfig, desugarConfig, defaultConfig, mergeConfig, spawn, show, focusColumn, binding, parseTrigger,
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

/// One expected line per entry, as the JavaScript joins them.
fn lines(expected: &[&str]) -> String {
    expected.join("\n")
}

// -- Dwindle ------------------------------------------------------------------------------------------------------

#[test]
fn dwindle_matches_the_lua_tiling_goldens() {
    // Lua: "Dwindle: 2 and 3 windows split the last cell by aspect", "split horizontal puts every window side by
    // side", "ratio changes re-arrange with no handler". Windows arrive one at a time, as `ws:add` did.
    let got = run(r#"
        const grow = (layout, n, ctx = {}) => {
            const all = nodes(n);
            return Array.from({ length: n }, (_, i) => lay(layout, all.slice(0, i + 1), ctx));
        };
        const aspect = new DwindleLayout();
        const flat = new DwindleLayout({ split: "horizontal" });
        const ratio = new DwindleLayout();
        const two = nodes(2);
        const out = [
            grow(aspect, 3).slice(1).join("\n"),
            grow(flat, 3).slice(2).join("\n"),
        ];
        lay(ratio, two);
        ratio.ratio = 0.25;
        out.push(lay(ratio, two));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,400 500,400,500,400",
            "0,0,500,800 500,0,250,800 750,0,250,800",
            "0,0,250,800 250,0,750,800",
        ])
    );
}

#[test]
fn dwindle_has_the_lua_defaults_and_refuses_a_bad_split() {
    // Lua: "the options have the old layouts' defaults": split aspect, ratio 0.5, gap 0 (unset here, so the
    // workspace's gap applies). The error is the Lua class's, in the layout's name.
    let got = run(r#"
        const d = new DwindleLayout();
        emit([
            [d.split, d.ratio, d.gap].join(","),
            thrown(() => lay(new DwindleLayout({ split: "diagonal" as any }), nodes(1))),
            thrown(() => lay(new DwindleLayout({ ratio: Number.NaN }), nodes(1))),
            thrown(() => new DwindleLayout({ gap: -1 })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "aspect,0.5,",
            "TypeError: DwindleLayout split must be aspect, horizontal or vertical",
            "RangeError: DwindleLayout ratio must be a finite number",
            "RangeError: DwindleLayout gap must be a finite number of at least 0, got -1",
        ])
    );
}

#[test]
fn dwindle_clamps_its_ratio_like_lua() {
    // Lua: `clamp(self.ratio, 0.1, 0.9)`. A ratio of 5 gives the first child 0.9 of the width.
    let got = run(r#"
        const all = nodes(2);
        const high = new DwindleLayout({ ratio: 5 });
        const low = new DwindleLayout({ ratio: -1 });
        emit([lay(high, all), lay(low, all)].join("\n"));
    "#);
    assert_eq!(got, lines(&["0,0,900,800 900,0,100,800", "0,0,100,800 100,0,900,800"]));
}

#[test]
fn dwindle_gap_collapse_and_focus_insertion_are_derived_from_the_lua_algorithm() {
    // Derived: the Lua tests give no gap, no closing and no focus to Dwindle. The Lua algorithm: the gap is
    // taken off the width before the ratio ((1000 - 20) * 0.5 = 490); a closed window's sibling takes the split's
    // place; a new window splits the cell of the window that had focus when it arrived.
    let got = run(r#"
        const all = nodes(4);
        const gapped = new DwindleLayout({ gap: 20 });
        const viaWorkspace = new DwindleLayout();
        const collapse = new DwindleLayout();
        lay(collapse, all.slice(0, 3));
        const focus = new DwindleLayout();
        lay(focus, all.slice(0, 2), { focused: all[0] });
        emit([
            lay(gapped, all.slice(0, 2)),
            lay(viaWorkspace, all.slice(0, 2), { gap: { inner: 20, outer: 0 } }),
            lay(collapse, [all[0], all[2]]),
            // w3 arrives and the compositor has already focused it: it splits w1's cell, the previous focus.
            lay(focus, all.slice(0, 3), { focused: all[2] }),
            // w4 arrives; w3 had the focus, so w3's cell (500 by 400, wider than tall) splits side by side.
            lay(focus, all, { focused: all[3] }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,490,800 510,0,490,800",
            "0,0,490,800 510,0,490,800",
            "0,0,500,800 500,0,500,800",
            "0,0,500,400 500,0,500,800 0,400,500,400",
            "0,0,500,400 500,0,500,800 0,400,250,400 250,400,250,400",
        ])
    );
}

// -- Kiosk --------------------------------------------------------------------------------------------------------

#[test]
fn kiosk_shows_one_full_window_and_hides_the_rest_like_lua() {
    // Lua: "Kiosk: 1, 2 and 3 windows show one full-area window and hide the rest".
    let got = run(r#"
        const l = new KioskLayout();
        const shown = (placed) => placed.map((p) => (p.visible ? "T" : "F")).join("");
        const out = [];
        for (const n of [1, 2, 3]) {
            const children = nodes(n);
            const placed = l.compute({ children, area: area() });
            out.push(tiled(placed, children) + " " + shown(placed));
        }
        // The shown window is the focused child, else the first.
        const children = nodes(3);
        out.push(shown(l.compute({ children, area: area(), focused: children[2] })));
        out.push(shown(l.compute({ children, area: area(), focused: { name: "not a child" } })));
        out.push(String(l.compute({ children: [], area: area() }).length));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,1000,800 T",
            "0,0,1000,800 0,0,1000,800 TF",
            "0,0,1000,800 0,0,1000,800 0,0,1000,800 TFF",
            "FFT",
            "TFF",
            "0",
        ])
    );
}

#[test]
fn dwindle_and_kiosk_replay_to_identical_rectangles_and_stay_whole_pixels() {
    // Lua: "each class: the same sequence replayed twice gives identical rectangles" (three windows, a gap of 4, the
    // second closed, two more opened) and "200 windows give finite integral rectangles".
    let got = run(r#"
        const replay = (make) => {
            const layout = make();
            const all = nodes(5);
            lay(layout, all.slice(0, 3));
            return lay(layout, [all[0], all[2], all[3], all[4]]);
        };
        const results = [];
        for (const make of [() => new DwindleLayout({ gap: 4 }), () => new KioskLayout()]) {
            results.push(replay(make) === replay(make));
        }
        const big = new DwindleLayout({ gap: 4 });
        const crowd = lay(big, nodes(200));
        results.push(crowd.split(" ").length === 200);
        emit(results.join(","));
    "#);
    assert_eq!(got, "true,true,true");
}

// -- Strip --------------------------------------------------------------------------------------------------------

#[test]
fn a_column_fraction_clamps_to_a_tenth_and_one_and_a_non_number_is_an_error_like_lua() {
    // Lua: "a column fraction clamps to 0.1..1 and a non-number or NaN is an error".
    let got = run(r#"
        emit([
            clampColumn(0.5), clampColumn(0), clampColumn(-Infinity), clampColumn(5), clampColumn(Infinity),
            thrown(() => clampColumn("wide")),
            thrown(() => clampColumn(undefined)),
            thrown(() => clampColumn(Number.NaN)),
            thrown(() => new StripLayout({ column: Number.NaN })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0.5",
            "0.1",
            "0.1",
            "1",
            "1",
            "TypeError: column must be a number, got wide",
            "TypeError: column must be a number, got undefined",
            "TypeError: column must be a number, got NaN",
            "TypeError: column must be a number, got NaN",
        ])
    );
}

#[test]
fn a_strip_lays_out_one_column_per_node_and_scrolls_to_keep_the_focused_one_in_view_like_lua() {
    // Lua: "a strip lays out one column per window and scrolls to keep the focused one in view": 3 windows, the
    // last focused; a column is half the strip wide and the full height; column 3 centres (view 750) then clamps to
    // the 3 * 500 - 1000 = 500 the strip can scroll, so it sits at 500, column 2 at 0 and column 1 at -500.
    let got = run(r#"
        const { list, state } = focusables(3);
        state.current = list[2];
        const l = new StripLayout();
        const placed = l.compute({ children: list, area: area(), focused: state.current });
        emit([
            named(placed),
            // Derived: a column of 0.25 leaves the strip narrower than the view, so nothing scrolls.
            named(new StripLayout({ column: 0.25 }).compute({ children: list, area: area(), focused: list[2] })),
            // Derived: a strip does not floor (the Lua one did not): 0.5 of 1001 is 500.5.
            named(new StripLayout().compute({ children: list, area: area(1001), focused: list[2] })),
            // Derived: an area that does not start at the origin moves every column with it.
            named(new StripLayout().compute({ children: list, area: { x: 10, y: 20, width: 1000, height: 800 }, focused: list[2] })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "w1:-500,0,500,800 w2:0,0,500,800 w3:500,0,500,800",
            "w1:0,0,250,800 w2:250,0,250,800 w3:500,0,250,800",
            "w1:-500.5,0,500.5,800 w2:0,0,500.5,800 w3:500.5,0,500.5,800",
            "w1:-490,20,500,800 w2:10,20,500,800 w3:510,20,500,800",
        ])
    );
}

#[test]
fn a_new_node_opens_after_the_focused_column_not_at_the_end_like_lua() {
    // Lua: "a new window opens after the focused column, not at the end": three windows, focus on the first, a new
    // one -> w1, new, w2, w3. The compositor focuses the arrival in the same frame, and the layout still places it
    // after the column that had focus when it came (the focus it saw last).
    let got = run(r#"
        const { list, state } = focusables(3);
        state.current = list[2];
        const l = new StripLayout();
        const order = (placed) => placed.map((p) => p.node.name).join(" ");
        const compute = (children) => l.compute({ children, area: area(), focused: children.find((c) => c.focused) ?? null });
        compute(list);
        state.current = list[0];
        compute(list);
        const fresh = { name: "new", get focused() { return state.current === this; }, focus() { state.current = this; } };
        const withNew = [...list, fresh];
        const first = order(compute(withNew));
        state.current = fresh;
        const second = order(compute(withNew));
        // Several arrivals in one frame keep their arrival order after the focused column.
        const other = new StripLayout();
        const a = focusables(2);
        a.state.current = a.list[0];
        other.compute({ children: a.list, area: area(), focused: a.state.current });
        const x = { name: "x" };
        const y = { name: "y" };
        const third = order(other.compute({ children: [...a.list, x, y], area: area(), focused: a.list[0] }));
        emit([first, second, third].join("\n"));
    "#);
    assert_eq!(got, lines(&["w1 new w2 w3", "w1 new w2 w3", "w1 x y w2"]));
}

#[test]
fn focus_left_and_right_move_through_the_columns_and_stop_at_the_ends_like_lua() {
    // Lua: "focus_left and focus_right move through the columns and stop at the ends".
    let got = run(r#"
        const { list, state } = focusables(3);
        state.current = list[2];
        const l = new StripLayout();
        l.compute({ children: list, area: area(), focused: state.current });
        const workspace = { clients: list };
        const seen: string[] = [];
        for (const step of [l.focusLeft, l.focusLeft, l.focusLeft, l.focusRight, l.focusRight, l.focusRight]) {
            step.call(l, workspace);
            seen.push(state.current.name);
        }
        emit(seen.join(","));
    "#);
    assert_eq!(got, "w2,w1,w1,w2,w3,w3");
}

#[test]
fn scroll_to_pins_the_view_until_the_focused_node_changes_like_lua() {
    // Lua: "scroll_to pins the view until the focused window changes" and "scroll_to rejects a non-finite offset".
    let got = run(r#"
        const { list, state } = focusables(3);
        state.current = list[2];
        const l = new StripLayout();
        const compute = () => l.compute({ children: list, area: area(), focused: state.current });
        const x = (placed, name) => placed.find((p) => p.node.name === name).bounds.x;
        compute();
        l.scrollTo(0);
        const pinned = x(compute(), "w1");
        // The focus is unchanged, so the pin holds although column 3 is off view.
        const held = x(compute(), "w1");
        // Focus moves to column 2, which is in view: the pin ends without a follow.
        state.current = list[1];
        const inView = x(compute(), "w1");
        // Focus moves to column 3, which is off view: the view follows, centred and clamped to 500.
        state.current = list[2];
        const followed = x(compute(), "w3");
        emit([
            [pinned, held, inView, followed, l.pin].join(","),
            thrown(() => l.scrollTo(Number.NaN)),
            thrown(() => l.scrollTo(Infinity)),
            thrown(() => l.scrollTo("left" as any)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,0,500,",
            "TypeError: scrollTo needs a finite number, got NaN",
            "TypeError: scrollTo needs a finite number, got Infinity",
            "TypeError: scrollTo needs a finite number, got left",
        ])
    );
}

#[test]
fn closing_the_focused_column_moves_focus_to_its_neighbour_like_lua() {
    // Lua: "closing the focused column moves focus to its neighbour, not to the closed one".
    let got = run(r#"
        const { list, state } = focusables(3);
        state.current = list[1];
        const l = new StripLayout();
        l.compute({ children: list, area: area(), focused: state.current });
        // A column that is not focused closes: focus stays.
        l.onClientRemoved(list[0], { children: [list[1], list[2]], area: area() });
        const unrelated = state.current.name;
        l.onClientRemoved(list[1], { children: [list[0], list[2]], area: area() });
        const moved = state.current.name;
        // The last column closing hands focus to the one before it.
        const b = focusables(2);
        b.state.current = b.list[1];
        const last = new StripLayout();
        last.compute({ children: b.list, area: area(), focused: b.state.current });
        last.onClientRemoved(b.list[1], { children: [b.list[0]], area: area() });
        emit([unrelated, moved, b.state.current.name].join(","));
    "#);
    assert_eq!(got, "w2,w3,w1");
}

// -- the ring and Carousel3d --------------------------------------------------------------------------------------

#[test]
fn the_ring_puts_the_focused_card_at_the_front_unscaled_and_opaque_like_lua() {
    // Lua: "the ring puts the focused card at the front, unscaled and fully opaque".
    let got = run(r#"
        const s = ring(4, 1, area())[0];
        emit([
            s.rotation === 0,
            Math.abs(s.opacity - 1) < 1e-9,
            Math.abs(s.width - 1000 * 0.36) < 1e-6,
            Math.abs(s.x + s.width / 2 - 500) < 1e-6,
        ].join(","));
    "#);
    assert_eq!(got, "true,true,true,true");
}

#[test]
fn the_ring_is_symmetric_about_the_front_card_and_fades_toward_the_back_like_lua() {
    // Lua: "the ring is symmetric about the front card and fades toward the back" (focus 1 of 4: slot 2 is the right
    // neighbour, slot 4 the left, slot 3 the back).
    let got = run(r#"
        const slots = ring(4, 1, area());
        const [left, right, back] = [slots[3], slots[1], slots[2]];
        emit([
            Math.abs(left.rotation + right.rotation) < 1e-9,
            Math.abs(left.x + left.width / 2 + (right.x + right.width / 2) - 1000) < 1e-6,
            Math.abs(left.opacity - right.opacity) < 1e-9,
            back.opacity < left.opacity,
            back.width < right.width,
            Math.abs(Math.abs(back.rotation) - 180) < 1e-9,
        ].join(","));
    "#);
    assert_eq!(got, "true,true,true,true,true,true");
}

#[test]
fn the_ring_with_no_nodes_is_empty_and_a_bad_focus_index_is_an_error_like_lua() {
    // Lua: "the ring with no windows is empty, and a bad focus index is an error".
    let got = run(r#"
        emit([
            ring(0, 1, area()).length,
            thrown(() => ring(3, 0, area())),
            thrown(() => ring(3, 4, area())),
            thrown(() => ring(3, 1.5, area())),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0",
            "RangeError: focus must be an index in 1..3, got 0",
            "RangeError: focus must be an index in 1..3, got 4",
            "RangeError: focus must be an index in 1..3, got 1.5",
        ])
    );
}

#[test]
fn a_carousel_shows_the_focused_node_at_the_front_and_rotate_wraps_round_the_ring_like_lua() {
    // Lua: "a Carousel3d shows the focused window at the front and rotate wraps round the ring". The front card of
    // two has no rotation and sits at depth -F, F = (card height / 2) / tan(fov / 2); its opacity is left to the rules.
    let got = run(r#"
        const { list, state } = focusables(2);
        state.current = list[1];
        const c = new Carousel3dLayout();
        const placed = c.compute({ children: list, area: area(), focused: state.current });
        const front = placed[1];
        const focal = 240 / Math.tan((20 * Math.PI) / 180);
        const workspace = { clients: list };
        c.rotate(workspace, 1);
        const wrapped = state.current.name;
        c.rotate(workspace, -1);
        const back = state.current.name;
        emit([
            Math.abs(front.bounds.width - 360) < 1e-9,
            Math.abs(front.bounds.x + front.bounds.width / 2 - 500) < 1e-6,
            Math.abs(front.z + focal) < 1e-9,
            front.rotation.join(","),
            placed.every((p) => !("opacity" in p) && p.tiled === undefined),
            // With no focus among the nodes the first is the front card.
            new Carousel3dLayout().compute({ children: list, area: area() })[0].rotation.join(","),
            wrapped,
            back,
            thrown(() => c.rotate(workspace, "one" as any)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true",
            "true",
            "true",
            "0,0,0",
            "true",
            "0,0,0",
            "w1",
            "w2",
            "TypeError: rotate needs a number of steps, got one",
        ])
    );
}

#[test]
fn a_carousel_turns_the_side_cards_about_the_vertical_axis_by_the_rings_angle() {
    // Derived: the Lua class wrote only x, y, width and height; the placement carries the ring's angle as a rotation
    // about the vertical axis (radians) and its depth as `z`, which the node schema has. Of four cards focused on
    // the first, the right neighbour is a quarter turn round the ring and the back card half a turn.
    let got = run(r#"
        const { list, state } = focusables(4);
        state.current = list[0];
        const placed = new Carousel3dLayout().compute({ children: list, area: area(), focused: state.current });
        const turn = (p) => Math.round((p.rotation[1] * 180) / Math.PI);
        emit([
            placed.map(turn).join(","),
            placed.map((p) => (p.rotation[0] === 0 && p.rotation[2] === 0 ? "t" : "f")).join(""),
            placed[2].z < placed[1].z && placed[1].z < placed[0].z,
        ].join("\n"));
    "#);
    assert_eq!(got, lines(&["0,90,180,-90", "tttt", "true"]));
}

// -- the workspace slide gesture ----------------------------------------------------------------------------------

/// A fake shell of three workspaces of one window each, the first shown, and the events a gesture sends.
const SLIDE_WORLD: &str = r#"
function world(extra = {}) {
    const log: string[] = [];
    const timers: (() => void)[] = [];
    const windows = [100, 100, 100].map((x, i) => ({
        name: `w${i + 1}`,
        x,
        visible: i === 0,
        alive: true,
        animate(values, options) {
            log.push(`${this.name} animate x=${values.x} ${options.duration} ${options.easing}`);
            this.x = values.x;
        },
    }));
    const spaces = windows.map((w, i) => ({ name: `${i + 1}`, index: i, active: i === 0, clients: [w] }));
    const shell = {
        workspaces: {
            area: { width: 1000 },
            at: (i) => spaces[i],
            show(space, options) {
                log.push(`show ${space.name} ${options.duration} ${options.progress}`);
                for (const s of spaces) s.active = s === space;
            },
            [Symbol.iterator]: () => spaces[Symbol.iterator](),
        },
        after(seconds, run) {
            log.push(`after ${seconds}`);
            timers.push(run);
        },
        ...extra,
    };
    const binding = new WorkspaceSlideBinding({ host: shell });
    const xs = () => windows.map((w) => `${w.x}${w.visible ? "+" : "-"}`).join(" ");
    return { log, timers, windows, spaces, shell, binding, xs };
}
"#;

fn slide(body: &str) -> String {
    run(&format!("{SLIDE_WORLD}\n{body}"))
}

#[test]
fn a_swipe_left_follows_the_fingers_and_lifting_past_a_quarter_slides_on_to_the_next_workspace() {
    // Lua (shell_vm.rs): "a three-finger swipe on the input path slides the stock shell to the next workspace": the
    // workspace the fingers bring in is showing before they lift, and lifting after more than a quarter of the
    // travel finishes the slide: `desk:show(to, progress)` with 0.25 * (1 - progress) seconds.
    let got = slide(
        r#"
        const w = world();
        const state = w.binding.onStart({} as any) as object;
        w.binding.onUpdate({ dx: -300, dy: 0 } as any, state);
        const pulled = w.xs();
        w.binding.onEnd({ dx: -300, dy: 0 } as any, state);
        emit([pulled, w.xs(), w.log.join("; ")].join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            // a: 100 + (-1) * 0.75 * 1000; b: 100 + (-1) * (0.75 - 1) * 1000; c untouched and hidden
            "-650+ 350+ 100-",
            // back at rest for the shell's own slide to start from
            "100+ 100+ 100-",
            "show 2 0.0625 0.75",
        ])
    );
}

#[test]
fn a_swipe_short_of_the_commit_slides_back_and_the_incoming_workspace_is_hidden_after() {
    // Lua (shell_vm.rs): "a swipe that is short of the commit slides back": 50 of 400 units is 0.125, under 0.25.
    // The shown workspace slides back to rest and the other one out the way it came, over 0.2 seconds, and is hidden
    // when that ends.
    let got = slide(
        r#"
        const w = world();
        const state = w.binding.onStart({} as any) as object;
        w.binding.onUpdate({ dx: -50, dy: 0 } as any, state);
        const pulled = w.xs();
        w.binding.onEnd({ dx: -50, dy: 0 } as any, state);
        const settling = w.log.join("; ");
        for (const run of w.timers) run();
        emit([pulled, settling, w.xs(), w.spaces.map((s) => s.active).join(",")].join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            "-25+ 975+ 100-",
            "w1 animate x=100 0.2 out-cubic; w2 animate x=1100 0.2 out-cubic; after 0.2",
            "100+ 100- 100-",
            "true,false,false",
        ])
    );
}

#[test]
fn a_cancel_slides_back_and_an_end_that_is_more_vertical_than_horizontal_does_too() {
    // Lua: `phase == "end"` commits only when `math.abs(e.dx) >= math.abs(e.dy)`; a cancel never does.
    let got = slide(
        r#"
        const cancelled = world();
        const s1 = cancelled.binding.onStart({} as any) as object;
        cancelled.binding.onUpdate({ dx: -300, dy: 0 } as any, s1);
        cancelled.binding.onCancel({} as any, s1);
        const steep = world();
        const s2 = steep.binding.onStart({} as any) as object;
        steep.binding.onUpdate({ dx: -300, dy: 0 } as any, s2);
        steep.binding.onEnd({ dx: -300, dy: -400 } as any, s2);
        emit([cancelled.log.join("; "), steep.log.join("; ")].join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            "w1 animate x=100 0.2 out-cubic; w2 animate x=1100 0.2 out-cubic; after 0.2",
            "w1 animate x=100 0.2 out-cubic; w2 animate x=1100 0.2 out-cubic; after 0.2",
        ])
    );
}

#[test]
fn a_swipe_with_no_workspace_to_reveal_or_a_vertical_one_moves_nothing_like_lua() {
    // Lua: "A gesture that is more vertical than horizontal, or that has no workspace to reveal, moves nothing."
    // Fingers moving right bring the previous workspace in, and the first workspace has none.
    let got = slide(
        r#"
        const w = world();
        const state = w.binding.onStart({} as any) as object;
        w.binding.onUpdate({ dx: 0, dy: 0 } as any, state);
        w.binding.onUpdate({ dx: 10, dy: -200 } as any, state);
        w.binding.onUpdate({ dx: 300, dy: 0 } as any, state);
        w.binding.onEnd({ dx: 300, dy: 0 } as any, state);
        emit([w.xs(), w.log.length].join("\n"));
    "#,
    );
    assert_eq!(got, lines(&["100+ 100- 100-", "0"]));
}

#[test]
fn a_swipe_right_from_the_second_workspace_brings_the_previous_one_in_from_the_left() {
    // Lua: sign +1 for fingers moving right; the target is `children[position(from) - sign]`; the incoming one starts
    // a screen's width to the left: x = sign * (progress - 1) * W.
    let got = slide(
        r#"
        const w = world();
        w.spaces[0].active = false;
        w.spaces[1].active = true;
        w.windows[0].visible = false;
        w.windows[1].visible = true;
        const state = w.binding.onStart({} as any) as object;
        w.binding.onUpdate({ dx: 200, dy: 0 } as any, state);
        const pulled = w.xs();
        w.binding.onEnd({ dx: 200, dy: 10 } as any, state);
        emit([pulled, w.log.join("; ")].join("\n"));
    "#,
    );
    assert_eq!(got, lines(&["-400+ 600+ 100-", "show 1 0.125 0.5"]));
}

#[test]
fn changing_direction_hides_the_workspace_that_came_in_and_something_else_switching_ends_the_slide() {
    // Lua: `if sign ~= s.sign then reset(s) ...`; and "something else switched workspace under the fingers": the slide
    // lets go when the new workspace is the one switched to, and resets it when it is not.
    let got = slide(
        r#"
        const turned = world();
        const s1 = turned.binding.onStart({} as any) as object;
        turned.binding.onUpdate({ dx: -300, dy: 0 } as any, s1);
        turned.binding.onUpdate({ dx: 200, dy: 0 } as any, s1);
        const reset = turned.xs();
        const away = world();
        const s2 = away.binding.onStart({} as any) as object;
        away.binding.onUpdate({ dx: -300, dy: 0 } as any, s2);
        away.shell.workspaces.show(away.spaces[2], { duration: 0, progress: 0 });
        away.binding.onUpdate({ dx: -310, dy: 0 } as any, s2);
        const gone = away.xs();
        away.binding.onEnd({ dx: -310, dy: 0 } as any, s2);
        const landed = world();
        const s3 = landed.binding.onStart({} as any) as object;
        landed.binding.onUpdate({ dx: -300, dy: 0 } as any, s3);
        landed.shell.workspaces.show(landed.spaces[1], { duration: 0, progress: 0 });
        landed.binding.onUpdate({ dx: -310, dy: 0 } as any, s3);
        emit([reset, gone, away.log.length, landed.xs()].join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            "100+ 100- 100-",
            // the other workspace was switched to: what the slide moved goes back to rest
            "100+ 100- 100-",
            "1",
            // the workspace the slide was bringing in became the shown one: it is left where the fingers had it
            "-650+ 350+ 100-",
        ])
    );
}

#[test]
fn the_slide_binding_has_the_lua_travel_and_commit_and_refuses_a_start_without_a_screen() {
    let got = slide(
        r#"
        const w = world();
        const b = w.binding;
        const noScreen = world();
        (noScreen.shell.workspaces as any).area = null;
        const none = world();
        for (const s of none.spaces) s.active = false;
        emit([
            [b.id, b.trigger, b.travel, b.commit, b.seconds].join(","),
            noScreen.binding.onStart({} as any),
            none.binding.onStart({} as any),
            thrown(() => new WorkspaceSlideBinding({ travel: 0 })),
            thrown(() => new WorkspaceSlideBinding({ commit: 2 })),
            thrown(() => new WorkspaceSlideBinding({ trigger: "Swipe3Left" })) === "did not throw",
        ].join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            "workspace-slide,Swipe3,400,0.25,0.25",
            "false",
            "false",
            "RangeError: WorkspaceSlideBinding travel must be a finite number above 0",
            "RangeError: WorkspaceSlideBinding commit must be a number from 0 to 1",
            "true",
        ])
    );
}

// -- workspace sets -----------------------------------------------------------------------------------------------

const SETS_HEADER: &str = r#"
const DECLARED = new Set(["1", "2", "3"]);
const set = (name, entries) => ({ name, entries });
const text = (r) => JSON.stringify(r.ok ? r.value : r.error);
"#;

fn sets(body: &str) -> String {
    run(&format!("{SETS_HEADER}\n{body}"))
}

#[test]
fn the_pure_set_cases_of_the_lua_module_give_the_same_results() {
    // Lua sets_test.lua, case by case: unknown set, empty set, undeclared workspace (whole list refused), a removed
    // workspace refuses to activate, duplicate and empty names, two entries for one output, an unknown output falls
    // back to the primary with a warning, each workspace on its own output, the live set again changes nothing.
    let got = sets(
        r#"
        const work = defineSets([set("work", [{ workspace: "2" }])], DECLARED);
        const list = (work as any).value;
        const primary = { declared: DECLARED, outputs: [], primary: "DP-1" };
        const both = { declared: DECLARED, outputs: ["DP-1", "HDMI-1"], primary: "DP-1" };
        const split = defineSets(
            [set("split", [{ workspace: "1", output: "DP-1" }, { workspace: "2", output: "HDMI-1" }])],
            DECLARED,
        ) as any;
        const gone = defineSets([set("work", [{ workspace: "3", output: "GONE-1" }])], DECLARED) as any;
        const live = defineSets([set("work", [{ workspace: "2", output: "HDMI-1" }])], DECLARED) as any;
        emit([
            text(activateSet(list, "nope", primary)),
            text(defineSets([set("empty", [])], DECLARED)),
            text(defineSets([set("good", [{ workspace: "1" }]), set("bad", [{ workspace: "99" }])], DECLARED)),
            text(activateSet(defineSets([set("work", [{ workspace: "3" }])], DECLARED).value as any, "work", { ...primary, declared: new Set(["1", "2"]) })),
            text(defineSets([set("work", [{ workspace: "1" }]), set("work", [{ workspace: "2" }])], DECLARED)),
            text(defineSets([set("", [{ workspace: "1" }])], DECLARED)),
            text(defineSets([set("clash", [{ workspace: "1", output: "HDMI-1" }, { workspace: "2", output: "HDMI-1" }])], DECLARED)),
            JSON.stringify(activateSet(gone.value, "work", both).value),
            JSON.stringify(activateSet(split.value, "split", both).value),
            [activateSet(live.value, "work", both).value.changed, activateSet(live.value, "work", { ...both, active: "work" }).value.changed].join(","),
        ].join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            r#"{"kind":"unknown_set","name":"nope"}"#,
            r#"{"kind":"empty_set","name":"empty"}"#,
            r#"{"kind":"undeclared_workspace","set":"bad","workspace":"99"}"#,
            r#"{"kind":"undeclared_workspace","set":"work","workspace":"3"}"#,
            r#"{"kind":"duplicate_name","name":"work"}"#,
            r#"{"kind":"empty_name"}"#,
            r#"{"kind":"duplicate_output","set":"clash","output":"HDMI-1"}"#,
            r#"{"set":"work","placements":[{"workspace":"3","output":"DP-1"}],"warnings":[{"kind":"output_fallback","set":"work","output":"GONE-1"}],"changed":true}"#,
            r#"{"set":"split","placements":[{"workspace":"1","output":"DP-1"},{"workspace":"2","output":"HDMI-1"}],"warnings":[],"changed":true}"#,
            "true,false",
        ])
    );
}

#[test]
fn workspace_sets_show_the_named_workspaces_and_keep_the_sets_in_force_when_a_list_is_refused() {
    // Lua sets_test.lua, the stock-shell cases: the user's sets show each named workspace on its own output; a
    // refused new list is reported and the old sets stay (here it throws and they stay); an unknown set is refused; an
    // output that is gone shows its workspace on the primary output, with a report; the live set again changes
    // nothing. The environment is told the output of every placement, so a workspace for an output that is not the
    // primary one is shown there (`tests/outputs.rs` proves the shell shows it).
    let got = sets(
        r#"
        const shown: string[] = [];
        const where: string[] = [];
        const reports: string[] = [];
        let declared = new Set(["1", "2", "3"]);
        const sets = new WorkspaceSets({
            declared: () => declared,
            outputs: () => ({ names: ["DP-1", "HDMI-1"], primary: "DP-1" }),
            show: (id, output) => { shown.push(id); where.push(`${id}@${output}`); },
            report: (message) => reports.push(message),
        });
        sets.define([set("work", [{ workspace: "2", output: "DP-1" }]), set("away", [{ workspace: "3", output: "GONE-1" }]), set("split", [{ workspace: "1", output: "DP-1" }, { workspace: "2", output: "HDMI-1" }])]);
        const out: string[] = [];
        out.push(thrown(() => sets.define([set("empty", [])])));
        out.push(sets.list.map((s) => s.name).join(","));
        sets.activate("work");
        out.push(shown.join(",") + "|" + sets.active);
        sets.activate("work");
        out.push(shown.join(",") + "|live again changes nothing");
        out.push(thrown(() => sets.activate("nope")));
        sets.activate("away");
        out.push(shown.join(",") + "|" + reports.length + "|" + reports[0]);
        sets.activate("split");
        out.push(shown.join(",") + "|" + reports.length + "|" + reports[1]);
        declared = new Set(["1", "2"]);
        const refused = (() => { try { sets.activate("away"); return "ok"; } catch (e: any) { return `${e.name}: ${e.message} ${e instanceof WorkspaceSetError}`; } })();
        out.push(refused);
        out.push(sets.active as string);
        out.push(where.join(","));
        emit(out.join("\n"));
    "#,
    );
    assert_eq!(
        got,
        lines(&[
            "WorkspaceSetError: set \"empty\" has no entries",
            "work,away,split",
            "2|work",
            "2|live again changes nothing",
            "WorkspaceSetError: no set is named \"nope\"",
            "2,3|1|workspace set \"away\": output \"GONE-1\" is not connected; its workspace shows on the primary output",
            "2,3,1,2|1|undefined",
            "WorkspaceSetError: set \"away\" names workspace \"3\", which no workspace has true",
            "split",
            "2@DP-1,3@DP-1,1@DP-1,2@HDMI-1",
        ])
    );
}

// -- the stock keys -----------------------------------------------------------------------------------------------

#[test]
fn the_stock_config_binds_the_lua_shells_workspace_strip_and_gesture_keys() {
    // The Lua default shell binds `<Super>1` to `<Super>9` (each shows the workspace at that position), `<Super>Left` and
    // `<Super>Right` (focus between a Strip's columns, nothing on another layout) and the `Swipe3` slide; the launcher
    // is `<Super>space`, which this config leaves unbound until a command is configured.
    let got = run(r#"
        const plan = desugarConfig(defaultConfig);
        const by = (trigger: string) => plan.bindings.find((b) => b.trigger === trigger) as any;
        emit([
            plan.bindings.map((b) => b.trigger).join(" "),
            JSON.stringify(by("Super+1").action),
            JSON.stringify(by("Super+9").action),
            JSON.stringify(by("Super+Left").action),
            JSON.stringify(by("Super+Right").action),
            by("Swipe3").action.kind + ":" + by("Swipe3").action.create().constructor.name + ":" + by("Swipe3").action.create().trigger,
            plan.bindings.some((b) => b.trigger === "Super+space"),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "Super+1 Super+2 Super+3 Super+4 Super+5 Super+6 Super+7 Super+8 Super+9 Super+Left Super+Right Swipe3 XF86AudioRaiseVolume XF86AudioLowerVolume XF86AudioMute XF86AudioMicMute XF86AudioPlay XF86AudioNext XF86AudioPrev",
            r#"{"kind":"show-position","position":1}"#,
            r#"{"kind":"show-position","position":9}"#,
            r#"{"kind":"focus-column","direction":"left"}"#,
            r#"{"kind":"focus-column","direction":"right"}"#,
            "binding:WorkspaceSlideBinding:Swipe3",
            "false",
        ])
    );
}

#[test]
fn the_launcher_command_is_bound_to_super_space_through_spawn_unless_keys_binds_it() {
    // The Lua launcher spawned the text typed into its entry on Return. The in-shell entry is not ported; a configured
    // command (a command line or an argv) is run by `<Super>space` through `spawn`.
    let got = run(r#"
        const action = (config: any) => JSON.stringify(desugarConfig(config).bindings.map((b) => `${b.trigger} ${JSON.stringify(b.action)}`));
        emit([
            action({ launcher: "wofi --show drun" }),
            action({ launcher: ["rofi", "-show", "drun"] }),
            action({ launcher: "wofi", keys: { "super+space": spawn("fuzzel") } }),
            action({}),
            thrown(() => desugarConfig({ launcher: 3 as any })),
            thrown(() => desugarConfig({ launcher: "   " })),
            thrown(() => desugarConfig({ launcher: [] })),
            JSON.stringify((mergeConfig(defaultConfig, { launcher: "wofi" }) as any).launcher),
            JSON.stringify((mergeConfig({ ...defaultConfig, launcher: "base" } as any, {}) as any).launcher),
            JSON.stringify((mergeConfig({ ...defaultConfig, launcher: "base" } as any, { launcher: "mine" }) as any).launcher),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            r#"["Super+space {\"kind\":\"spawn\",\"argv\":[\"wofi\",\"--show\",\"drun\"]}"]"#,
            r#"["Super+space {\"kind\":\"spawn\",\"argv\":[\"rofi\",\"-show\",\"drun\"]}"]"#,
            r#"["super+space {\"kind\":\"spawn\",\"argv\":[\"fuzzel\"]}"]"#,
            "[]",
            "TypeError: config.launcher must be a command line or an argv array",
            "TypeError: config.launcher must be a non-empty command line or an argv of strings",
            "TypeError: config.launcher must be a non-empty command line or an argv of strings",
            r#""wofi""#,
            r#""base""#,
            r#""mine""#,
        ])
    );
}

#[test]
fn the_new_key_actions_check_their_arguments() {
    let got = run(r#"
        emit([
            JSON.stringify(show.position(3)),
            JSON.stringify(focusColumn("right")),
            binding(() => new WorkspaceSlideBinding()).kind,
            thrown(() => show.position(0)),
            thrown(() => show.position(10)),
            thrown(() => show.position(1.5)),
            thrown(() => show.positions("Super")),
            thrown(() => focusColumn("up" as any)),
            thrown(() => binding(3 as any)),
            thrown(() => desugarConfig({ keys: { "Super+X": { kind: "focus-column", direction: "up" } as any } })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            r#"{"kind":"show-position","position":3}"#,
            r#"{"kind":"focus-column","direction":"right"}"#,
            "binding",
            "RangeError: show.position needs a whole number from 1 to 9, got 0",
            "RangeError: show.position needs a whole number from 1 to 9, got 10",
            "RangeError: show.position needs a whole number from 1 to 9, got 1.5",
            "TypeError: show.positions needs a template with {n}, such as \"Super+{n}\"",
            "TypeError: focusColumn needs \"left\" or \"right\", got up",
            "TypeError: binding needs a function that makes the Binding",
            "TypeError: keys[\"Super+X\"] must be spawn(...), show(...), show.position(...), focusColumn(...), binding(...), \"close\" or a function",
        ])
    );
}
