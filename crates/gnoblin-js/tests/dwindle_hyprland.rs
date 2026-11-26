//! `DwindleLayout` against Hyprland's dwindle layout (`crates/gnoblin-std-impl/src/builtin/dwindle.rs`, the port of
//! `data/js/gnoblin-std/layouts/dwindle.ts`; `docs/recipes/dwindle-like-hyprland.md`): splits that follow the shape of
//! their cell, the options `forceSplit`, `preserveSplit`, `smartSplit`, `splitWidthMultiplier`, `useActiveForSplits`,
//! `splitBias`, `permanentDirectionOverride` and `smartResizing`, and the actions `toggleSplit`, `swapSplit`,
//! `rotateSplit`, `preselect`, `moveToRoot`, `splitRatio` and `pseudotile`.
//!
//! Every expectation was worked out by hand from Hyprland's `DwindleAlgorithm.cpp` (the `recalcSizePosRecursive`,
//! `addTarget`, `layoutMsg`, `rotateSplit` and `moveToRoot` functions) on Gnoblin's 0.1 to 0.9 fractions: a Hyprland
//! `splitRatio` is twice a Gnoblin `ratio`. Rectangles are written `x,y,width,height` for each window, in the order of the
//! list, on an area of 1000 by 800 unless the case says otherwise.
//!
//! The generated layout takes only real clients and acts on the workspace that has it. The cases that only ask for a
//! placement call `compute` on real clients of a fake compositor (`run`). The cases that act on a workspace (the actions of
//! the layout read the workspace's windows and the window with the keyboard focus) run a workspace of their own over the
//! fake compositor and read the rectangles the compositor holds (`Action`).

use std::fs;
use std::path::Path;

use gnoblin_js::JsConfig;
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::{FakeCompositor, LIBRARY_GLOBALS, ROOT, Rig, rig_setup, text};

/// The helpers every case shares. A layout is made on a workspace of its own (`mk`) that no window joins, and `compute` is
/// called on the first windows of the compositor.
const HARNESS: &str = r##"
import { DwindleLayout, Workspace } from "gnoblin-std";
export const area = (width = 1000, height = 800) => ({ x: 0, y: 0, width, height });
export const nodes = (n) => gnoblin.clients.toList().slice(0, n);
const num = (v) => String(Math.round(v * 1000) / 1000);
export const rect = (r) => [r.x, r.y, r.width, r.height].map(num).join(",");
let serial = 0;
export function mk(options = {}) {
    const layout = new DwindleLayout(options);
    gnoblin.workspaces.add(new Workspace(`d${serial++}`, { layout }));
    return layout;
}
export function tiled(placements, list) {
    return placements
        .map((p, i) => {
            if (p.node.id !== list[i].id) throw new Error(`placement ${i} is for the wrong node`);
            if (p.tiled !== true) throw new Error(`placement ${i} is not tiled`);
            for (const v of [p.bounds.x, p.bounds.y, p.bounds.width, p.bounds.height]) {
                if (!Number.isInteger(v)) throw new Error(`placement ${i} is not whole pixels: ${rect(p.bounds)}`);
            }
            return rect(p.bounds);
        })
        .join(" ");
}
export function lay(layout, list, ctx = {}) {
    return tiled(layout.compute({ workspace: "d", children: list, area: area(), ...ctx }), list);
}
// Windows `list[0 .. n]` arrive one at a time, each taking the focus as it does: the result of the last compute. A
// layout that was given windows before continues from the count of the last call.
const seen = new WeakMap();
export function arrive(layout, list, n, ctx = {}) {
    let out = "";
    for (let i = (seen.get(layout) ?? 0) + 1; i <= n; i++) {
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

/// The entry of a case: the harness, one workspace that takes the windows, and the case as a function the test calls once the
/// windows are known.
fn entry(body: &str) -> String {
    format!(
        "import {{ area, nodes, tiled, lay, arrive, thrown, rect, emit, mk }} from \"harness\";\n\
         import {{ DwindleLayout, Workspace, FloatingLayout }} from \"gnoblin-std\";\n\
         gnoblin.workspaces.add(new Workspace(\"main\", {{ layout: new FloatingLayout() }}));\n\
         (globalThis as any).__case = () => {{\n{body}\n}};\n\
         void DwindleLayout;\n"
    )
}

/// Four windows (ids 3 to 6) and one screen of 1000 by 800 in a fake compositor.
fn world() -> FakeCompositor {
    let comp = FakeCompositor::new();
    comp.add_screen(20, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
    for id in 3..=6 {
        comp.add_node(
            id,
            ROOT,
            "window",
            vec![
                ("title", text(&format!("w{}", id - 2))),
                ("app_id", text("app")),
                ("width", MirrorValue::Double(400.0)),
                ("height", MirrorValue::Double(300.0)),
            ],
        );
    }
    comp
}

/// Evaluates `body` (TypeScript) with the stock library imported and returns what it passed to `emit`. The shell runs over a core with
/// the std library loaded, which is what computes the layout.
fn run(body: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), entry(body)).expect("shell.ts is written");
    let (mut config, clock) = common::timed_config(&root);
    config.js = std::mem::replace(&mut config.js, JsConfig::new(&root)).embed("harness", HARNESS);
    let comp = world();
    let (mut shell, core) = common::native::shell_over_core(&comp, std::sync::Arc::new(clock), config);
    let diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for _ in 0..4 {
        shell.frame().expect("a frame");
        core.dispatch(&shell);
    }
    shell
        .eval("globalThis.__case(); globalThis.out")
        .expect("the case runs")
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
    assert_eq!(got, "by_pointer,false,false,true,false,1,true,first,0.5,aspect,none");
}

#[test]
fn a_bad_option_is_refused_in_the_layouts_name() {
    // The texts are the library's: it, not the script, decides what an option may be.
    let got = run(r#"
        const bad = (options: object) => thrown(() => new DwindleLayout(options as any));
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
            "TypeError: 3 is not a value of dwindle_force_split (by_pointer, first, second)",
            "TypeError: \"left\" is not a value of dwindle_force_split (by_pointer, first, second)",
            "TypeError: 2 is not a value of dwindle_split_bias (first, opened)",
            "TypeError: dwindle.split_width_multiplier must be a finite number, got NaN",
            "TypeError: `DwindleLayout.splitWidthMultiplier` must be a number, got text",
            "TypeError: `DwindleLayout.preserveSplit` must be a boolean, got text",
            "TypeError: `DwindleLayout.smartSplit` must be a boolean, got an integer",
            "TypeError: `DwindleLayout.useActiveForSplits` must be a boolean, got an integer",
            "TypeError: `DwindleLayout.smartResizing` must be a boolean, got text",
            "TypeError: `DwindleLayout.permanentDirectionOverride` must be a boolean, got an integer",
            "TypeError: 5 is not a value of dwindle_pointer (none, seat, fixed)",
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
            const l = mk(options);
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
            arrive(mk(), two, 2, square),
            arrive(mk({ preserveSplit: true }), two, 2, square),
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
            const l = mk(options);
            out.push(arrive(l, all, 3));
            out.push(lay(l, [all[1], all[2]], { focused: all[2] }));
            const m = mk(options);
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
        const l = mk();
        arrive(l, all, 3);
        const divider = l.findSplit(all[0], 500, 100)[0];
        l.setSplit(divider.token, 0.3);
        const out = [lay(l, all.slice(0, 3), { focused: all[2] })];
        // w4 arrives with w3 focused: it halves w3's cell (700 by 400), which is wide, side by side.
        out.push(arrive(l, all, 4));
        out.push(lay(l, [all[0], all[1], all[3]], { focused: all[3] }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,300,800 300,0,700,400 300,400,700,400",
            "0,0,300,800 300,0,700,400 300,400,350,400 650,400,350,400",
            "0,0,300,800 300,0,700,400 300,400,700,400",
        ])
    );
}

#[test]
#[ignore = "vrnw: needs Layout.setSplit to bump the layout's revision (the revision stays, so a workspace that only asked the layout would not compute again)"]
fn a_dragged_ratio_bumps_the_revision() {
    let got = run(r#"
        const all = nodes(3);
        const l = mk();
        arrive(l, all, 3);
        const divider = l.findSplit(all[0], 500, 100)[0];
        const revision = l.revision();
        l.setSplit(divider.token, 0.3);
        emit(l.revision() > revision);
    "#);
    assert_eq!(got, "true");
}

// -- forceSplit -----------------------------------------------------------------------------------------------------------

#[test]
fn force_split_one_puts_the_new_window_left_or_on_top() {
    let got = run(r#"
        const two = nodes(2);
        const wide = mk({ forceSplit: "first" });
        const tall = mk({ forceSplit: "first" });
        const second = mk({ forceSplit: "second" });
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
        const at = (x: number, y: number) => mk({ pointer: "fixed", pointerX: x, pointerY: y });
        emit([
            arrive(at(100, 100), two, 2),
            arrive(at(900, 100), two, 2),
            arrive(at(100, 100), two, 2, { area: area(600, 1000) }),
            arrive(at(100, 900), two, 2, { area: area(600, 1000) }),
            arrive(mk({ pointer: "none" }), two, 2),
            arrive(mk(), two, 2),
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
        const at = (x: number, y: number) => mk({ smartSplit: true, pointer: "fixed", pointerX: x, pointerY: y });
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
        const l = mk({ smartSplit: true, pointer: "fixed", pointerX: 950, pointerY: 400 });
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
            arrive(mk({ smartSplit: true }), two, 2),
            arrive(mk({ smartSplit: true, forceSplit: "first" }), two, 2),
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
        const by = (splitWidthMultiplier: number) => arrive(mk({ splitWidthMultiplier }), two, 2);
        emit([by(2), by(0.5), by(1), by(50), by(0), by(-3)].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,1000,400 0,400,1000,400",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,800",
            "0,0,1000,400 0,400,1000,400",
            "0,0,500,800 500,0,500,800",
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
            const l = mk({ forceSplit: "second", pointer: "fixed", pointerX: 100, pointerY: 100, ...options });
            arrive(l, all, 2);
            return lay(l, all, { focused: all[2] });
        };
        const noPointer = mk({ useActiveForSplits: false });
        arrive(noPointer, all, 2);
        const far = mk({ useActiveForSplits: false, pointer: "fixed", pointerX: 5000, pointerY: 5000 });
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
        const make = (forceSplit: string, splitBias: string) =>
            arrive(mk({ ratio: 0.3, forceSplit, splitBias } as any), two, 2);
        emit([make("first", "first"), make("first", "opened"), make("second", "first"), make("second", "opened")].join("\n"));
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
        const l = mk();
        l.preselect("down");
        const out = [l.preselected(), lay(l, all.slice(0, 1))];
        out.push(arrive(l, all, 2));
        out.push(l.preselected());
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
            const l = mk();
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
        const l = mk({ permanentDirectionOverride: true });
        l.preselect("down");
        const out = [arrive(l, all, 3), l.preselected()];
        l.preselect("none");
        out.push(l.preselected());
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
        const l = mk();
        emit(thrown(() => l.preselect("sideways" as any)));
    "#);
    assert_eq!(
        got,
        "TypeError: \"sideways\" is not a value of dwindle_direction (left, right, up, down, none)"
    );
}

// -- the layout actions ---------------------------------------------------------------------------------------------------

/// A workspace `w` that is shown and has the layout `g.l` (`options` are its options), over a screen of 1000 by 800, on which
/// windows arrive one at a time with the keyboard focus, as they do on a desk. The actions of the layout read that
/// workspace's windows and the one with the focus.
struct Action {
    rig: Rig,
    windows: u64,
}

const SCREEN: u64 = 20;

impl Action {
    fn new(options: &str) -> Action {
        let init = format!(
            "(globalThis as any).g = {{ l: new DwindleLayout({options}) }};\ngnoblin.workspaces.add(new Workspace('w', {{ layout: (globalThis as any).g.l }}));\n"
        );
        let mut rig = rig_setup(LIBRARY_GLOBALS, &init, |comp| {
            comp.add_screen(SCREEN, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
        });
        rig.settle();
        Action { rig, windows: 0 }
    }

    /// The window `k` (counting from one) is the node `2 + k`.
    fn node(k: u64) -> u64 {
        2 + k
    }

    /// Windows arrive until there are `to`, each one taking the focus.
    fn arrive(&mut self, to: u64) {
        while self.windows < to {
            self.windows += 1;
            let node = Self::node(self.windows);
            self.rig
                .comp
                .create_bare_window(node, &format!("w{}", self.windows), "app");
            self.focus(self.windows);
        }
    }

    /// The window `k` has the focus (zero: none has).
    fn focus(&mut self, k: u64) {
        for other in 1..=self.windows {
            self.rig
                .comp
                .external_set(Self::node(other), "focused", MirrorValue::Bool(other == k));
        }
        self.rig.settle();
    }

    fn eval(&mut self, source: &str) -> String {
        let value = self.rig.eval(source);
        self.rig.settle();
        value
    }

    fn revision(&mut self) -> f64 {
        self.rig.eval("g.l.revision()").parse().expect("a number")
    }

    /// The rectangles of the first `n` windows.
    fn cells(&self, n: u64) -> String {
        (1..=n)
            .map(|k| self.rig.comp.rect(Self::node(k)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[test]
fn toggle_split_turns_the_split_of_the_focused_window_when_it_is_preserved() {
    let mut out = Vec::new();
    {
        let mut keep = Action::new("{ preserveSplit: true }");
        keep.arrive(2);
        out.push(keep.eval("String(g.l.toggleSplit())"));
        out.push(keep.cells(2));
        out.push(keep.eval("String(g.l.toggleSplit())"));
        out.push(keep.cells(2));
    }
    // Without preserveSplit the shape of the cell decides again at once, as in Hyprland.
    {
        let mut follow = Action::new("{}");
        follow.arrive(2);
        out.push(follow.eval("String(g.l.toggleSplit())"));
        out.push(follow.cells(2));
    }
    // Nothing focused, or a window alone: there is no split to turn.
    {
        let mut alone = Action::new("{ preserveSplit: true }");
        alone.arrive(1);
        out.push(alone.eval("String(g.l.toggleSplit())"));
        alone.focus(0);
        out.push(alone.eval("String(g.l.toggleSplit())"));
    }
    assert_eq!(
        out.join("\n"),
        lines(&[
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
    let mut action = Action::new("{}");
    action.arrive(3);
    // w3 is focused: it and w2 share the right half.
    let mut out = vec![action.eval("String(g.l.swapSplit())"), action.cells(3)];
    action.focus(1);
    out.push(action.eval("String(g.l.swapSplit())"));
    out.push(action.cells(3));
    assert_eq!(
        out.join("\n"),
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
    let mut out = Vec::new();
    for angle in [90, 180, 270, 360, -90, -180, 0] {
        let mut action = Action::new("{ preserveSplit: true }");
        action.arrive(2);
        action.eval(&format!("g.l.rotateSplit({angle})"));
        out.push(format!("{angle}: {}", action.cells(2)));
    }
    let mut twice = Action::new("{ preserveSplit: true }");
    twice.arrive(2);
    twice.eval("g.l.rotateSplit(90)");
    twice.eval("g.l.rotateSplit(90)");
    out.push(format!("90, 90: {}", twice.cells(2)));
    let thrown =
        |source: &str| format!("try {{ {source}; 'did not throw' }} catch (e) {{ e.name + ': ' + e.message }}");
    out.push(twice.eval(&thrown("g.l.rotateSplit(45)")));
    out.push(twice.eval(&thrown("g.l.rotateSplit(Number.NaN)")));
    assert_eq!(
        out.join("\n"),
        lines(&[
            "90: 0,0,1000,400 0,400,1000,400",
            "180: 500,0,500,800 0,0,500,800",
            "270: 0,400,1000,400 0,0,1000,400",
            "360: 0,0,500,800 500,0,500,800",
            "-90: 0,400,1000,400 0,0,1000,400",
            "-180: 500,0,500,800 0,0,500,800",
            "0: 0,0,500,800 500,0,500,800",
            "90, 90: 500,0,500,800 0,0,500,800",
            "TypeError: dwindle.rotate_split angle must be a multiple of 90, got 45",
            "TypeError: dwindle.rotate_split.angle must be a finite number, got NaN",
        ])
    );
}

#[test]
fn move_to_root_makes_the_window_a_child_of_the_root_split() {
    // w1 | (w2 over (w3 | w4)). Moving w4 to the root puts it where w1 was and w1 where w4 was. Stable (the default)
    // leaves w4 on the same side of the screen, unstable on the other.
    let make = || {
        let mut action = Action::new("{}");
        action.arrive(4);
        action
    };
    let mut out = Vec::new();
    {
        let mut action = make();
        out.push(action.cells(4));
        out.push(action.eval("String(g.l.moveToRoot(true))"));
        out.push(action.cells(4));
        out.push(action.eval("String(g.l.moveToRoot(true))"));
    }
    {
        let mut action = make();
        out.push(action.eval("String(g.l.moveToRoot(false))"));
        out.push(action.cells(4));
    }
    {
        let mut action = make();
        action.focus(1);
        out.push(action.eval("String(g.l.moveToRoot(true))"));
        action.focus(2);
        out.push(action.eval("String(g.l.moveToRoot(true))"));
        out.push(action.cells(4));
    }
    assert_eq!(
        out.join("\n"),
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
    // Hyprland's `splitratio`, in Gnoblin's fractions (Hyprland's 0.6 is 0.3 here): a delta by default, `setSplitRatio` to
    // set it, held to 0.1 to 0.9.
    // A window with no split above it has no ratio to change.
    let alone_result = {
        let mut alone = Action::new("{}");
        alone.arrive(1);
        alone.eval("String(g.l.splitRatio(0.1))")
    };
    let mut action = Action::new("{}");
    action.arrive(2);
    let step = |action: &mut Action, call: &str| {
        let done = action.eval(&format!("String(g.l.{call})"));
        format!("{done} {}", action.cells(2))
    };
    let mut out = vec![
        step(&mut action, "setSplitRatio(0.3)"),
        step(&mut action, "splitRatio(0.25)"),
        step(&mut action, "splitRatio(-0.25)"),
        step(&mut action, "splitRatio(-0.5)"),
        step(&mut action, "setSplitRatio(5)"),
        step(&mut action, "splitRatio(-5)"),
    ];
    let thrown =
        |source: &str| format!("try {{ {source}; 'did not throw' }} catch (e) {{ e.name + ': ' + e.message }}");
    out.push(action.eval(&thrown("g.l.splitRatio(Number.NaN)")));
    out.push(action.eval(&thrown("g.l.splitRatio('wide')")));
    out.push(alone_result);
    action.focus(0);
    out.push(action.eval("String(g.l.splitRatio(0.1))"));
    // A later change of the layout's own ratio does not move a split that was set.
    action.focus(2);
    action.eval("g.l.ratio = 0.7");
    out.push(action.cells(2));
    assert_eq!(
        out.join("\n"),
        lines(&[
            "true 0,0,300,800 300,0,700,800",
            "true 0,0,550,800 550,0,450,800",
            "true 0,0,300,800 300,0,700,800",
            "true 0,0,100,800 100,0,900,800",
            "true 0,0,900,800 900,0,100,800",
            "true 0,0,100,800 100,0,900,800",
            "TypeError: dwindle.split_ratio.amount must be a finite number, got NaN",
            "TypeError: `DwindleLayout.splitRatio`: `amount` must be a number, got text",
            "false",
            "false",
            "0,0,100,800 100,0,900,800",
        ])
    );
}

#[test]
fn a_split_that_was_set_keeps_its_ratio_and_a_new_one_uses_the_layouts() {
    // The ratio belongs to one split: w3 halves w2's cell (750 by 800, so top and bottom) with the layout's 0.4.
    let mut action = Action::new("{ ratio: 0.4 }");
    action.arrive(2);
    action.eval("g.l.setSplitRatio(0.25)");
    action.arrive(3);
    assert_eq!(action.cells(3), "0,0,250,800 250,0,750,320 250,320,750,480");
}

#[test]
fn pseudotile_keeps_the_windows_own_size_centred_in_its_cell() {
    let mut action = Action::new("{}");
    action.arrive(2);
    let second = Action::node(2);
    action
        .rig
        .comp
        .external_set(second, "width", MirrorValue::Double(300.0));
    action
        .rig
        .comp
        .external_set(second, "height", MirrorValue::Double(200.0));
    action.rig.settle();
    let client = "gnoblin.clients.get('4')";
    let mut out = vec![
        action.eval(&format!("String(g.l.isPseudotiled({client}))")),
        action.eval("String(g.l.pseudotile())"),
        action.eval(&format!("String(g.l.isPseudotiled({client}))")),
        action.cells(2),
    ];
    // w3 halves the cell of w2: the size stays, and the centre moves with the cell.
    action.arrive(3);
    out.push(action.cells(3));
    // A cell smaller than the window clamps it.
    action
        .rig
        .comp
        .external_set(SCREEN, "width", MirrorValue::Double(300.0));
    action
        .rig
        .comp
        .external_set(SCREEN, "height", MirrorValue::Double(200.0));
    action.rig.settle();
    out.push(action.cells(3));
    action
        .rig
        .comp
        .external_set(SCREEN, "width", MirrorValue::Double(1000.0));
    action
        .rig
        .comp
        .external_set(SCREEN, "height", MirrorValue::Double(800.0));
    action.rig.settle();
    // Turning it off gives the window its whole cell back.
    action.focus(2);
    out.push(action.eval("String(g.l.pseudotile())"));
    out.push(action.eval(&format!("String(g.l.isPseudotiled({client}))")));
    out.push(action.cells(3));
    assert_eq!(
        out.join("\n"),
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
    let mut action = Action::new("{}");
    action.arrive(2);
    // The window has no size of its own.
    let second = Action::node(2);
    action.rig.comp.external_set(second, "width", MirrorValue::Double(0.0));
    action.rig.comp.external_set(second, "height", MirrorValue::Double(0.0));
    action.rig.settle();
    action.eval("g.l.pseudotile(200, 100)");
    let mut out = vec![action.cells(2)];
    action.eval("g.l.pseudotile()");
    // No size on the window and none given: it keeps the size its cell has at the next compute.
    action.eval("g.l.pseudotile()");
    out.push(action.cells(2));
    action
        .rig
        .comp
        .external_set(SCREEN, "height", MirrorValue::Double(400.0));
    action.rig.settle();
    out.push(action.cells(2));
    out.push(action.eval("try { g.l.pseudotile(-1, 100); 'did not throw' } catch (e) { e.name + ': ' + e.message }"));
    assert_eq!(
        out.join("\n"),
        lines(&[
            "0,0,500,800 650,350,200,100",
            "0,0,500,800 500,0,500,800",
            "0,0,500,400 500,0,500,400",
            "TypeError: dwindle.pseudotile size must be numbers above 0, got -1 by 100",
        ])
    );
}

#[test]
#[ignore = "vrnw: needs the actions of the layout to bump layout.revision() (the revision stays; the workspace computes again all the same, see `every_action_that_changes_the_tree_makes_the_workspace_compute`)"]
fn every_action_that_changes_the_tree_bumps_the_revision_so_the_workspace_computes() {
    let mut action = Action::new("{ preserveSplit: true }");
    action.arrive(3);
    let mut moved = Vec::new();
    for (name, call) in [
        ("toggleSplit", "toggleSplit()"),
        ("swapSplit", "swapSplit()"),
        ("rotateSplit", "rotateSplit(90)"),
        ("moveToRoot", "moveToRoot(true)"),
        ("splitRatio", "splitRatio(0.1)"),
        ("pseudotile", "pseudotile(10, 10)"),
    ] {
        let before = action.revision();
        action.eval(&format!("g.l.{call}"));
        moved.push(format!("{name} {}", action.revision() > before));
    }
    // Nothing focused: there is nothing to act on.
    action.focus(0);
    let before = action.revision();
    action.eval("g.l.toggleSplit()");
    moved.push(format!("a refused toggleSplit {}", action.revision() > before));
    assert_eq!(
        moved.join("\n"),
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

#[test]
fn every_action_that_changes_the_tree_makes_the_workspace_compute() {
    let mut action = Action::new("{ preserveSplit: true }");
    action.arrive(3);
    let mut moved = Vec::new();
    for (name, call) in [
        ("toggleSplit", "toggleSplit()"),
        ("swapSplit", "swapSplit()"),
        ("rotateSplit", "rotateSplit(90)"),
        ("moveToRoot", "moveToRoot(true)"),
        ("splitRatio", "splitRatio(0.1)"),
        ("pseudotile", "pseudotile(10, 10)"),
    ] {
        let before = action.cells(3);
        action.eval(&format!("g.l.{call}"));
        moved.push(format!("{name} {}", action.cells(3) != before));
    }
    // Nothing focused: there is nothing to act on.
    action.focus(0);
    let before = action.cells(3);
    action.eval("g.l.toggleSplit()");
    moved.push(format!("a refused toggleSplit {}", action.cells(3) != before));
    assert_eq!(
        moved.join("\n"),
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
