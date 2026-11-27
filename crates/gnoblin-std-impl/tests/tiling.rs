//! The dwindle, strip, arc and carousel layouts, the split-resize drag, snapping and the tile swap of the native std, held to the
//! TypeScript std (`data/js/gnoblin-std`), which is the reference.
//!
//! One script of events (a window opens, the keyboard moves, a chord is pressed, a pointer chord is dragged) is played to a TypeScript
//! shell in this process over the fake compositor. After each `step` the windows' rectangles and the placement properties a layout
//! writes (tiled, visible, depth, rotation, scale) are read and held to the fixture `tiling_<name>`, which `objects.rs` and
//! `policy_replay.rs` replay to the native std.
//! The script language is `crates/gnoblin-core-impl/src/driver_script.rs`'s.

use std::fs;
use std::path::Path;
use std::time::Duration;

mod reference;
use reference::Reference;

use gnoblin_js::JsShellConfig;
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_testkit::oracle::{Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;

/// What one window looks like at a step.
#[derive(Clone, Debug, PartialEq)]
struct Obs {
    id: u64,
    /// `None` for a window that is gone.
    rect: Option<[f64; 4]>,
    tiled: bool,
    visible: bool,
    z: f64,
    rotation: Vec<f64>,
    scale: Vec<f64>,
}

type Steps = Vec<(String, Vec<Obs>)>;

// ---- the TypeScript side -----------------------------------------------------------------------------------------------------

/// The world of the launcher's scenarios: a 1000 by 800 screen, a seat and the window 40, with both spellings of the screen's geometry
/// because the two libraries read the rectangle by different keys.
fn world() -> FakeCompositor {
    let comp = FakeCompositor::new();
    let double = MirrorValue::Double;
    comp.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("screen_x", double(0.0)),
            ("screen_y", double(0.0)),
            ("screen_width", double(1000.0)),
            ("screen_height", double(800.0)),
            ("x", double(0.0)),
            ("y", double(0.0)),
            ("width", double(1000.0)),
            ("height", double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_window(40, "Terminal", "foot");
    comp
}

/// The TypeScript shell with the chords of the tiling script.
///
/// A shell runs on the native library unless its import graph reaches a module only the TypeScript library has
/// (`crates/gnoblin-js/src/choice.rs`), so the reference imports `gnoblin-std/layout`: without it this would run the generated
/// classes and compare the native std with itself.
const SOURCE: &str = r#"
import "gnoblin-std";
import "gnoblin-std/layout";
import { GestureBinding, PointerDragBinding } from "gnoblin-core";
import {
    Workspace, MasterStackLayout, DwindleLayout, StripLayout, ArcLayout, Carousel3dLayout, FloatingLayout, GridLayout,
} from "gnoblin-std";
const g = globalThis as any;
g.g = g;
const ws = new Workspace({ id: "main", layout: new MasterStackLayout(), gaps: { inner: 6, outer: 12 } });
gnoblin.workspaces.add(ws);
let dw: any = null;
let strip: any = null;
const dwindle = (options: object) => { dw = new DwindleLayout(options as any); ws.layout = dw; };
const stripOf = (column: number) => { strip = new StripLayout({ column }); ws.layout = strip; };
g.chords = {
    "Super+1": () => { ws.layout = new MasterStackLayout(); },
    "Super+2": () => dwindle({}),
    "Super+3": () => dwindle({ preserveSplit: true, ratio: 0.6, gap: 4 }),
    "Super+4": () => dwindle({ smartResizing: false }),
    "Super+5": () => dwindle({ smartSplit: true, pointer: () => ({ x: 700, y: 100 }) }),
    "Super+6": () => dwindle({
        useActiveForSplits: false, forceSplit: 1, splitBias: 1, pointer: () => ({ x: 100, y: 700 }),
    }),
    "Super+minus": () => dwindle({ split: "vertical", permanentDirectionOverride: true }),
    "Super+7": () => stripOf(0.5),
    "Super+8": () => stripOf(0.4),
    "Super+9": () => { ws.layout = new ArcLayout(); },
    "Super+0": () => { ws.layout = new Carousel3dLayout(); },
    "Super+equal": () => { ws.layout = new MasterStackLayout({ ratio: 0.6, masterCount: 2, orientation: "right" }); },
    "Super+space": () => { ws.layout = new FloatingLayout(); },
    "Super+g": () => { ws.layout = new GridLayout(); },
    "Super+F1": () => dw?.toggleSplit(ws),
    "Super+F2": () => dw?.swapSplit(ws),
    "Super+F3": () => dw?.rotateSplit(ws, 90),
    "Super+F4": () => dw?.rotateSplit(ws, -90),
    "Super+F5": () => dw?.preselect("right"),
    "Super+F6": () => dw?.preselect("down"),
    "Super+F7": () => dw?.preselect("left"),
    "Super+F8": () => dw?.moveToRoot(ws, { stable: true }),
    "Super+F9": () => dw?.moveToRoot(ws, { stable: false }),
    "Super+F10": () => dw?.splitRatio(ws, 0.1),
    "Super+F11": () => dw?.splitRatio(ws, 0.3, { exact: true }),
    "Super+F12": () => dw?.pseudotile(ws),
    "Super+Left": () => strip?.focusLeft(ws),
    "Super+Right": () => strip?.focusRight(ws),
    "Super+Up": () => strip?.scrollTo(300),
    "Super+Down": () => new Carousel3dLayout().rotate(ws, 1),
    "Super+m": () => {
        const c = ws.clients.at(0) as any;
        c.minSize = { width: 150, height: 120 };
        c.maxSize = { width: 500, height: 400 };
    },
};

// Bindings of the shell's own, for the binding scenarios: each writes numbers into a window's rectangle (a floating workspace leaves
// them alone), so a step shows which binding was asked, in what order and how it ended. `shell-policy` has the same ones.
const input = gnoblin.input.primary;
const first = () => ws.clients.at(0) as any;
const put = (client: any, patch: object) => { client.bounds = { ...client.bounds, ...patch }; };
class GProbe extends GestureBinding {
    starts: string; sign: number; offset: number; endBy: number; cancelBy: number;
    constructor(trigger: string, starts: string, sign: number, offset: number, endBy: number, cancelBy: number) {
        super({ id: trigger, trigger });
        this.starts = starts; this.sign = sign; this.offset = offset; this.endBy = endBy; this.cancelBy = cancelBy;
    }
    follow(e: any) { put(first(), { x: this.offset + this.sign * Math.round(e.progress * 1000) }); }
    onStart(_e: any) { const c = first(); put(c, { [this.starts]: c.bounds[this.starts] + 1 }); return {}; }
    onUpdate(e: any) { this.follow(e); }
    onEnd(e: any) { this.follow(e); const c = first(); put(c, { y: c.bounds.y + this.endBy }); }
    onCancel() { const c = first(); put(c, { y: c.bounds.y + this.cancelBy }); }
}
input.bind("Swipe3Left", new GProbe("Swipe3Left", "width", 1, 0, 10, 1000));
input.bind("Swipe3Right", new GProbe("Swipe3Right", "height", -1, 5000, 20, 2000));
input.bind("Swipe4", new GProbe("Swipe4", "width", 1, 10000, 30, 3000));
class DProbe extends PointerDragBinding {
    label: string;
    constructor(label: string, priority: number) {
        super({ id: label, trigger: "Super+MouseMiddle+Drag", priority });
        this.label = label;
    }
    onStart(e: any) { return e.client && (this.label === "B" || e.client.title !== "B") ? {} : false; }
    onUpdate(e: any) { if (e.client) put(e.client, this.label === "A" ? { x: e.position.x } : { y: e.position.y }); }
    onEnd(e: any) { if (e.client) put(e.client, { width: e.client.bounds.width + 1 }); }
}
input.bind("Super+MouseMiddle+Drag", new DProbe("B", 0));
input.bind("Super+MouseMiddle+Drag", new DProbe("A", 5));
"#;

fn observe(comp: &FakeCompositor, id: u64) -> Obs {
    if !comp.exists(id) {
        return Obs {
            id,
            rect: None,
            tiled: false,
            visible: true,
            z: 0.0,
            rotation: vec![0.0; 3],
            scale: vec![1.0; 3],
        };
    }
    let list_of = |key: &str, fallback: f64| match comp.value(id, key) {
        Some(MirrorValue::List(list)) => list,
        _ => vec![fallback; 3],
    };
    Obs {
        id,
        rect: comp.exists(id).then(|| {
            [
                comp.number(id, "x"),
                comp.number(id, "y"),
                comp.number(id, "width"),
                comp.number(id, "height"),
            ]
        }),
        tiled: comp.flag(id, "tiled").unwrap_or(false),
        visible: comp.flag(id, "visible").unwrap_or(true),
        z: comp.number(id, "z"),
        rotation: list_of("rotation", 0.0),
        scale: list_of("scale", 1.0),
    }
}

fn settle(shell: &mut Reference) {
    for _ in 0..6 {
        let diagnostics = shell.frame().expect("a frame").diagnostics;
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

fn point(text: &str) -> (f64, f64) {
    let (x, y) = text.split_once(',').unwrap_or((text, "0"));
    (x.parse().unwrap(), y.parse().unwrap())
}

/// `Super+Button1` as the compositor spells a claim: `<Super>Button1`.
fn claim_text(chord: &str) -> String {
    let (modifiers, button) = chord.rsplit_once('+').expect("a pointer chord");
    format!("<{}>{button}", modifiers.replace('+', "><"))
}

/// Plays `script` to the TypeScript shell.
fn reference(script: &str) -> Steps {
    let comp = world();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), SOURCE).expect("shell.ts is written");
    let mut config =
        JsShellConfig::new(&root).with_stock(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js"));
    config.entry_budget = Duration::from_secs(5);
    config.frame_budget = Duration::from_secs(10);
    config.clock = std::sync::Arc::new(ManualClock::default());
    let mut shell = Reference::new(&comp, config);
    let diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    settle(&mut shell);
    let mut windows = vec![40];
    let mut steps: Steps = Vec::new();
    for line in script.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let id = |at: usize| words.get(at).and_then(|word| word.parse::<u64>().ok()).unwrap_or(0);
        let real = |at: usize| words.get(at).and_then(|word| word.parse::<f64>().ok()).unwrap_or(0.0);
        let value = |text: &str| match text {
            "true" => MirrorValue::Bool(true),
            "false" => MirrorValue::Bool(false),
            number => MirrorValue::Double(number.parse().unwrap()),
        };
        match words.as_slice() {
            [] => {}
            [comment, ..] if comment.starts_with('#') => {}
            ["create", _, title, ..] => {
                comp.create_window(id(1), title, "app");
                windows.push(id(1));
                settle(&mut shell);
            }
            ["destroy", ..] => {
                comp.destroy(id(1));
                windows.retain(|window| *window != id(1));
                settle(&mut shell);
            }
            ["focus", ..] => {
                for window in &windows {
                    comp.external_set(*window, "focused", MirrorValue::Bool(*window == id(1)));
                }
                settle(&mut shell);
            }
            ["chord", name] => {
                shell
                    .eval(&format!("globalThis.chords[{name:?}]();"))
                    .expect("the chord runs");
                settle(&mut shell);
            }
            ["drag", chord, from, rest @ ..] if !rest.is_empty() => {
                let (last, path) = rest.split_last().unwrap();
                let claim_text = claim_text(chord);
                let claim = comp.claim_id(&claim_text);
                let from = point(from);
                comp.pointer_chord(claim, &claim_text, "start", from, from);
                shell.frame().expect("a frame");
                let mut at = from;
                for step in path {
                    at = point(step);
                    comp.pointer_chord(claim, &claim_text, "update", from, at);
                    shell.frame().expect("a frame");
                }
                comp.pointer_chord(claim, &claim_text, last, from, at);
                settle(&mut shell);
            }
            ["gesture", kind, fingers, phase, ..] => {
                let fingers: i64 = fingers.parse().unwrap_or(3);
                let claim = comp.gesture_claim(kind, fingers);
                comp.gesture(claim, kind, fingers, phase, (real(4), real(5)), real(6), real(7));
                settle(&mut shell);
            }
            ["place", ..] => {
                for (key, number) in [("x", real(2)), ("y", real(3)), ("width", real(4)), ("height", real(5))] {
                    comp.external_set(id(1), key, MirrorValue::Double(number));
                }
                settle(&mut shell);
            }
            ["seat", key, text] => {
                comp.external_set(21, key, value(text));
                settle(&mut shell);
            }
            ["window", _, key, text] => {
                comp.external_set(id(1), key, value(text));
                settle(&mut shell);
            }
            ["pump"] => settle(&mut shell),
            ["step", name, ids @ ..] => {
                steps.push((
                    (*name).to_owned(),
                    ids.iter().map(|one| observe(&comp, one.parse().unwrap())).collect(),
                ));
            }
            other => panic!("bad script line: {other:?}"),
        }
    }
    steps
}

/// What a step shows, as a fixture holds it.
fn steps_json(steps: &Steps) -> Value {
    Value::Array(
        steps
            .iter()
            .map(|(name, observed)| {
                json!({
                    "step": name,
                    "windows": observed.iter().map(|obs| json!({
                        "id": obs.id,
                        "rect": obs.rect,
                        "tiled": obs.tiled,
                        "visible": obs.visible,
                        "z": obs.z,
                        "rotation": obs.rotation,
                        "scale": obs.scale,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// Plays `script` to the TypeScript std and holds what it did to the fixture `tiling_<name>`; returns the steps for further checks. The
/// native std is held to the fixture by `objects.rs` (the layouts) and `policy_replay.rs` (the drags and gestures).
fn same(name: &str, script: &str) -> Steps {
    let reference = reference(script);
    reference::fixtures().json(
        &format!("tiling_{name}"),
        &json!({ "script": script, "steps": steps_json(&reference) }),
    );
    reference
}

fn rect(steps: &Steps, step: &str, id: u64) -> Option<[f64; 4]> {
    steps
        .iter()
        .find(|(name, _)| name == step)
        .and_then(|(_, observed)| observed.iter().find(|obs| obs.id == id))
        .and_then(|obs| obs.rect)
}

/// Four windows arrive one at a time, each taking the keyboard as it does, on the layout chord `layout`.
fn arrivals(layout: &str) -> String {
    format!(
        "focus 40\nchord {layout}\nstep one 40\ncreate 41 B\nfocus 41\nstep two 40 41\ncreate 42 C\nfocus 42\n\
         step three 40 41 42\ncreate 43 D\nfocus 43\nstep four 40 41 42 43\n"
    )
}

// ---- the dwindle -------------------------------------------------------------------------------------------------------------

#[test]
fn the_dwindle_places_arrivals_as_the_typescript_dwindle_does() {
    let steps = same("dwindle_arrivals", &arrivals("Super+2"));
    // Sanity: the first window fills the area (less the outer gap), and the second takes the right half of 1000 by 800.
    assert_eq!(rect(&steps, "one", 40), Some([12.0, 12.0, 976.0, 776.0]));
    assert_eq!(rect(&steps, "two", 40).map(|r| r[2]), Some(485.0));
}

#[test]
fn the_dwindles_actions_change_the_tree_as_the_typescript_dwindle_does() {
    // `Super+3` keeps each split's direction, so toggling a split shows. Every action runs on the window that has the keyboard.
    let mut script = arrivals("Super+3");
    for (name, chord) in [
        ("toggle", "Super+F1"),
        ("swap", "Super+F2"),
        ("rotate", "Super+F3"),
        ("rotate_back", "Super+F4"),
        ("to_root", "Super+F8"),
        ("to_root_unstable", "Super+F9"),
        ("ratio", "Super+F10"),
        ("ratio_exact", "Super+F11"),
        ("pseudo", "Super+F12"),
        ("pseudo_off", "Super+F12"),
    ] {
        script.push_str(&format!("chord {chord}\nstep {name} 40 41 42 43\n"));
    }
    script.push_str("focus 41\nchord Super+F1\nchord Super+F10\nstep other_window 40 41 42 43\n");
    script.push_str("destroy 42\nstep closed 40 41 42 43\ndestroy 40\nstep closed_root 40 41 42 43\n");
    same("dwindle_actions", &script);
}

#[test]
fn the_dwindle_preselects_the_side_of_the_next_window_as_the_typescript_dwindle_does() {
    let mut script = String::from("focus 40\nchord Super+2\ncreate 41 B\nfocus 41\n");
    // Right of the focused window, then down, then left; each choice is used once.
    for (n, chord) in [(42, "Super+F5"), (43, "Super+F6"), (44, "Super+F7")] {
        script.push_str(&format!(
            "chord {chord}\ncreate {n} W\nfocus {n}\nstep opened_{n} 40 41 42 43 44\n"
        ));
    }
    // A permanent choice (the vertical dwindle) stays for every window.
    script.push_str("chord Super+minus\nchord Super+F6\ncreate 45 X\nfocus 45\ncreate 46 Y\nfocus 46\nstep permanent 40 41 42 43 44 45 46\n");
    same("dwindle_preselect", &script);
}

#[test]
fn the_dwindles_pointer_options_choose_where_a_window_opens_as_the_typescript_dwindle_does() {
    // `Super+5`: smart split, the pointer is in the top right of the first cell. `Super+6`: the window under the pointer is split,
    // the new window goes first, and the layout's ratio is turned over for it.
    for chord in ["Super+5", "Super+6"] {
        same(
            &format!("dwindle_pointer_{}", chord.to_lowercase().replace('+', "_")),
            &arrivals(chord),
        );
    }
}

#[test]
fn the_dwindles_drag_moves_the_nearest_divider_and_a_cancel_puts_it_back_as_in_typescript() {
    let mut script = arrivals("Super+2");
    // Press on window 41 (the top right quarter of the area) near its left edge, which is the root divider.
    script.push_str("drag Super+Button3 480,200 500,200 620,200 end\nstep dragged 40 41 42 43\n");
    script.push_str("drag Super+Button3 700,300 700,360 700,300 cancel\nstep cancelled 40 41 42 43\n");
    // The nearest divider of a press low in the right column is the one between 42 and 43.
    script.push_str("drag Super+Button3 600,700 600,640 end\nstep lower 40 41 42 43\n");
    // Both dividers of the tile at once, without smart resizing.
    script.push_str("chord Super+4\nstep both_start 40 41 42 43\n");
    script.push_str("drag Super+Button3 600,700 650,660 end\nstep both 40 41 42 43\n");
    same("dwindle_drag", &script);
}

// ---- the strip ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_strip_scrolls_and_hands_the_keyboard_on_as_the_typescript_strip_does() {
    let mut script = arrivals("Super+7");
    for (name, chord) in [
        ("left", "Super+Left"),
        ("left_again", "Super+Left"),
        ("right", "Super+Right"),
        ("scrolled", "Super+Up"),
    ] {
        script.push_str(&format!("chord {chord}\nstep {name} 40 41 42 43\n"));
    }
    script.push_str("focus 42\nstep refocused 40 41 42 43\ndestroy 42\nstep closed 40 41 42 43\n");
    script.push_str("chord Super+8\nstep narrow 40 41 43\n");
    let steps = same("strip_scroll", &script);
    assert!(rect(&steps, "four", 43).is_some());
}

#[test]
fn the_strip_opens_a_window_after_the_focused_column_as_the_typescript_strip_does() {
    let mut script = String::from("focus 40\nchord Super+7\ncreate 41 B\nfocus 41\ncreate 42 C\nfocus 42\nfocus 40\n");
    script.push_str("create 43 D\nstep after_first 40 41 42 43\nfocus 43\nstep focused_new 40 41 42 43\n");
    same("strip_opens_after_focused", &script);
}

// ---- the arc and the carousel ------------------------------------------------------------------------------------------------

#[test]
fn the_arc_places_windows_round_the_viewer_as_the_typescript_arc_does() {
    let mut script = arrivals("Super+9");
    script.push_str("focus 41\nstep arc_focused 40 41 42 43\ndestroy 41\nstep arc_closed 40 41 42 43\n");
    let steps = same("arc", &script);
    let (_, observed) = &steps[3];
    assert!(observed.iter().any(|obs| obs.z != 0.0), "the arc gives depth");
}

#[test]
fn the_carousel_places_cards_on_a_ring_as_the_typescript_carousel_does() {
    let mut script = arrivals("Super+0");
    script.push_str("focus 41\nstep ring_focused 40 41 42 43\nchord Super+Down\nstep rotated 40 41 42 43\n");
    script.push_str("destroy 40\nstep ring_closed 40 41 42 43\n");
    same("carousel", &script);
}

// ---- the master and stack's divider ------------------------------------------------------------------------------------------

#[test]
fn the_split_resize_drag_moves_the_master_and_stack_divider_as_in_typescript() {
    let mut script =
        String::from("focus 40\nchord Super+1\ncreate 41 B\ncreate 42 C\ncreate 43 D\npump\nstep start 40 41 42 43\n");
    script.push_str("drag Super+Button3 800,200 850,200 900,210 end\nstep widened 40 41 42 43\n");
    script.push_str("drag Super+Button3 800,200 300,200 cancel\nstep cancelled 40 41 42 43\n");
    script.push_str("drag Super+Button3 800,200 100,200 end\nstep narrowed 40 41 42 43\n");
    // The master on the right grows as the pointer moves away from the right edge, so left.
    script.push_str("chord Super+equal\nstep right_start 40 41 42 43\n");
    script.push_str("drag Super+Button3 700,200 600,200 end\nstep right_wider 40 41 42 43\n");
    // A layout with no divider declines, and nothing moves.
    script.push_str(
        "chord Super+g\nstep grid 40 41 42 43\ndrag Super+Button1 700,200 600,200 end\nstep grid_swap 40 41 42 43\n",
    );
    same("split_resize_drag", &script);
}

// ---- the swap ----------------------------------------------------------------------------------------------------------------

#[test]
fn a_tile_dropped_on_another_swaps_places_in_the_dwindle_as_in_typescript() {
    let mut script = arrivals("Super+2");
    script.push_str("drag Super+Button1 700,700 400,400 100,100 end\nstep swapped 40 41 42 43\n");
    script.push_str("drag Super+Button1 700,100 9000,9000 end\nstep nowhere 40 41 42 43\n");
    script.push_str("chord Super+F1\nstep after_swap_toggle 40 41 42 43\n");
    same("tile_swap", &script);
}

// ---- floating: move, resize and snap -----------------------------------------------------------------------------------------

#[test]
fn a_window_dragged_to_a_screen_edge_snaps_as_in_typescript_and_unsnaps_when_taken_again() {
    let mut script = String::from("focus 40\nchord Super+space\nplace 40 300 200 400 300\nstep placed 40\n");
    // Edge tiling is off: a drop at the edge is just a move.
    script.push_str("drag Super+Button1 400,250 100,250 2,250 end\nstep no_snap 40\n");
    script.push_str("seat edge_tiling true\ndrag Super+Button1 100,250 300,250 500,250 end\nstep moved 40\n");
    script.push_str("drag Super+Button1 500,250 100,250 2,250 end\nstep left 40\n");
    script.push_str("drag Super+Button1 300,250 700,250 998,250 end\nstep right 40\n");
    script.push_str("drag Super+Button1 700,250 500,300 500,2 end\nstep top 40\n");
    // Taking a snapped window forgets its snap: it stays where the drag leaves it.
    script.push_str("drag Super+Button1 500,300 600,400 650,500 end\nstep taken 40\n");
    same("snap", &script);
}

#[test]
fn a_window_moved_or_resized_by_the_pointer_matches_typescript() {
    let mut script = String::from("focus 40\nchord Super+space\nplace 40 300 200 400 300\n");
    script.push_str("drag Super+Button1 400,250 450,280 end\nstep moved 40\n");
    script.push_str("drag Super+Button1 450,280 5000,-5000 end\nstep held_in 40\n");
    script.push_str("drag Super+Button3 900,100 800,50 end\nstep resized_top_right 40\n");
    script.push_str("drag Super+Button3 900,100 -4000,4000 end\nstep resized_min 40\n");
    script.push_str("drag Super+Button1 900,100 800,300 cancel\nstep cancelled 40\n");
    same("pointer_move_resize", &script);
}

// ---- bindings of the shell's own: priorities, decline, gestures, a window that closes ------------------------------------------

/// A floating workspace with the first window at the top left and a second one of the title `B` near the middle.
fn floating_world() -> String {
    String::from(
        "focus 40\nchord Super+space\ncreate 41 B\nplace 40 0 0 200 200\nplace 41 400 400 200 200\nstep placed 40 41\n",
    )
}

#[test]
fn two_bindings_on_one_chord_are_asked_by_priority_and_a_decline_leaves_the_drag_to_the_next() {
    let mut script = floating_world();
    // The press is on window 40: `A` (priority 5) takes it and moves it along x; its end counts in the width.
    script.push_str("drag Super+Button2 100,100 150,120 end\nstep a_takes 40 41\n");
    // The press is on window 41, whose title `A` declines: `B` takes it and moves it along y.
    script.push_str("drag Super+Button2 450,450 470,500 end\nstep b_takes 40 41\n");
    // A drag that stays under the threshold starts nothing.
    script.push_str("drag Super+Button2 100,100 102,101 end\nstep too_short 40 41\n");
    // A drag that is cancelled is cancelled on the binding that took it, which does nothing of its own.
    script.push_str("drag Super+Button2 100,100 160,120 cancel\nstep cancelled 40 41\n");
    let steps = same("binding_priority", &script);
    assert_eq!(
        rect(&steps, "a_takes", 40).map(|r| (r[0], r[2])),
        Some((150.0, 201.0)),
        "A took it"
    );
    assert_eq!(
        rect(&steps, "b_takes", 41).map(|r| (r[1], r[2])),
        Some((500.0, 201.0)),
        "B took it"
    );
}

#[test]
fn a_directed_gesture_starts_when_its_direction_is_known_and_only_the_matching_binding_runs() {
    let mut script = floating_world();
    script.push_str("seat pointer_x 50\nseat pointer_y 50\n");
    // Under 12 units a swipe has no direction yet: nothing starts.
    script.push_str("gesture swipe 3 begin 0 0 1 0\ngesture swipe 3 update -4 1 1 0\nstep undecided 40 41\n");
    script.push_str("gesture swipe 3 update -20 2 1 0\nstep left_started 40 41\n");
    script.push_str("gesture swipe 3 update -100 5 1 0\ngesture swipe 3 end -120 5 1 0\nstep left_end 40 41\n");
    // Rightwards: the other binding, from the start.
    script.push_str("gesture swipe 3 begin 0 0 1 0\ngesture swipe 3 update 30 0 1 0\ngesture swipe 3 end 40 0 1 0\nstep right_end 40 41\n");
    // A gesture the compositor cancels ends on the binding that took it.
    script.push_str("gesture swipe 3 begin 0 0 1 0\ngesture swipe 3 update -50 0 1 0\ngesture swipe 3 cancel -50 0 1 0\nstep cancelled 40 41\n");
    let steps = same("directed_gesture", &script);
    let (x, y) = rect(&steps, "left_end", 40).map(|r| (r[0], r[1])).expect("window 40");
    assert_eq!(
        (x, y),
        (300.0, 10.0),
        "the left binding followed 0.3 of a swipe and ended"
    );
}

#[test]
fn a_gesture_is_cancelled_when_the_window_under_it_closes_as_in_typescript() {
    let mut script = floating_world();
    // The pointer is on window 41; four fingers, no direction: the binding takes the gesture at once.
    script.push_str("seat pointer_x 450\nseat pointer_y 450\ngesture swipe 4 begin 0 0 1 0\ngesture swipe 4 update 100 0 1 0\nstep running 40 41\n");
    script.push_str("destroy 41\nstep closed 40 41\n");
    // What the fingers do after that reaches nobody.
    script.push_str("gesture swipe 4 update 200 0 1 0\ngesture swipe 4 end 200 0 1 0\nstep after 40 41\n");
    let steps = same("gesture_cancel_on_close", &script);
    let y = rect(&steps, "after", 40).map(|r| r[1]).expect("window 40");
    assert_eq!(y, 3000.0, "the cancel arrived once, and the end never did");
}

#[test]
fn a_resize_stops_at_the_limits_the_client_has_as_in_typescript() {
    let mut script =
        String::from("focus 40\nchord Super+space\nplace 40 300 200 400 300\nchord Super+m\nstep limited 40\n");
    // Grown past the largest size: the bottom right follows the pointer until 500 by 400.
    script.push_str("drag Super+Button3 690,490 900,700 end\nstep capped 40\n");
    // Shrunk past the smallest size.
    script.push_str("drag Super+Button3 790,590 100,100 end\nstep floored 40\n");
    // The top left corner stops at a limit too, and leaves the opposite corner where it was.
    script.push_str("drag Super+Button3 305,205 0,0 end\nstep top_left_capped 40\n");
    script.push_str("drag Super+Button3 100,100 600,500 end\nstep top_left_floored 40\n");
    let steps = same("resize_limits", &script);
    assert_eq!(rect(&steps, "capped", 40).map(|r| (r[2], r[3])), Some((500.0, 400.0)));
    assert_eq!(rect(&steps, "floored", 40).map(|r| (r[2], r[3])), Some((150.0, 120.0)));
}
