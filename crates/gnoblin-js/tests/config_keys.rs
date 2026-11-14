//! The keys of `defineConfig`: which sections of the config document are keys, what the new list and map keys plan,
//! and the messages a wrong key or value gets. Run in a real generation against a fake `gnoblin` global, so they
//! test the stock library (`data/js/gnoblin/config.ts`) and not a copy of it.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gnoblin_config::TOP_LEVEL_KEYS;
use gnoblin_js::{JsConfig, JsGeneration, STARTER_CONFIG, stock_modules};

/// The fake host and the helper every case shares. Plain JavaScript: embedded modules are not stripped.
const HARNESS: &str = r##"
globalThis.gnoblin = { __register() {} };
export function thrown(fn) {
    try {
        fn();
    } catch (e) {
        return `${e.name}: ${e.message}`;
    }
    return "did not throw";
}
export function emit(value) {
    globalThis.out = String(value);
}
"##;

const IMPORTS: &str = r#"
import { thrown, emit } from "harness";
import { desugarConfig, defaultConfig, mergeConfig, CONFIG_KEYS } from "gnoblin";
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

/// What the config document's section is to `defineConfig`.
enum Section {
    /// A key of that name.
    Key(&'static str),
    /// Not a key, and why: the design table (`design/js-runtime.md`) says the same.
    NotKey(&'static str),
}

/// Every top-level section of the config document, and what `defineConfig` makes of it. A section the schema gains
/// is missing here, which fails the first test below until someone decides.
const SECTIONS: &[(&str, Section)] = &[
    ("accessibility", Section::Key("accessibility")),
    (
        "animations",
        Section::NotKey("carried and read by nothing; use the animation objects of the shell"),
    ),
    ("appearance", Section::Key("appearance")),
    ("auth", Section::Key("auth")),
    ("autostart", Section::Key("autostart")),
    ("capture", Section::Key("capture")),
    ("compat", Section::Key("compat")),
    ("compositor", Section::Key("compositor")),
    ("cursor", Section::Key("cursor")),
    ("dialogs", Section::Key("dialogs")),
    ("input", Section::Key("input")),
    ("input-sources", Section::Key("inputSources")),
    (
        "keybindings",
        Section::NotKey("carried and read by nothing; bind a key with `keys`"),
    ),
    ("keyring", Section::Key("keyring")),
    ("layer-shell", Section::Key("layerShell")),
    ("location", Section::Key("location")),
    ("logging", Section::Key("logging")),
    ("monitors", Section::Key("monitors")),
    ("portals", Section::NotKey("carried and read by nothing")),
    ("protocols", Section::Key("protocols")),
    ("shortcuts", Section::NotKey("one way to bind a key: `keys`")),
    ("touchpad-gestures", Section::Key("touchpadGestures")),
    ("window-management", Section::Key("windowManagement")),
    ("workspaces", Section::Key("workspaces")),
    ("xwayland", Section::Key("xwayland")),
];

#[test]
fn every_section_of_the_config_document_is_a_key_or_a_recorded_non_key() {
    let keys: Vec<String> = run(r#"emit(CONFIG_KEYS.join(","));"#)
        .split(',')
        .map(str::to_owned)
        .collect();
    for section in TOP_LEVEL_KEYS {
        let Some((_, disposition)) = SECTIONS.iter().find(|(name, _)| name == section) else {
            panic!(
                "the document section `{section}` has no entry in SECTIONS: make it a defineConfig key or record why not"
            );
        };
        match disposition {
            Section::Key(key) => assert!(
                keys.iter().any(|known| known == key),
                "`{key}` is not a defineConfig key"
            ),
            Section::NotKey(reason) => assert!(!reason.is_empty()),
        }
    }
    for (name, _) in SECTIONS {
        assert!(
            TOP_LEVEL_KEYS.contains(name),
            "SECTIONS names `{name}`, which the document does not have"
        );
    }
}

#[test]
fn protocols_become_the_documents_kebab_case_table() {
    let got = run(r#"
        emit(JSON.stringify(desugarConfig({
            protocols: { wlrLayerShell: false, extBackgroundEffectV1: false, extIdleNotify: true },
        }).document));
    "#);
    assert_eq!(
        got,
        r#"{"protocols":{"wlr-layer-shell":false,"ext-background-effect-v1":false,"ext-idle-notify":true}}"#
    );
}

#[test]
fn touchpad_gestures_become_one_list_of_the_document() {
    let got = run(r#"
        emit(JSON.stringify(desugarConfig({
            touchpadGestures: [
                { name: "open-terminal", gesture: "swipe", fingers: 3, path: [{ x: 0, y: 0 }, { x: 1, y: 0 }], command: ["kgx"], tolerance: 0.2 },
                { name: "launcher", gesture: "pinch", fingers: 4, direction: "out", command: ["fuzzel"] },
            ],
        }).document));
    "#);
    assert_eq!(
        got,
        r#"{"touchpad-gestures":[{"name":"open-terminal","gesture":"swipe","fingers":3,"path":[{"x":0,"y":0},{"x":1,"y":0}],"command":["kgx"],"tolerance":0.2},{"name":"launcher","gesture":"pinch","fingers":4,"direction":"out","command":["fuzzel"]}]}"#
    );
}

#[test]
fn a_list_key_a_config_gives_replaces_the_stock_one_whole() {
    let got = run(r#"
        const gesture = (name: string) => ({ name, gesture: "pinch", fingers: 3, direction: "in", command: ["x"] });
        const base = { touchpadGestures: [gesture("a"), gesture("b")], autostart: ["one"] };
        const merged = mergeConfig(base as any, { touchpadGestures: [gesture("c")], autostart: ["two", "three"] } as any);
        const kept = mergeConfig(base as any, {} as any);
        emit([
            merged.touchpadGestures!.map((g) => g.name).join(" "),
            merged.autostart!.join(" "),
            kept.touchpadGestures!.map((g) => g.name).join(" "),
            "autostart" in mergeConfig(defaultConfig, {}) || "touchpadGestures" in mergeConfig(defaultConfig, {}),
        ].join("\n"));
    "#);
    assert_eq!(got, "c\ntwo three\na b\nfalse");
}

#[test]
fn autostart_commands_become_argv_lists_in_the_order_written() {
    let got = run(r#"
        emit(JSON.stringify(desugarConfig({
            autostart: ["waybar", "  foot   --server ", ["notify-send", "hello world"]],
        }).autostart));
    "#);
    assert_eq!(got, r#"[["waybar"],["foot","--server"],["notify-send","hello world"]]"#);
    assert_eq!(run(r#"emit(JSON.stringify(desugarConfig({}).autostart));"#), "[]");
}

#[test]
fn a_wrong_key_names_itself_the_nearest_valid_key_and_lists_the_valid_ones() {
    let got = run(r#"
        const bad = (config: any) => thrown(() => desugarConfig(config));
        emit([
            bad({ autostrt: [] }),
            bad({ Keys: {} }),
            bad({ "window-management": {} }),
            bad({ touchpadGesture: [] }),
            bad({ xwayand: {} }),
            bad({ colour: 1 }),
        ].join("\n"));
    "#);
    let listing = "expected one of gaps, style, outputs, workspaces, keys, launcher, autostart, rules, on, setup, decorations, menus, alerts, focus, edgeTiling, dragConstraint, gestureActions, workspaceSets, compositor, windowManagement, cursor, appearance, dialogs, input, inputSources, accessibility, layerShell, keyring, auth, location, compat, capture, monitors, xwayland, logging, protocols, touchpadGestures";
    assert_eq!(
        got,
        [
            format!(r#"TypeError: config has no "autostrt" (did you mean "autostart"?); {listing}"#),
            format!(r#"TypeError: config has no "Keys" (did you mean "keys"?); {listing}"#),
            format!(r#"TypeError: config has no "window-management" (did you mean "windowManagement"?); {listing}"#),
            format!(r#"TypeError: config has no "touchpadGesture" (did you mean "touchpadGestures"?); {listing}"#),
            format!(r#"TypeError: config has no "xwayand" (did you mean "xwayland"?); {listing}"#),
            format!(r#"TypeError: config has no "colour"; {listing}"#),
        ]
        .join("\n")
    );
}

#[test]
fn a_key_three_edits_away_is_not_suggested() {
    // Two edits is the limit: `kxxs` is two substitutions from `keys`, `kxxx` three, and nothing else is near.
    let near = run(r#"emit(thrown(() => desugarConfig({ kxxs: {} } as any)));"#);
    assert!(near.contains(r#"(did you mean "keys"?)"#), "{near}");
    let far = run(r#"emit(thrown(() => desugarConfig({ kxxx: {} } as any)));"#);
    assert!(!far.contains("did you mean"), "{far}");
}

#[test]
fn the_retired_era_sections_say_what_to_write_instead() {
    let got = run(r#"
        const bad = (config: any) => thrown(() => desugarConfig(config));
        emit([bad({ shortcuts: {} }), bad({ keybindings: {} }), bad({ "touchpad-gestures": [] })].join("\n"));
    "#);
    let lines: Vec<&str> = got.lines().collect();
    assert!(
        lines[0].starts_with(
            r#"TypeError: config has no "shortcuts" (bind a key with keys, for example keys: { "Super+Return": spawn("foot") }); expected one of "#
        ),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].contains(r#"config has no "keybindings" (bind a key with keys"#),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].contains(r#"config has no "touchpad-gestures" (write touchpadGestures)"#),
        "{}",
        lines[2]
    );
}

#[test]
fn a_wrong_value_names_the_path_and_the_expected_type() {
    let got = run(r#"
        const bad = (config: any) => thrown(() => desugarConfig(config));
        emit([
            bad({ workspaces: "web" }),
            bad({ keys: [] }),
            bad({ gaps: 4 }),
            bad({ rules: {} }),
            bad({ focus: "click" }),
            bad({ on: null }),
            bad({ autostart: "waybar" }),
            bad({ autostart: [5] }),
            bad({ autostart: [[]] }),
            bad({ autostart: [" "] }),
            bad({ autostart: [["a", 3]] }),
            bad({ autostart: ["a b", ["a", "b"]] }),
            bad({ touchpadGestures: {} }),
            bad({ protocols: 5 }),
            bad({ cursor: "big" }),
            bad({ decorations: "yes" }),
            bad({ edgeTiling: 1 }),
            bad({ launcher: 3 }),
            bad({ setup: "later" }),
            bad({ gaps: { inner: "wide" } }),
            bad({ protocols: { wlrLayerShell: () => 1 } }),
            bad({ touchpadGestures: [{ command: [Infinity] }] }),
        ].join("\n"));
    "#);
    assert_eq!(
        got,
        [
            r#"TypeError: config.workspaces must be a list of workspaces such as [{ id: "web" }], got string"#,
            "TypeError: config.keys must be a table of triggers and actions, got a list",
            "TypeError: config.gaps must be a table such as { inner: 8, outer: 8 }, got number",
            "TypeError: config.rules must be a list of window rules, got object",
            "TypeError: config.focus must be a table of focus settings, got string",
            "TypeError: config.on must be a table of listeners, got null",
            "TypeError: config.autostart must be a list of commands, got string",
            "TypeError: config.autostart[0] must be a command line or an argv array of strings, got number",
            "TypeError: config.autostart[0] must be a non-empty command line or an argv of strings",
            "TypeError: config.autostart[0] must be a non-empty command line or an argv of strings",
            "TypeError: config.autostart[0] must be a non-empty command line or an argv of strings",
            "Error: config.autostart[1] starts the same command as config.autostart[0]",
            "TypeError: config.touchpadGestures must be a list of gestures, got object",
            "TypeError: config.protocols must be a table of settings, got number",
            "TypeError: config.cursor must be a table of settings, got string",
            "TypeError: config.decorations: must be true or false",
            "TypeError: config.edgeTiling: must be true or false",
            "TypeError: config.launcher must be a command line or an argv array",
            "TypeError: setup must be a function",
            "RangeError: gaps.inner must be a finite number of at least 0, got wide",
            "TypeError: config.protocols.wlrLayerShell must be a string, a number, a boolean, a list or a table of settings",
            "RangeError: config.touchpadGestures[0].command[0] must be a finite number",
        ]
        .join("\n")
    );
}

// --- the starter `gnoblinctl config init` writes ---------------------------------------------------------------

/// Evaluates `probe` (TypeScript) in a module that imports the default export of `entry` as `starter`, with the stock
/// library imported, and returns what it passed to `emit`.
fn run_entry(entry: &str, probe: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), entry).expect("init.ts is written");
    fs::write(
        root.join("probe.ts"),
        format!("{IMPORTS}\nimport starter from \"./init.ts\";\n{probe}\n"),
    )
    .expect("probe.ts is written");
    let mut config = JsConfig::new(&root).embed("harness", HARNESS);
    for (name, source) in stock_modules() {
        config = config.embed(name, source);
    }
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    generation
        .eval_module(Path::new("probe.ts"))
        .expect("the starter evaluates, so every key in it is one defineConfig accepts");
    generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, String>("out"))
        .expect("the probe called emit")
}

/// What a config does once the host has laid it over the stock one and planned it.
const DESCRIBE: &str = r#"
const describe = (config: any) => {
    const plan = desugarConfig(mergeConfig(defaultConfig, config));
    return JSON.stringify({
        workspaces: plan.workspaces.map((w) => `${w.id}:${w.layout.constructor.name}`),
        keys: plan.bindings.map((b) => b.trigger),
        decorations: plan.decorations,
        gaps: plan.gaps,
        autostart: plan.autostart,
        rules: plan.rules.length,
        setup: plan.setup !== undefined,
        document: plan.document,
    });
};
"#;

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
fn the_starter_as_written_plans_exactly_what_the_stock_config_plans() {
    let got = run_entry(
        STARTER_CONFIG,
        &format!("{DESCRIBE}\nemit([describe(starter), describe(defaultConfig)].join(\"\\n\"));"),
    );
    let (starter, stock) = got.split_once('\n').expect("two lines");
    assert_eq!(starter, stock, "the starter reproduces zero-config");
    let plan: serde_json::Value = serde_json::from_str(stock).expect("the plan is JSON");
    assert_eq!(
        plan["workspaces"],
        serde_json::json!([
            "1:FloatingLayout",
            "2:FloatingLayout",
            "3:FloatingLayout",
            "4:FloatingLayout"
        ]),
        "the retired stock config's four workspaces"
    );
    assert_eq!(plan["decorations"], true);
    assert_eq!(plan["autostart"], serde_json::json!([]));
}

#[test]
fn the_starters_examples_plan_what_they_say_when_switched_on() {
    let on = switched_on(STARTER_CONFIG);
    assert_ne!(on, STARTER_CONFIG);
    let got = run_entry(&on, &format!("{DESCRIBE}\nemit(describe(starter));"));
    let plan: serde_json::Value = serde_json::from_str(&got).expect("the plan is JSON");
    assert_eq!(
        plan["workspaces"],
        serde_json::json!([
            "1:FloatingLayout",
            "2:FloatingLayout",
            "3:FloatingLayout",
            "4:FloatingLayout",
            "work:MasterStackLayout"
        ])
    );
    let keys: Vec<&str> = plan["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    for key in ["Super+Return", "Super+b", "Super+q", "Super+1", "XF86AudioMute"] {
        assert!(keys.contains(&key), "{key} in {keys:?}");
    }
    assert_eq!(plan["autostart"], serde_json::json!([["waybar"]]));
    assert_eq!(plan["rules"], 1);
    assert_eq!(plan["setup"], true);
}

#[test]
fn the_starter_is_about_forty_lines_and_ascii() {
    let lines = STARTER_CONFIG.lines().count();
    assert!((30..=60).contains(&lines), "{lines} lines");
    assert!(STARTER_CONFIG.is_ascii());
    assert!(STARTER_CONFIG.lines().all(|line| line.len() <= 120));
}

#[test]
fn the_config_interface_and_the_list_of_keys_agree() {
    // The interface is what the editor offers and `CONFIG_KEYS` is what the load accepts.
    let source = include_str!("../../../data/js/gnoblin/config.ts");
    let mut declared: Vec<String> = Vec::new();
    let mut inside = false;
    for line in source.lines() {
        if line == "export interface GnoblinConfig {" {
            inside = true;
        } else if inside && line == "}" {
            break;
        } else if inside && let Some(rest) = line.strip_prefix("    ") {
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !name.is_empty() && rest[name.len()..].starts_with(['?', '(']) {
                declared.push(name);
            }
        }
    }
    declared.sort();
    let mut accepted: Vec<String> = run(r#"emit(CONFIG_KEYS.join(","));"#)
        .split(',')
        .map(str::to_owned)
        .collect();
    accepted.sort();
    assert_eq!(declared, accepted);
}
