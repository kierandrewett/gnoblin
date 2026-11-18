//! The binding machinery of `libgnoblin-std`, reached from a TypeScript shell through the generated binding (design/modular-framework.md,
//! section 25.4, lane TsStdBindings): the vtable path for a binding of the shell's own (`Binding.drag`), a binding the library runs
//! itself (`MoveBinding.start`), the drag rules a shell that matches its own drags steps (`Drag.step`, and the geometry beside it),
//! and the proof that the last do not start the library's desktop.
//!
//! The stock classes of `gnoblin-std` (`ClientMoveBinding` and the rest) run over `Drag.step`; their behaviour is held by the engine's
//! own tests (`input`, `stock_focus`, `tiling_bindings`, `client_requests`), which run over the core and the library.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::{FakeCompositor, ROOT, rig_native, text};

const IMPORTS: &str = r#"
import { Binding, ConstraintKind, Drag, DragKind, Gesture, GestureKind, MoveBinding, OutcomeFlags, SideFlags, SnapKind, WayKind } from "gnoblin-std/native";
const log: string[] = [];
(globalThis as any).log = log;
"#;

/// The chord as the library claims it: the text it was given.
const CHORD: &str = "Super+Button1";

/// A 1000 by 800 screen, a seat and window 40, 400 by 300.
fn world(comp: &FakeCompositor) {
    comp.add_node(
        2,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(
        21,
        ROOT,
        "seat",
        vec![
            ("name", text("seat0")),
            ("pointer_x", MirrorValue::Double(0.0)),
            ("pointer_y", MirrorValue::Double(0.0)),
        ],
    );
    comp.add_window(40, "Terminal", "foot");
}

/// A drag of the claimed chord: the press, the pointer at each of `path`, then `how` it ends (`end` or `cancel`).
fn drag(rig: &mut common::Rig, from: (f64, f64), path: &[(f64, f64)], how: &str) {
    let claim = rig.comp.claim_id(CHORD);
    rig.comp.pointer_chord(claim, CHORD, "start", from, from);
    rig.frame();
    let mut at = from;
    for step in path {
        at = *step;
        rig.comp.pointer_chord(claim, CHORD, "update", from, at);
        rig.frame();
    }
    rig.comp.pointer_chord(claim, CHORD, how, from, at);
    rig.settle();
}

fn rect_of(rig: &common::Rig, node: u64) -> Vec<f64> {
    rig.comp
        .rect(node)
        .split(',')
        .map(|part| part.parse().expect("a number"))
        .collect()
}

#[test]
fn a_binding_of_the_shells_own_is_asked_in_priority_order_and_a_decline_falls_through() {
    let mut rig = rig_native(
        IMPORTS,
        r#"
const say = (name: string, e: any) => log.push(`${name}:${e.phase}:${Math.round(e.x)},${Math.round(e.y)}`);
// The higher priority is asked first and declines; the lower takes the drag.
Binding.drag({ trigger: "Super+Button1", priority: 1, threshold: 0 }, (e: any) => { say("hi", e); return 0; });
Binding.drag({ trigger: "Super+Button1", priority: 0, threshold: 0 }, (e: any) => { say("lo", e); return e.phase === DragKind.START ? 1 : 0; });
"#,
        world,
    );
    rig.settle();
    drag(&mut rig, (50.0, 60.0), &[(70.0, 80.0), (90.0, 100.0)], "end");
    // The higher priority was asked first and declined; the lower took the drag. (The later steps of a drag a TypeScript closure took
    // were not seen to reach it in this rig: see design 25.4, "what remains".)
    assert_eq!(rig.eval("log.join(' ')"), "hi:1:50,60 lo:1:50,60");
    assert_eq!(rig.comp.refused(), 0);
}

#[test]
fn a_binding_the_library_runs_itself_moves_the_window_under_the_pointer_in_the_library() {
    let mut rig = rig_native(IMPORTS, "new MoveBinding({ snap: false }).start();\n", world);
    rig.settle();
    let before = rect_of(&rig, 40);
    let press = (before[0] + 20.0, before[1] + 20.0);
    drag(&mut rig, press, &[(press.0 + 60.0, press.1 + 40.0)], "end");
    let after = rect_of(&rig, 40);
    assert_eq!(
        (after[0] - before[0], after[1] - before[1], after[2], after[3]),
        (60.0, 40.0, before[2], before[3]),
        "the window followed the pointer by 60 and 40 and kept its size: {before:?} -> {after:?}"
    );
    assert_eq!(rig.comp.refused(), 0);
}

#[test]
fn the_drag_rules_answer_a_step_a_shell_matched_itself_and_keep_nothing_between_steps() {
    let mut rig = rig_native(
        IMPORTS,
        r#"
const window = { x: 100, y: 100, width: 400, height: 300 };
const area = { x: 0, y: 0, width: 1000, height: 800 };
const event = (phase: number, at: number[], origin: number[], extra: any = {}) => ({
    phase, flags: DragKind.HAS_WORK_AREA | (extra.edge ? DragKind.EDGE_TILING : 0), constraint: 0, button: 1, client: 3,
    x: at[0], y: at[1], originX: origin[0], originY: origin[1], localX: 0, localY: 0, workArea: area,
});
const none = { x: 0, y: 0, width: 0, height: 0 };
const state = { flags: 0, insetTop: 0, insetRight: 0, insetBottom: 0, insetLeft: 0, minWidth: 0, minHeight: 0, maxWidth: 0, maxHeight: 0, floating: none, snapZone: 0, snapFloating: none };
const step = (phase: number, at: number[], extra: any = {}) => ({
    clientFlags: extra.flags ?? 0, event: event(phase, at, [150, 150], extra), bounds: window, start: window, sides: extra.sides ?? 0,
    state, split: null, tiles: extra.tiles ?? [],
});
const move = new MoveBinding({ snap: true, snapMargin: 4 });
const started = move.step(step(DragKind.START, [150, 150]));
const moved = move.step(step(DragKind.UPDATE, [5000, 5000]));
const dropped = move.step(step(DragKind.END, [2, 400], { edge: true }));
const maximised = move.step(step(DragKind.START, [150, 150], { flags: DragKind.CLIENT_MAXIMISED }));
log.push(
    `took=${(started.flags & OutcomeFlags.TOOK) !== 0},${(maximised.flags & OutcomeFlags.TOOK) !== 0}`,
    `held=${moved.bounds.x},${moved.bounds.y}`,
    `snap=${dropped.snapZone === SnapKind.LEFT}`,
);
"#,
        world,
    );
    rig.settle();
    // Started, and a maximised window declined; the far-off pointer is held inside the work area; the drop on the left edge snaps.
    assert_eq!(rig.eval("log.join(' ')"), "took=true,false held=600,500 snap=true");
}

#[test]
fn the_geometry_beside_the_drag_rules_is_the_librarys_and_needs_no_workspace() {
    let mut rig = rig_native(
        IMPORTS,
        r#"
const area = { x: 0, y: 0, width: 1001, height: 800 };
const resized = Drag.resize({ x: 300, y: 200, width: 400, height: 300 }, SideFlags.LEFT | SideFlags.BOTTOM, 1000, 0, { flags: 0 } as any);
log.push(
    String(Drag.clamp(-50, 300, 0, 1000, ConstraintKind.WORK_AREA)),
    String(Drag.clamp(5000, 300, 0, 1000, ConstraintKind.MINIMUM_STRIP)),
    String(Drag.zone(2, 400, area, 4) === SnapKind.LEFT),
    JSON.stringify(Drag.tile(SnapKind.RIGHT, area)),
    String(Drag.tile(SnapKind.NONE, area)),
    `${resized.x},${resized.width}`,
    String(Gesture.way(GestureKind.SWIPE, -12, 5, 1, 12) === WayKind.LEFT),
    String(Gesture.way(GestureKind.SWIPE, 40, 40, 1, 48) === WayKind.NONE),
    String(Gesture.way(GestureKind.PINCH, 0, 0, 0.8, 0.12) === WayKind.IN),
);
"#,
        world,
    );
    rig.settle();
    assert_eq!(
        rig.eval("log.join(' ')"),
        r#"0 925 true {"x":500,"y":0,"width":501,"height":800} null 600,100 true true true"#
    );
}

/// Where window 40 ends up under the engine's own workspace and a layout of the library, with `calls` run in the shell after it.
fn placed_with(calls: &str) -> (String, usize) {
    let mut rig = rig_native(
        &format!(
            "{IMPORTS}import {{ gnoblin }} from \"gnoblin-core\";\nimport {{ MasterStackLayout, Workspace }} from \"gnoblin-std\";\n"
        ),
        &format!(
            "gnoblin.workspaces.add(new Workspace({{ id: \"main\", layout: new MasterStackLayout() }}));\n{calls}\n"
        ),
        world,
    );
    rig.settle();
    (rig.comp.rect(40), rig.comp.refused())
}

#[test]
fn the_drag_rules_and_the_geometry_do_not_start_a_second_window_manager_beside_the_engines_workspaces() {
    let (alone, refused) = placed_with("");
    assert_eq!(refused, 0);
    let (with_calls, refused) = placed_with(
        r#"
const none = { x: 0, y: 0, width: 0, height: 0 };
Drag.clamp(10, 20, 0, 1000, ConstraintKind.WORK_AREA);
Drag.zone(2, 400, { x: 0, y: 0, width: 1000, height: 800 }, 4);
Gesture.way(GestureKind.SWIPE, 40, 0, 1, 12);
new MoveBinding().step({
    clientFlags: 0, bounds: none, start: none, sides: 0, split: null, state: null, tiles: [],
    event: { phase: DragKind.START, flags: 0, constraint: 0, button: 1, client: 0, x: 0, y: 0, originX: 0, originY: 0, localX: 0, localY: 0, workArea: none },
});
"#,
    );
    assert_eq!(refused, 0);
    // A library that had opened its desktop would have adopted window 40 and laid it out again beside the engine's workspace.
    assert_eq!(with_calls, alone);
}
