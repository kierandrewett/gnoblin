//! Logical displays (owner-spec sections 8 and 9): `OutputProfile`, `Display`, `Viewport`, `Camera2D` and `Camera3D` on a
//! viewport, `gnoblin.displays` and `input.activeDisplay`, over the fake compositor of `tests/common`.
//!
//! The scripts are written the way the specification writes them (no `defineConfig`). What the real compositor does with
//! the output requests, the mirror and the span is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

const IMPORTS: &str = "import * as lib from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n";

const SEAT: u64 = 21;
const A: u64 = 20;
const B: u64 = 22;

/// The script every test starts from: three workspaces whose layout gives each window the whole area, kept in `g.ws`, and
/// the profiles of the two monitors, `g.a` for `DP-1` and `g.b` for `HDMI-A-1`, which are not added to anything.
const START: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { ...c.area } })); }\n\
}\n\
const one = new Workspace({ id: \"one\", layout: new Fill() });\n\
const two = new Workspace({ id: \"two\", layout: new Fill() });\n\
const three = new Workspace({ id: \"three\", layout: new Fill() });\n\
for (const ws of [one, two, three]) gnoblin.workspaces.add(ws);\n\
g.ws = { one, two, three };\n\
g.a = new OutputProfile({ match: { connector: \"DP-1\" } });\n\
g.b = new OutputProfile({ match: { connector: \"HDMI-A-1\" } });\n";

/// A shell over screen `A` (`DP-1`, 1000 by 800, primary) and, when `second`, screen `B` (`HDMI-A-1`, 600 by 400 at
/// x 1000), a seat and one window (id 3), running `START` and then `extra`.
fn world(second: bool, extra: &str) -> Rig {
    let init = format!("{START}{extra}\n");
    let mut rig = rig_setup(IMPORTS, &init, move |comp| {
        comp.add_node(
            A,
            ROOT,
            "screen",
            vec![
                ("name", text("output-0")),
                ("connector", text("DP-1")),
                ("x", MirrorValue::Double(0.0)),
                ("y", MirrorValue::Double(0.0)),
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        if second {
            comp.add_node(
                B,
                ROOT,
                "screen",
                vec![
                    ("name", text("output-1")),
                    ("connector", text("HDMI-A-1")),
                    ("x", MirrorValue::Double(1000.0)),
                    ("y", MirrorValue::Double(0.0)),
                    ("width", MirrorValue::Double(600.0)),
                    ("height", MirrorValue::Double(400.0)),
                    ("primary", MirrorValue::Bool(false)),
                ],
            );
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
        comp.add_node(
            3,
            ROOT,
            "window",
            vec![
                ("title", text("win")),
                ("app_id", text("foot")),
                ("x", MirrorValue::Double(10.0)),
                ("y", MirrorValue::Double(20.0)),
                ("width", MirrorValue::Double(300.0)),
                ("height", MirrorValue::Double(200.0)),
            ],
        );
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

fn settle(rig: &mut Rig) -> Vec<String> {
    rig.settle()
}

/// Which workspace each output shows, as `name=id` words.
fn shown(rig: &mut Rig) -> String {
    rig.eval("gnoblin.outputs.map((o) => `${o.name}=${o.workspace?.id}`).join(' ')")
}

/// The displays as `id:mode:outputs:workspace` words, sorted.
fn displays(rig: &mut Rig) -> String {
    rig.eval(
        "gnoblin.displays.map((d) => `${d.id}:${d.mode}:${d.outputs.length}:${d.viewport.content?.id}`).sort().join(' ')",
    )
}

/// The output requests sent for screen `node`, in order.
fn requests(rig: &Rig, node: u64) -> Vec<String> {
    rig.comp
        .applies()
        .into_iter()
        .filter(|(target, ..)| *target == node)
        .map(|(_, text, _)| text)
        .collect()
}

/// Answers every request not answered yet with success.
fn answer_all(rig: &mut Rig, answered: &mut usize) {
    let applies = rig.comp.applies();
    for (_, _, token) in &applies[*answered..] {
        rig.comp.answer(*token, "", "");
    }
    *answered = applies.len();
    settle(rig);
}

// ---- gnoblin.displays and input.activeDisplay ---------------------------------------------------------------------

#[test]
fn the_shell_keeps_one_display_per_output_and_the_active_display_is_the_one_under_the_pointer() {
    let mut rig = world(true, "");
    assert_eq!(
        displays(&mut rig),
        "display-DP-1:single:1:one display-HDMI-A-1:single:1:two",
        "each output with a desk has a single display showing what its desk shows"
    );
    assert_eq!(
        rig.eval("gnoblin.input.primary.activeDisplay.id"),
        "display-DP-1",
        "the pointer is on the first output"
    );
    rig.comp.external_set(SEAT, "pointer_x", MirrorValue::Double(1100.0));
    settle(&mut rig);
    assert_eq!(rig.eval("gnoblin.input.primary.activeDisplay.id"), "display-HDMI-A-1");
    rig.comp.external_set(SEAT, "pointer_x", MirrorValue::Double(9000.0));
    settle(&mut rig);
    assert_eq!(
        rig.eval("gnoblin.input.primary.activeDisplay.id"),
        "display-DP-1",
        "a pointer outside every output falls back to the primary output's display"
    );
    assert_eq!(
        rig.eval("gnoblin.displays.get('display-DP-1').outputs.at(0).connector"),
        "DP-1",
        "a display's outputs are a collection of Output"
    );
}

#[test]
fn a_display_is_removed_with_its_output_and_returns_with_the_next_one() {
    let mut rig = world(true, "");
    rig.comp.destroy(B);
    settle(&mut rig);
    assert_eq!(displays(&mut rig), "display-DP-1:single:1:one");
    rig.comp
        .create_screen(30, "output-2", (1000.0, 0.0, 640.0, 480.0), false);
    rig.comp.external_set(30, "connector", text("DP-2"));
    settle(&mut rig);
    assert_eq!(
        displays(&mut rig),
        "display-DP-1:single:1:one display-DP-2:single:1:two",
        "a new output gets a display of its own"
    );
}

#[test]
fn assigning_a_viewports_content_shows_the_workspace_on_that_display_as_show_does() {
    let mut rig = world(true, "");
    exec(
        &mut rig,
        "g.left = gnoblin.displays.get('display-DP-1'); g.right = gnoblin.displays.get('display-HDMI-A-1');",
    );
    exec(&mut rig, "g.left.viewport.content = g.ws.three;");
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=three output-1=two");
    assert_eq!(rig.eval("g.left.viewport.content.id"), "three");
    // A workspace is on one display at a time: the one that another display shows moves, and that display takes the one
    // this display showed, exactly as `gnoblin.workspaces.show` does.
    exec(&mut rig, "g.right.viewport.content = g.ws.three;");
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=two output-1=three");
    assert_eq!(
        rig.eval("g.left.viewport.content.id"),
        "two",
        "a display reads what its desk shows"
    );
    assert_eq!(
        rig.eval("try { g.left.viewport.content = new Workspace({ id: 'loose' }); 'set' } catch (e) { e.code }"),
        "detached",
        "a workspace that is not in gnoblin.workspaces cannot be shown, as with `show`"
    );
    assert_eq!(
        rig.eval("try { g.left.viewport.content = 'one'; 'set' } catch (e) { e.constructor.name }"),
        "TypeError"
    );
}

#[test]
fn a_script_display_is_listed_and_an_output_is_presented_by_one_display_at_a_time() {
    let mut rig = world(true, "");
    exec(
        &mut rig,
        "g.mine = new Display({ id: 'left', outputs: [g.a], viewport: new Viewport({ content: g.ws.three, camera: new Camera2D() }) });\n\
         g.rival = new Display({ id: 'also-left', outputs: [g.a], viewport: new Viewport({ content: g.ws.two }) });\n\
         gnoblin.displays.add(g.mine);",
    );
    settle(&mut rig);
    assert_eq!(
        displays(&mut rig),
        "display-HDMI-A-1:single:1:two left:single:1:three",
        "the display for DP-1 is the script's, which shows the workspace it asked for"
    );
    let error = rig.eval("try { gnoblin.displays.add(g.rival); 'added' } catch (e) { `${e.code}|${e.message}` }");
    assert!(error.starts_with("output-conflict|"), "{error}");
    assert!(
        error.contains("\"also-left\"") && error.contains("\"left\"") && error.contains("DP-1"),
        "the error names both displays and the output: {error}"
    );
    // `replace` swaps them as one change.
    exec(&mut rig, "gnoblin.displays.replace(g.mine, g.rival);");
    settle(&mut rig);
    assert_eq!(
        displays(&mut rig),
        "also-left:single:1:two display-HDMI-A-1:single:1:three",
        "the rival shows its workspace on DP-1, and the output that showed it takes what DP-1 showed"
    );
    assert_eq!(rig.eval("String(gnoblin.displays.get('left'))"), "undefined");
}

#[test]
fn removing_a_script_display_gives_its_output_a_display_of_the_shells_again() {
    let mut rig = world(
        true,
        "g.mine = new Display({ id: 'left', outputs: [g.a], viewport: new Viewport({ content: g.ws.one }) });\ngnoblin.displays.add(g.mine);",
    );
    assert_eq!(
        rig.eval("gnoblin.displays.map((d) => d.id).sort().join(' ')"),
        "display-HDMI-A-1 left"
    );
    assert_eq!(rig.eval("String(gnoblin.displays.remove(g.mine))"), "true");
    settle(&mut rig);
    assert_eq!(
        rig.eval("gnoblin.displays.map((d) => d.id).sort().join(' ')"),
        "display-DP-1 display-HDMI-A-1"
    );
    assert_eq!(
        rig.eval("String(gnoblin.displays.remove(gnoblin.displays.at(0)))"),
        "false",
        "the display the shell keeps for an output stays"
    );
}

#[test]
fn a_display_whose_output_is_not_connected_is_parked_with_its_workspace_and_wakes_when_the_output_arrives() {
    let mut rig = world(
        true,
        "g.hdmi9 = new OutputProfile({ match: { connector: 'HDMI-9' } });\n\
         g.parked = new Display({ id: 'parked', outputs: [g.hdmi9], viewport: new Viewport({ content: g.ws.one }) });\n\
         gnoblin.displays.add(g.parked);",
    );
    assert_eq!(
        rig.eval("String(g.parked.outputs.length)"),
        "0",
        "no output matches: the display is parked"
    );
    assert_eq!(
        rig.eval("g.parked.viewport.content.id"),
        "one",
        "the workspace it was given is kept while it is parked"
    );
    assert_eq!(
        shown(&mut rig),
        "output-0=one output-1=two",
        "a parked display changes nothing"
    );
    rig.comp
        .create_screen(30, "output-2", (1600.0, 0.0, 640.0, 480.0), false);
    rig.comp.external_set(30, "connector", text("HDMI-9"));
    settle(&mut rig);
    assert_eq!(rig.eval("String(g.parked.outputs.length)"), "1");
    assert_eq!(
        rig.eval("gnoblin.outputs.find((o) => o.connector === 'HDMI-9').workspace.id"),
        "one",
        "the display shows its workspace on the output that arrived"
    );
}

#[test]
fn display_arguments_are_checked() {
    let mut rig = world(true, "");
    let problem = |rig: &mut Rig, source: &str| {
        rig.eval(&format!(
            "try {{ {source}; 'ok' }} catch (e) {{ e.constructor.name + ': ' + e.message }}"
        ))
    };
    assert!(problem(&mut rig, "new Display({ outputs: [] })").starts_with("TypeError: Display.outputs"));
    assert!(problem(&mut rig, "new Display({ outputs: [g.a, g.b] })").contains("\"single\" presents one output"));
    assert!(
        problem(&mut rig, "new Display({ outputs: [g.a], mode: 'tile' })")
            .contains("must be \"single\", \"mirror\" or \"span\"")
    );
    assert!(problem(&mut rig, "new Display({ outputs: [g.a], colour: 1 })").contains("Display.colour"));
    exec(
        &mut rig,
        "g.vp = new Viewport({ content: g.ws.one }); new Display({ outputs: [g.a], viewport: g.vp });",
    );
    assert!(problem(&mut rig, "new Display({ outputs: [g.b], viewport: g.vp })").contains("already shown by display"));
    assert!(problem(&mut rig, "new Viewport({ camera: {} })").contains("must be a Camera2D or a Camera3D"));
    assert!(problem(&mut rig, "new Viewport({ content: 3 })").contains("must be a Workspace"));
}

// ---- Mirror and span ----------------------------------------------------------------------------------------------

#[test]
fn a_mirror_display_asks_the_compositor_to_mirror_and_ends_the_mirror_when_it_is_removed() {
    let mut rig = world(
        true,
        "g.mirror = new Display({ id: 'mirror', outputs: [g.a, g.b], mode: 'mirror', viewport: new Viewport({ content: g.ws.one, camera: new Camera2D() }) });\n\
         gnoblin.displays.add(g.mirror);",
    );
    assert_eq!(
        requests(&rig, B),
        vec!["mirror_of=DP-1"],
        "the second output mirrors the first"
    );
    assert!(requests(&rig, A).is_empty());
    // The compositor takes the mirror: the second output has no desk then, and the display still presents both.
    rig.comp.external_set(B, "mirror_of", text("DP-1"));
    settle(&mut rig);
    assert_eq!(displays(&mut rig), "mirror:mirror:2:one");
    exec(&mut rig, "gnoblin.displays.remove(g.mirror);");
    settle(&mut rig);
    assert_eq!(
        requests(&rig, B),
        vec!["mirror_of=DP-1", "mirror_of="],
        "removing the display stops the mirror it made"
    );
}

#[test]
fn a_span_display_lays_one_workspace_out_over_the_union_of_its_outputs() {
    let mut rig = world(
        true,
        "g.span = new Display({ id: 'span', outputs: [g.a, g.b], mode: 'span', viewport: new Viewport({ content: g.ws.one, camera: new Camera2D() }) });\n\
         gnoblin.displays.add(g.span);",
    );
    assert_eq!(
        shown(&mut rig),
        "output-0=one output-1=one",
        "both outputs show the span's workspace"
    );
    assert_eq!(
        rig.comp.rect(3),
        "0,0,1600,800",
        "the layout area is the bounding box of the outputs' areas: 1000 by 800 and 600 by 400 at x 1000"
    );
    assert_eq!(displays(&mut rig), "span:span:2:one");
    exec(&mut rig, "g.span.viewport.content = g.ws.two;");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=two output-1=two",
        "the span shows one workspace at a time"
    );
    exec(&mut rig, "gnoblin.displays.remove(g.span);");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=two output-1=one",
        "without the span each output shows a workspace of its own again"
    );
}

// ---- OutputProfile -------------------------------------------------------------------------------------------------

#[test]
fn a_profile_applies_its_settings_as_one_request_and_places_its_neighbour_from_the_answer() {
    let mut rig = world(false, "");
    rig.comp
        .create_screen(B, "output-1", (1200.0, 0.0, 600.0, 400.0), false);
    rig.comp.external_set(B, "connector", text("HDMI-A-1"));
    settle(&mut rig);
    exec(
        &mut rig,
        "g.main = new OutputProfile({ match: { connector: 'DP-1' }, mode: { width: 1000, height: 800, refreshRate: 60 }, scale: 1.25, transform: 'normal', position: { x: 0, y: 0 } });\n\
         g.second = new OutputProfile({ match: { connector: 'HDMI-A-1' }, mode: 'preferred', position: { rightOf: g.main, align: 'top' } });\n\
         gnoblin.outputs.profiles.add(g.main);\n\
         gnoblin.outputs.profiles.add(g.second);",
    );
    settle(&mut rig);
    assert_eq!(
        requests(&rig, A),
        vec!["mode=1000x800@60\nscale=1.25\ntransform=normal\nposition=0,0"],
        "one request carries all of a profile's settings"
    );
    assert_eq!(
        requests(&rig, B),
        vec!["mode=preferred"],
        "a relative position waits for the neighbour"
    );
    let mut answered = 0;
    answer_all(&mut rig, &mut answered);
    assert_eq!(
        requests(&rig, B),
        vec!["mode=preferred", "position=1000,0"],
        "once the neighbour has answered, the profile is placed against its rectangle"
    );
    answer_all(&mut rig, &mut answered);
    let sent = rig.comp.applies().len();
    settle(&mut rig);
    assert_eq!(rig.comp.applies().len(), sent, "a settled profile sends nothing more");
}

#[test]
fn a_profile_applies_again_when_its_monitor_returns_and_a_refusal_is_reported_by_key() {
    let mut rig = world(
        true,
        "g.p = new OutputProfile({ match: { connector: 'HDMI-A-1' }, scale: 2 });\ngnoblin.outputs.profiles.add(g.p);",
    );
    let first = rig.comp.applies();
    assert_eq!(first.len(), 1);
    rig.comp
        .answer(first[0].2, "", "scale: the backend does not take 2 here");
    let diagnostics = settle(&mut rig);
    assert!(
        diagnostics
            .iter()
            .any(|line| line == "outputs.profiles.HDMI-A-1.scale: the backend does not take 2 here"),
        "{diagnostics:?}"
    );
    rig.comp.destroy(B);
    settle(&mut rig);
    rig.comp
        .create_screen(B + 10, "output-3", (1000.0, 0.0, 600.0, 400.0), false);
    rig.comp.external_set(B + 10, "connector", text("HDMI-A-1"));
    settle(&mut rig);
    assert_eq!(
        requests(&rig, B + 10),
        vec!["scale=2"],
        "the monitor is back: the profile applies to it again"
    );
}

#[test]
fn profiles_match_by_make_model_and_serial_and_report_vrr_as_not_applied() {
    let mut rig = world(true, "");
    rig.comp.external_set(B, "make", text("BOE"));
    rig.comp.external_set(B, "model", text("Panel"));
    rig.comp.external_set(B, "identity", text("BOE:0a1b:SN42"));
    settle(&mut rig);
    exec(
        &mut rig,
        "g.panel = new OutputProfile({ match: { make: 'BOE', model: 'Panel', serial: 'SN42' }, scale: 1.5, vrr: true });\n\
         gnoblin.outputs.profiles.add(g.panel);",
    );
    let diagnostics = settle(&mut rig);
    assert_eq!(requests(&rig, B), vec!["scale=1.5"]);
    assert!(
        requests(&rig, A).is_empty(),
        "a profile for one monitor leaves the other alone"
    );
    let notices: Vec<&String> = diagnostics.iter().filter(|line| line.contains("vrr")).collect();
    assert_eq!(notices.len(), 1, "{diagnostics:?}");
    assert!(
        notices[0].starts_with("not applied: OutputProfile \"BOE/Panel/SN42\" vrr"),
        "{notices:?}"
    );
}

#[test]
fn the_profile_collection_and_profile_arguments_are_checked() {
    let mut rig = world(true, "");
    let problem = |rig: &mut Rig, source: &str| {
        rig.eval(&format!(
            "try {{ {source}; 'ok' }} catch (e) {{ (e.code ?? e.constructor.name) + ': ' + e.message }}"
        ))
    };
    assert_eq!(rig.eval("String(gnoblin.outputs.profiles.length)"), "0");
    exec(&mut rig, "gnoblin.outputs.profiles.add(g.a);");
    assert_eq!(
        rig.eval("gnoblin.outputs.profiles.get('DP-1') === g.a ? 'same' : 'other'"),
        "same"
    );
    assert!(
        problem(
            &mut rig,
            "gnoblin.outputs.profiles.add(new OutputProfile({ match: { connector: 'DP-1' } }))"
        )
        .starts_with("duplicate: gnoblin.outputs.profiles.add: there is already a profile \"DP-1\"")
    );
    assert_eq!(rig.eval("String(gnoblin.outputs.profiles.remove(g.a))"), "true");
    assert_eq!(rig.eval("String(gnoblin.outputs.profiles.length)"), "0");
    assert!(problem(&mut rig, "new OutputProfile({ match: {} })").contains("OutputProfile.match: needs at least one"));
    assert!(problem(&mut rig, "new OutputProfile({ match: { port: 'DP-1' } })").contains("not a match key"));
    assert!(
        problem(
            &mut rig,
            "new OutputProfile({ match: { connector: 'DP-1' }, scale: 9 })"
        )
        .contains("OutputProfile \"DP-1\".scale must be a number from 0.5 to 4")
    );
    assert!(
        problem(
            &mut rig,
            "new OutputProfile({ match: { connector: 'DP-1' }, mode: { width: 1, height: 1 } })"
        )
        .contains("OutputProfile \"DP-1\".mode.refreshRate")
    );
    assert!(
        problem(
            &mut rig,
            "new OutputProfile({ match: { connector: 'DP-1' }, position: { rightOf: g.b, align: 'left' } })"
        )
        .contains("along rightOf it must be one of top, center, bottom")
    );
    assert!(
        problem(
            &mut rig,
            "g.a.position = { rightOf: g.b }; g.b.position = { leftOf: g.a }; g.b.position = { rightOf: g.a }"
        )
        .contains("placed relative to itself through"),
        "a placement that depends on itself is refused"
    );
}

// ---- Cameras -------------------------------------------------------------------------------------------------------

#[test]
fn a_camera3d_on_a_viewport_is_kept_and_reported_once_as_not_applied_and_a_flat_camera_is_read_back() {
    let mut rig = world(true, "");
    exec(
        &mut rig,
        "g.d = gnoblin.displays.get('display-DP-1');\n\
         g.flat = g.d.viewport.camera;\n\
         g.cam = new Camera3D({ projection: 'perspective', fov: 65, position: { x: 0, y: 0, z: 1600 } });\n\
         g.d.viewport.camera = g.cam;",
    );
    let diagnostics = settle(&mut rig);
    let notices: Vec<&String> = diagnostics.iter().filter(|line| line.contains("camera3d")).collect();
    assert_eq!(notices.len(), 1, "{diagnostics:?}");
    assert_eq!(notices[0], "not applied: camera3d (needs a projected workspace)");
    assert_eq!(
        rig.eval("[g.d.viewport.camera === g.cam, g.cam.fov, g.cam.z, g.cam.projection].join(' ')"),
        "true 65 1600 perspective",
        "the value stays readable"
    );
    exec(&mut rig, "g.d.viewport.camera = g.flat; g.d.viewport.camera = g.cam;");
    let again = settle(&mut rig);
    assert!(
        again.iter().all(|line| !line.contains("camera3d")),
        "once per reload: {again:?}"
    );
    // A flat camera: the identity is applied by construction and says nothing; the rest is kept and reported.
    assert_eq!(
        rig.eval("[g.flat.zoom, g.flat.position.x, g.flat.position.y, g.flat.rotation].join(' ')"),
        "1 0 0 0"
    );
    exec(
        &mut rig,
        "g.d.viewport.camera = new Camera2D({ position: { x: 10, y: 20 }, zoom: 2, rotation: 0.5 });",
    );
    let flat = settle(&mut rig);
    assert_eq!(
        flat.iter()
            .filter(|line| line.starts_with("not applied: camera2d"))
            .count(),
        1,
        "{flat:?}"
    );
    assert_eq!(
        rig.eval("(() => { const c = g.d.viewport.camera; return [c.zoom, c.position.x, c.position.y, c.rotation].join(' '); })()"),
        "2 10 20 0.5"
    );
    assert_eq!(
        rig.eval("try { new Camera2D({ zoom: 0 }); 'made' } catch (e) { e.constructor.name }"),
        "RangeError"
    );
}

#[test]
fn a_transition_may_assign_a_display_its_workspace_and_camera_and_a_failed_one_puts_them_back() {
    let mut rig = world(true, "");
    exec(
        &mut rig,
        "g.d = gnoblin.displays.get('display-DP-1');\n\
         g.cam = new Camera3D({ projection: 'perspective', fov: 50 });\n\
         g.t = gnoblin.requestTransition(() => { g.d.viewport.content = g.ws.three; g.d.viewport.camera = g.cam; }, { duration: 300 });",
    );
    settle(&mut rig);
    assert_eq!(shown(&mut rig), "output-0=three output-1=two");
    assert_eq!(rig.eval("g.d.viewport.camera === g.cam ? 'cam' : 'other'"), "cam");
    let before = rig.eval("g.d.viewport.camera.fov + ''");
    assert_eq!(before, "50");
    let failed = rig.eval(
        "try { gnoblin.requestTransition(() => { g.d.viewport.content = g.ws.one; g.d.viewport.camera = g.flat ?? new Camera2D(); throw new Error('no'); }); 'ran' } catch (e) { e.message }",
    );
    assert_eq!(failed, "no");
    settle(&mut rig);
    assert_eq!(
        shown(&mut rig),
        "output-0=three output-1=two",
        "the failed transition put the desks back"
    );
    assert_eq!(
        rig.eval("g.d.viewport.camera === g.cam ? 'cam' : 'other'"),
        "cam",
        "and the camera"
    );
}
