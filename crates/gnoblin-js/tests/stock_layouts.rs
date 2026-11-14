//! The stock TypeScript library (`data/js/gnoblin/`) run in a real generation against a fake `gnoblin` global.
//!
//! The geometry cases are the Lua shell's: `crates/gnoblin-lua/tests/lua/tiling_test.lua` for the tiled layouts
//! and `tests/shell/placement_cases.toml` for floating placement. A case taken from the Lua corpus says so; a
//! case marked "derived" is not in that corpus (the Lua class has no such option) and its expected rectangles
//! were worked out by hand from the same algorithm. Rectangles are written `x,y,width,height`, one node after
//! another, in the Lua tests' 1000 by 800 workspace.

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
const rectText = (r) => [r.x, r.y, r.width, r.height].join(",");
// The placements as "x,y,w,h" per node, after checking each is for its node, tiled, whole pixels and finite.
export function fmt(placements, list) {
    return placements
        .map((p, i) => {
            if (p.node !== list[i]) throw new Error(`placement ${i} is for the wrong node`);
            if (p.tiled !== true) throw new Error(`placement ${i} is not tiled`);
            for (const v of [p.bounds.x, p.bounds.y, p.bounds.width, p.bounds.height]) {
                if (!Number.isInteger(v)) throw new Error(`placement ${i} is not whole pixels: ${rectText(p.bounds)}`);
            }
            return rectText(p.bounds);
        })
        .join(" ");
}
// `n` fresh nodes laid out by `layout`; `ctx` overrides area, gap and focus.
export function lay(layout, n, ctx = {}) {
    const children = nodes(n);
    return fmt(layout.compute({ children, area: area(), ...ctx }), children);
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

const IMPORTS: &str = r#"
import { area, nodes, fmt, lay, thrown, rect, emit } from "harness";
import {
    Binding, BindingCollection, resolveTriggers, Layout, insetRect,
    FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout,
    defineConfig, desugarConfig, defaultConfig, mergeConfig, spawn, show,
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

#[test]
fn master_stack_matches_the_lua_tiling_goldens() {
    // Lua: "1 window fills", "2 and 3 windows give the master and stack rectangles", "the gap separates the
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
fn master_stack_tracks_ratio_gap_and_area_like_lua() {
    // Lua: "ratio, gap, width and height re-arrange with no handler": the same layout object over a shrinking
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
fn master_stack_200_nodes_share_the_stack_in_whole_pixels_like_lua() {
    // Lua: "200 windows share the stack in whole pixels".
    let got = run(r#"
        const children = nodes(200);
        const placed = new MasterStackLayout().compute({ children, area: area() });
        fmt(placed, children);
        emit([rect(placed[0].bounds), rect(placed[1].bounds), rect(placed[199].bounds), placed.length].join(" "));
    "#);
    assert_eq!(got, "0,0,500,800 500,0,500,4 500,795,500,4 200");
}

#[test]
fn master_stack_options_beyond_the_lua_corpus_are_derived_from_its_algorithm() {
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
            "0,0,1000,266 0,266,1000,266 0,533,1000,266",
        ])
    );
}

#[test]
fn grid_matches_the_lua_tiling_goldens() {
    // Lua: "1, 2 and 3 windows give the expected cells", "the gap separates cells and a partly filled last row is
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
    // Derived: the Lua class has no `columns`; a fixed count replaces the near-square side and keeps the rest.
    let got = run(r#"
        emit([
            lay(new GridLayout({ columns: 3 }), 4),
            lay(new GridLayout({ columns: 1 }), 3),
            lay(new GridLayout({ columns: "auto" }), 3),
            thrown(() => lay(new GridLayout({ columns: 0 }), 1)),
            thrown(() => lay(new GridLayout({ columns: 1.5 }), 1)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0,0,333,400 333,0,333,400 666,0,333,400 0,400,333,400",
            "0,0,1000,266 0,266,1000,266 0,533,1000,266",
            "0,0,500,400 500,0,500,400 0,400,500,400",
            r#"RangeError: GridLayout columns must be "auto" or a whole number of at least 1"#,
            r#"RangeError: GridLayout columns must be "auto" or a whole number of at least 1"#,
        ])
    );
}

#[test]
fn monocle_shows_the_focused_node_or_the_first_like_lua() {
    // Lua: "the focused window fills the workspace and the rest are hidden", "refocusing shows another window".
    let got = run(r#"
        const visible = (placed) => placed.map((p) => (p.visible ? "T" : "F")).join("");
        const l = new MonocleLayout();
        const children = nodes(3);
        const at = (focused) => {
            const placed = l.compute({ children, area: area(), focused });
            return fmt(placed, children) + " " + visible(placed);
        };
        emit([
            lay(l, 0),
            lay(l, 1),
            at(undefined),
            at(children[1]),
            at(children[2]),
            at({ name: "not a child" }),
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
    // Lua: "200 windows give finite integral rectangles" and "the same sequence replayed twice gives identical
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
fn layout_options_have_the_lua_defaults_and_bad_ones_are_refused() {
    // Lua: "the options have the old layouts' defaults": ratio 0.5, gap 0 (unset here, so the workspace's gap
    // applies), master count 1, orientation left; grid fill row; and the orientation and fill errors.
    let got = run(r#"
        const m = new MasterStackLayout();
        const g = new GridLayout();
        emit([
            [m.ratio, m.gap, m.masterCount, m.orientation].join(","),
            [g.columns, g.fill, g.gap].join(","),
            thrown(() => lay(new MasterStackLayout({ orientation: "middle" as any }), 1)),
            thrown(() => lay(new GridLayout({ fill: "diagonal" as any }), 1)),
            thrown(() => new MasterStackLayout({ gap: -1 })),
            thrown(() => new GridLayout({ gap: Number.NaN })),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "0.5,,1,left",
            "auto,row,",
            "TypeError: MasterStackLayout orientation must be left, right, top or bottom",
            "TypeError: GridLayout fill must be row or column",
            "RangeError: MasterStackLayout gap must be a finite number of at least 0, got -1",
            "RangeError: GridLayout gap must be a finite number of at least 0, got NaN",
        ])
    );
}

#[test]
fn the_workspace_gaps_inset_the_area_and_set_the_gap() {
    // Derived: LayoutContext.gap and the outer gap (spec-review.md section 4). An inner gap of 10 and an outer
    // gap of 5 on 1000 by 800 is a 990 by 790 area at (5, 5); a layout's own gap, even 0, wins over the inner
    // one. Monocle and the grid take the outer gap too; floating takes neither.
    let got = run(r#"
        const children = nodes(3);
        const ctx = { children, area: area(), gap: { inner: 10, outer: 5 } };
        emit([
            fmt(new MasterStackLayout().compute(ctx), children),
            fmt(new MasterStackLayout({ gap: 0 }).compute(ctx), children),
            fmt(new GridLayout().compute({ ...ctx, children: children.slice(0, 2) }), children.slice(0, 2)),
            fmt(new MonocleLayout().compute({ ...ctx, children: children.slice(0, 1) }), children.slice(0, 1)),
            rect(insetRect(area(10, 10), 8)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "5,5,490,790 505,5,490,390 505,405,490,390",
            "5,5,495,790 500,5,495,395 500,400,495,395",
            "5,5,490,790 505,5,490,790",
            "5,5,990,790",
            "8,8,0,0",
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
fn floating_placement_matches_every_lua_golden() {
    // The goldens the Lua `Floating`, `Cascade` and `Centred` classes are replayed against: free_space is the
    // `smart` placement, center and cascade keep their names.
    let (cases, count) = placement_goldens();
    assert!(count > 50, "the golden table is not empty: {count}");
    let got = run(&format!(
        r#"
        const cases: any[] = {cases};
        const placement: any = {{ free_space: "smart", center: "center", cascade: "cascade" }};
        const toRect = ([x, y, width, height]: number[]) => ({{ x, y, width, height }});
        const failures: string[] = [];
        for (const c of cases) {{
            const layout = new FloatingLayout({{ placement: placement[c.mode] }});
            const got = rect(layout.place(c.size[0], c.size[1], toRect(c.area), c.occupied.map(toRect)));
            if (got !== c.result.join(",")) failures.push(`${{c.name}}: got ${{got}}, want ${{c.result.join(",")}}`);
        }}
        emit(failures.length === 0 ? `ok ${{cases.length}}` : failures.join("\n"));
        "#
    ));
    assert_eq!(got, format!("ok {count}"));
}

#[test]
fn floating_places_a_client_once_when_it_is_added() {
    // Lua `Floating:on_child_added`: a client at its committed size opens in free space, one with no size opens
    // at 800 by 600, an invisible client is not an obstacle, the client stops being tiled, and a second call
    // does not move it. A client the host has already given floating bounds keeps them.
    let got = run(r#"
        const layout = new FloatingLayout();
        const screen = area(1920, 1080);
        const a: any = { bounds: { x: 0, y: 0, width: 400, height: 300 }, tiled: true };
        const b: any = { bounds: { x: 0, y: 0, width: 0, height: 0 } };
        const hidden: any = { bounds: { x: 0, y: 0, width: 1920, height: 1080 }, visible: false };
        const c: any = { bounds: { x: 0, y: 0, width: 400, height: 300 } };
        const kept: any = { bounds: { x: 7, y: 9, width: 10, height: 10 }, floatingBounds: { x: 7, y: 9, width: 10, height: 10 }, tiled: true };
        layout.onClientAdded(a, { children: [a], area: screen });
        layout.onClientAdded(b, { children: [a, b], area: screen });
        const afterB = rect(b.bounds);
        layout.onClientAdded(b, { children: [a, b], area: screen });
        layout.onClientAdded(c, { children: [a, b, hidden, c], area: screen });
        layout.onClientAdded(kept, { children: [kept], area: screen });
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
fn define_config_returns_its_argument_and_registers_it_with_the_host() {
    let got = run(r#"
        const registered = (globalThis as any).__registered;
        const config = { gaps: { inner: 12 } };
        const returned = defineConfig(config);
        const afterOne = registered.length;
        const same = registered[0] === config && returned === config;
        const failed = thrown(() => defineConfig({ gaps: { inner: -1 } }));
        const afterFailure = registered.length;
        // No host object, and a host with no __register hook, are both fine.
        const g = globalThis as any;
        const host = g.gnoblin;
        delete g.gnoblin;
        defineConfig({});
        g.gnoblin = {};
        defineConfig({});
        g.gnoblin = host;
        emit([afterOne, same, failed, afterFailure].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "1",
            "true",
            "RangeError: gaps.inner must be a finite number of at least 0, got -1",
            "1",
        ])
    );
}

#[test]
fn desugaring_a_config_gives_the_plan_of_the_spec_review_table() {
    let got = run(r##"
        const dev = new MasterStackLayout({ ratio: 0.65 });
        const handler = () => {};
        const setup = () => {};
        const plan = desugarConfig(defineConfig({
            gaps: { inner: 12, outer: 8 },
            style: { cornerRadius: 12, shadows: [{ y: 8, blur: 24, color: "#00000066" }] },
            outputs: { "DP-1": { mode: "3440x1440@144", scale: 1.25 } },
            workspaces: [{ id: "desktop", layout: "floating" }, { id: "dev", layout: dev }, { id: "tiles", layout: "grid" }],
            keys: {
                "Super+Return": spawn("ghostty"),
                "Super+Q": "close",
                "Super+Tab": handler,
                ...show.numbered("Super+{n}", ["desktop", "dev"]),
            },
            rules: [{ appId: "firefox", workspace: "dev", style: { cornerRadius: 18 } }],
            on: { clientAdded: handler },
            setup,
        }));
        emit([
            JSON.stringify(plan.gaps),
            plan.style.cornerRadius,
            JSON.stringify(plan.outputs),
            plan.workspaces.map((w) => w.id + ":" + w.layout.constructor.name).join(" "),
            plan.workspaces[1].layout === dev,
            JSON.stringify(plan.workspaces.map((w) => w.gaps)[2]),
            plan.bindings.map((b) => b.trigger + "=" + b.action.kind).join(" "),
            JSON.stringify(plan.bindings[0].action),
            JSON.stringify(plan.bindings[4].action),
            (plan.bindings[2].action as any).handler === handler,
            JSON.stringify(plan.rules),
            plan.listeners.map((l) => l.event + ":" + (l.handler === handler)).join(" "),
            plan.setup === setup,
        ].join("\n"));
    "##);
    assert_eq!(
        got,
        lines(&[
            r#"{"inner":12,"outer":8}"#,
            "12",
            r#"{"DP-1":{"mode":"3440x1440@144","scale":1.25}}"#,
            "desktop:FloatingLayout dev:MasterStackLayout tiles:GridLayout",
            "true",
            r#"{"inner":12,"outer":8}"#,
            "Super+Return=spawn Super+Q=close Super+Tab=call Super+1=show Super+2=show",
            r#"{"kind":"spawn","argv":["ghostty"]}"#,
            r#"{"kind":"show","workspace":"dev"}"#,
            "true",
            r#"[{"appId":"firefox","workspace":"dev","style":{"cornerRadius":18}}]"#,
            "clientAdded:true",
            "true",
        ])
    );
}

#[test]
fn a_missing_key_adds_nothing_except_that_no_workspace_means_the_stock_four() {
    let got = run(r#"
        const plan = desugarConfig({});
        emit([
            JSON.stringify(plan.gaps),
            JSON.stringify(plan.style),
            plan.workspaces.map((w) => w.id + ":" + w.layout.constructor.name).join(" "),
            plan.bindings.length,
            plan.rules.length,
            plan.listeners.length,
            plan.setup === undefined,
            desugarConfig({ workspaces: [] }).workspaces.length,
            desugarConfig({ workspaces: [{ id: "a" }] }).workspaces[0].layout.constructor.name,
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            r#"{"inner":0,"outer":0}"#,
            "{}",
            "1:FloatingLayout 2:FloatingLayout 3:FloatingLayout 4:FloatingLayout",
            "0",
            "0",
            "0",
            "true",
            "4",
            "FloatingLayout",
        ])
    );
}

#[test]
fn a_config_that_is_not_in_the_closed_set_is_refused_naming_the_key() {
    let got = run(r#"
        const bad = (config: any) => thrown(() => desugarConfig(config));
        emit([
            bad({ colour: 1 }),
            bad({ keys: { "Super+1": show("nowhere") } }),
            bad({ keys: { "Super+X": 5 } }),
            bad({ keys: { "Super+": "close" } }),
            bad({ keys: { "Super+X": { kind: "run" } } }),
            bad({ workspaces: [{ id: "a" }, { id: "a" }] }),
            bad({ workspaces: [{ id: "a", layout: "spiral" }] }),
            bad({ workspaces: [{ id: "a", tiled: true }] }),
            bad({ rules: [{ workspace: "desktop" }] }),
            bad({ rules: [{ appId: "x", workspace: "elsewhere" }] }),
            bad({ rules: [{ appId: "x", match: "y" }] }),
            bad({ gaps: { inner: -1 } }),
            bad({ gaps: { middle: 1 } }),
            bad({ on: { clientMoved() {} } }),
            bad({ on: { clientAdded: 3 } }),
            bad({ setup: "later" }),
            thrown(() => show.numbered("Super", ["a"])),
            thrown(() => show.numbered("Super+{n}", Array.from({ length: 10 }, (_, i) => "w" + i))),
            thrown(() => spawn([])),
            thrown(() => spawn("")),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "TypeError: config has no \"colour\"; expected one of gaps, style, outputs, workspaces, keys, launcher, autostart, rules, on, setup, decorations, menus, alerts, focus, edgeTiling, dragConstraint, gestureActions, workspaceSets, compositor, windowManagement, cursor, appearance, dialogs, input, inputSources, accessibility, layerShell, keyring, auth, location, compat, capture, monitors, xwayland, logging, protocols, touchpadGestures",
            "Error: keys[\"Super+1\"] shows workspace \"nowhere\", which the config does not declare",
            "TypeError: keys[\"Super+X\"] must be spawn(...), show(...), show.position(...), focusColumn(...), binding(...), \"close\" or a function",
            "TypeError: a key: an empty part after \"Super\"; write a literal plus as \"plus\" (token \"\" in \"Super+\")",
            "TypeError: keys[\"Super+X\"] must be spawn(...), show(...), show.position(...), focusColumn(...), binding(...), \"close\" or a function",
            "Error: workspaces[1]: a workspace with id \"a\" is already declared",
            "TypeError: workspaces[0].layout must be a Layout or one of floating, master-stack, grid, monocle, dwindle, kiosk, strip, carousel-3d, got spiral",
            "TypeError: workspaces[0] has no \"tiled\"; expected one of id, name, layout",
            "TypeError: rules[0]: needs a query: appId, title, focused or onWorkspace",
            "Error: rules[0].workspace: moves to workspace \"elsewhere\", which the config does not declare",
            "TypeError: rules[0] has no \"match\"; expected one of appId, title, role, focused, onWorkspace, workspace, floating, size, position, style, fullscreen, maximised, secure, skipFocus, first, reapply",
            "RangeError: gaps.inner must be a finite number of at least 0, got -1",
            "TypeError: gaps has no \"middle\"; expected one of inner, outer",
            "TypeError: on has no \"clientMoved\" (did you mean \"clientRemoved\"?); expected one of clientAdded, clientRemoved, titleChanged, focusChanged",
            "TypeError: on.clientAdded must be a function",
            "TypeError: setup must be a function",
            "TypeError: show.numbered needs a template with {n}, such as \"Super+{n}\"",
            "RangeError: show.numbered binds at most 9 workspaces, got 10",
            "TypeError: spawn needs a command name and string arguments",
            "TypeError: spawn needs a command name and string arguments",
        ])
    );
}

#[test]
fn the_default_config_is_four_floating_workspaces_with_the_stock_keys() {
    let got = run(r#"
        const plan = desugarConfig(defaultConfig);
        emit([
            plan.workspaces.map((w) => w.id + ":" + w.layout.constructor.name).join(" "),
            plan.bindings.map((b) => b.trigger).join(" "),
            JSON.stringify(plan.bindings[0].action),
            JSON.stringify(plan.bindings[12].action),
            Object.isFrozen(defaultConfig) && Object.isFrozen(defaultConfig.keys) && Object.isFrozen(defaultConfig.workspaces),
            (globalThis as any).__registered.length,
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "1:FloatingLayout 2:FloatingLayout 3:FloatingLayout 4:FloatingLayout",
            "Super+1 Super+2 Super+3 Super+4 Super+5 Super+6 Super+7 Super+8 Super+9 Super+Left Super+Right Swipe3 XF86AudioRaiseVolume XF86AudioLowerVolume XF86AudioMute XF86AudioMicMute XF86AudioPlay XF86AudioNext XF86AudioPrev",
            r#"{"kind":"show-position","position":1}"#,
            r#"{"kind":"spawn","argv":["wpctl","set-volume","@DEFAULT_AUDIO_SINK@","5%+"]}"#,
            "true",
            "0",
        ])
    );
}

#[test]
fn decorations_are_on_in_the_default_config_and_off_in_one_that_does_not_ask() {
    let got = run(r#"
        emit([
            desugarConfig(defaultConfig).decorations,
            desugarConfig({}).decorations,
            desugarConfig({ decorations: true }).decorations,
            desugarConfig({ decorations: false }).decorations,
            thrown(() => desugarConfig({ decorations: "yes" } as any)),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        lines(&[
            "true",
            "false",
            "true",
            "false",
            "TypeError: config.decorations: must be true or false",
        ])
    );
}

#[test]
fn a_user_config_layers_over_the_default_keys_merge_by_trigger_and_workspaces_replace_whole() {
    let got = run(r#"
        const keys = (c: any) => Object.keys(c.keys ?? {}).join(" ");
        const ids = (c: any) => (c.workspaces ?? []).map((w: any) => w.id).join(" ");
        const before = JSON.stringify(defaultConfig);
        const nothing = mergeConfig(defaultConfig, {});
        const own = mergeConfig(defaultConfig, { keys: { "super+1": spawn("one"), "Super+Return": spawn("term") } });
        const other = mergeConfig(defaultConfig, { workspaces: [{ id: "a" }, { id: "b", layout: "grid" }] });
        const same = mergeConfig(defaultConfig, { workspaces: [{ id: "desktop", layout: "grid" }] });
        const gaps = mergeConfig({ gaps: { inner: 2 } }, { gaps: { outer: 4 } });
        const on = mergeConfig({ on: { clientAdded() {} } }, { on: { titleChanged() {} } });
        emit([
            ids(nothing) + " | " + keys(nothing),
            ids(own) + " | " + keys(own),
            JSON.stringify(own.keys!["super+1"]),
            ids(other) + " | " + keys(other),
            ids(same) + " | " + (same.keys!["Super+1"] as any).kind,
            (same.workspaces![0] as any).layout,
            JSON.stringify(gaps.gaps),
            Object.keys(on.on!).join(" "),
            JSON.stringify(defaultConfig) === before,
            thrown(() => mergeConfig(defaultConfig, { kyes: {} } as any)),
            thrown(() => mergeConfig(defaultConfig, { keys: { "Super+!": "close" } })).startsWith("TypeError: a key: "),
            desugarConfig(other).bindings.map((b) => b.trigger).join(" "),
        ].join("\n"));
    "#);
    let media = "XF86AudioRaiseVolume XF86AudioLowerVolume XF86AudioMute XF86AudioMicMute XF86AudioPlay XF86AudioNext XF86AudioPrev";
    // The Lua shell's keys: Super+1 to Super+9 show the workspace at that position, so they name something whichever
    // workspaces the config declares and survive a config that drops the stock four.
    let others = "Super+2 Super+3 Super+4 Super+5 Super+6 Super+7 Super+8 Super+9 Super+Left Super+Right Swipe3";
    let stock = format!("Super+1 {others}");
    assert_eq!(
        got,
        lines(&[
            &format!("1 2 3 4 | {stock} {media}"),
            &format!("1 2 3 4 | {others} {media} super+1 Super+Return"),
            r#"{"kind":"spawn","argv":["one"]}"#,
            &format!("a b | {stock} {media}"),
            "desktop | show-position",
            "grid",
            r#"{"inner":2,"outer":4}"#,
            "clientAdded titleChanged",
            "true",
            "TypeError: config has no \"kyes\" (did you mean \"keys\"?); expected one of gaps, style, outputs, workspaces, keys, launcher, autostart, rules, on, setup, decorations, menus, alerts, focus, edgeTiling, dragConstraint, gestureActions, workspaceSets, compositor, windowManagement, cursor, appearance, dialogs, input, inputSources, accessibility, layerShell, keyring, auth, location, compat, capture, monitors, xwayland, logging, protocols, touchpadGestures",
            "true",
            &format!("{stock} {media}"),
        ])
    );
}
