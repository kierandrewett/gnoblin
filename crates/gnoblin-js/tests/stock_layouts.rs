//! The stock layouts of the library (`gnoblin-std`: `MasterStackLayout`, `GridLayout`, `MonocleLayout`, `FloatingLayout`) run in a
//! real generation over a fake compositor: `compute` is called on real clients (the library takes nothing else).
//!
//! The geometry cases are the retired shell's: `the retired retired corpus` for the tiled layouts
//! and `tests/shell/placement_cases.toml` for floating placement. A case taken from the retired corpus says so; a
//! case marked "derived" is not in that corpus (the retired class has no such option) and its expected rectangles
//! were worked out by hand from the same algorithm. Rectangles are written `x,y,width,height`, one node after
//! another, in the retired tests' 1000 by 800 workspace.
//!
//! The native layouts have tests of their own (`crates/gnoblin-std-impl/tests/tiling.rs`, `examples.rs`). Where a case here is
//! the same check, its test says which one.

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
// The first `n` windows of the compositor, as the library's clients.
export const nodes = (n) => gnoblin.clients.toList().slice(0, n);
const rectText = (r) => [r.x, r.y, r.width, r.height].join(",");
// The placements as "x,y,w,h" per node, after checking each is for its node, tiled, whole pixels and finite.
export function fmt(placements, list) {
    return placements
        .map((p, i) => {
            if (p.node.id !== list[i].id) throw new Error(`placement ${i} is for the wrong node`);
            if (p.tiled !== true) throw new Error(`placement ${i} is not tiled`);
            for (const v of [p.bounds.x, p.bounds.y, p.bounds.width, p.bounds.height]) {
                if (!Number.isInteger(v)) throw new Error(`placement ${i} is not whole pixels: ${rectText(p.bounds)}`);
            }
            return rectText(p.bounds);
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
// `n` windows (or the given list) laid out by `layout`; `ctx` overrides area, gap and focus.
export function lay(layout, what, ctx = {}) {
    const children = typeof what === "number" ? nodes(what) : what;
    return fmt(place(layout, children, ctx), children);
}
export function thrown(fn) {
    try {
        fn();
    } catch (e) {
        return `${e.name}: ${e.message}`;
    }
    return "did not throw";
}
export const rect = rectText;
export function emit(value) {
    globalThis.out = String(value);
}
"##;

/// The entry of a case: the harness imports and the case as a function the test calls once the windows are known.
fn entry(body: &str) -> String {
    format!(
        "import {{ area, nodes, fmt, place, lay, thrown, rect, emit }} from \"harness\";\n\
         import {{ FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout }} from \"gnoblin-std\";\n\
         (globalThis as any).__case = () => {{\n{body}\n}};\n\
         void [FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout];\n"
    )
}

/// A screen and 200 windows (ids 3 on), none of them on a workspace.
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
/// the std library loaded, which is what computes the layouts.
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

#[test]
fn master_stack_matches_the_retired_tiling_goldens() {
    // Before: "1 window fills", "2 and 3 windows give the master and stack rectangles", "the gap separates the
    // master from the stack and the stack rows", "orientation top puts the master across the top".
    let got = run(r#"
        const l = new MasterStackLayout();
        const gapped = new MasterStackLayout({ gap: 10 });
        const top = new MasterStackLayout({ orientation: "top" });
        emit([
            lay(l, 0),
            lay(l, 1),
            lay(l, 2),
            lay(l, 3),
            lay(gapped, 3),
            lay(new MasterStackLayout(), 3, { gap: { inner: 10, outer: 0 } }),
            lay(top, 2),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "",
            "0,0,1000,800",
            "0,0,500,800 500,0,500,800",
            "0,0,500,800 500,0,500,400 500,400,500,400",
            "0,0,495,800 505,0,495,395 505,405,495,395",
            "0,0,495,800 505,0,495,395 505,405,495,395",
            "0,0,1000,400 0,400,1000,400",
        ])
    );
}

#[test]
fn master_stack_tracks_ratio_gap_and_area_as_before() {
    // Before: "ratio, gap, width and height re-arrange with no handler": the same layout object over a shrinking
    // area, then its gap set to 0.
    let got = run(r#"
        const l = new MasterStackLayout({ ratio: 0.25, gap: 12 });
        const out = [lay(l, 2)];
        out.push(lay(l, 2, { area: area(500, 800) }));
        out.push(lay(l, 2, { area: area(500, 400) }));
        l.gap = 0;
        out.push(lay(l, 2, { area: area(500, 400) }));
        emit(out.join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,247,800 259,0,741,800",
            "0,0,122,800 134,0,366,800",
            "0,0,122,400 134,0,366,400",
            "0,0,125,400 125,0,375,400",
        ])
    );
}

#[test]
fn master_stack_200_nodes_share_the_stack_in_whole_pixels_as_before() {
    // Before: "200 windows share the stack in whole pixels".
    let got = run(r#"
        const children = nodes(200);
        const placed = place(new MasterStackLayout(), children);
        fmt(placed, children);
        emit([rect(placed[0].bounds), rect(placed[1].bounds), rect(placed[199].bounds), placed.length].join(" "));
    "#);
    // The retired shell floored each height on its own and ended at 799 of 800; the last tile now ends at the area's edge.
    assert_eq!(got, "0,0,500,800 500,0,500,4 500,795,500,5 200");
}

#[test]
fn tiles_that_touch_in_the_layouts_arithmetic_touch_in_pixels_so_the_area_is_covered_to_its_far_edges() {
    // Live verification: ratio 0.6 and a gap of 12 on 1024 by 768 gave a master of 607.2 and a stack of 404.8 that starts
    // at 619.2. Flooring each width apart made the stack 404 wide, and it ended a pixel short of the screen. Flooring
    // the edges gives 405, and the 200 windows of the retired corpus now end at 800 instead of 799.
    let got = run(r#"
        const many = place(new MasterStackLayout(), nodes(200));
        emit([
            lay(new MasterStackLayout({ ratio: 0.6, gap: 12 }), 3, { area: area(1024, 768) }),
            lay(new MasterStackLayout({ ratio: 0.6, gap: 12, orientation: "top" }), 3, { area: area(1024, 768) }),
            rect(many[199].bounds),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,607,768 619,0,405,378 619,390,405,378",
            "0,0,1024,453 0,465,506,303 518,465,506,303",
            "500,795,500,5",
        ])
    );
}

#[test]
#[ignore = "vrnw: needs client.frameInset (the cascade reads the frame a window has)"]
fn a_cascade_that_runs_out_of_room_never_lands_a_window_exactly_on_another() {
    // Live verification: three windows of 700 by 476 (frame included) on 1024 by 768. The second finds no free spot and
    // cascades into the corner; the third was pinned to the same corner and hid it. The third moves back one cascade step
    // instead. A window that does have a free spot is not moved: `placeRect` itself is pinned by the goldens.
    // The native cascade is `crates/gnoblin-std-impl/tests/policy.rs` (floating_world) and `examples.rs::floating_placement_matches_the_typescript_std`.
    let got = run(r#"
        const layout = new FloatingLayout();
        const screen = area(1024, 768);
        const clients = nodes(3);
        for (const c of clients) { c.bounds = { x: 0, y: 0, width: 700, height: 448 }; (c as any).frameInset = { top: 28, right: 0, bottom: 0, left: 0 }; }
        const [a, b, c] = clients;
        layout.clientAdded({ workspace: "w", children: [a], area: screen }, a);
        layout.clientAdded({ workspace: "w", children: [a, b], area: screen }, b);
        layout.clientAdded({ workspace: "w", children: [a, b, c], area: screen }, c);
        emit([a, b, c].map((w) => `${w.bounds.x},${w.bounds.y}`).join(" "));
    "#);
    assert_eq!(got, "0,28 324,320 292,288");
}

#[test]
fn master_stack_options_beyond_the_retired_corpus_are_derived_from_its_algorithm() {
    // Derived: master count, the right and bottom orientations, an offset area, a ratio outside 0.1..0.9, no
    // master at all, and more masters than nodes.
    let got = run(r#"
        emit([
            lay(new MasterStackLayout({ masterCount: 2 }), 4),
            lay(new MasterStackLayout({ orientation: "right" }), 2),
            lay(new MasterStackLayout({ orientation: "bottom" }), 2),
            lay(new MasterStackLayout(), 2, { area: { x: 10, y: 20, width: 1000, height: 800 } }),
            lay(new MasterStackLayout({ ratio: 5 }), 2),
            lay(new MasterStackLayout({ masterCount: 0 }), 2),
            lay(new MasterStackLayout({ masterCount: 5 }), 3),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,500,400 0,400,500,400 500,0,500,400 500,400,500,400",
            "500,0,500,800 0,0,500,800",
            "0,400,1000,400 0,0,1000,400",
            "10,20,500,800 510,20,500,800",
            "0,0,900,800 900,0,100,800",
            "0,0,1000,400 0,400,1000,400",
            "0,0,1000,266 0,266,1000,267 0,533,1000,267",
        ])
    );
}

#[test]
fn grid_matches_the_retired_tiling_goldens() {
    // Before: "1, 2 and 3 windows give the expected cells", "the gap separates cells and a partly filled last row is
    // left aligned", "fill column goes top to bottom then right", "gap changes re-arrange".
    let got = run(r#"
        emit([
            lay(new GridLayout(), 0),
            lay(new GridLayout(), 1),
            lay(new GridLayout(), 2),
            lay(new GridLayout(), 3),
            lay(new GridLayout({ gap: 10 }), 3),
            lay(new GridLayout({ fill: "column" }), 3),
            lay(new GridLayout({ gap: 20 }), 2),
            lay(new GridLayout(), 2, { gap: { inner: 20, outer: 0 } }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "",
            "0,0,1000,800",
            "0,0,500,800 500,0,500,800",
            "0,0,500,400 500,0,500,400 0,400,500,400",
            "0,0,495,395 505,0,495,395 0,405,495,395",
            "0,0,500,400 0,400,500,400 500,0,500,400",
            "0,0,490,800 510,0,490,800",
            "0,0,490,800 510,0,490,800",
        ])
    );
}

#[test]
fn grid_with_a_fixed_column_count_is_derived_from_the_auto_algorithm() {
    // Derived: the retired class has no `columns`; a fixed count replaces the near-square side and keeps the rest.
    // The generated class has the count as a number, 0 for the near-square side.
    let got = run(r#"
        emit([
            lay(new GridLayout({ columns: 3 }), 4),
            lay(new GridLayout({ columns: 1 }), 3),
            lay(new GridLayout({ columns: 0 }), 3),
            thrown(() => lay(new GridLayout({ columns: -1 }), 1)),
            thrown(() => lay(new GridLayout({ columns: 1.5 }), 1)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,333,400 333,0,333,400 666,0,334,400 0,400,333,400",
            "0,0,1000,266 0,266,1000,267 0,533,1000,267",
            "0,0,500,400 500,0,500,400 0,400,500,400",
            "TypeError: grid.columns must be a whole number of at least 0, got -1",
            "TypeError: `grid_layout.columns` must be an integer, got a number",
        ])
    );
}

#[test]
fn monocle_shows_the_focused_node_or_the_first_as_before() {
    // Before: "the focused window fills the workspace and the rest are hidden", "refocusing shows another window".
    let got = run(r#"
        const visible = (placed) => placed.map((p) => (p.visible ? "T" : "F")).join("");
        const l = new MonocleLayout();
        const children = nodes(3);
        const at = (focused) => {
            const placed = place(l, children, { focused });
            return fmt(placed, children) + " " + visible(placed);
        };
        emit([
            lay(l, 0),
            lay(l, 1),
            at(undefined),
            at(children[1]),
            at(children[2]),
            at(nodes(4)[3]),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "",
            "0,0,1000,800",
            "0,0,1000,800 0,0,1000,800 0,0,1000,800 TFF",
            "0,0,1000,800 0,0,1000,800 0,0,1000,800 FTF",
            "0,0,1000,800 0,0,1000,800 0,0,1000,800 FFT",
            "0,0,1000,800 0,0,1000,800 0,0,1000,800 TFF",
        ])
    );
}

#[test]
fn every_tiled_layout_gives_finite_whole_pixel_rectangles_for_200_nodes_and_is_deterministic() {
    // Before: "200 windows give finite integral rectangles" and "the same sequence replayed twice gives identical
    // rectangles". `fmt` throws on a fractional or missing rectangle.
    let got = run(r#"
        const results = [];
        for (const make of [
            () => new MasterStackLayout({ gap: 4 }),
            () => new GridLayout({ gap: 4 }),
            () => new GridLayout({ fill: "column", gap: 4 }),
            () => new MonocleLayout(),
        ]) {
            const first = lay(make(), 200);
            const second = lay(make(), 200);
            results.push(first === second && first.split(" ").length === 200);
        }
        emit(results.join(","));
    "#);
    assert_eq!(got, "true,true,true,true");
}

#[test]
fn layout_options_have_the_retired_defaults_and_bad_ones_are_refused() {
    // Before: "the options have the old layouts' defaults": ratio 0.5, gap 0 (unset here, so the workspace's gap
    // applies), master count 1, orientation left; grid fill row; and the orientation and fill errors.
    let got = run(r#"
        const m = new MasterStackLayout();
        const g = new GridLayout();
        emit([
            [m.ratio, m.gap, m.masterCount, m.orientation].join(","),
            [g.columns, g.fill, g.gap].join(","),
            thrown(() => new MasterStackLayout({ orientation: "middle" as any })),
            thrown(() => new GridLayout({ fill: "diagonal" as any })),
            thrown(() => new GridLayout({ gap: Number.NaN })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0.5,,1,left",
            "0,row,",
            "TypeError: \"middle\" is not a value of master_stack_orientation (left, right, top, bottom)",
            "TypeError: \"diagonal\" is not a value of grid_fill (row, column)",
            "TypeError: grid.gap must be a finite number, got NaN",
        ])
    );
}

#[test]
#[ignore = "vrnw: needs MasterStackLayout to refuse a negative gap (it takes gap -1 without a word)"]
fn master_stack_refuses_a_negative_gap() {
    let got = run(r#"
        emit(thrown(() => new MasterStackLayout({ gap: -1 })));
    "#);
    assert_eq!(
        got,
        "RangeError: MasterStackLayout gap must be a finite number of at least 0, got -1"
    );
}

#[test]
fn the_workspace_gaps_inset_the_area_and_set_the_gap() {
    // Derived: LayoutContext.gap and the outer gap (spec-review.md section 4). An inner gap of 10 and an outer
    // gap of 5 on 1000 by 800 is a 990 by 790 area at (5, 5); a layout's own gap, even 0, wins over the inner
    // one. Monocle and the grid take the outer gap too; floating takes neither.
    let got = run(r#"
        const children = nodes(3);
        const ctx = { gap: { inner: 10, outer: 5 } };
        emit([
            fmt(place(new MasterStackLayout(), children, ctx), children),
            fmt(place(new MasterStackLayout({ gap: 0 }), children, ctx), children),
            fmt(place(new GridLayout(), children.slice(0, 2), ctx), children.slice(0, 2)),
            fmt(place(new MonocleLayout(), children.slice(0, 1), ctx), children.slice(0, 1)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "5,5,490,790 505,5,490,390 505,405,490,390",
            "5,5,495,790 500,5,495,395 500,400,495,395",
            "5,5,490,790 505,5,490,790",
            "5,5,990,790",
        ])
    );
}

/// Every case of `tests/shell/placement_cases.toml` as a JavaScript array literal, and how many there are.
fn placement_goldens() -> (String, usize) {
    const GOLDENS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/shell/placement_cases.toml");
    let text = fs::read_to_string(GOLDENS).expect("the placement goldens exist");
    let table: toml::Table = text.parse().expect("the placement goldens are TOML");
    let cases = table["case"].as_array().expect("the goldens have [[case]] tables");
    let number = |table: &toml::Table, key: &str| table[key].as_integer().expect("a whole number");
    let rect = |value: &toml::Value| {
        let t = value.as_table().expect("a rect table");
        format!(
            "[{},{},{},{}]",
            number(t, "x"),
            number(t, "y"),
            number(t, "w"),
            number(t, "h")
        )
    };
    let mut js = String::from("[\n");
    for case in cases {
        let t = case.as_table().expect("a case table");
        let size = t["size"].as_table().expect("a size table");
        let occupied: Vec<String> = t["occupied"]
            .as_array()
            .expect("an occupied list")
            .iter()
            .map(rect)
            .collect();
        js.push_str(&format!(
            "{{ name: {:?}, mode: {:?}, size: [{}, {}], area: {}, occupied: [{}], result: {} }},\n",
            t["name"].as_str().expect("a name"),
            t["mode"].as_str().expect("a mode"),
            number(size, "w"),
            number(size, "h"),
            rect(&t["area"]),
            occupied.join(", "),
            rect(&t["result"]),
        ));
    }
    js.push(']');
    (js, cases.len())
}

#[test]
fn floating_placement_matches_every_retired_golden() {
    // The goldens the the retired shell `Floating`, `Cascade` and `Centred` classes are replayed against: free_space is the
    // `smart` placement, center and cascade keep their names. The same table is replayed natively by
    // `crates/gnoblin-std-impl/tests/examples.rs::floating_placement_matches_the_typescript_std`.
    let (cases, count) = placement_goldens();
    assert!(count > 50, "the golden table is not empty: {count}");
    let got = run(&format!(
        r#"
        const cases: any[] = {cases};
        const placement: any = {{ free_space: "smart", center: "center", cascade: "cascade" }};
        const toRect = ([x, y, width, height]: number[]) => ({{ x, y, width, height }});
        const failures: string[] = [];
        for (const c of cases) {{
            const got = rect(FloatingLayout.place({{ mode: placement[c.mode], width: c.size[0], height: c.size[1], area: toRect(c.area), occupied: c.occupied.map(toRect) }}));
            if (got !== c.result.join(",")) failures.push(`${{c.name}}: got ${{got}}, want ${{c.result.join(",")}}`);
        }}
        emit(failures.length === 0 ? `ok ${{cases.length}}` : failures.join("\n"));
        "#
    ));
    assert_eq!(got, format!("ok {count}"));
}

#[test]
#[ignore = "vrnw: needs client.floatingBounds and client.frameInset (the layout places a client once, at its committed size, and keeps floating bounds it was given)"]
fn floating_places_a_client_once_when_it_is_added() {
    // the retired shell `Floating:on_child_added`: a client at its committed size opens in free space, one with no size opens
    // at 800 by 600, an invisible client is not an obstacle, the client stops being tiled, and a second call
    // does not move it. A client the host has already given floating bounds keeps them.
    let got = run(r#"
        const layout = new FloatingLayout();
        const screen = area(1920, 1080);
        const [a, b, hidden, c, kept]: any[] = nodes(5);
        a.bounds = { x: 0, y: 0, width: 400, height: 300 }; a.tiled = true;
        b.bounds = { x: 0, y: 0, width: 0, height: 0 };
        hidden.bounds = { x: 0, y: 0, width: 1920, height: 1080 }; hidden.visible = false;
        c.bounds = { x: 0, y: 0, width: 400, height: 300 };
        kept.bounds = { x: 7, y: 9, width: 10, height: 10 }; kept.floatingBounds = { x: 7, y: 9, width: 10, height: 10 }; kept.tiled = true;
        layout.clientAdded({ workspace: "w", children: [a], area: screen }, a);
        layout.clientAdded({ workspace: "w", children: [a, b], area: screen }, b);
        const afterB = rect(b.bounds);
        layout.clientAdded({ workspace: "w", children: [a, b], area: screen }, b);
        layout.clientAdded({ workspace: "w", children: [a, b, hidden, c], area: screen }, c);
        layout.clientAdded({ workspace: "w", children: [kept], area: screen }, kept);
        emit([rect(a.bounds), a.tiled, afterB, rect(b.bounds), rect(c.bounds), rect(kept.bounds), kept.tiled].join("\n"));
    "#);
    // c is 400 by 300 and the free space after a (0..400) and b (400..1200 by 0..600) is x=1200.
    assert_eq!(
        got,
        lines(&[
            "0,0,400,300",
            "false",
            "400,0,800,600",
            "400,0,800,600",
            "1200,0,400,300",
            "7,9,10,10",
            "false",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: `Binding` and `BindingCollection` are the hand-written TypeScript classes (a trigger string parsed in script, ids kept in a script collection); the generated `Binding` is the native one (see native_bindings.rs)"]
fn bindings_carry_the_lifecycle_hooks_and_a_collection_keeps_them_by_id() {
    let got = run(r#"
        const b = new Binding({ id: "swap", trigger: "Super+MouseLeft+Drag" });
        const bindings = new BindingCollection();
        bindings.add(b);
        emit([
            [b.id, b.trigger, b.enabled, b.priority].join(","),
            b.onStart({ timestamp: 0 }),
            [bindings.size, bindings.has("swap"), bindings.get("swap") === b, [...bindings].length].join(","),
            thrown(() => bindings.add(new Binding({ id: "swap", trigger: "Super+x" }))),
            bindings.remove("swap") && bindings.size,
            thrown(() => new Binding({ id: "", trigger: "Super+x" })),
            thrown(() => new Binding({ id: "x", trigger: "Super++" })),
            thrown(() => new Binding({ id: "x", trigger: "Super+x", priority: Number.NaN })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "swap,Super+MouseLeft+Drag,true,0",
            "false",
            "1,true,true,1",
            r#"Error: a binding with id "swap" is already in this collection"#,
            "0",
            "TypeError: Binding id must be a non-empty string",
            r#"TypeError: Binding "x" trigger: an empty part after "Super"; write a literal plus as "plus" (token "" in "Super++")"#,
            r#"RangeError: Binding "x" priority must be a finite number"#,
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: `DragBinding` and `PointerDragBinding` are abstract TypeScript base classes of the old binding library; a script now extends nothing (it gives `input.bindDrag` a function) and the native drags are MoveBinding, ResizeBinding and SwapBinding"]
fn drag_binding_is_the_abstract_base_of_the_pointer_drags() {
    let got = run(r#"
        class Mine extends DragBinding {
            onStart() {
                return {};
            }
        }
        const mine = new Mine({ id: "mine", trigger: "Super+MouseLeft+Drag" });
        emit([
            thrown(() => new DragBinding({ id: "x", trigger: "Super+x" })),
            [mine instanceof DragBinding, mine instanceof Binding, new PointerDragBinding({ id: "p", trigger: "Super+MouseLeft+Drag" }) instanceof DragBinding].join(","),
            JSON.stringify(mine.onStart({ timestamp: 0 })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "TypeError: DragBinding is abstract: extend it, or PointerDragBinding",
            "true,true,true",
            "{}",
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: a TypeScript layout carried `triggers` (swap, resize) that `resolveTriggers` merged; the generated layouts carry no triggers, the std bindings (SwapBinding, ResizeBinding, MoveBinding) take their `trigger` option"]
fn an_explicit_undefined_trigger_keeps_the_default_and_null_turns_it_off() {
    // spec-review.md 1.19: `{ ...defaults, ...triggers }` would let `undefined` erase the default.
    let got = run(r#"
        const t = (triggers: any) => JSON.stringify(new MasterStackLayout({ triggers }).triggers);
        emit([
            t(undefined),
            t({ swap: undefined }),
            t({ swap: null }),
            t({ swap: "Alt+MouseLeft+Drag", resize: undefined }),
            thrown(() => t({ swapp: "Alt+x" })),
            JSON.stringify(resolveTriggers({ a: "A+1" }, { a: undefined })),
            JSON.stringify(new FloatingLayout({ triggers: { move: null } }).triggers),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            r#"{"swap":"Super+MouseLeft+Drag","resize":"Super+MouseRight+Drag"}"#,
            r#"{"swap":"Super+MouseLeft+Drag","resize":"Super+MouseRight+Drag"}"#,
            r#"{"swap":null,"resize":"Super+MouseRight+Drag"}"#,
            r#"{"swap":"Alt+MouseLeft+Drag","resize":"Super+MouseRight+Drag"}"#,
            r#"TypeError: unknown trigger "swapp"; expected one of swap, resize"#,
            r#"{"a":"A+1"}"#,
            r#"{"move":null,"resize":"Super+MouseRight+Drag"}"#,
        ])
    );
}

#[test]
#[ignore = "vrnw: TS internal: the TypeScript `Layout` base class (constructible, a `bindings` collection, hooks kept on the prototype); the generated `Layout` is abstract and has no `bindings`"]
fn the_layout_base_class_places_nothing_and_hooks_stay_on_the_prototype() {
    // An optional hook must not become an own `undefined` field that hides a subclass's method.
    let got = run(r#"
        const base = new Layout();
        const floating = new FloatingLayout();
        const grid = new GridLayout();
        emit([
            base.compute({ children: nodes(2), area: area() }).length,
            base.bindings instanceof BindingCollection,
            Object.hasOwn(floating, "onClientAdded"),
            typeof floating.onClientAdded,
            typeof grid.onClientAdded,
            floating instanceof Layout && grid instanceof Layout,
            floating.compute({ children: nodes(2), area: area() }).length,
        ].join(","));
    "#);
    assert_eq!(got, "0,true,false,function,undefined,true,0");
}

#[test]
#[ignore = "vrnw: TS internal: `checkWorkspaceOptions` of `gnoblin-std/workspace` (option checking of the TypeScript Workspace, layout by stock name); the generated Workspace takes an id and a Layout instance and checks them natively"]
fn a_workspaces_options_are_checked_and_its_layout_is_a_stock_name_or_an_instance() {
    let got = run(r#"
        import { checkWorkspaceOptions } from "gnoblin-std/workspace";
        const dev = new MasterStackLayout({ ratio: 0.65 });
        const bad = (options: any) => thrown(() => checkWorkspaceOptions(options));
        const checked = checkWorkspaceOptions({ id: "dev", layout: dev, gaps: { inner: 12 } });
        const plain = checkWorkspaceOptions({ id: "desktop" });
        emit([
            checked.layout === dev,
            JSON.stringify(checked.gaps),
            plain.name + ":" + plain.layout.constructor.name + ":" + JSON.stringify(plain.gaps),
            checkWorkspaceOptions({ id: "tiles", layout: "grid" }).layout.constructor.name,
            bad({ id: "a", layout: "spiral" }),
            bad({ id: "a", tiled: true }),
            bad({ id: "" }),
            bad({ id: "a", gaps: { inner: -1 } }),
            bad({ id: "a", gaps: { middle: 1 } }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true",
            r#"{"inner":12,"outer":0}"#,
            r#"desktop:FloatingLayout:{"inner":0,"outer":0}"#,
            "GridLayout",
            "TypeError: Workspace options.layout must be a Layout or one of floating, master-stack, grid, monocle, dwindle, kiosk, strip, carousel-3d, got spiral",
            "TypeError: Workspace options has no \"tiled\"; expected one of id, name, layout, gaps",
            "TypeError: Workspace options.id must be a non-empty string",
            "RangeError: Workspace options.gaps.inner must be a finite number of at least 0, got -1",
            "TypeError: Workspace options.gaps has no \"middle\"; expected one of inner, outer",
        ])
    );
}
