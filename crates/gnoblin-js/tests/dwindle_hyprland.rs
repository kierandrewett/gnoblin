//! `DwindleLayout` against Hyprland's dwindle layout (`data/js/gnoblin-std/layouts/dwindle.ts`; `docs/recipes/dwindle-like-hyprland.md`):
//! splits that follow the shape of their cell, the options `forceSplit`, `preserveSplit`, `smartSplit`,
//! `splitWidthMultiplier`, `useActiveForSplits`, `splitBias`, `permanentDirectionOverride` and `smartResizing`, and the
//! actions `toggleSplit`, `swapSplit`, `rotateSplit`, `preselect`, `moveToRoot`, `splitRatio` and `pseudotile`.
//!
//! Every expectation was worked out by hand from Hyprland's `DwindleAlgorithm.cpp` (the `recalcSizePosRecursive`,
//! `addTarget`, `layoutMsg`, `rotateSplit` and `moveToRoot` functions) on Gnoblin's 0.1 to 0.9 fractions: a Hyprland
//! `splitRatio` is twice a Gnoblin `ratio`. Rectangles are written `x,y,width,height` for each window, in the order of the
//! list, on an area of 1000 by 800 unless the case says otherwise.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, stock_modules};

/// The helpers every case shares. There is no `gnoblin` host: the cases call the library only.
const HARNESS: &str = r##"
export const area = (width = 1000, height = 800) => ({ x: 0, y: 0, width, height });
export const nodes = (n) => Array.from({ length: n }, (_, i) => ({ name: `w${i + 1}` }));
const num = (v) => String(Math.round(v * 1000) / 1000);
export const rect = (r) => [r.x, r.y, r.width, r.height].map(num).join(",");
// `n` nodes that know whether they hold the focus, as a client does; `ws` is what an action takes of a workspace.
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
    return { list, state, ws: { clients: list } };
}
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
// Windows `list[0 .. n]` arrive one at a time, each taking the focus as it does: the result of the last compute. A
// layout that was given windows before continues from the count of the last call.
const seen = new WeakMap();
export function arrive(layout, list, n, ctx = {}) {
    let out = "";
    for (let i = (seen.get(layout) ?? 0) + 1; i <= n; i++) {
        list[i - 1].focus?.();
        out = lay(layout, list.slice(0, i), { focused: list[i - 1], ...ctx });
    }
    seen.set(layout, n);
    return out;
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
import { area, nodes, focusables, tiled, lay, arrive, thrown, rect, emit } from "harness";
import { DwindleLayout } from "gnoblin-std";
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

// -- defaults and refusals --------------------------------------------------------------------------------------------

#[test]
fn the_options_default_to_hyprlands() {
    // Hyprland: force_split 0, preserve_split false, smart_split false, smart_resizing true, permanent_direction_override
    // false, split_width_multiplier 1.0, use_active_for_splits true, split_bias 0, default_split_ratio 1.0 (here 0.5).
    let got = run(r#"
        const d = new DwindleLayout();
        emit([
            d.forceSplit, d.preserveSplit, d.smartSplit, d.smartResizing, d.permanentDirectionOverride,
            d.splitWidthMultiplier, d.useActiveForSplits, d.splitBias, d.ratio, d.split, d.pointer,
        ].join(","));
    "#);
    assert_eq!(got, "0,false,false,true,false,1,true,0,0.5,aspect,");
}

#[test]
fn a_bad_option_is_refused_in_the_layouts_name() {
    let got = run(r#"
        const one = nodes(1);
        const bad = (options: object) => thrown(() => lay(new DwindleLayout(options as any), one));
        emit([
            bad({ forceSplit: 3 }),
            bad({ forceSplit: "left" }),
            bad({ splitBias: 2 }),
            bad({ splitWidthMultiplier: Number.NaN }),
            bad({ splitWidthMultiplier: "wide" }),
            bad({ preserveSplit: "yes" }),
            bad({ smartSplit: 1 }),
            bad({ useActiveForSplits: 0 }),
            bad({ smartResizing: "no" }),
            bad({ permanentDirectionOverride: 1 }),
            bad({ pointer: 5 }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "RangeError: DwindleLayout forceSplit must be 0, 1 or 2, got 3",
            "RangeError: DwindleLayout forceSplit must be 0, 1 or 2, got left",
            "RangeError: DwindleLayout splitBias must be 0 or 1, got 2",
            "RangeError: DwindleLayout splitWidthMultiplier must be a finite number",
            "RangeError: DwindleLayout splitWidthMultiplier must be a finite number",
            "TypeError: DwindleLayout preserveSplit must be true or false",
            "TypeError: DwindleLayout smartSplit must be true or false",
            "TypeError: DwindleLayout useActiveForSplits must be true or false",
            "TypeError: DwindleLayout smartResizing must be true or false",
            "TypeError: DwindleLayout permanentDirectionOverride must be true or false",
            "TypeError: DwindleLayout pointer must be a function or null",
        ])
    );
}

// -- splits follow their cell ---------------------------------------------------------------------------------------------

#[test]
fn splits_follow_the_shape_of_their_cell_unless_preserved() {
    // Hyprland: "Dwindle splits are NOT PERMANENT. The split is determined dynamically with the W/H ratio of the parent
    // node", unless `preserve_split`. Two windows side by side in 1000 by 800; the area then turns tall (600 by 1000).
    let got = run(r#"
        const two = nodes(2);
        const out = [];
        for (const options of [{}, { preserveSplit: true }]) {
            const l = new DwindleLayout(options);
            out.push(lay(l, two.slice(0, 1)) + " | " + arrive(l, two, 2));
            out.push(lay(l, two, { area: area(600, 1000) }));
            out.push(lay(l, two));
        }
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,1000,800 | 0,0,500,800 500,0,500,800",
            "0,0,600,500 0,500,600,500",
            "0,0,500,800 500,0,500,800",
            "0,0,1000,800 | 0,0,500,800 500,0,500,800",
            "0,0,300,1000 300,0,300,1000",
            "0,0,500,800 500,0,500,800",
        ])
    );
}

#[test]
fn a_square_cell_splits_side_by_side_and_with_preserve_split_top_and_bottom() {
    // Hyprland's `recalcSizePosRecursive` is `splitTop = box.h * mult > box.w`, so a square is side by side; its
    // `addTarget` takes `SIDEBYSIDE = box.w > box.h * mult` for a split that is to be preserved, so a square is top and
    // bottom there. Both read from DwindleAlgorithm.cpp on the main branch.
    let got = run(r#"
        const two = nodes(2);
        const square = { area: area(800, 800) };
        emit([
            arrive(new DwindleLayout(), two, 2, square),
            arrive(new DwindleLayout({ preserveSplit: true }), two, 2, square),
        ].join("\n"));
    "#);
    assert_eq!(got, lines(&["0,0,400,800 400,0,400,800", "0,0,800,400 0,400,800,400"]));
}

#[test]
fn closing_a_window_collapses_its_split_and_the_survivors_split_by_their_new_shape() {
    // w1 | (w2 over w3). Closing w1 leaves w2 and w3 in a root that is wide again, so they sit side by side; with
    // preserveSplit they stay on top of each other. Closing w2 leaves w1 | w3 in both.
    let got = run(r#"
        const all = nodes(3);
        const out = [];
        for (const options of [{}, { preserveSplit: true }]) {
            const l = new DwindleLayout(options);
            out.push(arrive(l, all, 3));
            out.push(lay(l, [all[1], all[2]], { focused: all[2] }));
            const m = new DwindleLayout(options);
            arrive(m, all, 3);
            out.push(lay(m, [all[0], all[2]], { focused: all[2] }));
        }
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,500,800 500,0,500,400 500,400,500,400",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,400 500,400,500,400",
            "0,0,1000,400 0,400,1000,400",
            "0,0,500,800 500,0,500,800",
        ])
    );
}

#[test]
fn a_dragged_ratio_persists_through_new_windows_and_closes() {
    // The drag of `SplitResizeBinding` sets the ratio of one split (`findSplit`). It stays while windows come and go
    // around it, whatever direction the cells inside it come to split in.
    let got = run(r#"
        const all = nodes(4);
        const l = new DwindleLayout();
        arrive(l, all, 3);
        const divider = l.findSplit(all[0], { x: 500, y: 100 });
        const revision = l.revision;
        divider.ratio = 0.3;
        const out = [l.revision > revision, lay(l, all.slice(0, 3), { focused: all[2] })];
        // w4 arrives with w3 focused: it halves w3's cell (700 by 400), which is wide, side by side.
        out.push(arrive(l, all, 4));
        out.push(lay(l, [all[0], all[1], all[3]], { focused: all[3] }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true",
            "0,0,300,800 300,0,700,400 300,400,700,400",
            "0,0,300,800 300,0,700,400 300,400,350,400 650,400,350,400",
            "0,0,300,800 300,0,700,400 300,400,700,400",
        ])
    );
}

// -- forceSplit -----------------------------------------------------------------------------------------------------------

#[test]
fn force_split_one_puts_the_new_window_left_or_on_top() {
    let got = run(r#"
        const two = nodes(2);
        const wide = new DwindleLayout({ forceSplit: 1 });
        const tall = new DwindleLayout({ forceSplit: 1 });
        const second = new DwindleLayout({ forceSplit: 2 });
        emit([
            arrive(wide, two, 2),
            arrive(tall, two, 2, { area: area(600, 1000) }),
            arrive(second, two, 2),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "500,0,500,800 0,0,500,800",
            "0,500,600,500 0,0,600,500",
            "0,0,500,800 500,0,500,800",
        ])
    );
}

#[test]
fn force_split_zero_follows_the_pointer_and_puts_the_window_second_without_one() {
    // Hyprland: the new window goes on the side of the cell the pointer is on (the cell's first half: first).
    let got = run(r#"
        const two = nodes(2);
        const at = (x: number, y: number) => new DwindleLayout({ pointer: () => ({ x, y }) });
        emit([
            arrive(at(100, 100), two, 2),
            arrive(at(900, 100), two, 2),
            arrive(at(100, 100), two, 2, { area: area(600, 1000) }),
            arrive(at(100, 900), two, 2, { area: area(600, 1000) }),
            arrive(new DwindleLayout({ pointer: () => null }), two, 2),
            arrive(new DwindleLayout(), two, 2),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "500,0,500,800 0,0,500,800",
            "0,0,500,800 500,0,500,800",
            "0,500,600,500 0,0,600,500",
            "0,0,600,500 0,500,600,500",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,800",
        ])
    );
}

// -- smartSplit -----------------------------------------------------------------------------------------------------------

#[test]
fn smart_split_chooses_the_direction_by_the_triangle_the_pointer_is_in() {
    // Hyprland: the cell is divided into four triangles by its diagonals; the pointer's triangle gives the direction.
    let got = run(r#"
        const two = nodes(2);
        const at = (x: number, y: number) => new DwindleLayout({ smartSplit: true, pointer: () => ({ x, y }) });
        emit([
            arrive(at(500, 50), two, 2),
            arrive(at(500, 750), two, 2),
            arrive(at(50, 400), two, 2),
            arrive(at(950, 400), two, 2),
            arrive(at(900, 700), two, 2),
            arrive(at(800, 780), two, 2),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,400,1000,400 0,0,1000,400",
            "0,0,1000,400 0,400,1000,400",
            "500,0,500,800 0,0,500,800",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,800",
            "0,0,1000,400 0,400,1000,400",
        ])
    );
}

#[test]
fn smart_split_turns_preserve_split_on() {
    let got = run(r#"
        const two = nodes(2);
        const l = new DwindleLayout({ smartSplit: true, pointer: () => ({ x: 950, y: 400 }) });
        arrive(l, two, 2);
        emit(lay(l, two, { area: area(600, 1000) }));
    "#);
    assert_eq!(got, "0,0,300,1000 300,0,300,1000");
}

#[test]
fn smart_split_without_a_pointer_falls_back_to_force_split() {
    let got = run(r#"
        const two = nodes(2);
        emit([
            arrive(new DwindleLayout({ smartSplit: true }), two, 2),
            arrive(new DwindleLayout({ smartSplit: true, forceSplit: 1 }), two, 2),
        ].join("\n"));
    "#);
    assert_eq!(got, lines(&["0,0,500,800 500,0,500,800", "500,0,500,800 0,0,500,800"]));
}

// -- splitWidthMultiplier -------------------------------------------------------------------------------------------------

#[test]
fn the_split_width_multiplier_scales_the_height_before_the_comparison() {
    // Hyprland: `splitTop = box.h * split_width_multiplier > box.w`. At 2, a 1000 by 800 cell is split top and bottom
    // (1600 > 1000); at 0.5 it is side by side (400 > 1000 is false). Out of 0.1 to 3.0 it is held to that range.
    let got = run(r#"
        const two = nodes(2);
        const by = (splitWidthMultiplier: number) => arrive(new DwindleLayout({ splitWidthMultiplier }), two, 2);
        emit([by(2), by(0.5), by(1), by(50), by(0)].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,1000,400 0,400,1000,400",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,800",
            "0,0,1000,400 0,400,1000,400",
            "0,0,500,800 500,0,500,800",
        ])
    );
}

// -- useActiveForSplits ---------------------------------------------------------------------------------------------------

#[test]
fn use_active_for_splits_chooses_the_focused_window_or_the_one_under_the_pointer() {
    // w1 | w2 with w2 focused when w3 arrives and the pointer over w1.
    let got = run(r#"
        const all = nodes(3);
        const make = (options: object) => {
            const l = new DwindleLayout({ forceSplit: 2, pointer: () => ({ x: 100, y: 100 }), ...options });
            arrive(l, all, 2);
            return lay(l, all, { focused: all[2] });
        };
        const noPointer = new DwindleLayout({ useActiveForSplits: false });
        arrive(noPointer, all, 2);
        const far = new DwindleLayout({ useActiveForSplits: false, pointer: () => ({ x: 5000, y: 5000 }) });
        arrive(far, all, 2);
        emit([
            make({}),
            make({ useActiveForSplits: false }),
            lay(noPointer, all, { focused: all[2] }),
            lay(far, all, { focused: all[2] }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,500,800 500,0,500,400 500,400,500,400",
            "0,0,500,400 500,0,500,800 0,400,500,400",
            "0,0,500,400 500,0,500,800 0,400,500,400",
            "0,0,500,800 500,0,500,400 500,400,500,400",
        ])
    );
}

// -- splitBias --------------------------------------------------------------------------------------------------------------

#[test]
fn split_bias_gives_the_ratio_to_the_directional_window_or_the_current_one() {
    // Hyprland: bias 0 gives the ratio to the first (top or left) child, bias 1 to the window the new one was opened
    // on. With a ratio of 0.3 (Hyprland 0.6) and the new window first, bias 1 leaves the current window 0.3.
    let got = run(r#"
        const two = nodes(2);
        const make = (forceSplit: number, splitBias: number) =>
            arrive(new DwindleLayout({ ratio: 0.3, forceSplit, splitBias }), two, 2);
        emit([make(1, 0), make(1, 1), make(2, 0), make(2, 1)].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "300,0,700,800 0,0,300,800",
            "700,0,300,800 0,0,700,800",
            "0,0,300,800 300,0,700,800",
            "0,0,300,800 300,0,700,800",
        ])
    );
}

// -- preselect ------------------------------------------------------------------------------------------------------------

#[test]
fn preselect_overrides_the_split_of_the_next_window_only() {
    let got = run(r#"
        const all = nodes(3);
        const l = new DwindleLayout();
        l.preselect("down");
        const out = [l.preselected, lay(l, all.slice(0, 1))];
        out.push(arrive(l, all, 2));
        out.push(l.preselected);
        // The split the override made stays top and bottom, and the next window splits by its cell: 1000 by 400.
        out.push(arrive(l, all, 3));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "down",
            "0,0,1000,800",
            "0,0,1000,400 0,400,1000,400",
            "none",
            "0,0,1000,400 0,400,500,400 500,400,500,400",
        ])
    );
}

#[test]
fn preselect_takes_each_of_the_four_directions() {
    // On a tall area, where the cell's own shape would split top and bottom: left and right force side by side.
    let got = run(r#"
        const two = nodes(2);
        const to = (direction: string, ctx = {}) => {
            const l = new DwindleLayout();
            l.preselect(direction as any);
            return arrive(l, two, 2, ctx);
        };
        const tall = { area: area(600, 1000) };
        emit([to("up"), to("down"), to("left", tall), to("right", tall), to("none"), to("none", tall)].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,400,1000,400 0,0,1000,400",
            "0,0,1000,400 0,400,1000,400",
            "300,0,300,1000 0,0,300,1000",
            "0,0,300,1000 300,0,300,1000",
            "0,0,500,800 500,0,500,800",
            "0,0,600,500 0,500,600,500",
        ])
    );
}

#[test]
fn permanent_direction_override_keeps_the_preselection_until_it_is_cleared() {
    let got = run(r#"
        const all = nodes(4);
        const l = new DwindleLayout({ permanentDirectionOverride: true });
        l.preselect("down");
        const out = [arrive(l, all, 3), l.preselected];
        l.preselect("none");
        out.push(l.preselected);
        out.push(arrive(l, all, 4));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,1000,400 0,400,1000,200 0,600,1000,200",
            "down",
            "none",
            "0,0,1000,400 0,400,1000,200 0,600,500,200 500,600,500,200",
        ])
    );
}

#[test]
fn preselect_refuses_a_word_that_is_not_a_direction() {
    let got = run(r#"
        const l = new DwindleLayout();
        emit(thrown(() => l.preselect("sideways" as any)));
    "#);
    assert_eq!(
        got,
        "TypeError: DwindleLayout preselect direction must be left, right, up, down or none, got sideways"
    );
}

// -- the layout actions ---------------------------------------------------------------------------------------------------

#[test]
fn toggle_split_turns_the_split_of_the_focused_window_when_it_is_preserved() {
    let got = run(r#"
        const { list, ws } = focusables(2);
        const keep = new DwindleLayout({ preserveSplit: true });
        arrive(keep, list, 2);
        const revision = keep.revision;
        const out = [keep.toggleSplit(ws), keep.revision > revision, lay(keep, list, { focused: list[1] })];
        out.push(keep.toggleSplit(ws), lay(keep, list, { focused: list[1] }));
        // Without preserveSplit the shape of the cell decides again at once, as in Hyprland.
        const follow = new DwindleLayout();
        arrive(follow, list, 2);
        out.push(follow.toggleSplit(ws), lay(follow, list, { focused: list[1] }));
        // Nothing focused, or a window alone: there is no split to turn.
        const alone = new DwindleLayout({ preserveSplit: true });
        arrive(alone, list, 1);
        out.push(alone.toggleSplit(ws), alone.toggleSplit({ clients: [] }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true",
            "true",
            "0,0,1000,400 0,400,1000,400",
            "true",
            "0,0,500,800 500,0,500,800",
            "true",
            "0,0,500,800 500,0,500,800",
            "false",
            "false",
        ])
    );
}

#[test]
fn swap_split_trades_the_two_halves_of_the_focused_windows_split() {
    let got = run(r#"
        const { list, ws } = focusables(3);
        const l = new DwindleLayout();
        arrive(l, list, 3);
        // w3 is focused: it and w2 share the right half.
        const out = [l.swapSplit(ws), lay(l, list, { focused: list[2] })];
        list[0].focus();
        out.push(l.swapSplit(ws), lay(l, list, { focused: list[0] }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true",
            "0,0,500,800 500,400,500,400 500,0,500,400",
            "true",
            "500,0,500,800 0,400,500,400 0,0,500,400",
        ])
    );
}

#[test]
fn rotate_split_turns_by_multiples_of_ninety_degrees() {
    // Hyprland's `rotateSplit`: 90 turns the split clockwise (first child top, then right, then bottom), -90 the other
    // way, 180 swaps the halves. It acts on a preserved split.
    let got = run(r#"
        const out = [];
        for (const angle of [90, 180, 270, 360, -90, -180, 0]) {
            const { list, ws } = focusables(2);
            const l = new DwindleLayout({ preserveSplit: true });
            arrive(l, list, 2);
            l.rotateSplit(ws, angle);
            out.push(`${angle}: ` + lay(l, list, { focused: list[1] }));
        }
        const { list, ws } = focusables(2);
        const twice = new DwindleLayout({ preserveSplit: true });
        arrive(twice, list, 2);
        twice.rotateSplit(ws);
        twice.rotateSplit(ws);
        out.push("90, 90: " + lay(twice, list, { focused: list[1] }));
        out.push(thrown(() => twice.rotateSplit(ws, 45)), thrown(() => twice.rotateSplit(ws, Number.NaN)));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "90: 0,0,1000,400 0,400,1000,400",
            "180: 500,0,500,800 0,0,500,800",
            "270: 0,400,1000,400 0,0,1000,400",
            "360: 0,0,500,800 500,0,500,800",
            "-90: 0,400,1000,400 0,0,1000,400",
            "-180: 500,0,500,800 0,0,500,800",
            "0: 0,0,500,800 500,0,500,800",
            "90, 90: 500,0,500,800 0,0,500,800",
            "RangeError: DwindleLayout rotateSplit angle must be a multiple of 90, got 45",
            "RangeError: DwindleLayout rotateSplit angle must be a multiple of 90, got NaN",
        ])
    );
}

#[test]
fn move_to_root_makes_the_window_a_child_of_the_root_split() {
    // w1 | (w2 over (w3 | w4)). Moving w4 to the root puts it where w1 was and w1 where w4 was. Stable (the default)
    // leaves w4 on the same side of the screen, unstable on the other.
    let got = run(r#"
        const make = () => {
            const { list, ws } = focusables(4);
            const l = new DwindleLayout();
            arrive(l, list, 4);
            return { l, list, ws };
        };
        const out = [];
        {
            const { l, list, ws } = make();
            out.push(lay(l, list, { focused: list[3] }));
            out.push(l.moveToRoot(ws), lay(l, list, { focused: list[3] }));
            out.push(l.moveToRoot(ws));
        }
        {
            const { l, list, ws } = make();
            out.push(l.moveToRoot(ws, { stable: false }), lay(l, list, { focused: list[3] }));
        }
        {
            const { l, list, ws } = make();
            list[0].focus();
            out.push(l.moveToRoot(ws));
            list[1].focus();
            out.push(l.moveToRoot(ws), lay(l, list, { focused: list[1] }));
        }
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,500,800 500,0,500,400 500,400,250,400 750,400,250,400",
            "true",
            "250,400,250,400 0,0,500,400 0,400,250,400 500,0,500,800",
            "false",
            "true",
            "750,400,250,400 500,0,500,400 500,400,250,400 0,0,500,800",
            "false",
            "true",
            "0,0,500,400 500,0,500,800 0,400,250,400 250,400,250,400",
        ])
    );
}

#[test]
fn split_ratio_sets_or_changes_the_ratio_of_the_focused_windows_split() {
    // Hyprland's `splitratio`, in Gnoblin's fractions (Hyprland's 0.6 is 0.3 here): a delta by default, `exact` to set
    // it, held to 0.1 to 0.9.
    let got = run(r#"
        const { list, ws } = focusables(2);
        const l = new DwindleLayout();
        arrive(l, list, 2);
        const step = (...args: [number, { exact: boolean }?]) => {
            const done = l.splitRatio(ws, ...args);
            return `${done} ` + lay(l, list, { focused: list[1] });
        };
        const out = [
            step(0.3, { exact: true }),
            step(0.25),
            step(-0.25),
            step(-0.5),
            step(5, { exact: true }),
            step(-5),
            thrown(() => l.splitRatio(ws, Number.NaN)),
            thrown(() => l.splitRatio(ws, "wide" as any)),
        ];
        // A window with no split above it, and no focus, have no ratio to change.
        const alone = new DwindleLayout();
        arrive(alone, list, 1);
        out.push(alone.splitRatio(ws, 0.1), l.splitRatio({ clients: [] }, 0.1));
        // A later change of the layout's own ratio does not move a split that was set.
        l.ratio = 0.7;
        out.push(lay(l, list, { focused: list[1] }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true 0,0,300,800 300,0,700,800",
            "true 0,0,550,800 550,0,450,800",
            "true 0,0,300,800 300,0,700,800",
            "true 0,0,100,800 100,0,900,800",
            "true 0,0,900,800 900,0,100,800",
            "true 0,0,100,800 100,0,900,800",
            "TypeError: DwindleLayout splitRatio needs a finite number, got NaN",
            "TypeError: DwindleLayout splitRatio needs a finite number, got wide",
            "false",
            "false",
            "0,0,100,800 100,0,900,800",
        ])
    );
}

#[test]
fn a_split_that_was_set_keeps_its_ratio_and_a_new_one_uses_the_layouts() {
    // The ratio belongs to one split: w3 halves w2's cell (750 by 800, so top and bottom) with the layout's 0.4.
    let got = run(r#"
        const { list, ws } = focusables(3);
        const l = new DwindleLayout({ ratio: 0.4 });
        arrive(l, list, 2);
        l.splitRatio(ws, 0.25, { exact: true });
        emit(arrive(l, list, 3));
    "#);
    assert_eq!(got, "0,0,250,800 250,0,750,320 250,320,750,480");
}

#[test]
fn pseudotile_keeps_the_windows_own_size_centred_in_its_cell() {
    let got = run(r#"
        const { list, ws } = focusables(3);
        Object.assign(list[1], { width: 300, height: 200 });
        const l = new DwindleLayout();
        arrive(l, list, 2);
        const out = [l.pseudotiled(list[1]), l.pseudotile(ws), l.pseudotiled(list[1])];
        out.push(lay(l, list.slice(0, 2), { focused: list[1] }));
        // w3 halves the cell of w2: the size stays, and the centre moves with the cell.
        out.push(arrive(l, list, 3));
        // A cell smaller than the window clamps it.
        out.push(lay(l, list, { focused: list[2], area: area(300, 200) }));
        // Turning it off gives the window its whole cell back.
        list[1].focus();
        out.push(l.pseudotile(ws), l.pseudotiled(list[1]), lay(l, list, { focused: list[1] }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "false",
            "true",
            "true",
            "0,0,500,800 600,300,300,200",
            "0,0,500,800 600,100,300,200 500,400,500,400",
            "0,0,150,200 150,0,150,100 150,100,150,100",
            "false",
            "false",
            "0,0,500,800 500,0,500,400 500,400,500,400",
        ])
    );
}

#[test]
fn pseudotile_takes_an_explicit_size_and_the_cell_size_for_a_window_that_has_none() {
    let got = run(r#"
        const { list, ws } = focusables(2);
        const l = new DwindleLayout();
        arrive(l, list, 2);
        l.pseudotile(ws, { width: 200, height: 100 });
        const out = [lay(l, list, { focused: list[1] })];
        l.pseudotile(ws);
        // No size on the window and none given: it keeps the size its cell has at the next compute.
        l.pseudotile(ws);
        out.push(lay(l, list, { focused: list[1] }));
        out.push(lay(l, list, { focused: list[1], area: area(1000, 400) }));
        out.push(thrown(() => l.pseudotile(ws, { width: -1, height: 100 })));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,500,800 650,350,200,100",
            "0,0,500,800 500,0,500,800",
            "0,0,500,400 500,0,500,400",
            "RangeError: DwindleLayout pseudotile size must be finite numbers above 0, got -1 by 100",
        ])
    );
}

#[test]
fn every_action_that_changes_the_tree_bumps_the_revision_so_the_workspace_computes() {
    let got = run(r#"
        const { list, ws } = focusables(3);
        const l = new DwindleLayout({ preserveSplit: true });
        arrive(l, list, 3);
        const moved: string[] = [];
        const mark = (name: string, act: () => unknown) => {
            const before = l.revision;
            act();
            moved.push(`${name} ${l.revision > before}`);
        };
        mark("toggleSplit", () => l.toggleSplit(ws));
        mark("swapSplit", () => l.swapSplit(ws));
        mark("rotateSplit", () => l.rotateSplit(ws, 90));
        mark("moveToRoot", () => l.moveToRoot(ws));
        mark("splitRatio", () => l.splitRatio(ws, 0.1));
        mark("pseudotile", () => l.pseudotile(ws, { width: 10, height: 10 }));
        mark("a refused toggleSplit", () => l.toggleSplit({ clients: [] }));
        emit(moved.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "toggleSplit true",
            "swapSplit true",
            "rotateSplit true",
            "moveToRoot true",
            "splitRatio true",
            "pseudotile true",
            "a refused toggleSplit false",
        ])
    );
}
