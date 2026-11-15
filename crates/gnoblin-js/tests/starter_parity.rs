//! The stock default is one world however it is reached.
//!
//! Three ways to boot over the same fake compositor: no `init.ts` at all (`load_default_config`, which runs
//! `installDefaults`), the `init.ts` `gnoblinctl config init` writes, and a file that calls `installDefaults`. Each is
//! digested into the workspaces, the claimed keys and gestures, the titlebars, the window menu, the not-responding alert
//! and what each stock key does when it is pressed, and the three digests must be equal. A last test holds the
//! statements of `installDefaults` and of the starter file together, so the readable file cannot drift from what runs.

use std::fs;
use std::path::Path;

use gnoblin_js::{JsShell, STARTER_CONFIG};
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

/// Puts the library's names where `rig.eval` can reach them, as `stock_ui.rs` does.
const GLOBALS: &str = "import * as g from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

/// A screen of 1000 by 800, one seat, one lock and two windows (ids 3 and 4).
fn populate(comp: &FakeCompositor) {
    comp.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_node(22, ROOT, "lock", Vec::new());
    comp.add_window(3, "one", "foot");
    comp.add_window(4, "two", "foot");
}

/// A shell over the populated compositor that has loaded `init` as `init.ts`, or the stock default when `init` is
/// `None`, and then the library globals.
fn boot(init: Option<&str>) -> Rig {
    let comp = FakeCompositor::new();
    populate(&comp);
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("globals.ts"), GLOBALS).expect("globals.ts is written");
    let (config, clock) = timed_config(&root);
    let mut shell = JsShell::new(Box::new(comp.clone()), config).expect("the shell boots");
    let load_diagnostics = match init {
        Some(source) => {
            fs::write(root.join("init.ts"), source).expect("init.ts is written");
            shell.load_entry(Path::new("init.ts")).expect("init.ts loads")
        }
        None => shell.load_default_config().expect("the stock default loads"),
    };
    shell.load_entry(Path::new("globals.ts")).expect("globals load");
    Rig {
        shell,
        comp,
        load_diagnostics,
        clock,
        _dir: dir,
    }
}

/// The chords whose press the digest records, as the compositor writes them.
const PRESSES: [&str; 9] = [
    "<Super>1",
    "<Super>2",
    "<Super>9",
    "<Super>Left",
    "<Super>Right",
    "XF86AudioMute",
    "XF86AudioPrev",
    "XF86AudioMicMute",
    "<Super>space",
];

/// Everything observable about the world a boot made.
fn digest(rig: &mut Rig) -> serde_json::Value {
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    let mut claims = rig.comp.claimed();
    claims.sort();
    // A window the compositor maps after the shell is up gets its bar, which shows as an inset and a child node; the
    // new window also takes the seat's focus under the built-in focus policy.
    rig.comp.create_window(5, "three", "foot");
    rig.settle();
    let children: Vec<usize> = [3_u64, 4, 5].iter().map(|id| rig.comp.children(*id).len()).collect();
    let frames = serde_json::json!({
        "children": children,
        "inset": rig.eval("String(gnoblin.clients.at(2).frameInset.top)"),
        "focus": format!("{:?}", rig.comp.value(21, "focus")),
    });
    let world = rig.eval(
        "JSON.stringify({\
         workspaces: gnoblin.workspaces.map((w) => ({ id: w.id, name: w.name, layout: w.layout.constructor.name, \
             active: w.active, gaps: w.gaps, clients: w.clients.map((c) => c.id) })), \
         seatBindings: [...gnoblin.input.primary.bindings].map((b) => `${b.constructor.name}:${b.trigger}`).sort(), \
         menu: windowMenu.started, alert: notRespondingAlerts.started })",
    );
    // What each key does is read by putting recorders where the handlers reach: the spawn call, the strip layout's
    // column moves, and the launcher entry (a canvas on the screen).
    rig.eval(
        "globalThis.probe = { spawned: [], moved: [] }; \
         gnoblin.spawn = (argv) => { probe.spawned.push(argv.join(' ')); }; \
         for (const side of ['focusLeft', 'focusRight']) { \
             StripLayout.prototype[side] = function () { probe.moved.push(side); }; } \
         gnoblin.workspaces.map((w) => { w.layout = new StripLayout(); return 0; }); 0",
    );
    rig.settle();
    let before = rig.comp.canvases().len();
    let mut presses = Vec::new();
    for chord in PRESSES {
        let id = rig.comp.claim_id(chord);
        rig.comp.chord(id, chord, "press");
        rig.advance(1000);
        rig.settle();
        let seen = rig.eval(
            "JSON.stringify({ spawned: probe.spawned.splice(0), moved: probe.moved.splice(0), \
             shown: gnoblin.workspaces.map((w) => (w.active ? w.id : '')).join('') })",
        );
        presses.push(serde_json::json!({
            "chord": chord,
            "seen": serde_json::from_str::<serde_json::Value>(&seen).expect("JSON"),
            "canvases": rig.comp.canvases().len() - before,
        }));
    }
    serde_json::json!({
        "claims": claims,
        "frames": frames,
        "world": serde_json::from_str::<serde_json::Value>(&world).expect("JSON"),
        "presses": presses,
    })
}

#[test]
fn every_way_to_boot_the_stock_default_builds_one_world() {
    let none = digest(&mut boot(None));
    let starter = digest(&mut boot(Some(STARTER_CONFIG)));
    let call = digest(&mut boot(Some(
        "import { installDefaults } from \"gnoblin/starter\";\ninstallDefaults(gnoblin);\n",
    )));

    // The world is real: four floating workspaces, the stock keys, the swipe, titlebars, menu and alert.
    assert_eq!(
        none["world"]["workspaces"].as_array().map(Vec::len),
        Some(4),
        "{none:#}"
    );
    assert!(
        none["claims"].as_array().expect("claims").len() >= 20,
        "the stock keys and the floating layout's drags are claimed: {none:#}"
    );
    assert_eq!(none["world"]["menu"], true);
    assert_eq!(none["world"]["alert"], true);
    assert_eq!(none["frames"]["inset"], "28", "titlebars: {none:#}");
    // The presses recorded something: a workspace key shows its workspace, a strip key moves a column, a media key
    // spawns its command and the launcher key opens an entry.
    let seen = |at: usize| &none["presses"][at]["seen"];
    assert_eq!(seen(1)["shown"], "2", "{none:#}");
    assert_eq!(seen(4)["moved"], serde_json::json!(["focusRight"]), "{none:#}");
    assert_eq!(
        seen(5)["spawned"],
        serde_json::json!(["wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"]),
        "{none:#}"
    );
    assert!(
        none["presses"][8]["canvases"].as_u64().unwrap_or(0) > 0,
        "the launcher key draws an entry: {none:#}"
    );

    assert_eq!(starter, none, "the starter file is the stock default");
    assert_eq!(call, none, "installDefaults is the stock default");
}

#[test]
fn a_config_that_does_not_call_the_defaults_has_none_of_them() {
    let mut rig = boot(Some("export {};\n"));
    rig.settle();
    assert_eq!(rig.eval("gnoblin.workspaces.map((w) => w.id).join()"), "desktop");
    assert_eq!(rig.eval("String(windowMenu.started)"), "false");
    assert_eq!(rig.eval("String(notRespondingAlerts.started)"), "false");
    assert!(
        !rig.comp.claimed().iter().any(|claim| claim.contains("<Super>1")),
        "no stock keys: {:?}",
        rig.comp.claimed()
    );
}

/// The starter with every example switched on. A comment of the form `// code` whose code ends the way a line of the
/// config does is uncommented; prose ends in a word or a full stop and stays.
fn switched_on(starter: &str) -> String {
    let mut text = starter
        .lines()
        .map(|line| {
            let body = line.trim_start();
            match body.strip_prefix("// ") {
                Some(code) if code.ends_with([',', '{', '}', ';', '[', ']', ')']) => {
                    format!("{}{code}", &line[..line.len() - body.len()])
                }
                _ => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

#[test]
fn the_starters_examples_do_what_they_say_when_switched_on() {
    let on = switched_on(STARTER_CONFIG);
    assert_ne!(on, STARTER_CONFIG);
    let mut rig = boot(Some(&on));
    rig.settle();
    assert_eq!(
        rig.eval("gnoblin.workspaces.map((w) => `${w.id}:${w.layout.constructor.name}`).join()"),
        "1:FloatingLayout,2:FloatingLayout,3:FloatingLayout,4:FloatingLayout,work:MasterStackLayout"
    );
    let claimed = rig.comp.claimed();
    for chord in ["<Super>Return", "<Super>q", "<Super>1"] {
        assert!(
            claimed.iter().any(|held| held.contains(chord)),
            "{chord} in {claimed:?}"
        );
    }
    assert_eq!(rig.eval("String(gnoblin.clients.style.cornerRadius)"), "12");
}

#[test]
fn the_starter_is_a_short_readable_ascii_file() {
    let lines = STARTER_CONFIG.lines().count();
    assert!((60..=140).contains(&lines), "{lines} lines");
    assert!(STARTER_CONFIG.is_ascii());
    assert!(STARTER_CONFIG.lines().all(|line| line.len() <= 120));
}

/// The code lines of `lines`: no blanks and no comments, with the indentation of the first one removed.
fn statements(lines: &[&str]) -> Vec<String> {
    let code: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with("//"))
        .collect();
    let indent = code.first().map_or(0, |line| line.len() - line.trim_start().len());
    code.iter()
        .map(|line| line[indent.min(line.len())..].to_owned())
        .collect()
}

#[test]
fn the_starter_file_and_install_defaults_are_the_same_statements() {
    let library = include_str!("../../../data/js/gnoblin/starter.ts");
    let open = library
        .find("export function installDefaults(")
        .expect("the library function");
    let body: Vec<&str> = library[open..]
        .lines()
        .skip(1)
        .take_while(|line| *line != "}")
        .collect();
    let file: Vec<&str> = STARTER_CONFIG.lines().collect();
    let after_imports = file
        .iter()
        .position(|line| *line == "} from \"gnoblin\";")
        .expect("the starter's import block")
        + 1;
    assert_eq!(
        statements(&file[after_imports..]),
        statements(&body),
        "edit data/js/starter/init.ts and the body of installDefaults in data/js/gnoblin/starter.ts together"
    );
}
