//! A shell that imports only `gnoblin-core`: the proof that the two packages are split in fact and not only in the file
//! tree.
//!
//! The shell below observes windows (their `Client` objects, the live `Collection` of them, the typed listeners), places
//! them itself by writing their geometry (its own tiling, with no workspace, layout or scene), binds a key through the
//! input claims and starts a program. The import log of the loader says which embedded modules the generation loaded:
//! none of them may be a module of `gnoblin-std`. A control shell that imports `gnoblin-std` loads std modules and has the
//! members std adds, so the log is shown to see what it is meant to.

use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

/// The entry of the core-only shell: everything it imports is `gnoblin-core`.
const SHELL: &str = r#"
import { gnoblin, ClientListener, ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

const log: string[] = [];
(globalThis as any).log = log;

// It sees the windows: titles, focus, geometry and the end of a window are told to a typed listener.
class Watch extends ClientListener {
    onTitleChanged(client: Client, title: string): void {
        log.push(`title ${client.id} ${title}`);
    }
    onClosed(client: Client): void {
        log.push(`closed ${client.id}`);
    }
}
gnoblin.clients.addListener(new Watch());

// It places them itself: the windows side by side across the first output, each as tall as the output.
function tile(): void {
    const output = gnoblin.outputs.at(0);
    if (output === undefined) {
        return;
    }
    const windows = gnoblin.clients.toList();
    const width = output.width / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x + at * width, y: output.y, width, height: output.height };
    });
}
class Tiler extends ClientCollectionListener {
    onAdded(client: Client): void {
        log.push(`added ${client.id}`);
        tile();
    }
    onRemoved(client: Client): void {
        log.push(`removed ${client.id}`);
        tile();
    }
}
gnoblin.clients.addListener(new Tiler());

// It binds a key through the seat's claims and starts a program from it.
gnoblin.input.primary.bind("Super+Return", () => {
    (globalThis as any).job = gnoblin.spawn(["sh", "-c", "exit 0"]);
    log.push("spawned");
});
"#;

/// A 1000 by 600 output, a seat and two windows that were there at boot.
fn world(comp: &FakeCompositor) {
    comp.add_screen(20, "DP-1", (0.0, 0.0, 1000.0, 600.0), true);
    comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_window(3, "one", "app.one");
    comp.add_window(4, "two", "app.two");
}

fn number(comp: &FakeCompositor, node: u64, key: &str) -> f64 {
    match comp.value(node, key) {
        Some(MirrorValue::Double(value)) => value,
        other => panic!("{key} of {node} is not a number: {other:?}"),
    }
}

/// Runs frames until `check` holds; a state that never came fails the test.
#[track_caller]
fn until(rig: &mut Rig, what: &str, mut check: impl FnMut(&mut Rig) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        rig.frame();
        if check(rig) {
            return;
        }
        assert!(Instant::now() < deadline, "never reached: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_shell_that_imports_only_gnoblin_core_observes_places_binds_and_spawns_and_loads_no_std_module() {
    let mut rig = rig_setup(SHELL, "", world);
    let told = rig.settle();
    assert!(told.is_empty(), "{told:?} {:?}", rig.load_diagnostics);

    // It saw the two windows that were there (a collection listener hears each member on attach) and tiled them: two
    // columns of 500, written to the windows through core alone.
    assert_eq!(rig.eval("log.join('|')"), "added 3|added 4");
    assert_eq!(rig.eval("gnoblin.clients.length"), "2");
    for (node, x) in [(3, 0.0), (4, 500.0)] {
        assert_eq!(number(&rig.comp, node, "x"), x, "window {node}");
        assert_eq!(number(&rig.comp, node, "y"), 0.0);
        assert_eq!(number(&rig.comp, node, "width"), 500.0);
        assert_eq!(number(&rig.comp, node, "height"), 600.0);
    }

    // A window that appears is announced and tiled again: three columns.
    rig.comp.create_window(5, "three", "app.three");
    rig.settle();
    assert_eq!(rig.eval("log.slice(2).join('|')"), "added 5");
    let third = 1000.0 / 3.0;
    for (node, at) in [(3, 0.0), (4, 1.0), (5, 2.0)] {
        assert!(
            (number(&rig.comp, node, "x") - at * third).abs() < 1e-6,
            "window {node}"
        );
        assert!((number(&rig.comp, node, "width") - third).abs() < 1e-6, "window {node}");
    }

    // A title change reaches the typed listener, and a window that goes is removed from the collection, told to the
    // listeners, and the rest are tiled again.
    rig.comp.external_set(3, "title", text("renamed"));
    rig.settle();
    rig.comp.destroy(4);
    rig.settle();
    assert_eq!(
        rig.eval("log.slice(3).join('|')"),
        "title 3 renamed|closed 4|removed 4",
        "the listeners of the window and of the collection hear it"
    );
    assert_eq!(rig.eval("gnoblin.clients.length"), "2");
    assert!((number(&rig.comp, 3, "width") - 500.0).abs() < 1e-6);
    assert!((number(&rig.comp, 5, "x") - 500.0).abs() < 1e-6);

    // The key is claimed on the seat, and the chord starts the program.
    let claimed = rig.comp.claimed();
    assert!(
        claimed.iter().any(|text| text.contains("<Super>Return")),
        "the shell's key is claimed: {claimed:?}"
    );
    rig.comp
        .chord(rig.comp.claim_id("<Super>Return"), "<Super>Return", "press");
    until(&mut rig, "the key ran its action", |rig| {
        rig.eval("log.includes('spawned')") == "true"
    });
    until(&mut rig, "the program ended", |rig| rig.eval("job.alive") == "false");
    assert_eq!(rig.eval("job.exitCode"), "0");

    // What the packages are made of: std adds members to the core classes, and none of them is there.
    assert_eq!(
        rig.eval(
            "[typeof gnoblin.workspaces, typeof gnoblin.displays, typeof gnoblin.scene, typeof gnoblin.requestTransition, \
             typeof gnoblin.outputs.profiles, 'workspace' in gnoblin.clients.at(0), 'style' in gnoblin.clients.at(0), \
             'floating' in gnoblin.clients.at(0), 'area' in gnoblin.outputs.at(0), 'style' in gnoblin.clients].join()"
        ),
        "undefined,undefined,undefined,undefined,undefined,false,false,false,false,false"
    );

    // The import log: the core library was loaded, and no module of `gnoblin-std` was.
    let loaded = rig.shell.embedded_modules();
    assert!(
        loaded.iter().any(|name| name == "gnoblin-core/generated/objects"),
        "the import log lists what was loaded: {loaded:?}"
    );
    let std: Vec<_> = loaded.iter().filter(|name| name.starts_with("gnoblin-std")).collect();
    assert!(std.is_empty(), "a core-only shell loaded std modules: {std:?}");
}

#[test]
fn a_shell_that_imports_gnoblin_std_loads_its_modules_and_has_the_members_it_adds() {
    let mut rig = rig_setup(
        "import { gnoblin } from \"gnoblin-core\"; import { FloatingLayout, Workspace } from \"gnoblin-std\";\n\
         gnoblin.workspaces.add(new Workspace(\"main\", { layout: new FloatingLayout() }));\n",
        "",
        world,
    );
    let told = rig.settle();
    assert!(told.is_empty(), "{told:?} {:?}", rig.load_diagnostics);
    assert_eq!(rig.eval("gnoblin.clients.at(0).workspace.id"), "main");
    assert_eq!(
        rig.eval("[typeof gnoblin.workspaces, typeof gnoblin.requestTransition, 'style' in gnoblin.clients].join()"),
        "object,function,true"
    );
    let loaded = rig.shell.embedded_modules();
    assert!(
        loaded.iter().any(|name| name == "gnoblin-std/generated/objects"),
        "the import log shows the std modules that were loaded: {loaded:?}"
    );
}

#[test]
fn a_core_only_shell_controls_a_windows_frame_callbacks_and_the_seats_built_in_motion() {
    let mut rig = rig_setup(
        "import { gnoblin } from \"gnoblin-core\";\n\
         const client = gnoblin.clients.at(0)!;\n\
         client.framePolicy = \"throttle\";\n\
         client.frameRate = 10;\n\
         client.reportPresented = true;\n\
         (globalThis as any).before = gnoblin.input.primary.builtinMotion;\n\
         gnoblin.input.primary.builtinMotion = false;\n",
        "",
        world,
    );
    let told = rig.settle();
    assert!(told.is_empty(), "{told:?} {:?}", rig.load_diagnostics);
    assert_eq!(rig.comp.value(3, "frame_policy"), Some(text("throttle")));
    assert_eq!(rig.comp.value(3, "frame_rate"), Some(MirrorValue::Double(10.0)));
    assert_eq!(rig.comp.value(3, "report_presented"), Some(MirrorValue::Bool(true)));
    assert_eq!(rig.comp.value(21, "builtin_motion"), Some(MirrorValue::Bool(false)));
    assert_eq!(
        rig.eval("String(before)"),
        "true",
        "the seat plays the compositor's own motion until told"
    );

    // Read back as the compositor publishes them: what it says it presented, and the policy it keeps.
    assert_eq!(rig.eval("gnoblin.clients.at(0).presented"), "0");
    rig.comp.external_set(3, "presented", MirrorValue::Uint(1234));
    rig.settle();
    assert_eq!(rig.eval("gnoblin.clients.at(0).presented"), "1234");
    assert_eq!(rig.eval("gnoblin.clients.at(0).framePolicy"), "throttle");
    assert_eq!(
        rig.eval("gnoblin.clients.at(1).framePolicy"),
        "auto",
        "the schema default"
    );
    assert_eq!(rig.eval("gnoblin.clients.at(1).frameRate"), "30");
    assert_eq!(rig.eval("gnoblin.input.primary.builtinMotion"), "false");
}
