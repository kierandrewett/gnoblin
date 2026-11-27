//! The second set of stock layouts of the library (`gnoblin-std`): Dwindle, Monocle (the old Kiosk), Strip and Carousel, and the
//! workspace slide gesture binding, run in a real generation over a fake compositor: `compute` is called on real clients (the
//! library takes nothing else).
//!
//! The cases are the retired shell's: `the retired retired corpus` (Dwindle, Kiosk),
//! `the retired retired corpus` (Strip, Carousel3d, the ring), `retired corpus sets` (the pure set cases)
//! and the gesture cases of `crates/gnoblin-compositor/tests/shell_vm.rs` (swipe left slides on, a short swipe
//! slides back). A case taken from that corpus says so; a case marked "derived" is not in it (the retired class has no
//! such option, or the case was a property of the retired node tree) and its expectation was worked out by hand from the
//! same algorithm. Rectangles are written `x,y,width,height`, in the retired tests' 1000 by 800 workspace.
//!
//! The native layouts have tests of their own (`crates/gnoblin-std-impl/tests/tiling.rs`, `objects.rs`). Where a case here is the
//! same check, its test says which one.

use std::fs;
use std::path::Path;

use gnoblin_js::JsConfig;
use gnoblin_testkit::compositor::FakeCompositor;
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::{ROOT, text};

/// The helpers every case shares. Plain JavaScript: embedded modules are not stripped.
const HARNESS: &str = r##"
export const area = (width = 1000, height = 800) => ({ x: 0, y: 0, width, height });
// The first `n` windows of the compositor, as the library's clients. Their titles are w1, w2, ...
export const nodes = (n) => gnoblin.clients.toList().slice(0, n);
const num = (v) => String(Math.round(v * 1000) / 1000);
export const rect = (r) => [r.x, r.y, r.width, r.height].map(num).join(",");
// The placements as "x,y,w,h" per node in `list` order, after checking each is for its node, tiled and whole pixels.
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
// What the layout computes for `children` in the context `ctx`: the old `gap: { inner, outer }` is the two gaps of the context.
export function place(layout, children, ctx = {}) {
    const { gap, ...rest } = ctx;
    const context = { workspace: "w", children, area: area(), ...rest };
    if (gap) {
        context.innerGap = gap.inner;
        context.outerGap = gap.outer;
    }
    return layout.compute(context);
}
export function lay(layout, list, ctx = {}) {
    return tiled(place(layout, list, ctx), list);
}
// The placements as "title:x,y,w,h" in placement order, none of them tiled (the strip and the ring do not tile).
export function named(placements) {
    return placements
        .map((p) => {
            if (p.tiled === true) throw new Error(`${p.node.title} is marked tiled`);
            return `${p.node.title}:${rect(p.bounds)}`;
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

/// The entry of a case: the harness imports and the case as a function the test calls once the windows are known.
fn entry(body: &str) -> String {
    format!(
        "import {{ area, nodes, tiled, place, lay, named, thrown, rect, emit }} from \"harness\";\n\
         import {{ DwindleLayout, MonocleLayout, StripLayout, CarouselLayout, WorkspaceSlideBinding, Workspace }} from \"gnoblin-std\";\n\
         (globalThis as any).__case = () => {{\n{body}\n}};\n\
         void [DwindleLayout, MonocleLayout, StripLayout, CarouselLayout, WorkspaceSlideBinding, Workspace];\n"
    )
}

/// A screen and 200 windows (ids 3 on, titles w1, w2, ...), none of them on a workspace.
fn world() -> FakeCompositor {
    let comp = FakeCompositor::new();
    comp.add_screen(20, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
    for id in 3..203 {
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
/// the std library loaded, which computes the stock layouts it uses.
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

/// One expected line per entry, as the JavaScript joins them.
fn lines(expected: &[&str]) -> String {
    expected.join("\n")
}

// -- Dwindle ------------------------------------------------------------------------------------------------------

#[test]
fn dwindle_matches_the_retired_tiling_goldens() {
    // Before: "Dwindle: 2 and 3 windows split the last cell by aspect", "split horizontal puts every window side by
    // side", "ratio changes re-arrange with no handler". Windows arrive one at a time, as `ws:add` did.
    // The native check is `tiling.rs::the_dwindle_places_arrivals_as_the_typescript_dwindle_does`.
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
fn dwindle_has_the_retired_defaults_and_refuses_a_bad_split() {
    // Before: "the options have the old layouts' defaults": split aspect, ratio 0.5, gap 0 (unset here, so the
    // workspace's gap applies). The error is the library's, in its own words.
    let got = run(r#"
        const d = new DwindleLayout();
        emit([
            [d.split, d.ratio, d.gap].join(","),
            thrown(() => new DwindleLayout({ split: "diagonal" as any })),
            thrown(() => new DwindleLayout({ ratio: Number.NaN })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "aspect,0.5,",
            "TypeError: \"diagonal\" is not a value of dwindle_split (aspect, horizontal, vertical)",
            "TypeError: dwindle.ratio must be a finite number, got NaN",
        ])
    );
}

#[test]
fn dwindle_refuses_a_negative_gap() {
    let got = run(r#"
        emit(thrown(() => new DwindleLayout({ gap: -1 })));
    "#);
    assert_eq!(
        got,
        "TypeError: dwindle.gap must be a finite number of at least 0, got -1"
    );
}

#[test]
fn dwindle_clamps_its_ratio_as_before() {
    // Before: `clamp(self.ratio, 0.1, 0.9)`. A ratio of 5 gives the first child 0.9 of the width.
    let got = run(r#"
        const all = nodes(2);
        const high = new DwindleLayout({ ratio: 5 });
        const low = new DwindleLayout({ ratio: -1 });
        emit([lay(high, all), lay(low, all)].join("\n"));
    "#);
    assert_eq!(got, lines(&["0,0,900,800 900,0,100,800", "0,0,100,800 100,0,900,800"]));
}

#[test]
fn dwindle_gap_collapse_and_focus_insertion_are_derived_from_the_retired_algorithm() {
    // Derived: the retired tests give no gap, no closing and no focus to Dwindle. The retired algorithm: the gap is
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

// -- Kiosk (the library's Monocle) --------------------------------------------------------------------------------

#[test]
fn kiosk_shows_one_full_window_and_hides_the_rest_as_before() {
    // Before: "Kiosk: 1, 2 and 3 windows show one full-area window and hide the rest". The library's `MonocleLayout` is what the
    // TypeScript library called `KioskLayout`.
    let got = run(r#"
        const l = new MonocleLayout();
        const shown = (placed) => placed.map((p) => (p.visible ? "T" : "F")).join("");
        const out = [];
        for (const n of [1, 2, 3]) {
            const children = nodes(n);
            const placed = place(l, children);
            out.push(tiled(placed, children) + " " + shown(placed));
        }
        // The shown window is the focused child, else the first.
        const children = nodes(3);
        out.push(shown(place(l, children, { focused: children[2] })));
        out.push(shown(place(l, children, { focused: nodes(4)[3] })));
        out.push(String(place(l, []).length));
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
    // Before: "each class: the same sequence replayed twice gives identical rectangles" (three windows, a gap of 4, the
    // second closed, two more opened) and "200 windows give finite integral rectangles".
    let got = run(r#"
        const replay = (make) => {
            const layout = make();
            const all = nodes(5);
            lay(layout, all.slice(0, 3));
            return lay(layout, [all[0], all[2], all[3], all[4]]);
        };
        const results = [];
        for (const make of [() => new DwindleLayout({ gap: 4 }), () => new MonocleLayout()]) {
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
#[ignore = "vrnw: TS internal: `clampColumn` is a helper function of the TypeScript Strip module; the generated StripLayout checks its `column` natively"]
fn a_column_fraction_clamps_to_a_tenth_and_one_and_a_non_number_is_an_error_as_before() {
    // Before: "a column fraction clamps to 0.1..1 and a non-number or NaN is an error".
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
fn a_strip_lays_out_one_column_per_node_and_scrolls_to_keep_the_focused_one_in_view_as_before() {
    // Before: "a strip lays out one column per window and scrolls to keep the focused one in view": 3 windows, the
    // last focused; a column is half the strip wide and the full height; column 3 centres (view 750) then clamps to
    // the 3 * 500 - 1000 = 500 the strip can scroll, so it sits at 500, column 2 at 0 and column 1 at -500.
    // The native check is `tiling.rs::the_strip_scrolls_and_hands_the_keyboard_on_as_the_typescript_strip_does`.
    let got = run(r#"
        const list = nodes(3);
        const l = new StripLayout();
        const placed = place(l, list, { focused: list[2] });
        emit([
            named(placed),
            // Derived: a column of 0.25 leaves the strip narrower than the view, so nothing scrolls.
            named(place(new StripLayout({ column: 0.25 }), list, { focused: list[2] })),
            // Derived: a strip does not floor (the retired one did not): 0.5 of 1001 is 500.5.
            named(place(new StripLayout(), list, { area: area(1001), focused: list[2] })),
            // Derived: an area that does not start at the origin moves every column with it.
            named(place(new StripLayout(), list, { area: { x: 10, y: 20, width: 1000, height: 800 }, focused: list[2] })),
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
fn a_new_node_opens_after_the_focused_column_not_at_the_end_as_before() {
    // Before: "a new window opens after the focused column, not at the end": three windows, focus on the first, a new
    // one -> w1, new, w2, w3. The compositor focuses the arrival in the same frame, and the layout still places it
    // after the column that had focus when it came (the focus it saw last). The new windows here are w4 (and w3, w4 in the last
    // case) instead of the fake nodes `new`, `x` and `y`.
    // The native check is `tiling.rs::the_strip_opens_a_window_after_the_focused_column_as_the_typescript_strip_does`.
    let got = run(r#"
        const four = nodes(4);
        const list = four.slice(0, 3);
        const l = new StripLayout();
        const order = (placed) => placed.map((p) => p.node.title).join(" ");
        const compute = (layout, children, focused) => place(layout, children, { focused });
        compute(l, list, list[2]);
        compute(l, list, list[0]);
        const fresh = four[3];
        const withNew = [...list, fresh];
        const first = order(compute(l, withNew, list[0]));
        const second = order(compute(l, withNew, fresh));
        // Several arrivals in one frame keep their arrival order after the focused column.
        const other = new StripLayout();
        const a = four.slice(0, 2);
        compute(other, a, a[0]);
        const third = order(compute(other, [...a, four[2], four[3]], a[0]));
        emit([first, second, third].join("\n"));
    "#);
    assert_eq!(got, lines(&["w1 w4 w2 w3", "w1 w4 w2 w3", "w1 w3 w4 w2"]));
}

#[test]
#[ignore = "vrnw: TS internal: the TypeScript strip moved focus through a fake workspace object (`step.call(l, { clients })`); the generated strip acts on a real workspace and hands the keyboard on natively (covered by tiling.rs::the_strip_scrolls_and_hands_the_keyboard_on_as_the_typescript_strip_does)"]
fn focus_left_and_right_move_through_the_columns_and_stop_at_the_ends_as_before() {
    // Before: "focus_left and focus_right move through the columns and stop at the ends".
    let got = run(r#"
        const list = nodes(3);
        const l = new StripLayout();
        place(l, list, { focused: list[2] });
        const seen: string[] = [];
        for (const step of [l.focusLeft, l.focusLeft, l.focusLeft, l.focusRight, l.focusRight, l.focusRight]) {
            step.call(l);
            seen.push("?");
        }
        emit(seen.join(","));
    "#);
    assert_eq!(got, "w2,w1,w1,w2,w3,w3");
}

#[test]
fn scroll_to_pins_the_view_until_the_focused_node_changes_as_before() {
    // Before: "scroll_to pins the view until the focused window changes" and "scroll_to rejects a non-finite offset".
    let got = run(r#"
        const list = nodes(3);
        const l = new StripLayout();
        gnoblin.workspaces.add(new Workspace("s", { layout: l }));
        const compute = (focused) => place(l, list, { focused });
        const x = (placed, name) => placed.find((p) => p.node.title === name).bounds.x;
        compute(list[2]);
        l.scrollTo(0);
        const pinned = x(compute(list[2]), "w1");
        // The focus is unchanged, so the pin holds although column 3 is off view.
        const held = x(compute(list[2]), "w1");
        // Focus moves to column 2, which is in view: the pin ends without a follow.
        const inView = x(compute(list[1]), "w1");
        // Focus moves to column 3, which is off view: the view follows, centred and clamped to 500.
        const followed = x(compute(list[2]), "w3");
        emit([
            [pinned, held, inView, followed].join(","),
            thrown(() => l.scrollTo(Number.NaN)),
            thrown(() => l.scrollTo(Infinity)),
            thrown(() => l.scrollTo("left" as any)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,0,500",
            "TypeError: strip.scroll_to.offset must be a finite number, got NaN",
            "TypeError: strip.scroll_to.offset must be a finite number, got inf",
            "TypeError: `StripLayout.scrollTo`: `offset` must be a number, got text",
        ])
    );
}

#[test]
#[ignore = "vrnw: needs StripLayout.pin to be settable (it is read-only: scrollTo pins the view and nothing clears it)"]
fn the_pin_can_be_read_set_and_cleared_as_a_field_as_before() {
    // Before: `pin` was a plain field: setting it pinned the view, null let it follow the focus again, and the layout was dirty either way.
    let got = run(r#"
        const list = nodes(3);
        const l = new StripLayout();
        const x = (placed, name) => placed.find((p) => p.node.title === name).bounds.x;
        const compute = () => place(l, list, { focused: list[2] });
        compute();
        const before = l.revision();
        (l as any).pin = 100;
        const set = [l.pin, x(compute(), "w1"), l.revision() > before];
        (l as any).pin = null;
        const cleared = [l.pin, x(compute(), "w1")];
        emit([
            set.join(","),
            cleared.join(","),
            thrown(() => { (l as any).pin = Number.NaN; }),
            thrown(() => { (l as any).pin = "left" as any; }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "100,-100,true",
            ",-500",
            "TypeError: scrollTo needs a finite number, got NaN",
            "TypeError: scrollTo needs a finite number, got left",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: the TypeScript strip's `onClientRemoved` handed the keyboard to a neighbour through a fake workspace; the generated strip does it natively when its workspace loses a window (covered by tiling.rs::the_strip_scrolls_and_hands_the_keyboard_on_as_the_typescript_strip_does)"]
fn closing_the_focused_column_moves_focus_to_its_neighbour_as_before() {
    // Before: "closing the focused column moves focus to its neighbour, not to the closed one".
    let got = run(r#"
        const list = nodes(3);
        const l = new StripLayout();
        place(l, list, { focused: list[1] });
        l.clientRemoved({ workspace: "w", children: [list[1], list[2]], area: area() }, list[0]);
        emit("?");
    "#);
    assert_eq!(got, "w2,w3,w1");
}

// -- the ring and Carousel ----------------------------------------------------------------------------------------

#[test]
#[ignore = "vrnw: TS internal: `ring(count, focus, area)` is a helper function of the TypeScript carousel module; the generated CarouselLayout computes the ring inside (the front card is checked by the carousel cases below and tiling.rs::the_carousel_places_cards_on_a_ring_as_the_typescript_carousel_does)"]
fn the_ring_puts_the_focused_card_at_the_front_unscaled_and_opaque_as_before() {
    // Before: "the ring puts the focused card at the front, unscaled and fully opaque".
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
#[ignore = "vrnw: TS internal: `ring(count, focus, area)` is a helper function of the TypeScript carousel module (see the front-card case)"]
fn the_ring_is_symmetric_about_the_front_card_and_fades_toward_the_back_as_before() {
    // Before: "the ring is symmetric about the front card and fades toward the back" (focus 1 of 4: slot 2 is the right
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
#[ignore = "vrnw: TS internal: `ring(count, focus, area)` is a helper function of the TypeScript carousel module (see the front-card case)"]
fn the_ring_with_no_nodes_is_empty_and_a_bad_focus_index_is_an_error_as_before() {
    // Before: "the ring with no windows is empty, and a bad focus index is an error".
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
fn a_carousel_shows_the_focused_node_at_the_front() {
    // Before: "a Carousel3d shows the focused window at the front". The front card of two has no rotation and sits at depth -F,
    // F = (card height / 2) / tan(fov / 2). The retired card had an opacity left to the rules: the generated placement has none.
    let got = run(r#"
        const list = nodes(2);
        const c = new CarouselLayout();
        const placed = place(c, list, { focused: list[1] });
        const front = placed[1];
        const focal = 240 / Math.tan((20 * Math.PI) / 180);
        emit([
            Math.abs(front.bounds.width - 360) < 1e-9,
            Math.abs(front.bounds.x + front.bounds.width / 2 - 500) < 1e-6,
            Math.abs(front.z + focal) < 1e-9,
            (front.rotation ?? [0, 0, 0]).join(","),
            placed.every((p) => p.tiled !== true),
            // With no focus among the nodes the first is the front card.
            (place(new CarouselLayout(), list)[0].rotation ?? [0, 0, 0]).join(","),
        ].join("\n"));
    "#);
    assert_eq!(got, lines(&["true", "true", "true", "0,0,0", "true", "0,0,0"]));
}

#[test]
#[ignore = "vrnw: TS internal: `rotate(workspace, steps)` on a fake workspace object moved the focus round the ring; the generated carousel's `rotate(steps)` acts on the workspace that has it (covered by tiling.rs::the_carousel_places_cards_on_a_ring_as_the_typescript_carousel_does)"]
fn a_carousel_rotate_wraps_round_the_ring_as_before() {
    let got = run(r#"
        const list = nodes(2);
        const c = new CarouselLayout();
        place(c, list, { focused: list[1] });
        c.rotate(1);
        emit(thrown(() => c.rotate("one" as any)));
    "#);
    assert_eq!(got, "Error: E_TYPE: carousel.rotate.steps must be a whole number");
}

#[test]
fn a_carousel_turns_the_side_cards_about_the_vertical_axis_by_the_rings_angle() {
    // Derived: the retired class wrote only x, y, width and height; the placement carries the ring's angle as a rotation
    // about the vertical axis (radians) and its depth as `z`, which the node schema has. Of four cards focused on
    // the first, the right neighbour is a quarter turn round the ring and the back card half a turn.
    let got = run(r#"
        const list = nodes(4);
        const placed = place(new CarouselLayout(), list, { focused: list[0] });
        const turn = (p) => Math.round(((p.rotation?.[1] ?? 0) * 180) / Math.PI);
        emit([
            placed.map(turn).join(","),
            placed.map((p) => ((p.rotation?.[0] ?? 0) === 0 && (p.rotation?.[2] ?? 0) === 0 ? "t" : "f")).join(""),
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
        timers: {
            after(ms, run) {
                log.push(`after ${ms}`);
                timers.push(run);
            },
        },
        requestTransition(change, options) {
            log.push(`transition ${options.duration} ${options.easing}`);
            change();
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
#[ignore = "vrnw: TS internal: the gesture hooks `onStart`/`onUpdate`/`onEnd` of the TypeScript WorkspaceSlideBinding, driven here by a fake shell host; the generated binding is driven by a compositor gesture claim (native: crates/gnoblin-std-impl/tests/policy.rs, and `input.rs` of this crate)"]
fn a_swipe_left_follows_the_fingers_and_lifting_past_a_quarter_slides_on_to_the_next_workspace() {
    // the retired shell (the retired the retired shell-rig test): "a three-finger swipe on the input path slides the stock shell to the next workspace": the
    // workspace the fingers bring in is showing before they lift, and lifting after more than a quarter of the
    // travel finishes the slide: `desk:show(to, progress)` with 250 * (1 - progress) milliseconds.
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
            "show 2 62.5 0.75",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: the gesture hooks of the TypeScript WorkspaceSlideBinding on a fake shell host (see the swipe-left case)"]
fn a_swipe_short_of_the_commit_slides_back_and_the_incoming_workspace_is_hidden_after() {
    // the retired shell (the retired the retired shell-rig test): "a swipe that is short of the commit slides back": 50 of 400 units is 0.125, under 0.25.
    // The shown workspace slides back to rest and the other one out the way it came, over 200 milliseconds, and is hidden
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
            "transition 200 ease-out; after 200",
            "100+ 100- 100-",
            "true,false,false",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: the gesture hooks of the TypeScript WorkspaceSlideBinding on a fake shell host (see the swipe-left case)"]
fn a_cancel_slides_back_and_an_end_that_is_more_vertical_than_horizontal_does_too() {
    // Before: `phase == "end"` commits only when `math.abs(e.dx) >= math.abs(e.dy)`; a cancel never does.
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
            "transition 200 ease-out; after 200",
            "transition 200 ease-out; after 200",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: the gesture hooks of the TypeScript WorkspaceSlideBinding on a fake shell host (see the swipe-left case)"]
fn a_swipe_with_no_workspace_to_reveal_or_a_vertical_one_moves_nothing_as_before() {
    // Before: "A gesture that is more vertical than horizontal, or that has no workspace to reveal, moves nothing."
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
#[ignore = "vrnw: TS internal: the gesture hooks of the TypeScript WorkspaceSlideBinding on a fake shell host (see the swipe-left case)"]
fn a_swipe_right_from_the_second_workspace_brings_the_previous_one_in_from_the_left() {
    // Before: sign +1 for fingers moving right; the target is `children[position(from) - sign]`; the incoming one starts
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
    assert_eq!(got, lines(&["-400+ 600+ 100-", "show 1 125 0.5"]));
}

#[test]
#[ignore = "vrnw: TS internal: the gesture hooks of the TypeScript WorkspaceSlideBinding on a fake shell host (see the swipe-left case)"]
fn changing_direction_hides_the_workspace_that_came_in_and_something_else_switching_ends_the_slide() {
    // Before: `if sign ~= s.sign then reset(s) ...`; and "something else switched workspace under the fingers": the slide
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
fn the_slide_binding_has_the_retired_travel_and_commit_and_refuses_bad_ones() {
    let got = run(r#"
        const b = new WorkspaceSlideBinding();
        emit([
            [b.trigger, b.travel, b.commit, b.duration].join(","),
            thrown(() => new WorkspaceSlideBinding({ travel: 0 })),
            thrown(() => new WorkspaceSlideBinding({ commit: 2 })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "Swipe3,400,0.25,250",
            "TypeError: WorkspaceSlideBinding travel is out of range, got 0",
            "TypeError: WorkspaceSlideBinding commit is out of range, got 2",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: `onStart` of the TypeScript WorkspaceSlideBinding answered false for a shell with no screen or no shown workspace (a fake host); the generated binding is driven by a gesture claim (see the swipe-left case)"]
fn the_slide_binding_refuses_a_start_without_a_screen() {
    let got = slide(
        r#"
        const noScreen = world();
        (noScreen.shell.workspaces as any).area = null;
        const none = world();
        for (const s of none.spaces) s.active = false;
        emit([
            noScreen.binding.onStart({} as any),
            none.binding.onStart({} as any),
        ].join("\n"));
    "#,
    );
    assert_eq!(got, lines(&["false", "false"]));
}
