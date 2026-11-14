//! Outputs and desks of the stock shell: `gnoblin.outputs`, the workspace each output shows, the windows of an output
//! that is unplugged, `Client.moveToOutput`, and dialogs that stay with their parent, over the fake compositor of
//! `tests/common`.
//!
//! The cases are the Lua shell's `crates/gnoblin-lua/tests/lua/shell_test.lua` (moving, unplugging, reconnecting,
//! dialogs), keeping their inputs and expected values. Where the TypeScript model differs the case says why. The
//! difference is the model's: a Lua `Desk` owns its own copy of every workspace, so each output shows "its" workspace
//! 1; a TypeScript `Workspace` is the script's own object and is on one output at a time, so showing a workspace that
//! another output shows changes the two outputs' places (a swap). A window's coordinates are the compositor's global
//! ones, so a window on an output at `x = 1000` has `x >= 1000`; the Lua windows were relative to their desk. A case
//! with no Lua original says "new". What the real compositor does with the writes is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::{MirrorChange, MirrorChangeKind, MirrorValue};

mod common;
use common::*;

const IMPORTS: &str = "import * as lib from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n";

const SEAT: u64 = 21;
const A: u64 = 20;
const B: u64 = 22;

/// A layout that gives every window the whole area it is offered: what the area of an output is, as a window reads it.
const FILL: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { ...c.area } })); }\n\
}\n";

/// A shell over one screen `A` (1000 by 800, primary) and, when `second`, a screen `B` of 600 by 400 at x 1000, a seat,
/// and `n` windows (ids 3 on, 300 by 200 at 10 n, 20 n), on the workspaces `1` to `3`, whose layouts place nothing
/// unless `layout` says so. `config` is more keys of the config object.
fn world(n: u64, second: bool, layout: &str, config: &str) -> Rig {
    let init = format!(
        "{FILL}defineConfig({{ workspaces: [{{ id: \"1\", layout: {layout} }}, {{ id: \"2\", layout: {layout} }}, {{ id: \"3\", layout: {layout} }}], decorations: false, {config} }});\n"
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
    rig.eval("gnoblin.outputs.map((o) => `${o.name}=${o.workspace?.id}`).join(' ')")
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
    let mut rig = world(0, true, "new Layout()", "");
    assert_eq!(rig.eval("String(gnoblin.outputs.length)"), "2");
    assert_eq!(
        rig.eval("gnoblin.outputs.map((o) => `${o.name}:${o.x},${o.y},${o.width}x${o.height}:${o.primary}`).join(' ')"),
        "output-0:0,0,1000x800:true output-1:1000,0,600x400:false"
    );
    assert_eq!(
        rig.eval(
            "gnoblin.outputs.where({ primary: true }).at(0).name + ' ' + gnoblin.outputs.find((o) => o.x === 1000).name"
        ),
        "output-0 output-1"
    );
    assert_eq!(
        rig.eval("[gnoblin.outputs.at(0).make, gnoblin.outputs.at(0).model, gnoblin.outputs.at(0).mode].join('|')"),
        "||",
        "a property the compositor has not published reads as the schema's empty default"
    );
    assert_eq!(
        rig.eval("(() => { const o = gnoblin.outputs.at(0); return [o.scale, o.transform, o.enabled, o.connected].join('|'); })()"),
        "1|normal|true|true"
    );
    assert_eq!(
        rig.eval("gnoblin.outputs.get(gnoblin.outputs.at(1).id) === gnoblin.outputs.at(1) ? 'same' : 'other'"),
        "same",
        "one object per screen"
    );
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.outputs.at(1).area) + JSON.stringify(gnoblin.workspaces.area)"),
        "{\"x\":1000,\"y\":0,\"width\":600,\"height\":400}{\"x\":0,\"y\":0,\"width\":1000,\"height\":800}",
        "the area is per output; the collection's is the primary output's"
    );
    assert_eq!(
        rig.eval("try { gnoblin.outputs.at(0).width = 1; 'wrote' } catch (e) { e.constructor.name }"),
        "TypeError",
        "an output's properties are the compositor's"
    );
}

#[test]
fn a_published_make_model_mode_and_scale_are_what_an_output_reads() {
    // New: the compositor publishes them on the screen node; the output reads them.
    let mut rig = world(0, false, "new Layout()", "");
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
    // New: `OutputListener.onConnected` and `onDisconnected`, and `CollectionListener` membership, over the Lua
    // `Screen:on_ready` and `Screen:on_destroyed` that made and dropped a `Desk`.
    let mut rig = world(0, false, "new Layout()", "");
    exec(
        &mut rig,
        "g.log = [];\n\
         gnoblin.outputs.addListener(new (class extends OutputListener {\n\
             onConnected(o) { g.log.push('connected ' + o.name); }\n\
             onDisconnected(o) { g.log.push('disconnected ' + o.name + ' ' + o.connected + ' ' + o.width); }\n\
             onModeChanged(o, m) { g.log.push('mode ' + o.name + ' ' + m); }\n\
             onScaleChanged(o, s) { g.log.push('scale ' + o.name + ' ' + s); }\n\
             onGeometryChanged(o, r) { g.log.push('geometry ' + o.name + ' ' + JSON.stringify(r)); }\n\
         })());\n\
         gnoblin.outputs.addListener(new (class extends CollectionListener {\n\
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
        "disconnected output-1 false 800;removed output-1",
        "the listener reads the output's last values, and it is no longer connected"
    );
    assert_eq!(
        rig.eval("(() => { const o = g.gone; return [o.alive, o.connected, o.name, o.width, o.workspace === null].join('|'); })()"),
        "false|false|output-1|800|true"
    );
}

// ---- the workspace each output shows ---------------------------------------------------------------------------------

#[test]
fn each_output_shows_its_own_workspace_and_a_workspace_is_on_one_output() {
    // Lua: "Desk builds" (one desk per screen, each with a workspace showing). Derived: a TypeScript workspace is on
    // one output, so the second output starts on the first workspace no output shows.
    let mut rig = world(0, true, "new Layout()", "");
    assert_eq!(shown(&mut rig), "output-0=1 output-1=2");
    assert_eq!(
        rig.eval("gnoblin.workspaces.map((w) => `${w.id}:${w.active}:${w.output?.name ?? '-'}`).join(' ')"),
        "1:true:output-0 2:true:output-1 3:false:-"
    );
    // Showing "3" on the second output leaves the first alone.
    exec(&mut rig, "gnoblin.outputs.at(1).show('3')");
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=1 output-1=3");
    assert_eq!(
        rig.eval("gnoblin.workspaces.map((w) => `${w.active}`).join(' ')"),
        "true false true"
    );
    // Showing "1" on the second output takes it from the first, which takes the second's workspace.
    exec(&mut rig, "gnoblin.outputs.at(1).show(gnoblin.workspaces.get('1'))");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=3 output-1=1",
        "a swap: no workspace is on two outputs"
    );
    assert_eq!(
        rig.eval("gnoblin.workspaces.where({ active: true }).map((w) => w.id).join(',')"),
        "1,3"
    );
    assert_eq!(
        rig.eval("try { gnoblin.outputs.at(0).show('nope'); 'no' } catch (e) { e.code }"),
        "unknown-workspace"
    );
}

#[test]
fn a_plain_show_acts_on_the_output_under_the_pointer_and_the_primary_one_when_no_pointer_says() {
    // Lua: `desk()` is the desk under the pointer (`seat.screen`). Derived: with no pointer screen the primary output.
    let mut rig = world(0, true, "new Layout()", "");
    exec(&mut rig, "gnoblin.show('3')");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=3 output-1=2",
        "no pointer screen: the primary output"
    );
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    exec(&mut rig, "gnoblin.workspaces.get('1').show()");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=3 output-1=1",
        "the pointer is on the second output"
    );
}

#[test]
fn a_new_window_joins_the_workspace_shown_on_the_output_under_the_pointer() {
    // Lua: `Window:on_ready` adds the window to `desk().active`.
    let mut rig = world(0, true, "new Layout()", "");
    rig.comp.external_set(SEAT, "screen", MirrorValue::Node(B));
    settle(&mut rig);
    rig.comp.create_window(30, "second", "foot");
    settle(&mut rig);
    assert_eq!(workspace_of(&mut rig, 30), "2", "the workspace the second output shows");
    assert_eq!(
        rig.eval("gnoblin.clients.get('30').output?.name"),
        "output-1",
        "a client's output is the one its workspace is shown on"
    );
}

#[test]
fn a_layout_tiles_in_the_area_of_the_output_that_shows_its_workspace() {
    // Lua: `Desk:arrange` gives each desk's workspaces the desk's own size. Derived: the area is in the compositor's
    // coordinates, so the second output's starts at its own x.
    let mut rig = world(0, true, "new Fill()", "");
    rig.comp.create_window(30, "one", "foot");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 30),
        "0,0,1000,800",
        "a window joins the first output's workspace"
    );
    exec(&mut rig, "gnoblin.outputs.at(1).show('1')");
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
    // Lua: bars are children of a `Desk` (`Desk:arrange` stacks them on that desk's edges). New: `Bar` takes an output.
    let mut rig = world(0, true, "new Fill()", "");
    exec(
        &mut rig,
        "g.main = new Bar({ edge: 'top', height: 32 });\n\
         g.side = new Bar({ edge: 'bottom', height: 20, output: gnoblin.outputs.at(1) });",
    );
    rig.comp.create_window(30, "one", "foot");
    settle(&mut rig);
    assert_eq!(rect(&rig, 30), "0,32,1000,768", "the first output's top bar");
    exec(&mut rig, "gnoblin.outputs.at(1).show('1')");
    settle(&mut rig);
    assert_eq!(
        rect(&rig, 30),
        "1000,0,600,380",
        "the second output has its own bottom bar and none on top"
    );
    assert_eq!(
        rig.eval(
            "[g.main.onOutput === gnoblin.outputs.at(0), g.side.onOutput === gnoblin.outputs.at(1), String(barInsets(gnoblin.outputs.at(1)).bottom), String(barInsets().top)].join('|')"
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
    // Lua: "unplugging one of two outputs moves its windows to the other output".
    let mut rig = world(0, true, "new Layout()", "");
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
    assert_eq!(rig.eval("gnoblin.clients.get('31').output?.name"), "output-0");
    assert_eq!(
        rig.eval("gnoblin.workspaces.map((w) => `${w.id}:${w.output?.name ?? '-'}`).join(' ')"),
        "1:output-0 2:- 3:-",
        "the workspace of the unplugged output is shown nowhere"
    );
}

#[test]
fn windows_moved_off_an_unplugged_output_are_fitted_to_the_output_that_remains() {
    // Lua: `Window:move_to` refits (`fit_into`) what it moves; the unplug uses the same. The second output was 600 by
    // 400 at x 1000; the window moves to the first (1000 by 800 at the origin).
    let mut rig = world(0, true, "new Layout()", "");
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
    // Lua: "unplugging the last output keeps its windows hidden and drops focus".
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    assert_eq!(focus(&rig), 3);
    unplug(&mut rig, A);
    assert_eq!(workspace_of(&mut rig, 3), "1", "the window must survive the unplug");
    assert!(!visible(&rig, 3), "the window's workspace must be hidden");
    assert_eq!(focus(&rig), 0, "a window with no output cannot hold focus");
    assert_eq!(rig.eval("String(gnoblin.outputs.length)"), "0");
    assert_eq!(
        rig.eval("String(gnoblin.workspaces.get('1').active)"),
        "false",
        "a parked workspace is not on show"
    );
}

#[test]
fn reconnecting_an_output_adopts_the_parked_windows_and_focuses_one() {
    // Lua: "reconnecting an output adopts the parked windows and focuses one".
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    unplug(&mut rig, A);
    rig.comp.create_screen(A, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
    settle(&mut rig);
    assert!(visible(&rig, 3), "the adopted workspace must show");
    assert_eq!(
        rig.eval("gnoblin.clients.get('3').output?.name"),
        "output-0",
        "the window must be on the new output"
    );
    assert_eq!(focus(&rig), 3, "reconnecting must focus the window again");
    assert_eq!(rig.eval("String(gnoblin.workspaces.get('1').active)"), "true");
}

#[test]
fn a_window_on_a_second_output_goes_to_the_first_when_it_is_unplugged_and_comes_back_with_the_next_output() {
    // New: the acceptance case. The second output's window follows to the first when the second is removed; a
    // reconnected output starts on the first free workspace, and the window moves back when asked.
    let mut rig = world(0, true, "new Fill()", "");
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
    // Lua: "moving a window to an output that is not a screen is rejected".
    let mut rig = world(1, true, "new Layout()", "");
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
    // Lua: "moving a window to another output reshapes it into that output's work area".
    let mut rig = world(1, true, "new Layout()", "");
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
    // Lua: "a maximised window fills the new output when it moves".
    let mut rig = world(1, true, "new Layout()", "");
    exec(&mut rig, "windowState.maximise(gnoblin.clients.get('3'));");
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
    // Lua: "a fullscreen window does not move to another workspace" and "... another output".
    let mut rig = world(1, true, "new Layout()", "");
    exec(&mut rig, "gnoblin.clients.get('3').fullscreen = true");
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToOutput(gnoblin.outputs.at(1)))"),
        "false",
        "a fullscreen window must refuse an output move"
    );
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('3')))"),
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
fn a_moved_window_that_held_the_focus_hands_it_to_a_window_that_stays() {
    // Lua: "moving the focused window off the active workspace gives focus to a window that stays".
    let mut rig = world(2, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('2')))"),
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
        rig.eval("String(gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('2')))"),
        "false",
        "the workspace it is on already"
    );
}

#[test]
fn moving_the_only_focused_window_off_the_screen_leaves_nobody_focused() {
    // New: with no window left to take the focus, the seat holds none.
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    exec(
        &mut rig,
        "gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('2'))",
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
    // Lua: `Window:on_ready` adds a dialog to its parent and centres it: `(p.width - self.width) // 2`.
    let mut rig = world(1, false, "new Layout()", "");
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
    assert_eq!(rig.eval("String(gnoblin.clients.get('3').parentWindow)"), "null");
}

#[test]
fn a_dialog_whose_parent_is_on_another_workspace_goes_with_its_parent() {
    // Lua: "a dialog whose parent is on another workspace goes with its parent".
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.workspaces.get('2').clients.add(gnoblin.clients.get('3'))",
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
    // Lua: "a dialog whose parent is closed has no parent_window and is placed as a normal window".
    let mut rig = world(1, false, "new Layout()", "");
    rig.comp.destroy(3);
    settle(&mut rig);
    create_dialog(&mut rig, 30, 3);
    assert_eq!(rig.eval("String(gnoblin.clients.get('30').parentWindow)"), "null");
    assert_eq!(
        workspace_of(&mut rig, 30),
        "1",
        "placed on the workspace the output shows"
    );
}

#[test]
fn a_dialog_does_not_move_to_another_workspace_on_its_own_but_goes_with_its_parent() {
    // Lua: "a dialog does not move to another workspace on its own" and "moving a parent window takes its dialog with it".
    let mut rig = world(1, false, "new Layout()", "");
    create_dialog(&mut rig, 30, 3);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('30').moveToWorkspace(gnoblin.workspaces.get('2')))"),
        "false",
        "a dialog move must be refused"
    );
    assert_eq!(workspace_of(&mut rig, 30), "1");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    settle(&mut rig);
    assert_eq!(workspace_of(&mut rig, 30), "2", "the dialog must follow its parent");
}

#[test]
fn a_dialog_follows_its_parent_to_another_output_and_keeps_its_place_in_it() {
    // Lua: "a dialog follows its parent to another output".
    let mut rig = world(1, true, "new Layout()", "");
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
    // Lua: a dialog is a child node of its parent, so it moves with it and the workspace's layout never places it.
    let mut rig = world(1, false, "new Fill()", "");
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

// ---- workspace sets on several outputs -------------------------------------------------------------------------------

#[test]
fn a_workspace_set_shows_each_workspace_on_its_own_output() {
    // Lua: "sets_test.lua" and `workspace_sets` in init.lua: each entry shows `d:show(ws)` on the desk of its output.
    // The TypeScript shell used to report an entry for a non-primary output and not show it.
    let mut rig = world(0, true, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.workspaces.sets.define([\n\
            { name: 'split', entries: [{ workspace: '3', output: 'output-0' }, { workspace: '1', output: 'output-1' }] },\n\
            { name: 'away', entries: [{ workspace: '2', output: 'GONE-1' }] },\n\
         ]);\n\
         gnoblin.workspaces.sets.activate('split');",
    );
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=3 output-1=1");
    assert_eq!(rig.eval("String(gnoblin.workspaces.sets.active)"), "split");
    exec(&mut rig, "gnoblin.workspaces.sets.activate('away');");
    let reported = rig.settle();
    assert_eq!(
        reported.len(),
        1,
        "an output that is gone is reported once: {reported:?}"
    );
    assert!(
        reported[0].contains("output \"GONE-1\" is not connected"),
        "{reported:?}"
    );
    assert_eq!(
        shown(&mut rig),
        "output-0=2 output-1=1",
        "an output that is gone shows its workspace on the primary one"
    );
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
fn a_launched_window_does_not_take_focus_unless_new_windows_are_allowed_to() {
    // Lua: `Focus.opens_focused(focus_settings(), self.from_launcher)`: a window from a launcher takes focus only when
    // `focus_new_windows` is `allow`. The Lua `from_launcher` was a field of the fake core only; here it is the real
    // node's property.
    let mut rig = world(1, false, "new Layout()", "");
    give_focus(&mut rig, 3);
    create_launched(&mut rig, 30);
    assert_eq!(rig.eval("String(gnoblin.clients.get('30').fromLauncher)"), "true");
    assert_eq!(focus(&rig), 3, "the default `prevent` keeps focus where it was");
    rig.comp.create_window(31, "own", "foot");
    settle(&mut rig);
    assert_eq!(focus(&rig), 31, "a window nobody launched takes focus");

    let mut allowing = world(1, false, "new Layout()", "focus: { newWindows: 'allow' },");
    give_focus(&mut allowing, 3);
    create_launched(&mut allowing, 30);
    assert_eq!(focus(&allowing), 30, "`allow` lets a launched window take focus");
}

// ---- a reload ------------------------------------------------------------------------------------------------------

#[test]
fn a_reload_puts_each_output_back_on_the_workspace_it_showed() {
    // New: the snapshot a reload carries holds what each output showed, by the output's name.
    let mut rig = world(0, true, "new Layout()", "");
    exec(&mut rig, "gnoblin.outputs.at(1).show('3')");
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
    let init = format!(
        "{IMPORTS}defineConfig({{ workspaces: [{{ id: \"1\" }}, {{ id: \"2\" }}, {{ id: \"3\" }}], decorations: false }});\n"
    );
    std::fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let mut second =
        gnoblin_js::JsShell::new(Box::new(rig.comp.clone()), honest_config(&root)).expect("the shell boots");
    second
        .load_entry(std::path::Path::new("init.ts"))
        .expect("init.ts loads");
    second.reconcile(&snapshot).expect("reconciles");
    for _ in 0..4 {
        second.frame().expect("a frame");
    }
    let shown = second
        .eval("gnoblin.outputs.map((o) => `${o.name}=${o.workspace?.id}`).join(' ')")
        .expect("the script runs")
        .expect("a value");
    assert_eq!(shown, "output-0=1 output-1=3");
}
