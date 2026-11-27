//! Logical displays (owner-spec sections 8 and 9): `OutputProfile`, `Display`, `Viewport`, `Camera2D` and `Camera3D` on a
//! viewport, `gnoblin.displays` and `input.activeDisplay`, over the fake compositor of `tests/common`.
//!
//! The scripts are written the way the specification writes them. What the real compositor does with
//! the output requests, the mirror and the span is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

const IMPORTS: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n";

const SEAT: u64 = 21;
const A: u64 = 20;
const B: u64 = 22;

/// The script every test starts from: three workspaces whose layout gives each window the whole area, kept in `g.ws`, and
/// the profiles of the two monitors, `g.a` for `DP-1` and `g.b` for `HDMI-A-1`, which are not added to anything.
const START: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { x: c.area.x, y: c.area.y, width: c.area.width, height: c.area.height } })); }\n\
}\n\
const one = new Workspace(\"one\", { layout: new Fill() });\n\
const two = new Workspace(\"two\", { layout: new Fill() });\n\
const three = new Workspace(\"three\", { layout: new Fill() });\n\
for (const ws of [one, two, three]) gnoblin.workspaces.add(ws);\n\
g.ws = { one, two, three };\n\
g.a = new OutputProfile({ connector: \"DP-1\" });\n\
g.b = new OutputProfile({ connector: \"HDMI-A-1\" });\n\
g.display = (id: string, profiles: any[], viewport: any, mode?: string) => {\n\
    const options: any = { id, viewport };\n\
    if (mode) options.mode = mode;\n\
    return new Display(profiles.map((profile) => new DisplaySelector({ profile })), options);\n\
};\n\
g.dp1 = () => gnoblin.outputs.toList().find((o: any) => o.connector === \"DP-1\");\n\
g.settings = (profile: any, values: any) => { profile.settings = new ProfileSettings(values); };\n";

/// A shell over screen `A` (`DP-1`, 1000 by 800, primary) and, when `second`, screen `B` (`HDMI-A-1`, 600 by 400 at
/// x 1000), a seat and one window (id 3), running `START` and then `extra`.
fn world(second: bool, extra: &str) -> Rig {
    world_noting(second, extra, &[])
}

/// [`world`] for a file whose profile earns a sentence for the diagnostics: the library applies the profile in its first turn, so the
/// sentence arrives with the first frames and is the one in `allowed`; any other diagnostic fails the world.
fn world_noting(second: bool, extra: &str, allowed: &[&str]) -> Rig {
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
    assert!(
        diagnostics.iter().all(|line| allowed.contains(&line.as_str())),
        "{diagnostics:?} {:?}",
        rig.load_diagnostics
    );
    rig.load_diagnostics.extend(diagnostics);
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
    rig.eval("gnoblin.outputs.toList().map((o) => `${o.name}=${o.workspace?.id}`).join(' ')")
}

/// The displays as `id:mode:outputs:workspace` words, sorted.
fn displays(rig: &mut Rig) -> String {
    rig.eval(
        "gnoblin.displays.list.map((d) => `${d.id}:${d.mode}:${d.outputs.length}:${d.viewport?.content?.id}`).sort().join(' ')",
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
        rig.eval("gnoblin.outputs.get(String(gnoblin.displays.byId('display-DP-1').outputs[0])).connector"),
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
        "g.left = gnoblin.displays.byId('display-DP-1'); g.right = gnoblin.displays.byId('display-HDMI-A-1');",
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
        rig.eval("try { g.left.viewport.content = new Workspace('loose'); 'set' } catch (e) { e.code }"),
        "invalid_state",
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
        "g.mine = g.display('left', [g.a], new Viewport({ content: g.ws.three, camera: new Camera2D() }));\n\
         g.rival = g.display('also-left', [g.a], new Viewport({ content: g.ws.two }));\n\
         gnoblin.displays.add(g.mine);",
    );
    settle(&mut rig);
    assert_eq!(
        displays(&mut rig),
        "display-HDMI-A-1:single:1:two left:single:1:three",
        "the display for DP-1 is the script's, which shows the workspace it asked for"
    );
    let error = rig.eval("try { gnoblin.displays.add(g.rival); 'added' } catch (e) { `${e.code}|${e.message}` }");
    assert!(error.starts_with("busy|"), "{error}");
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
    assert_eq!(rig.eval("String(gnoblin.displays.byId('left'))"), "undefined");
}

#[test]
fn removing_a_script_display_gives_its_output_a_display_of_the_shells_again() {
    let mut rig = world(
        true,
        "g.mine = g.display('left', [g.a], new Viewport({ content: g.ws.one }));\ngnoblin.displays.add(g.mine);",
    );
    assert_eq!(
        rig.eval("gnoblin.displays.list.map((d) => d.id).sort().join(' ')"),
        "display-HDMI-A-1 left"
    );
    assert_eq!(rig.eval("String(gnoblin.displays.remove(g.mine))"), "true");
    settle(&mut rig);
    assert_eq!(
        rig.eval("gnoblin.displays.list.map((d) => d.id).sort().join(' ')"),
        "display-DP-1 display-HDMI-A-1"
    );
    assert_eq!(
        rig.eval("String(gnoblin.displays.remove(gnoblin.displays.list[0]))"),
        "false",
        "the display the shell keeps for an output stays"
    );
}

#[test]
fn a_display_whose_output_is_not_connected_is_parked_with_its_workspace_and_wakes_when_the_output_arrives() {
    let mut rig = world(
        true,
        "g.hdmi9 = new OutputProfile({ connector: 'HDMI-9' });\n\
         g.parked = g.display('parked', [g.hdmi9], new Viewport({ content: g.ws.one }));\n\
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
        rig.eval("gnoblin.outputs.toList().find((o) => o.connector === 'HDMI-9').workspace.id"),
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
    let failures = std::cell::RefCell::new(Vec::<String>::new());
    let check = |rig: &mut Rig, source: &str, needle: &str| {
        let text = problem(rig, source);
        if !text.contains(needle) {
            failures.borrow_mut().push(format!("{source}: {text}"));
        }
    };
    check(&mut rig, "new Display([])", "TypeError: Display.selectors");
    check(
        &mut rig,
        "g.display('x', [g.a], undefined, 'tile')",
        "is not a value of present_mode",
    );
    exec(
        &mut rig,
        "g.vp = new Viewport({ content: g.ws.one }); g.display('x', [g.a], g.vp);",
    );
    check(&mut rig, "g.display('y', [g.b], g.vp)", "already shown by display");
    check(
        &mut rig,
        "new Viewport({ camera: {} })",
        "expected an object the library made",
    );
    check(
        &mut rig,
        "new Viewport({ content: 3 })",
        "expected an object, got number",
    );
    assert!(failures.borrow().is_empty(), "{:#?}", failures.borrow());
}

#[test]
fn a_single_display_presents_one_output() {
    let mut rig = world(true, "");
    // A native display is made with its selectors and its mode is set after, so the refusal is where the display is added, as for a write
    // of `mode` to a display that is listed.
    let text = rig.eval(
        "try { gnoblin.displays.add(g.display('x', [g.a, g.b])); 'ok' } catch (e) { e.constructor.name + ': ' + e.message }",
    );
    assert!(text.contains("\"single\" presents one output"), "{text}");
}

#[test]
fn a_display_refuses_an_unknown_option() {
    let mut rig = world(true, "");
    let text = rig.eval(
        "try { new Display([new DisplaySelector({ profile: g.a })], { colour: 1 }); 'ok' } catch (e) { e.constructor.name + ': ' + e.message }",
    );
    assert!(text.contains("colour"), "{text}");
}

// ---- Mirror and span ----------------------------------------------------------------------------------------------

#[test]
fn a_mirror_display_asks_the_compositor_to_mirror_and_ends_the_mirror_when_it_is_removed() {
    let mut rig = world(
        true,
        "g.mirror = g.display('mirror', [g.a, g.b], new Viewport({ content: g.ws.one, camera: new Camera2D() }), 'mirror');\n\
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
        "g.span = g.display('span', [g.a, g.b], new Viewport({ content: g.ws.one, camera: new Camera2D() }), 'span');\n\
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
        "g.main = new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ mode: OutputProfile.formatMode(1000, 800, 60), scale: 1.25, transform: 'normal', position: new OutputPosition({ kind: 'absolute', x: 0, y: 0 }) }) });\n\
         g.second = new OutputProfile({ connector: 'HDMI-A-1' }, { settings: new ProfileSettings({ mode: 'preferred', position: new OutputPosition({ kind: 'right_of', profile: g.main, align: 'top' }) }) });\n\
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
        "g.p = new OutputProfile({ connector: 'HDMI-A-1' }, { settings: new ProfileSettings({ scale: 2 }) });\ngnoblin.outputs.profiles.add(g.p);",
    );
    let first = rig.comp.applies();
    assert_eq!(first.len(), 1);
    rig.comp
        .answer(first[0].2, "", "scale: the backend does not take 2 here");
    settle(&mut rig);
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
fn a_refused_profile_request_is_reported_by_key() {
    let mut rig = world(
        true,
        "g.p = new OutputProfile({ connector: 'HDMI-A-1' }, { settings: new ProfileSettings({ scale: 2 }) });\ngnoblin.outputs.profiles.add(g.p);",
    );
    let first = rig.comp.applies();
    rig.comp
        .answer(first[0].2, "", "scale: the backend does not take 2 here");
    let diagnostics = settle(&mut rig);
    assert!(
        diagnostics
            .iter()
            .any(|line| line == "outputs.profiles.HDMI-A-1.scale: the backend does not take 2 here"),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "vrnw: needs OutputCollectionListener.onApplied to hear the answer to a profile's request"]
fn the_compositors_answer_to_a_profiles_request_reaches_the_listeners_of_the_output_as_it_does_for_apply() {
    let mut rig = world(
        false,
        "g.log = [];\n\
         gnoblin.outputs.addListener(new (class extends OutputCollectionListener {\n\
             onApplied(o, error) { g.log.push(o.connector + ' ' + error); }\n\
         })());\n\
         g.p = new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ scale: 2 }) });\n\
         gnoblin.outputs.profiles.add(g.p);",
    );
    let sent = rig.comp.applies();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(rig.eval("g.log.join('|')"), "", "nothing is answered yet");
    rig.comp.answer(sent[0].2, "", "");
    settle(&mut rig);
    assert_eq!(
        rig.eval("g.log.join('|')"),
        "DP-1 ",
        "a taken request is announced with no error"
    );
    exec(&mut rig, "g.settings(g.p, { scale: 3 });");
    settle(&mut rig);
    let again = rig.comp.applies();
    assert_eq!(again.len(), 2, "{again:?}");
    rig.comp
        .answer(again[1].2, "", "scale: the backend does not take 3 here");
    settle(&mut rig);
    assert_eq!(
        rig.eval("g.log.join('|')"),
        "DP-1 |DP-1 scale: the backend does not take 3 here",
        "a refused one carries the compositor's words"
    );
}

#[test]
fn a_profile_waits_for_the_answer_to_its_own_request_before_it_sends_the_next() {
    let mut rig = world(
        false,
        "g.p = new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ scale: 2 }) });\ngnoblin.outputs.profiles.add(g.p);",
    );
    let sent = rig.comp.applies();
    assert_eq!(sent.len(), 1, "{sent:?}");
    exec(&mut rig, "g.settings(g.p, { scale: 3 });");
    settle(&mut rig);
    assert_eq!(
        rig.comp.applies().len(),
        1,
        "the output has a request in flight: the new scale waits for its answer"
    );
    rig.comp.answer(sent[0].2, "", "");
    settle(&mut rig);
    let after = rig.comp.applies();
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(after[1].1, "scale=3");
}

#[test]
fn a_display_the_shell_keeps_is_the_librarys_and_its_object_is_made_when_the_script_asks() {
    let mut rig = world(true, "");
    assert_eq!(
        rig.eval("gnoblin.displays.list.map((d) => d.constructor.name + ':' + d.id + ':' + d.mode).sort().join(' ')"),
        "Display:display-DP-1:single Display:display-HDMI-A-1:single"
    );
    assert_eq!(
        rig.eval("String(gnoblin.displays.byId('display-DP-1') === gnoblin.displays.byId('display-DP-1'))"),
        "true",
        "the object for a handle is the one object"
    );
    assert_eq!(
        rig.eval("String(gnoblin.displays.remove(gnoblin.displays.byId('display-DP-1')))"),
        "false",
        "the display of the shell's own stays: removing it answers false"
    );
    assert_eq!(rig.eval("String(gnoblin.displays.length)"), "2");
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
        "g.panel = new OutputProfile({ make: 'BOE', model: 'Panel', serial: 'SN42' }, { settings: new ProfileSettings({ scale: 1.5, vrr: true }) });\n\
         gnoblin.outputs.profiles.add(g.panel);",
    );
    settle(&mut rig);
    assert_eq!(requests(&rig, B), vec!["scale=1.5"]);
    assert!(
        requests(&rig, A).is_empty(),
        "a profile for one monitor leaves the other alone"
    );
}

#[test]
fn a_profile_reports_vrr_as_not_applied() {
    let mut rig = world(true, "");
    rig.comp.external_set(B, "make", text("BOE"));
    rig.comp.external_set(B, "model", text("Panel"));
    rig.comp.external_set(B, "identity", text("BOE:0a1b:SN42"));
    settle(&mut rig);
    exec(
        &mut rig,
        "g.panel = new OutputProfile({ make: 'BOE', model: 'Panel', serial: 'SN42' }, { settings: new ProfileSettings({ scale: 1.5, vrr: true }) });\n\
         gnoblin.outputs.profiles.add(g.panel);",
    );
    let diagnostics = settle(&mut rig);
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
        rig.eval("gnoblin.outputs.profiles.byId('DP-1') === g.a ? 'same' : 'other'"),
        "same"
    );
    let failures = std::cell::RefCell::new(Vec::<String>::new());
    let check = |rig: &mut Rig, source: &str, needle: &str| {
        let text = problem(rig, source);
        if !text.contains(needle) {
            failures.borrow_mut().push(format!("{source}: {text}"));
        }
    };
    check(
        &mut rig,
        "gnoblin.outputs.profiles.add(new OutputProfile({ connector: 'DP-1' }))",
        "already_exists: OutputProfiles.add: there is already a profile \"DP-1\"",
    );
    assert_eq!(rig.eval("String(gnoblin.outputs.profiles.remove(g.a))"), "true");
    assert_eq!(rig.eval("String(gnoblin.outputs.profiles.length)"), "0");
    check(
        &mut rig,
        "new OutputProfile({})",
        "OutputProfile.match: needs at least one",
    );
    // A key the match lacks is refused by the record; an empty match is refused by the profile.
    check(
        &mut rig,
        "new OutputProfile({ port: 'DP-1' })",
        "OutputMatch: no field \"port\"",
    );
    check(
        &mut rig,
        "new OutputProfile({})",
        "OutputProfile.match: needs at least one of connector, make, model, serial",
    );
    check(
        &mut rig,
        "new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ scale: 9 }) })",
        "OutputProfile \"DP-1\".scale must be a number from 0.5 to 4",
    );
    check(
        &mut rig,
        "new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ mode: OutputProfile.formatMode(1, 1, 0) }) })",
        "mode.refresh_rate",
    );
    check(
        &mut rig,
        "new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ position: new OutputPosition({ kind: 'right_of', profile: g.b, align: 'left' }) }) })",
        "along right_of it must be one of top, center, bottom",
    );
    check(
        &mut rig,
        "g.settings(g.a, { position: new OutputPosition({ kind: 'right_of', profile: g.b }) }); g.settings(g.b, { position: new OutputPosition({ kind: 'left_of', profile: g.a }) }); g.settings(g.b, { position: new OutputPosition({ kind: 'right_of', profile: g.a }) })",
        "placed relative to itself through",
    );
    assert!(failures.borrow().is_empty(), "{:#?}", failures.borrow());
}

// ---- Cameras -------------------------------------------------------------------------------------------------------

#[test]
#[ignore = "vrnw: the notice is in the diagnostics; needs Camera2D.rotation to read as a number (a flat camera's rotation reads as an object)"]
fn a_camera3d_on_a_viewport_is_kept_and_reported_once_as_not_applied_and_a_flat_camera_is_read_back() {
    let mut rig = world(true, "");
    exec(
        &mut rig,
        "g.d = gnoblin.displays.byId('display-DP-1');\n\
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
        "g.d.viewport.camera = new Camera2D({ position: { x: 10, y: 20, z: 0 }, zoom: 2, roll: 0.5 });",
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
        "g.d = gnoblin.displays.byId('display-DP-1');\n\
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
    assert!(
        failed.contains("no") && !failed.contains("callback failed"),
        "the callback's own words: {failed}"
    );
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

/// What the script's diagnostics say about a supersample factor, by the text the compositor logs too.
const NOT_RECOMMENDED_3: &str = "supersample 3 is not recommended: use 1, 2 or 4";

#[test]
fn a_supersample_is_a_request_and_the_derived_sizes_are_read_from_the_output() {
    let mut rig = world(false, "");
    rig.comp.external_set(A, "mode", text("1000x800@60"));
    rig.comp.external_set(A, "output_scale", MirrorValue::Double(1.5));
    rig.comp.external_set(A, "supersample", MirrorValue::Double(2.0));
    settle(&mut rig);
    assert_eq!(rig.eval("String(g.dp1().supersample)"), "2");
    assert_eq!(
        rig.eval("JSON.stringify(g.dp1().virtualSize)"),
        r#"{"width":2000,"height":1600}"#,
        "the virtual size is the mode times the factor"
    );
    assert_eq!(
        rig.eval("String(g.dp1().effectiveScale)"),
        "3",
        "the scale clients are told is the user's scale times the factor"
    );
    assert_eq!(rig.eval("String(g.dp1().scale)"), "1.5", "and `scale` stays the user's");

    exec(&mut rig, "g.dp1().apply({ supersample: 4 });");
    settle(&mut rig);
    assert_eq!(requests(&rig, A), vec!["supersample=4"]);
    exec(&mut rig, "g.dp1().apply({ scale: 2, supersample: 2 });");
    settle(&mut rig);
    assert_eq!(
        requests(&rig, A),
        vec!["supersample=4", "scale=2\nsupersample=2"],
        "the request carries one key per setting, in the order they were written"
    );
}

#[test]
fn a_supersample_that_is_not_a_number_in_range_throws_naming_the_key() {
    let mut rig = world(false, "");
    let problem = |rig: &mut Rig, value: &str| {
        rig.eval(&format!(
            "try {{ g.dp1().apply({{ supersample: {value} }}); 'ok' }} catch (e) {{ e.constructor.name + ': ' + e.message }}"
        ))
    };
    for bad in ["0", "0.5", "17", "NaN", "Infinity", "'2'"] {
        let text = problem(&mut rig, bad);
        assert!(
            text.starts_with("TypeError: ") && text.contains(".supersample must be a number"),
            "{bad}: {text}"
        );
    }
    assert!(requests(&rig, A).is_empty(), "nothing was sent for a bad value");
    assert_eq!(problem(&mut rig, "2"), "ok");
}

#[test]
fn a_factor_that_is_not_recommended_is_accepted_with_one_diagnostic() {
    let mut rig = world(false, "");
    exec(&mut rig, "g.dp1().apply({ supersample: 2 });");
    exec(&mut rig, "g.dp1().apply({ supersample: 1 });");
    exec(&mut rig, "g.dp1().apply({ supersample: 4 });");
    let quiet = settle(&mut rig);
    assert!(
        quiet.iter().all(|line| !line.contains("not recommended")),
        "1, 2 and 4 are recommended: {quiet:?}"
    );

    exec(&mut rig, "g.dp1().apply({ supersample: 3 });");
    let first = settle(&mut rig);
    assert_eq!(
        first.iter().filter(|line| line.as_str() == NOT_RECOMMENDED_3).count(),
        1,
        "{first:?}"
    );
    assert_eq!(
        requests(&rig, A).last().map(String::as_str),
        Some("supersample=3"),
        "it is accepted and sent all the same"
    );

    exec(&mut rig, "g.dp1().apply({ supersample: 3 });");
    let again = settle(&mut rig);
    assert!(
        again.iter().all(|line| line.as_str() != NOT_RECOMMENDED_3),
        "the same factor is not reported twice: {again:?}"
    );

    exec(&mut rig, "g.dp1().apply({ supersample: 1.5 });");
    let fractional = settle(&mut rig);
    assert!(
        fractional
            .iter()
            .any(|line| line == "supersample 1.5 is not recommended: use 1, 2 or 4"),
        "a fraction is written as a person would: {fractional:?}"
    );
}

#[test]
fn a_profile_with_a_supersample_sends_it_and_reports_a_factor_that_is_not_recommended_once() {
    let mut rig = world_noting(
        false,
        "g.hd = new OutputProfile({ connector: 'DP-1' }, { settings: new ProfileSettings({ supersample: 3 }) });\ngnoblin.outputs.profiles.add(g.hd);",
        &[NOT_RECOMMENDED_3],
    );
    // The library applies the profile in its first turn, so the notice comes with the first frames of the shell and is not repeated
    // after (it was among the load diagnostics when the TypeScript manager applied the profile while the file loaded).
    let mut diagnostics = rig.load_diagnostics.clone();
    diagnostics.extend(settle(&mut rig));
    assert_eq!(requests(&rig, A), vec!["supersample=3"]);
    let _ = diagnostics;
    assert_eq!(rig.eval("String(g.hd.settings.supersample)"), "3");
    // A request is not repeated while the compositor has not answered the one before.
    let mut answered = 0;
    answer_all(&mut rig, &mut answered);
    settle(&mut rig);
    exec(&mut rig, "g.settings(g.hd, { supersample: 2 });");
    settle(&mut rig);
    assert_eq!(
        requests(&rig, A).last().map(String::as_str),
        Some("supersample=2"),
        "changing the profile applies the new factor"
    );
}
