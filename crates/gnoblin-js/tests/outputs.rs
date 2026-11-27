//! Outputs and desks of the stock shell: `gnoblin.outputs`, the workspace each output shows, the windows of an output
//! that is unplugged, `Client.moveToOutput`, and dialogs that stay with their parent, over the fake compositor of
//! `tests/common`.
//!
//! The cases are the retired shell's `the retired retired corpus` (moving, unplugging, reconnecting,
//! dialogs), keeping their inputs and expected values. Where the TypeScript model differs the case says why. The
//! difference is the model's: a the retired shell `Desk` owns its own copy of every workspace, so each output shows "its" workspace
//! 1; a TypeScript `Workspace` is the script's own object and is on one output at a time, so showing a workspace that
//! another output shows changes the two outputs' places (a swap). A window's coordinates are the compositor's global
//! ones, so a window on an output at `x = 1000` has `x >= 1000`; the retired windows were relative to their desk. A case
//! with no retired original says "new". What the real compositor does with the writes is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::{MirrorChange, MirrorChangeKind, MirrorValue};

mod common;
use common::*;

const IMPORTS: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n";

const SEAT: u64 = 21;
const A: u64 = 20;
const B: u64 = 22;

/// A layout that gives every window the whole area it is offered: what the area of an output is, as a window reads it.
const FILL: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { x: c.area.x, y: c.area.y, width: c.area.width, height: c.area.height } })); }\n\
}\n\
class Nothing extends Layout { compute() { return []; } }\n\
g.outputOf = (c: any) => gnoblin.outputs.toList().find((o: any) => o.workspace?.id === c.workspace?.id)?.name;\n\
g.outputOfWorkspace = (w: any) => gnoblin.outputs.toList().find((o: any) => o.workspace?.id === w.id)?.name ?? '-';\n";

/// A shell over one screen `A` (1000 by 800, primary) and, when `second`, a screen `B` of 600 by 400 at x 1000, a seat,
/// and `n` windows (ids 3 on, 300 by 200 at 10 n, 20 n), on the workspaces `1` to `3`, whose layouts place nothing
/// unless `layout` says so (`new Layout()` is the layout that places nothing).
fn world(n: u64, second: bool, layout: &str) -> Rig {
    let layout = layout.replace("new Layout()", "new Nothing()");
    let init = format!(
        "{FILL}for (const id of [\"1\", \"2\", \"3\"]) gnoblin.workspaces.add(new Workspace(id, {{ layout: {layout} }}));\n"
    );
    let mut rig = rig_setup(IMPORTS, &init, move |comp| {
        comp.add_screen(A, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
        if second {
            comp.add_screen(B, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
        }
        comp.add_node(
            SEAT,
            ROOT,
            "seat",
            vec![
                ("name", text("seat0")),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
            ],
        );
        for i in 1..=n {
            comp.add_node(
                2 + i,
                ROOT,
                "window",
                vec![
                    ("title", text(&format!("win{i}"))),
                    ("app_id", text("foot")),
                    ("x", MirrorValue::Double(10.0 * i as f64)),
                    ("y", MirrorValue::Double(20.0 * i as f64)),
                    ("width", MirrorValue::Double(300.0)),
                    ("height", MirrorValue::Double(200.0)),
                ],
            );
        }
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// Runs a statement in the script. `Rig::eval` wants a value back; a statement has none.
fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

fn settle(rig: &mut Rig) {
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// The seat's focus: a window's node, or 0 for none.
fn focus(rig: &Rig) -> u64 {
    match rig.comp.value(SEAT, "focus") {
        Some(MirrorValue::Node(id)) => id,
        _ => 0,
    }
}

fn give_focus(rig: &mut Rig, node: u64) {
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(node));
    settle(rig);
}

fn rect(rig: &Rig, node: u64) -> String {
    rig.comp.rect(node)
}

fn numbers(rig: &Rig, node: u64) -> Vec<f64> {
    rect(rig, node)
        .split(',')
        .map(|n| n.parse().expect("a number"))
        .collect()
}

fn visible(rig: &Rig, node: u64) -> bool {
    rig.comp.flag(node, "visible") != Some(false)
}

/// The workspace id of window `node`, by the script.
fn workspace_of(rig: &mut Rig, node: u64) -> String {
    rig.eval(&format!("gnoblin.clients.get('{node}')?.workspace?.id ?? 'none'"))
}

/// Which workspace each output shows, as `name=id` words.
fn shown(rig: &mut Rig) -> String {
    rig.eval("gnoblin.outputs.toList().map((o) => `${o.name}=${o.workspace?.id}`).join(' ')")
}

/// Unplugs screen `id`: the compositor destroys its node.
fn unplug(rig: &mut Rig, id: u64) {
    rig.comp.destroy(id);
    settle(rig);
}

// ---- gnoblin.outputs -------------------------------------------------------------------------------------------------

#[test]
fn outputs_are_a_live_collection_of_the_compositors_screens_with_their_properties() {
    // New: the d.ts declared `gnoblin.outputs` and the runtime had none.
    let mut rig = world(0, true, "new Layout()");
    assert_eq!(rig.eval("String(gnoblin.outputs.length)"), "2");
    assert_eq!(
        rig.eval("gnoblin.outputs.toList().map((o) => `${o.name}:${o.x},${o.y},${o.width}x${o.height}:${o.primary}`).join(' ')"),
        "output-0:0,0,1000x800:true output-1:1000,0,600x400:false"
    );
    assert_eq!(
        rig.eval(
            "gnoblin.outputs.where({ primary: true }).at(0).name + ' ' + gnoblin.outputs.toList().find((o) => o.x === 1000).name"
        ),
        "output-0 output-1"
    );
    assert_eq!(
        rig.eval("[gnoblin.outputs.at(0).make, gnoblin.outputs.at(0).model, gnoblin.outputs.at(0).mode].join('|')"),
        "||",
        "a property the compositor has not published reads as the schema's empty default"
    );
    assert_eq!(
        rig.eval("(() => { const o = gnoblin.outputs.at(0); return [o.scale, o.transform, o.enabled].join('|'); })()"),
        "1|normal|true"
    );
    assert_eq!(
        rig.eval("gnoblin.outputs.get(gnoblin.outputs.at(1).id) === gnoblin.outputs.at(1) ? 'same' : 'other'"),
        "same",
        "one object per screen"
    );
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.workspaces.area)"),
        "{\"x\":0,\"y\":0,\"width\":1000,\"height\":800}",
        "the collection's area is the primary output's"
    );
    assert_eq!(
        rig.eval("try { gnoblin.outputs.at(0).width = 1; 'wrote' } catch (e) { e.constructor.name }"),
        "TypeError",
        "an output's properties are the compositor's"
    );
}

#[test]
#[ignore = "vrnw: needs Output.area and Output.connected"]
fn an_output_has_its_own_area_and_is_connected() {
    let mut rig = world(0, true, "new Layout()");
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.outputs.at(1).area)"),
        "{\"x\":1000,\"y\":0,\"width\":600,\"height\":400}",
        "the area is per output"
    );
    assert_eq!(
        rig.eval("(() => { const o = gnoblin.outputs.at(0); return String(o.connected); })()"),
        "true"
    );
}

#[test]
fn a_published_make_model_mode_and_scale_are_what_an_output_reads() {
    // New: the compositor publishes them on the screen node; the output reads them.
    let mut rig = world(0, false, "new Layout()");
    rig.comp.external_set(A, "make", text("Gnoblin"));
    rig.comp.external_set(A, "model", text("HEADLESS-1"));
    rig.comp.external_set(A, "mode", text("1000x800@60"));
    rig.comp.external_set(A, "output_scale", MirrorValue::Double(2.0));
    settle(&mut rig);
    assert_eq!(
        rig.eval("(() => { const o = gnoblin.outputs.at(0); return [o.make, o.model, o.mode, o.scale].join('|'); })()"),
        "Gnoblin|HEADLESS-1|1000x800@60|2"
    );
}

#[test]
fn a_hotplugged_output_is_announced_and_an_unplugged_one_is_removed_with_its_last_values_readable() {
    // New: `OutputListener.onConnected` and `onDisconnected`, and `CollectionListener` membership, over the the retired shell
    // `Screen:on_ready` and `Screen:on_destroyed` that made and dropped a `Desk`.
    let mut rig = world(0, false, "new Layout()");
    exec(
        &mut rig,
        "g.log = [];\n\
         gnoblin.outputs.addListener(new (class extends OutputCollectionListener {\n\
             onConnected(o) { g.log.push('connected ' + o.name); }\n\
             onDisconnected(o) { g.log.push('disconnected ' + o.name + ' ' + o.width); }\n\
             onModeChanged(o, m) { g.log.push('mode ' + o.name + ' ' + m); }\n\
             onScaleChanged(o, s) { g.log.push('scale ' + o.name + ' ' + s); }\n\
             onGeometryChanged(o, r) { g.log.push('geometry ' + o.name + ' ' + JSON.stringify(r)); }\n\
             onAdded(o) { g.log.push('added ' + o.name); }\n\
             onRemoved(o) { g.log.push('removed ' + o.name); }\n\
         })());",
    );
    // The listener on the collection replays the output that was there as an addition, not as a connection.
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join(';')"), "added output-0");

    rig.comp
        .create_screen(B, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
    settle(&mut rig);
    assert_eq!(rig.eval("String(gnoblin.outputs.length)"), "2");
    assert_eq!(
        rig.eval("g.log.slice(1).join(';')"),
        "added output-1;connected output-1",
        "a screen made while the shell runs is a connection"
    );

    rig.comp.external_set(B, "mode", text("800x600@60"));
    rig.comp.external_set(B, "output_scale", MirrorValue::Double(2.0));
    rig.comp.external_set(B, "width", MirrorValue::Double(800.0));
    settle(&mut rig);
    assert_eq!(
        rig.eval("g.log.slice(3).join(';')"),
        "mode output-1 800x600@60;scale output-1 2;geometry output-1 {\"x\":1000,\"y\":0,\"width\":800,\"height\":400}"
    );

    exec(&mut rig, "g.gone = gnoblin.outputs.at(1); g.log.length = 0;");
    unplug(&mut rig, B);
    assert_eq!(rig.eval("String(gnoblin.outputs.length)"), "1");
    assert_eq!(
        rig.eval("g.log.join(';')"),
        "disconnected output-1 800;removed output-1",
        "the listener reads the output's last values, and it is no longer connected"
    );
    // A gone output's std members answer none rather than throw, as the TypeScript library's did (js-VrnwB `00487d166`).
    assert_eq!(
        rig.eval("(() => { const o = g.gone; let w; try { w = String(o.workspace); } catch (e) { w = e.code; } return [o.alive, o.name, o.width, w].join('|'); })()"),
        "false|output-1|800|undefined"
    );
}

// ---- the workspace each output shows ---------------------------------------------------------------------------------

#[test]
fn each_output_shows_its_own_workspace_and_a_workspace_is_on_one_output() {
    // Before: "Desk builds" (one desk per screen, each with a workspace showing). Derived: a TypeScript workspace is on
    // one output, so the second output starts on the first workspace no output shows.
    let mut rig = world(0, true, "new Layout()");
    assert_eq!(shown(&mut rig), "output-0=1 output-1=2");
    assert_eq!(
        rig.eval("gnoblin.workspaces.list.map((w) => `${w.id}:${w.active}:${g.outputOfWorkspace(w)}`).join(' ')"),
        "1:true:output-0 2:true:output-1 3:false:-"
    );
    // Showing "3" on the second output leaves the first alone.
    exec(&mut rig, "gnoblin.outputs.at(1).show(gnoblin.workspaces.byId('3'))");
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=1 output-1=3");
    assert_eq!(
        rig.eval("gnoblin.workspaces.list.map((w) => `${w.active}`).join(' ')"),
        "true false true"
    );
    // Showing "1" on the second output takes it from the first, which takes the second's workspace.
    exec(&mut rig, "gnoblin.outputs.at(1).show(gnoblin.workspaces.byId('1'))");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=3 output-1=1",
        "a swap: no workspace is on two outputs"
    );
    assert_eq!(
        rig.eval("gnoblin.workspaces.list.filter((w) => w.active).map((w) => w.id).join(',')"),
        "1,3"
    );
    assert_eq!(
        rig.eval("try { gnoblin.outputs.at(0).show(new Workspace('nope')); 'no' } catch (e) { e.code }"),
        "invalid_state"
    );
}

#[test]
fn a_plain_show_acts_on_the_output_under_the_pointer_and_the_primary_one_when_no_pointer_says() {
    // Before: `desk()` is the desk under the pointer (`seat.screen`). Derived: with no pointer screen the primary output.
    let mut rig = world(0, true, "new Layout()");
    exec(&mut rig, "gnoblin.workspaces.showById('3')");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=3 output-1=2",
        "no pointer screen: the primary output"
    );
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    exec(&mut rig, "gnoblin.workspaces.byId('1').show()");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=3 output-1=1",
        "the pointer is on the second output"
    );
}

#[test]
fn a_new_window_joins_the_workspace_shown_on_the_output_under_the_pointer() {
    // Before: `Window:on_ready` adds the window to `desk().active`.
    let mut rig = world(0, true, "new Layout()");
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    rig.comp.create_window(30, "second", "foot");
    settle(&mut rig);
    assert_eq!(workspace_of(&mut rig, 30), "2", "the workspace the second output shows");
    assert_eq!(
        rig.eval("g.outputOf(gnoblin.clients.get('30'))"),
        "output-1",
        "a client's output is the one its workspace is shown on"
    );
}

#[test]
fn a_layout_tiles_in_the_area_of_the_output_that_shows_its_workspace() {
    // Before: `Desk:arrange` gives each desk's workspaces the desk's own size. Derived: the area is in the compositor's
    // coordinates, so the second output's starts at its own x.
    let mut rig = world(0, true, "new Fill()");
    rig.comp.create_window(30, "one", "foot");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 30),
        "0,0,1000,800",
        "a window joins the first output's workspace"
    );
    exec(&mut rig, "gnoblin.outputs.at(1).show(gnoblin.workspaces.byId('1'))");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 30),
        "1000,0,600,400",
        "the workspace moved to the second output, and its layout filled that output's area"
    );
    // The second output grows: the layout computes again for the new area.
    rig.comp.external_set(B, "width", MirrorValue::Double(700.0));
    rig.comp.external_set(B, "height", MirrorValue::Double(500.0));
    settle(&mut rig);
    assert_eq!(rect(&rig, 30), "1000,0,700,500");
}

#[test]
fn bars_reserve_room_on_their_own_output_and_give_way_to_a_fullscreen_window_there() {
    // Before: bars are children of a `Desk` (`Desk:arrange` stacks them on that desk's edges). New: `Bar` takes an output.
    let mut rig = world(0, true, "new Fill()");
    exec(&mut rig, "Bar.onOutputs(); Bar.giveWayTo();");
    exec(
        &mut rig,
        "g.main = new Bar({ edge: 'top', height: 32 });\n\
         g.side = new Bar({ edge: 'bottom', height: 20, output: Number(gnoblin.outputs.at(1).id) });",
    );
    rig.comp.create_window(30, "one", "foot");
    settle(&mut rig);
    assert_eq!(rect(&rig, 30), "0,32,1000,768", "the first output's top bar");
    exec(&mut rig, "gnoblin.outputs.at(1).show(gnoblin.workspaces.byId('1'))");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 30),
        "1000,0,600,380",
        "the second output has its own bottom bar and none on top"
    );
    // A bar that names no output keeps `output` 0 (the primary output, whichever it is) and takes its room on the first output.
    assert_eq!(
        rig.eval(
            "[g.main.output === 0, g.side.output === Number(gnoblin.outputs.at(1).id), String(Bar.insets(Number(gnoblin.outputs.at(1).id)).bottom), String(Bar.insets(Number(gnoblin.outputs.at(0).id)).top)].join('|')"
        ),
        "true|true|20|32"
    );
    // A fullscreen window on the second output hides that output's bar and not the first's.
    exec(&mut rig, "gnoblin.clients.get('30').fullscreen = true");
    settle(&mut rig);
    assert_eq!(
        rig.eval("[g.main.shown, g.side.shown].join('|')"),
        "true|false",
        "only the output whose workspace holds the fullscreen window gives way"
    );
    // The output goes: its bar is hidden and takes no room.
    unplug(&mut rig, B);
    assert_eq!(rig.eval("String(g.side.shown)"), "false");
}

// ---- unplugging and reconnecting -------------------------------------------------------------------------------------

#[test]
fn unplugging_one_of_two_outputs_moves_its_windows_to_the_other_output() {
    // Before: "unplugging one of two outputs moves its windows to the other output".
    let mut rig = world(0, true, "new Layout()");
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    rig.comp.create_window(31, "two", "foot");
    settle(&mut rig);
    assert_eq!(workspace_of(&mut rig, 31), "2");
    unplug(&mut rig, B);
    assert_eq!(
        workspace_of(&mut rig, 31),
        "1",
        "the window must move to the workspace the remaining output shows"
    );
    assert_eq!(rig.eval("g.outputOf(gnoblin.clients.get('31'))"), "output-0");
    assert_eq!(
        rig.eval("gnoblin.workspaces.list.map((w) => `${w.id}:${g.outputOfWorkspace(w)}`).join(' ')"),
        "1:output-0 2:- 3:-",
        "the workspace of the unplugged output is shown nowhere"
    );
}

#[test]
fn windows_moved_off_an_unplugged_output_are_fitted_to_the_output_that_remains() {
    // Before: `Window:move_to` refits (`fit_into`) what it moves; the unplug uses the same. The second output was 600 by
    // 400 at x 1000; the window moves to the first (1000 by 800 at the origin).
    let mut rig = world(0, true, "new Layout()");
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    rig.comp.create_window(30, "far", "foot");
    settle(&mut rig);
    exec(
        &mut rig,
        "gnoblin.clients.get('30').bounds = { x: 1100, y: 50, width: 300, height: 200 };",
    );
    settle(&mut rig);
    unplug(&mut rig, B);
    let r = numbers(&rig, 30);
    assert!(
        r[0] >= 0.0 && r[0] + r[2] <= 1000.0 && r[1] >= 0.0 && r[1] + r[3] <= 800.0,
        "the window must fit the remaining output: {r:?}"
    );
}

#[test]
fn unplugging_the_last_output_keeps_its_windows_hidden_and_drops_focus() {
    // Before: "unplugging the last output keeps its windows hidden and drops focus".
    let mut rig = world(1, false, "new Layout()");
    give_focus(&mut rig, 3);
    assert_eq!(focus(&rig), 3);
    unplug(&mut rig, A);
    assert_eq!(workspace_of(&mut rig, 3), "1", "the window must survive the unplug");
    assert!(!visible(&rig, 3), "the window's workspace must be hidden");
    assert_eq!(focus(&rig), 0, "a window with no output cannot hold focus");
    assert_eq!(rig.eval("String(gnoblin.outputs.length)"), "0");
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.byId('1').active)"),
        "false",
        "a parked workspace is not on show"
    );
}

#[test]
fn reconnecting_an_output_adopts_the_parked_windows_and_focuses_one() {
    // Before: "reconnecting an output adopts the parked windows and focuses one".
    let mut rig = world(1, false, "new Layout()");
    give_focus(&mut rig, 3);
    unplug(&mut rig, A);
    rig.comp.create_screen(A, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
    settle(&mut rig);
    assert!(visible(&rig, 3), "the adopted workspace must show");
    assert_eq!(
        rig.eval("g.outputOf(gnoblin.clients.get('3'))"),
        "output-0",
        "the window must be on the new output"
    );
    assert_eq!(focus(&rig), 3, "reconnecting must focus the window again");
    assert_eq!(rig.eval("String(gnoblin.workspaces.byId('1').active)"), "true");
}

#[test]
fn a_window_on_a_second_output_goes_to_the_first_when_it_is_unplugged_and_comes_back_with_the_next_output() {
    // New: the acceptance case. The second output's window follows to the first when the second is removed; a
    // reconnected output starts on the first free workspace, and the window moves back when asked.
    let mut rig = world(0, true, "new Fill()");
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    rig.comp.create_window(30, "second", "foot");
    settle(&mut rig);
    assert_eq!(rect(&rig, 30), "1000,0,600,400");
    unplug(&mut rig, B);
    assert_eq!(
        rect(&rig, 30),
        "0,0,1000,800",
        "the window follows to the remaining output"
    );
    rig.comp
        .create_screen(B, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=1 output-1=2",
        "the new output shows the first workspace no output shows"
    );
    // The window moves back when asked: see `a_window_is_moved_back_to_a_reconnected_output`.
}

#[test]
fn a_window_is_moved_back_to_a_reconnected_output() {
    let mut rig = world(0, true, "new Fill()");
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    rig.comp.create_window(30, "second", "foot");
    settle(&mut rig);
    unplug(&mut rig, B);
    rig.comp
        .create_screen(B, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
    settle(&mut rig);
    exec(
        &mut rig,
        "gnoblin.clients.get('30').moveToOutput(gnoblin.outputs.at(1))",
    );
    settle(&mut rig);
    assert_eq!(rect(&rig, 30), "1000,0,600,400");
}

// ---- moving a window to another output or workspace ------------------------------------------------------------------

#[test]
fn moving_a_window_to_an_output_that_is_gone_is_rejected() {
    // Before: "moving a window to an output that is not a screen is rejected".
    let mut rig = world(1, true, "new Layout()");
    exec(&mut rig, "g.second = gnoblin.outputs.at(1);");
    unplug(&mut rig, B);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToOutput(g.second))"),
        "false"
    );
    assert_eq!(workspace_of(&mut rig, 3), "1");
    assert_eq!(
        rig.eval("try { gnoblin.clients.get('3').moveToOutput({}); 'no' } catch (e) { e.name }"),
        "TypeError"
    );
}

#[test]
fn moving_a_window_to_another_output_reshapes_it_into_that_outputs_work_area() {
    // Before: "moving a window to another output reshapes it into that output's work area".
    let mut rig = world(1, true, "new Layout()");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').bounds = { x: 100, y: 100, width: 900, height: 700 };",
    );
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToOutput(gnoblin.outputs.at(1)))"),
        "true"
    );
    settle(&mut rig);
    let r = numbers(&rig, 3);
    assert!(
        r[0] >= 1000.0 && r[0] + r[2] <= 1600.0,
        "the window must fit the new output horizontally: {r:?}"
    );
    assert!(
        r[1] >= 0.0 && r[1] + r[3] <= 400.0,
        "the window must fit the new output vertically: {r:?}"
    );
    assert_eq!(
        workspace_of(&mut rig, 3),
        "2",
        "the window joins the workspace the output shows"
    );
}

#[test]
fn a_maximised_window_fills_the_new_output_when_it_moves() {
    // Before: "a maximised window fills the new output when it moves".
    let mut rig = world(1, true, "new Layout()");
    exec(&mut rig, "WindowState.get().maximise(gnoblin.clients.get('3'));");
    settle(&mut rig);
    assert_eq!(rect(&rig, 3), "0,0,1000,800");
    exec(&mut rig, "gnoblin.clients.get('3').moveToOutput(gnoblin.outputs.at(1))");
    settle(&mut rig);
    assert!(
        rig.comp.flag(3, "maximised") == Some(true),
        "the window must stay maximised"
    );
    assert_eq!(rect(&rig, 3), "1000,0,600,400", "it must fill the new output");
}

#[test]
fn a_fullscreen_window_does_not_move_to_another_workspace_or_output() {
    // Before: "a fullscreen window does not move to another workspace" and "... another output".
    let mut rig = world(1, true, "new Layout()");
    exec(&mut rig, "gnoblin.clients.get('3').fullscreen = true");
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.byId('3')))"),
        "false"
    );
    assert_eq!(
        workspace_of(&mut rig, 3),
        "1",
        "a refused window must stay on its workspace"
    );
    assert_eq!(
        rig.comp.flag(3, "fullscreen"),
        Some(true),
        "a refused window must stay fullscreen"
    );
}

#[test]
fn a_fullscreen_window_does_not_move_to_another_output() {
    let mut rig = world(1, true, "new Layout()");
    exec(&mut rig, "gnoblin.clients.get('3').fullscreen = true");
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToOutput(gnoblin.outputs.at(1)))"),
        "false",
        "a fullscreen window must refuse an output move"
    );
}

#[test]
fn a_moved_window_that_held_the_focus_hands_it_to_a_window_that_stays() {
    // Before: "moving the focused window off the active workspace gives focus to a window that stays".
    let mut rig = world(2, false, "new Layout()");
    give_focus(&mut rig, 3);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.byId('2')))"),
        "true"
    );
    settle(&mut rig);
    assert_eq!(
        focus(&rig),
        4,
        "focus must pass to the window left on the active workspace"
    );
    assert_eq!(workspace_of(&mut rig, 3), "2");
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.byId('2')))"),
        "false",
        "the workspace it is on already"
    );
}

#[test]
fn moving_the_only_focused_window_off_the_screen_leaves_nobody_focused() {
    // New: with no window left to take the focus, the seat holds none.
    let mut rig = world(1, false, "new Layout()");
    give_focus(&mut rig, 3);
    exec(
        &mut rig,
        "gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.byId('2'))",
    );
    settle(&mut rig);
    assert_eq!(focus(&rig), 0);
}

// ---- dialogs ---------------------------------------------------------------------------------------------------------

/// A dialog of window `parent`, mapped as the compositor does: created with its `parent_window` and size.
fn create_dialog(rig: &mut Rig, id: u64, parent: u64) {
    let props = [
        ("title", text("dialog")),
        ("app_id", text("dialog")),
        ("width", MirrorValue::Double(100.0)),
        ("height", MirrorValue::Double(60.0)),
        ("parent_window", MirrorValue::Node(parent)),
    ];
    rig.comp.add_node(id, ROOT, "window", props.to_vec());
    let mut changes = vec![
        MirrorChange {
            class: "window".to_owned(),
            ..change(MirrorChangeKind::Created, id)
        },
        MirrorChange {
            parent: ROOT,
            index: 99,
            ..change(MirrorChangeKind::Parented, id)
        },
    ];
    changes.extend(props.into_iter().map(|(key, value)| set_change(id, key, value)));
    rig.comp.publish(changes);
    settle(rig);
}

#[test]
fn a_dialog_is_centred_in_its_parent_and_joins_its_workspace() {
    // Before: `Window:on_ready` adds a dialog to its parent and centres it: `(p.width - self.width) // 2`.
    let mut rig = world(1, false, "new Layout()");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').bounds = { x: 200, y: 100, width: 400, height: 300 };",
    );
    settle(&mut rig);
    create_dialog(&mut rig, 30, 3);
    assert_eq!(
        rect(&rig, 30),
        "350,220,100,60",
        "centred in the parent: x = 200 + (400 - 100) / 2, y = 100 + (300 - 60) / 2"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.get('30').parentWindow === gnoblin.clients.get('3') ? 'parent' : 'none'"),
        "parent"
    );
    assert_eq!(rig.eval("String(gnoblin.clients.get('3').parentWindow)"), "undefined");
}

#[test]
fn a_dialog_whose_parent_is_on_another_workspace_goes_with_its_parent() {
    // Before: "a dialog whose parent is on another workspace goes with its parent".
    let mut rig = world(1, false, "new Layout()");
    exec(
        &mut rig,
        "gnoblin.workspaces.byId('2').clients.add(gnoblin.clients.get('3'))",
    );
    settle(&mut rig);
    create_dialog(&mut rig, 30, 3);
    assert_eq!(
        workspace_of(&mut rig, 30),
        "2",
        "the dialog must follow its parent's workspace"
    );
    assert_eq!(workspace_of(&mut rig, 3), "2", "the parent stays on its own workspace");
    assert_eq!(
        rig.eval("gnoblin.outputs.at(0).workspace.id"),
        "1",
        "showing the dialog must not switch workspace"
    );
}

#[test]
fn a_dialog_whose_parent_is_closed_has_no_parent_and_is_placed_as_a_normal_window() {
    // Before: "a dialog whose parent is closed has no parent_window and is placed as a normal window".
    let mut rig = world(1, false, "new Layout()");
    rig.comp.destroy(3);
    settle(&mut rig);
    create_dialog(&mut rig, 30, 3);
    assert_eq!(rig.eval("String(gnoblin.clients.get('30').parentWindow)"), "undefined");
    assert_eq!(
        workspace_of(&mut rig, 30),
        "1",
        "placed on the workspace the output shows"
    );
}

#[test]
fn a_dialog_does_not_move_to_another_workspace_on_its_own_but_goes_with_its_parent() {
    // Before: "a dialog does not move to another workspace on its own" and "moving a parent window takes its dialog with it".
    let mut rig = world(1, false, "new Layout()");
    create_dialog(&mut rig, 30, 3);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('30').moveToWorkspace(gnoblin.workspaces.byId('2')))"),
        "false",
        "a dialog move must be refused"
    );
    assert_eq!(workspace_of(&mut rig, 30), "1");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.byId('2'))",
    );
    settle(&mut rig);
    assert_eq!(workspace_of(&mut rig, 30), "2", "the dialog must follow its parent");
}

#[test]
fn a_dialog_follows_its_parent_to_another_output_and_keeps_its_place_in_it() {
    // Before: "a dialog follows its parent to another output".
    let mut rig = world(1, true, "new Layout()");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').bounds = { x: 100, y: 100, width: 300, height: 200 };",
    );
    settle(&mut rig);
    create_dialog(&mut rig, 30, 3);
    assert_eq!(rect(&rig, 30), "200,170,100,60");
    exec(&mut rig, "gnoblin.clients.get('3').moveToOutput(gnoblin.outputs.at(1))");
    settle(&mut rig);
    assert_eq!(
        rig.eval("gnoblin.clients.get('30').output?.name"),
        "output-1",
        "the dialog must follow its parent to the new output"
    );
    let parent = numbers(&rig, 3);
    let dialog = numbers(&rig, 30);
    assert_eq!(
        (dialog[0] - parent[0], dialog[1] - parent[1]),
        (100.0, 70.0),
        "the dialog keeps its place in its parent when the parent moves"
    );
}

#[test]
fn a_dialog_moves_with_its_parent_and_a_tiling_layout_leaves_it_alone() {
    // Before: a dialog is a child node of its parent, so it moves with it and the workspace's layout never places it.
    let mut rig = world(1, false, "new Fill()");
    create_dialog(&mut rig, 30, 3);
    assert_eq!(rect(&rig, 3), "0,0,1000,800", "the layout filled the parent");
    assert_eq!(rect(&rig, 30), "450,370,100,60", "the dialog is centred, not tiled");
    rig.comp.external_set(3, "x", MirrorValue::Double(40.0));
    rig.comp.external_set(3, "y", MirrorValue::Double(30.0));
    settle(&mut rig);
    // The parent was moved by the compositor (a drag); the dialog goes the same way.
    assert_eq!(rect(&rig, 3), "40,30,1000,800");
    assert_eq!(rect(&rig, 30), "490,400,100,60", "the dialog went with the parent");
}

// ---- a window the launcher started ---------------------------------------------------------------------------------

/// A window the compositor maps with `from_launcher` set, as one that redeemed a launcher's activation token.
fn create_launched(rig: &mut Rig, id: u64) {
    let props = [
        ("title", text("launched")),
        ("app_id", text("foot")),
        ("from_launcher", MirrorValue::Bool(true)),
    ];
    rig.comp.add_node(id, ROOT, "window", props.to_vec());
    let mut changes = vec![
        MirrorChange {
            class: "window".to_owned(),
            ..change(MirrorChangeKind::Created, id)
        },
        MirrorChange {
            parent: ROOT,
            index: 99,
            ..change(MirrorChangeKind::Parented, id)
        },
    ];
    changes.extend(props.into_iter().map(|(key, value)| set_change(id, key, value)));
    rig.comp.publish(changes);
    settle(rig);
}

#[test]
fn a_launched_window_does_not_take_focus_but_a_window_nobody_launched_does() {
    // Before: `Focus.opens_focused(focus_settings(), self.from_launcher)`: a window from a launcher takes focus only when
    // `focus_new_windows` is `allow`. The the retired shell `from_launcher` was a field of the fake core only; here it is the real
    // node's property. The `allow` half went with the `focus` key.
    let mut rig = world(1, false, "new Layout()");
    exec(&mut rig, "FocusPolicy.get().start()");
    give_focus(&mut rig, 3);
    create_launched(&mut rig, 30);
    assert_eq!(rig.eval("String(gnoblin.clients.get('30').fromLauncher)"), "true");
    assert_eq!(focus(&rig), 3, "the default `prevent` keeps focus where it was");
    rig.comp.create_window(31, "own", "foot");
    settle(&mut rig);
    assert_eq!(focus(&rig), 31, "a window nobody launched takes focus");
}

// ---- a reload ------------------------------------------------------------------------------------------------------

#[test]
#[ignore = "vrnw: needs JsShell::workspace_snapshot / reconcile over the generated workspaces (the snapshot is empty)"]
fn a_reload_puts_each_output_back_on_the_workspace_it_showed() {
    // New: the snapshot a reload carries holds what each output showed, by the output's name.
    let mut rig = world(0, true, "new Layout()");
    exec(&mut rig, "gnoblin.outputs.at(1).show(gnoblin.workspaces.byId('3'))");
    settle(&mut rig);
    let snapshot = rig.shell.workspace_snapshot().expect("a snapshot");
    assert_eq!(
        snapshot
            .outputs
            .iter()
            .map(|shown| format!("{}={}", shown.output, shown.workspace))
            .collect::<Vec<_>>(),
        vec!["output-0=1".to_owned(), "output-1=3".to_owned()]
    );
    assert_eq!(snapshot.active.as_deref(), Some("1"));

    // The next generation starts on the same compositor and is told what the last one showed.
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = std::fs::canonicalize(dir.path()).expect("the temp dir exists");
    let init = format!("{IMPORTS}for (const id of [\"1\", \"2\", \"3\"]) gnoblin.workspaces.add(new Workspace(id));\n");
    std::fs::write(root.join("shell.ts"), init).expect("shell.ts is written");
    // The core's registry is one per process: the first generation is gone before the second starts.
    let comp = rig.comp.clone();
    drop(rig);
    let (mut second, core) = common::native::shell_over_core(
        &comp,
        std::sync::Arc::new(gnoblin_shell_host::supervisor::ManualClock::default()),
        honest_config(&root),
    );
    second
        .load_entry(std::path::Path::new("shell.ts"))
        .expect("shell.ts loads");
    second.reconcile(&snapshot).expect("reconciles");
    for _ in 0..4 {
        second.frame().expect("a frame");
        core.dispatch(&second);
    }
    let shown = second
        .eval("gnoblin.outputs.toList().map((o) => `${o.name}=${o.workspace?.id}`).join(' ')")
        .expect("the script runs")
        .expect("a value");
    assert_eq!(shown, "output-0=1 output-1=3");
}

// ---- Output.apply, Output.mirror, OutputProfile -------------------------------------------------------------------------

/// A shell over two screens that name their connectors: `A` is `DP-1` (1000 by 800) and `B` is `HDMI-1` (600 by 400). Its
/// `shell.ts` is `body`.
fn world_with_connectors(body: &str) -> Rig {
    let init = format!("{body}\n");
    let mut rig = rig_setup(IMPORTS, &init, |comp| {
        for (id, name, connector, rect, primary) in [
            (A, "output-0", "DP-1", (0.0, 0.0, 1000.0, 800.0), true),
            (B, "output-1", "HDMI-1", (1000.0, 0.0, 600.0, 400.0), false),
        ] {
            comp.add_node(
                id,
                ROOT,
                "screen",
                vec![
                    ("name", text(name)),
                    ("connector", text(connector)),
                    ("x", MirrorValue::Double(rect.0)),
                    ("y", MirrorValue::Double(rect.1)),
                    ("width", MirrorValue::Double(rect.2)),
                    ("height", MirrorValue::Double(rect.3)),
                    ("primary", MirrorValue::Bool(primary)),
                ],
            );
        }
    });
    rig.settle();
    rig
}

#[test]
fn apply_sends_the_request_and_the_compositors_answer_settles_the_operation_and_reaches_the_listeners() {
    let mut rig = world_with_connectors("");
    exec(
        &mut rig,
        "g.log = [];\n\
         gnoblin.outputs.addListener(new (class extends OutputCollectionListener {\n\
             onApplied(o, error) { g.log.push(o.connector + ' ' + error); }\n\
         })());\n\
         g.app = gnoblin.outputs.at(1).apply({ mode: '1920x1080@60', scale: 1.25, positionX: 1000, positionY: 0, enabled: true });\n\
         g.app.then(() => g.log.push('done ' + g.app.output.connector));\n\
         g.app.catch((error, code) => g.log.push('done ' + code));",
    );
    settle(&mut rig);
    let sent = rig.comp.applies();
    assert_eq!(sent.len(), 1, "one call per apply: {sent:?}");
    assert_eq!(sent[0].0, B);
    assert_eq!(
        sent[0].1,
        "mode=1920x1080@60\nscale=1.25\nposition=1000,0\nenabled=true"
    );
    assert_eq!(
        rig.eval("`${g.app.constructor.name} ${g.app.status} ${g.app.error}`"),
        "OutputOperation pending ",
        "an operation like the other asynchronous verbs; nothing is known until it answers"
    );

    rig.comp.answer(sent[0].2, "ok", "");
    settle(&mut rig);
    assert_eq!(
        rig.eval("`${g.app.status} ${g.app.output === gnoblin.outputs.at(1)} ${g.app.error}`"),
        "succeeded true ",
        "it succeeds with the output"
    );
    assert_eq!(
        rig.eval("g.log.join('|')"),
        "done HDMI-1|HDMI-1 ",
        "the operation's callback and the output's listeners hear the one answer, the operation's first"
    );

    // A refusal is the compositor's text, under the same object and to the same listeners: the operation fails with those
    // words as its message, so the key they name is still the first thing a script reads.
    exec(
        &mut rig,
        "g.refused = gnoblin.outputs.at(1).apply({ mode: '1x1@1' });\n\
         g.refused.catch((error, code) => g.log.push('done ' + code));",
    );
    settle(&mut rig);
    let token = rig.comp.applies()[1].2;
    rig.comp
        .answer(token, "", "mode: HDMI-1: the backend refused the change");
    settle(&mut rig);
    assert_eq!(
        rig.eval("`${g.refused.status} ${g.refused.errorCode}: ${g.refused.error}`"),
        "failed denied: mode: HDMI-1: the backend refused the change"
    );
    assert_eq!(
        rig.eval("g.log.join('|')"),
        "done HDMI-1|HDMI-1 |done denied|HDMI-1 mode: HDMI-1: the backend refused the change"
    );
}

#[test]
fn an_apply_the_compositor_refuses_fails_with_its_words_and_a_code_that_says_what_kind_of_refusal() {
    let mut rig = world_with_connectors("");
    // The compositor's own refusals, as `gnoblin-wayland`'s output policy words them, and the shell's word for a call it
    // had to give up on: the key a refusal names stays in the message, exactly as the compositor said it.
    let cases = [
        ("transform: this compositor draws only `normal`", "unsupported"),
        ("the output is not connected", "not_found"),
        (
            "failed: the transaction was refused unknown-node and the call was not sent again",
            "not_found",
        ),
        (
            "failed: the transaction was refused read-only and the call was not sent again",
            "read_only",
        ),
        ("mirror_of: DP-9 is not another connected output", "denied"),
        ("no output would stay enabled", "denied"),
    ];
    for (index, (words, code)) in cases.iter().enumerate() {
        exec(
            &mut rig,
            &format!("g.op{index} = gnoblin.outputs.at(1).apply({{ scale: 1.{index} }});"),
        );
        settle(&mut rig);
        let token = rig.comp.applies()[index].2;
        rig.comp.answer(token, "", words);
        settle(&mut rig);
        assert_eq!(
            rig.eval(&format!(
                "`${{g.op{index}.status}}|${{g.op{index}.errorCode}}|${{g.op{index}.error}}`"
            )),
            format!("failed|{code}|{words}")
        );
    }
}

#[test]
fn an_apply_on_an_output_that_is_gone_throws_and_a_mirror_is_an_operation_too() {
    let mut rig = world_with_connectors("");
    exec(
        &mut rig,
        "g.gone = gnoblin.outputs.at(1); g.mirrored = g.gone.mirror(gnoblin.outputs.at(0));",
    );
    settle(&mut rig);
    let sent = rig.comp.applies();
    assert_eq!(sent[0].1, "mirror_of=DP-1");
    assert_eq!(rig.eval("`${g.mirrored.status}`"), "pending");
    rig.comp.answer(sent[0].2, "", "");
    settle(&mut rig);
    assert_eq!(rig.eval("g.mirrored.status"), "succeeded");
    unplug(&mut rig, B);
    assert_eq!(
        rig.eval("try { g.gone.apply({ scale: 1 }); 'none' } catch (e) { `${e.constructor.name}: ${e.code}: ${e.message.endsWith('the output is gone')}` }"),
        "GnoblinError: not_found: true",
        "a call that cannot be made throws, as the other verbs do"
    );
}

/// What `apply(options)` throws on the second output and how many requests the compositor was sent.
fn bad_apply(options: &str) -> (String, usize) {
    let mut rig = world_with_connectors("");
    let thrown = rig.eval(&format!(
        "try {{ gnoblin.outputs.at(1).apply({options}); 'none' }} catch (e) {{ e.constructor.name + ': ' + e.message }}"
    ));
    (thrown, rig.comp.applies().len())
}

#[test]
fn a_bad_scale_throws_naming_the_key_and_sends_nothing() {
    let (scale, sent) = bad_apply("{ scale: 9 }");
    assert!(scale.starts_with("TypeError: ") && scale.contains(".scale"), "{scale}");
    assert_eq!(sent, 0, "a request that does not check is never sent");
}

#[test]
fn a_bad_mode_throws_naming_the_key_and_sends_nothing() {
    let (mode, sent) = bad_apply("{ mode: '1080p' }");
    assert!(mode.starts_with("TypeError: ") && mode.contains(".mode"), "{mode}");
    assert_eq!(sent, 0, "a request that does not check is never sent");
}

#[test]
#[ignore = "vrnw: needs OutputConfig/apply to refuse a key it does not have (an unknown key is dropped silently)"]
fn an_unknown_key_throws_naming_the_key_and_sends_nothing() {
    let (key, sent) = bad_apply("{ colour: 'red' }");
    assert!(key.starts_with("TypeError: ") && key.contains(".colour"), "{key}");
    assert_eq!(sent, 0, "a request that does not check is never sent");
}

#[test]
#[ignore = "vrnw: needs OutputConfig.primary refused with 'derives the primary output' (the key is dropped silently)"]
fn a_primary_the_layout_would_ignore_is_refused_with_its_reason() {
    let (primary, sent) = bad_apply("{ primary: true }");
    assert!(
        primary.starts_with("TypeError: ")
            && primary.contains(".primary")
            && primary.contains("derives the primary output"),
        "a primary the layout would ignore is refused with its reason: {primary}"
    );
    assert_eq!(sent, 0, "a request that does not check is never sent");
}

#[test]
fn a_fractional_position_throws_naming_the_key_and_sends_nothing() {
    let (position, sent) = bad_apply("{ positionX: 1.5, positionY: 0 }");
    assert!(position.contains("position"), "{position}");
    assert_eq!(sent, 0, "a request that does not check is never sent");
}

#[test]
fn a_mirror_of_that_is_not_a_connector_throws_naming_the_key_and_sends_nothing() {
    let (mirror, sent) = bad_apply("{ mirrorOf: 3 }");
    assert!(mirror.contains(".mirrorOf"), "{mirror}");
    assert_eq!(sent, 0, "a request that does not check is never sent");
}

#[test]
fn mirror_names_the_source_by_connector_and_null_stops_it() {
    let mut rig = world_with_connectors("");
    exec(
        &mut rig,
        "gnoblin.outputs.at(1).mirror(gnoblin.outputs.at(0)); gnoblin.outputs.at(1).mirror();",
    );
    settle(&mut rig);
    let sent: Vec<String> = rig.comp.applies().into_iter().map(|(_, text, _)| text).collect();
    assert_eq!(sent, vec!["mirror_of=DP-1".to_owned(), "mirror_of=".to_owned()]);
}

#[test]
fn the_profiles_of_the_config_are_applied_by_connector_and_a_refusal_names_the_key_path() {
    let mut rig = world_with_connectors(
        "gnoblin.outputs.profiles.add(new OutputProfile({ connector: 'HDMI-1' }, { settings: new ProfileSettings({ scale: 1.25, position: new OutputPosition({ kind: 'absolute', x: 0, y: 800 }) }) }));\n\
         gnoblin.outputs.profiles.add(new OutputProfile({ connector: 'DP-9' }, { settings: new ProfileSettings({ scale: 2 }) }));",
    );
    let sent = rig.comp.applies();
    assert_eq!(
        sent.len(),
        1,
        "one call for the one connected output the table names: {sent:?}"
    );
    assert_eq!((sent[0].0, sent[0].1.as_str()), (B, "scale=1.25\nposition=0,800"));

    // A refusal about the entry as a whole has no key of its own.
    exec(&mut rig, "g.o = gnoblin.outputs.at(1).apply({ scale: 1 });");
    settle(&mut rig);
    let token = rig.comp.applies()[1].2;
    rig.comp.answer(token, "", "no output would stay enabled");
    let diagnostics = rig.settle();
    assert!(
        diagnostics.iter().all(|line| !line.starts_with("outputs.")),
        "a script's own apply reports on its operation, not as a config notice: {diagnostics:?}"
    );
}

#[test]
#[ignore = "vrnw: needs the config notice a refused profile request carries (outputs.profiles.<connector>.<key>: <words>) in the shell's diagnostics"]
fn a_refused_profile_request_names_the_key_path_in_a_notice() {
    let mut rig = world_with_connectors(
        "gnoblin.outputs.profiles.add(new OutputProfile({ connector: 'HDMI-1' }, { settings: new ProfileSettings({ scale: 1.25, position: new OutputPosition({ kind: 'absolute', x: 0, y: 800 }) }) }));",
    );
    let sent = rig.comp.applies();
    rig.comp
        .answer(sent[0].2, "", "position: HDMI-1 and DP-1 would overlap");
    let diagnostics = rig.settle();
    assert!(
        diagnostics
            .iter()
            .any(|line| line == "outputs.profiles.HDMI-1.position: HDMI-1 and DP-1 would overlap"),
        "the notice carries the key path: {diagnostics:?}"
    );
}

#[test]
fn a_profile_that_does_not_check_fails_the_config_naming_the_profile() {
    let init = "gnoblin.outputs.profiles.add(new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ scale: 9 }) }));\n";
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = std::fs::canonicalize(dir.path()).expect("the temp dir exists");
    std::fs::write(root.join("shell.ts"), format!("{IMPORTS}{init}")).expect("shell.ts is written");
    let (mut shell, _core) = common::native::shell_over_core(
        &FakeCompositor::new(),
        std::sync::Arc::new(gnoblin_shell_host::supervisor::ManualClock::default()),
        honest_config(&root),
    );
    let error = shell
        .load_entry(std::path::Path::new("shell.ts"))
        .expect_err("the config does not check");
    assert!(error.to_string().contains("OutputProfile \"DP-1\".scale"), "{error}");
}

#[test]
fn a_mirrored_or_disabled_output_has_no_desk_and_gets_one_back_when_it_is_ordinary_again() {
    let mut rig = world_with_connectors("for (const id of ['1', '2']) gnoblin.workspaces.add(new Workspace(id));");
    assert_eq!(shown(&mut rig), "output-0=1 output-1=2");

    // The compositor publishes the mirror: the output keeps its screen node and loses its desk.
    rig.comp.external_set(B, "mirror_of", text("DP-1"));
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=1 output-1=undefined");
    assert_eq!(rig.eval("gnoblin.outputs.at(1).mirrorOf?.connector ?? 'none'"), "DP-1");
    assert_eq!(
        rig.eval("gnoblin.outputs.at(0).mirrorOf === undefined ? 'null' : 'set'"),
        "null"
    );
    assert_eq!(
        rig.eval("gnoblin.outputs.length.toString()"),
        "2",
        "the output is still in the collection"
    );

    rig.comp.external_set(B, "mirror_of", text(""));
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=1 output-1=2");

    rig.comp.external_set(B, "enabled", MirrorValue::Bool(false));
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=1 output-1=undefined");
}
