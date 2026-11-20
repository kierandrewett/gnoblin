//! The converter over whole configs: the shipped starter, a config of every construct that converts, a config of
//! settings the specification has no home for, the shortcuts and the rules. Nothing here runs Lua or the TypeScript
//! runtime; the `gnoblinctl` tests load the output in the shell's dry-run host.

use std::collections::BTreeMap;

use gnoblin_config_migrate::{
    ConfigSourceLoader, MigrateError, MigrateSource, MigrationOutput, glob_matches, is_glob, migrate, resolve_load_path,
};

/// The stock starter, `shells/default/shell.ts`: what `gnoblinctl config init` writes and what a shell with no
/// `shell.ts` runs.
const STARTER_TS: &str = include_str!("../../../shells/default/shell.ts");

/// The Lua starter as it shipped in the Mutter-based releases, the `data/default-config` of the `main` branch.
const STARTER_INIT: &str = include_str!("fixtures/starter/init.lua");
const STARTER_FILES: &[(&str, &str)] = &[
    (
        "config/10-windows.lua",
        include_str!("fixtures/starter/config/10-windows.lua"),
    ),
    (
        "config/20-input.lua",
        include_str!("fixtures/starter/config/20-input.lua"),
    ),
    (
        "config/30-session.lua",
        include_str!("fixtures/starter/config/30-session.lua"),
    ),
    (
        "config/40-shortcuts.lua",
        include_str!("fixtures/starter/config/40-shortcuts.lua"),
    ),
];

/// Where the migration page lists the settings that have no TypeScript home yet.
const DROPPED: &str = "docs/migrating-from-lua.md#settings-with-no-typescript-home-yet";

/// The files of a config directory in memory.
struct MapLoader(BTreeMap<String, String>);

impl MapLoader {
    fn new(files: &[(&str, &str)]) -> Self {
        MapLoader(
            files
                .iter()
                .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
                .collect(),
        )
    }
}

impl ConfigSourceLoader for MapLoader {
    fn expand(&self, from: &str, pattern: &str) -> Result<Vec<MigrateSource>, String> {
        let resolved = resolve_load_path(from, pattern);
        let source = |(name, text): (&String, &String)| MigrateSource {
            name: name.clone(),
            text: text.clone(),
        };
        if is_glob(&resolved) {
            Ok(self
                .0
                .iter()
                .filter(|(name, _)| glob_matches(&resolved, name))
                .map(source)
                .collect())
        } else {
            self.0
                .get_key_value(&resolved)
                .map(source)
                .map(|found| vec![found])
                .ok_or_else(|| format!("{resolved} does not exist"))
        }
    }
}

fn convert_with(init: &str, files: &[(&str, &str)], shell: Option<&str>) -> MigrationOutput {
    let entry = MigrateSource {
        name: "init.lua".to_owned(),
        text: init.to_owned(),
    };
    let followers: Vec<MigrateSource> = shell
        .map(|text| MigrateSource {
            name: "shell.lua".to_owned(),
            text: text.to_owned(),
        })
        .into_iter()
        .collect();
    migrate(&entry, &followers, &MapLoader::new(files)).expect("the fixture is Lua")
}

fn convert(init: &str) -> MigrationOutput {
    convert_with(init, &[], None)
}

fn shell(text: &str) -> MigrationOutput {
    convert_with("-- nothing here\n", &[], Some(text))
}

/// The unmapped items as `file:line`, in the order found.
fn places(output: &MigrationOutput) -> Vec<String> {
    output
        .report
        .unmapped
        .iter()
        .map(|item| format!("{}:{}", item.file, item.line))
        .collect()
}

/// The reasons of the unmapped items, in the order found.
fn reasons(output: &MigrationOutput) -> Vec<&str> {
    output.report.unmapped.iter().map(|item| item.reason.as_str()).collect()
}

/// The lines of a file that are code: not blank and not a comment, indentation kept.
fn code_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with("//"))
        .map(|line| line.trim_end().to_owned())
        .collect()
}

// --- The shipped starter -----------------------------------------------------------------------------------------

#[test]
fn the_starter_converts_its_workspaces_and_keys_and_reports_each_setting_of_a_section() {
    let output = convert_with(STARTER_INIT, STARTER_FILES, None);
    assert_eq!(
        output.report.sources,
        [
            "init.lua",
            "config/10-windows.lua",
            "config/20-input.lua",
            "config/30-session.lua",
            "config/40-shortcuts.lua"
        ]
    );
    let counts = &output.report.converted;
    assert_eq!(
        (counts.workspaces, counts.keys, counts.rules, counts.options),
        (4, 7, 0, 0)
    );
    // `window-management`, `compositor` and `layer-shell` hold 19 settings, and the specification has no home for any;
    // the empty tables of `cursor`, `input` and `protocols` hold none, so they lose nothing.
    let mut expected: Vec<String> = [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 18]
        .iter()
        .map(|line| format!("config/10-windows.lua:{line}"))
        .collect();
    expected.extend(
        [4, 5, 6, 7, 8, 14]
            .iter()
            .map(|line| format!("config/30-session.lua:{line}")),
    );
    assert_eq!(places(&output), expected);
    assert_eq!(
        output.typescript.lines().next(),
        Some("// INCOMPLETE: 19 unmapped settings, see comments")
    );
    assert_eq!(output.typescript.matches("UNMAPPED (").count(), 19);
    for item in &output.report.unmapped {
        assert_eq!(item.pointer, DROPPED, "{item:?}");
    }
}

#[test]
fn the_starter_is_this_typescript() {
    let output = convert_with(STARTER_INIT, STARTER_FILES, None);
    assert_eq!(output.typescript, include_str!("fixtures/starter/shell.ts.golden"));
}

#[test]
fn an_empty_config_converts_to_the_stock_desktop_statement_for_statement() {
    let output = convert("-- nothing\n");
    assert_eq!(output.report.unmapped, Vec::new());
    assert!(!output.typescript.contains("INCOMPLETE"), "{}", output.typescript);
    assert!(!output.typescript.contains("UNMAPPED"), "{}", output.typescript);
    // The stock desktop is the starter's statements. The order of two blocks differs (`const input` stands above them
    // here), so the lines are compared as a set; the dry-run parity test in `gnoblinctl` compares what they do.
    let mut written = code_lines(&output.typescript);
    let mut starter = code_lines(STARTER_TS);
    written.sort();
    starter.sort();
    assert_eq!(written, starter);
}

#[test]
fn a_stock_key_the_config_rebinds_is_written_once() {
    let output = convert(
        r#"gnoblin.configure { shortcuts = {
    a = { binding = "<Super>2", command = { "x" } },
    b = { binding = "<Super>space", command = { "launcher" } },
    c = { binding = "AudioMute", command = { "m" } },
} }
"#,
    );
    let ts = &output.typescript;
    assert_eq!(output.report.unmapped, Vec::new(), "{ts}");
    assert_eq!(ts.matches("\"Super+2\"").count(), 1, "{ts}");
    assert!(
        ts.contains("for (const position of [1, 3, 4, 5, 6, 7, 8, 9]) {"),
        "{ts}"
    );
    assert!(ts.contains("gnoblin.spawn([\"launcher\"]);"), "{ts}");
    assert!(!ts.contains("openLauncher"), "{ts}");
    assert_eq!(ts.matches("XF86AudioMute").count(), 1, "{ts}");
    assert!(ts.contains("    XF86AudioMicMute: [\"wpctl\""), "{ts}");
}

// --- Every construct that converts -------------------------------------------------------------------------------

const EVERYTHING: &str = r#"
gnoblin.configure({
    workspaces = {
        { id = "web", name = "Web" },
        { id = "dev", name = "Development" },
    },
    shortcuts = {
        terminal = { binding = "<Super>Return", command = { "foot" } },
        browser = { binding = "<Super><Shift>b", command = "firefox" },
        mute = { binding = "XF86AudioMute", command = { "wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle" } },
        off = { binding = "<Super>x", command = { "xterm" }, enable = false },
    },
})
"#;

const EVERYTHING_SHELL: &str = r#"
seat.data.window_rules = {
    { match = { app_id = "^org%.example%.Editor$" }, workspace = { id = "dev" }, maximised = true },
    { match = { title = "Picture.in.Picture" }, fullscreen = true },
    { match = { app_id = "^pavucontrol$", title = "^Volume Control$" }, opacity = 0.9 },
    { match = { app_id = "^org%.x%.(%w+)$" }, workspace = "web", maximised = false },
}
seat.data.alerts = false
"#;

#[test]
fn a_config_of_every_construct_that_converts_leaves_nothing_out() {
    let output = convert_with(EVERYTHING, &[], Some(EVERYTHING_SHELL));
    let ts = &output.typescript;
    assert_eq!(output.report.unmapped, Vec::new(), "{ts}");
    assert!(!ts.contains("INCOMPLETE"), "{ts}");
    // Workspaces are objects, and the stock four are not written beside them.
    assert!(
        ts.contains(
            "gnoblin.workspaces.add(new Workspace({ id: \"web\", name: \"Web\", layout: new FloatingLayout() }));"
        ),
        "{ts}"
    );
    assert!(
        ts.contains("gnoblin.workspaces.add(new Workspace({ id: \"dev\", name: \"Development\", layout: new FloatingLayout() }));"),
        "{ts}"
    );
    assert!(!ts.contains("for (const id of"), "{ts}");
    // Shortcuts are bindings on the primary seat.
    assert!(ts.contains("const input = gnoblin.input.primary;\n"), "{ts}");
    assert!(
        ts.contains("input.bind(\"Super+Return\", () => {\n    gnoblin.spawn([\"foot\"]);\n});"),
        "{ts}"
    );
    assert!(
        ts.contains("input.bind(\"Super+Shift+b\", () => {\n    gnoblin.spawn([\"firefox\"]);\n});"),
        "{ts}"
    );
    assert!(
        ts.contains(
            "input.bind(\"XF86AudioMute\", () => {\n    gnoblin.spawn([\"wpctl\", \"set-mute\", \"@DEFAULT_AUDIO_SINK@\", \"toggle\"]);\n});"
        ),
        "{ts}"
    );
    assert_eq!(ts.matches("XF86AudioMute").count(), 1, "the stock key gives way: {ts}");
    assert!(!ts.contains("xterm"), "a disabled entry is left out: {ts}");
    // Rules: a style on an exact selection, and one listener for the rest.
    assert!(
        ts.contains(
            "gnoblin.clients.where({ appId: \"pavucontrol\", title: \"Volume Control\" }).style.opacity = 0.9;"
        ),
        "{ts}"
    );
    assert_eq!(ts.matches("addListener(").count(), 1, "{ts}");
    assert!(
        ts.contains(
            "            if (client.appId === \"org.example.Editor\") {\n                gnoblin.workspaces.get(\"dev\")?.clients.add(client);\n                client.maximised = true;\n            }"
        ),
        "{ts}"
    );
    assert!(
        ts.contains("            if (/Picture.in.Picture/s.test(client.title)) {\n                client.fullscreen = true;\n            }"),
        "{ts}"
    );
    assert!(
        ts.contains(
            "            if (/^org\\.x\\.[A-Za-z0-9]+$/.test(client.appId)) {\n                gnoblin.workspaces.get(\"web\")?.clients.add(client);\n                client.maximised = false;\n            }"
        ),
        "{ts}"
    );
    assert!(ts.contains("import type { Client } from \"gnoblin-core\";"), "{ts}");
    assert!(
        ts.contains("import { CollectionListener } from \"gnoblin-core\";\n"),
        "{ts}"
    );
    // `alerts = false` leaves the alert out of the stock desktop.
    assert!(!ts.contains("\nstartNotRespondingAlert();"), "{ts}");
    assert!(
        !ts.lines().any(|line| line.trim() == "startNotRespondingAlert,"),
        "the alert is not imported: {ts}"
    );
    let counts = &output.report.converted;
    assert_eq!(
        (counts.workspaces, counts.keys, counts.rules, counts.options),
        (2, 3, 4, 1)
    );
    assert_eq!(
        output.report.notes.len(),
        1,
        "the disabled shortcut is noted: {:?}",
        output.report.notes
    );
    assert!(ts.contains("// From shell.lua."), "{ts}");
}

#[test]
fn a_comment_names_each_file_that_contributed_something() {
    let output = convert_with(STARTER_INIT, STARTER_FILES, None);
    for name in [
        "config/10-windows.lua",
        "config/30-session.lua",
        "config/40-shortcuts.lua",
    ] {
        assert!(
            output.typescript.contains(&format!("// From {name}.")),
            "{name}\n{}",
            output.typescript
        );
    }
    assert!(
        !output.typescript.contains("// From config/20-input.lua."),
        "that file holds only empty tables: {}",
        output.typescript
    );
    assert!(
        output
            .typescript
            .contains("// Sources, in load order: init.lua, config/10-windows.lua")
    );
}

#[test]
fn a_later_configure_call_replaces_the_list_of_workspaces() {
    let first = "gnoblin.configure { workspaces = { { id = \"1\", name = \"One\" } } }\ngnoblin.load(\"more.lua\")\n";
    let more = "gnoblin.configure { workspaces = { { id = \"2\", name = \"Two\" } } }\n";
    let output = convert_with(first, &[("more.lua", more)], None);
    let ts = &output.typescript;
    assert!(
        ts.contains("new Workspace({ id: \"2\"") && !ts.contains("new Workspace({ id: \"1\""),
        "a list is replaced whole: {ts}"
    );
    assert_eq!(output.report.converted.workspaces, 1);
}

#[test]
fn a_load_with_no_match_is_a_note_and_a_missing_file_is_left_out() {
    let output = convert("gnoblin.load(\"nothing/*.lua\")\ngnoblin.load(\"gone.lua\")\n");
    assert_eq!(output.report.notes.len(), 1, "{:?}", output.report.notes);
    assert!(
        output.report.notes[0].contains("matches no file"),
        "{:?}",
        output.report.notes
    );
    assert_eq!(places(&output), ["init.lua:2"]);
    assert!(
        output.report.unmapped[0].reason.contains("gone.lua"),
        "{:?}",
        output.report.unmapped
    );
}

#[test]
fn a_file_that_loads_itself_is_left_out_once() {
    let output = convert_with(
        "gnoblin.load(\"a.lua\")\n",
        &[("a.lua", "gnoblin.load(\"a.lua\")\n")],
        None,
    );
    assert_eq!(places(&output), ["a.lua:1"]);
    assert!(output.report.unmapped[0].reason.contains("loads itself"));
}

#[test]
fn a_load_is_relative_to_the_file_that_makes_it() {
    let output = convert_with(
        "gnoblin.load(\"config/*.lua\")\n",
        &[
            ("config/a.lua", "gnoblin.load(\"sub/b.lua\")\n"),
            (
                "config/sub/b.lua",
                "gnoblin.configure { workspaces = { { id = \"deep\", name = \"Deep\" } } }\n",
            ),
        ],
        None,
    );
    assert_eq!(output.report.unmapped, Vec::new());
    assert!(output.typescript.contains("new Workspace({ id: \"deep\""));
    assert_eq!(output.report.sources, ["init.lua", "config/a.lua", "config/sub/b.lua"]);
}

// --- Settings the specification has no home for ------------------------------------------------------------------

const NO_HOME: &str = r##"local delay = 5 * 100
gnoblin.configure {
    ["window-management"] = { ["focus-mode"] = "hover", ["auto-raise-delay"] = delay, ["edge-tiling"] = false },
    cursor = { size = 28, theme = "Adwaita" .. "-dark" },
    protocols = { ["wlr-screencopy"] = false },
    keyring = { enabled = false },
    dialogs = { style = { accent = "#7c3aed" } },
    input = { keyboard = { ["repeat-rate"] = 30 } },
    ["touchpad-gestures"] = {
        { name = "term", gesture = "swipe", fingers = 3, command = { "foot" } },
    },
    autostart = {
        bar = { command = { "waybar" } },
        off = { command = { "never" }, enable = false },
    },
}
gnoblin.configure({ autostart = { "waybar --x" } })
"##;

#[test]
fn a_setting_with_no_home_is_reported_with_what_a_home_needs_and_the_file_is_marked_incomplete() {
    let output = convert(NO_HOME);
    let ts = &output.typescript;
    assert_eq!(
        places(&output),
        [
            "init.lua:3",
            "init.lua:3",
            "init.lua:3",
            "init.lua:4",
            "init.lua:4",
            "init.lua:5",
            "init.lua:6",
            "init.lua:7",
            "init.lua:8",
            "init.lua:10",
            "init.lua:13",
            "init.lua:17",
        ],
        "{ts}"
    );
    assert_eq!(
        ts.lines().next(),
        Some("// INCOMPLETE: 12 unmapped settings, see comments"),
        "{ts}"
    );
    assert_eq!(ts.matches("// UNMAPPED (init.lua:").count(), 12, "{ts}");
    let reasons = reasons(&output);
    let has = |needle: &str| reasons.iter().any(|reason| reason.contains(needle));
    assert!(
        has("`window-management.focus-mode` has no TypeScript home yet"),
        "{reasons:?}"
    );
    assert!(has("no focus policy"), "{reasons:?}");
    assert!(has("`window-management.edge-tiling`"), "{reasons:?}");
    assert!(has("no snapping policy"), "{reasons:?}");
    assert!(has("`cursor.theme`"), "{reasons:?}");
    assert!(has("no compositor-wide settings object"), "{reasons:?}");
    assert!(has("`protocols.wlr-screencopy`"), "{reasons:?}");
    assert!(has("keeps its default, enabled"), "{reasons:?}");
    assert!(has("`dialogs.style.accent`") && has("dialog manager"), "{reasons:?}");
    assert!(has("`input.keyboard.repeat-rate`") && has("`Keyboard`"), "{reasons:?}");
    assert!(
        has("a touchpad gesture") && has("no path gesture trigger"),
        "{reasons:?}"
    );
    assert!(
        has("`autostart.bar`") && has("session-scoped startup list"),
        "{reasons:?}"
    );
    assert!(has("an autostart command"), "{reasons:?}");
    for item in &output.report.unmapped {
        assert_eq!(item.pointer, DROPPED, "{item:?}");
    }
    // The converted statements are the stock desktop and nothing the Lua said.
    assert!(
        !ts.contains("waybar\"]") && !ts.contains("gnoblin.spawn([\"foot\"])"),
        "{ts}"
    );
    // A disabled autostart entry starts nothing, so it is a note and not a loss.
    assert!(!ts.contains("never"), "{ts}");
    assert_eq!(output.report.notes.len(), 1, "{:?}", output.report.notes);
    assert!(output.report.notes[0].contains("disabled"), "{:?}", output.report.notes);
}

#[test]
fn an_unknown_or_carried_section_is_left_out_with_a_reason() {
    let output = convert(
        "gnoblin.configure {\n    cursor = { size = 24 },\n    animations = { x = 1 },\n    typo = {},\n    shell = \"x\",\n}\n",
    );
    assert_eq!(
        places(&output),
        ["init.lua:2", "init.lua:3", "init.lua:4", "init.lua:5"]
    );
    assert!(output.report.unmapped[0].reason.contains("no TypeScript home yet"));
    assert!(output.report.unmapped[1].reason.contains("read by nothing"));
    assert!(output.report.unmapped[2].reason.contains("not a section"));
    assert!(output.report.unmapped[3].reason.contains("shell.ts"));
}

#[test]
fn the_seats_settings_in_a_shell_file_are_a_switch_and_reports() {
    let output = shell(
        "seat.data.alerts = false\nseat.edge_tiling = true\nseat.drag_constraint = \"minimum_strip\"\nseat.data.prompts = false\nseat.data.workspace_sets = { { name = \"work\", entries = { { workspace = \"a\" } } }, { name = \"two\", entries = {} } }\nseat.other = 1\n",
    );
    let ts = &output.typescript;
    assert_eq!(
        places(&output),
        [
            "shell.lua:2",
            "shell.lua:3",
            "shell.lua:4",
            "shell.lua:5",
            "shell.lua:5",
            "shell.lua:6"
        ]
    );
    let reasons = reasons(&output);
    assert!(reasons[0].contains("`seat.edge_tiling`") && reasons[0].contains("no snapping policy"));
    assert!(reasons[1].contains("`seat.drag_constraint`"));
    assert!(reasons[2].contains("no switch"));
    assert!(reasons[3].contains("a workspace set") && reasons[3].contains("no workspace sets"));
    assert!(reasons[5].contains("`seat.other` is not read"));
    assert_eq!(output.report.converted.options, 1, "alerts is the one switch");
    assert!(!ts.contains("\nstartNotRespondingAlert();"), "{ts}");
}

// --- Shortcuts ---------------------------------------------------------------------------------------------------

#[test]
fn shortcut_edge_cases() {
    let output = convert(
        r#"gnoblin.configure { shortcuts = {
    a = { binding = "<Primary>Tab", command = { "x" } },
    b = { binding = "<Super><Shift>q", command = { "y", "--z" } },
    c = { binding = "<Super>Return", command = { "foot" }, trigger = "release" },
    d = { binding = "<Super>Button1", command = { "w" } },
    e = { binding = "<Super>Q", command = { "v" } },
    f = { binding = "<Shift><Super>q", command = { "dup" } },
    g = { binding = "<Super>t", command = "two words" },
    h = { binding = "<Super>u" },
    i = { command = { "nobinding" } },
    j = { binding = "AudioMute", command = { "m" } },
    k = { binding = "<Super>k", command = { "k" }, colour = "red" },
} }"#,
    );
    let ts = &output.typescript;
    assert!(
        ts.contains("input.bind(\"Ctrl+Tab\", () => {\n    gnoblin.spawn([\"x\"]);\n});"),
        "{ts}"
    );
    assert!(
        ts.contains("input.bind(\"Super+Shift+q\", () => {\n    gnoblin.spawn([\"y\", \"--z\"]);\n});"),
        "{ts}"
    );
    assert!(
        ts.contains("input.bind(\"XF86AudioMute\", () => {\n    gnoblin.spawn([\"m\"]);\n});"),
        "{ts}"
    );
    assert!(
        ts.contains("input.bind(\"Super+k\", () => {\n    gnoblin.spawn([\"k\"]);\n});"),
        "{ts}"
    );
    assert!(
        !ts.contains("gnoblin.spawn([\"foot\"])"),
        "a release shortcut is not a key: {ts}"
    );
    let reasons = reasons(&output);
    let has = |needle: &str| reasons.iter().any(|reason| reason.contains(needle));
    assert!(has("key release"), "{reasons:?}");
    assert!(has("pointer"), "{reasons:?}");
    assert!(has("upper-case"), "{reasons:?}");
    assert!(has("binds Shift+Super+q"), "the duplicate is reported: {reasons:?}");
    assert!(has("split by the shell"), "{reasons:?}");
    assert!(has("no command"), "{reasons:?}");
    assert!(has("no binding"), "{reasons:?}");
    assert!(has("`colour` is not read"), "{reasons:?}");
    assert_eq!(output.report.converted.keys, 4);
}

#[test]
fn a_later_file_changes_a_shortcut_of_the_same_name_field_by_field() {
    let output = convert_with(
        "gnoblin.configure { shortcuts = { t = { binding = \"<Super>t\", command = { \"foot\" } } } }\ngnoblin.load(\"more.lua\")\n",
        &[(
            "more.lua",
            "gnoblin.configure { shortcuts = { t = { command = { \"alacritty\" } } } }\n",
        )],
        None,
    );
    assert!(
        output.typescript.contains("gnoblin.spawn([\"alacritty\"]);"),
        "{}",
        output.typescript
    );
    assert!(!output.typescript.contains("foot"));
    let disabled = convert_with(
        "gnoblin.configure { shortcuts = { t = { binding = \"<Super>t\", command = { \"foot\" } } } }\ngnoblin.load(\"more.lua\")\n",
        &[(
            "more.lua",
            "gnoblin.configure { shortcuts = { t = { enable = false } } }\n",
        )],
        None,
    );
    assert_eq!(disabled.report.converted.keys, 0);
}

#[test]
fn a_keybindings_entry_that_is_a_function_is_left_out() {
    let output = convert("gnoblin.configure { keybindings = { close = \"<Super>q\", run = function() end } }\n");
    assert_eq!(output.report.unmapped.len(), 2, "{:?}", output.report.unmapped);
    assert!(output.report.unmapped[0].reason.contains("keybindings entry"));
    assert!(output.report.unmapped[0].reason.contains("`input.bind`"));
}

// --- Window rules ------------------------------------------------------------------------------------------------

#[test]
fn a_rule_that_only_sets_opacity_on_an_exact_match_is_a_selections_style() {
    let output = shell(
        "seat.data.window_rules = {\n    { match = { app_id = \"^foot$\" }, opacity = 0.8 },\n    { match = { title = \"^Notes$\" }, opacity = 1 },\n}",
    );
    let ts = &output.typescript;
    assert_eq!(output.report.unmapped, Vec::new(), "{ts}");
    assert!(
        ts.contains("// shell.lua:2\ngnoblin.clients.where({ appId: \"foot\" }).style.opacity = 0.8;"),
        "{ts}"
    );
    assert!(
        ts.contains("gnoblin.clients.where({ title: \"Notes\" }).style.opacity = 1;"),
        "{ts}"
    );
    assert!(!ts.contains("addListener"), "no listener is needed: {ts}");
    assert!(!ts.contains("CollectionListener"), "{ts}");
    assert_eq!(output.report.converted.rules, 2);
}

#[test]
fn opacity_stays_in_one_layer_when_any_rule_cannot_be_a_selection() {
    // A selection's style and a window's own style are two layers, so mixing them would let an earlier rule win.
    let output = shell(
        "seat.data.window_rules = {\n    { match = { app_id = \"^foot$\" }, opacity = 0.8 },\n    { match = { app_id = \"^foot\" }, opacity = 0.5 },\n}",
    );
    let ts = &output.typescript;
    assert!(!ts.contains("where("), "{ts}");
    assert!(
        ts.contains("if (client.appId === \"foot\") {\n                client.style.opacity = 0.8;"),
        "{ts}"
    );
    assert!(
        ts.contains("if (client.appId.startsWith(\"foot\")) {\n                client.style.opacity = 0.5;"),
        "{ts}"
    );
}

#[test]
fn a_glob_is_a_comparison_the_text_says() {
    let output = shell(
        "seat.data.window_rules = {\n    { match = { app_id = \"^org%.mozilla%.\" }, maximised = true },\n    { match = { title = \"Notes$\" }, maximised = true },\n    { match = { title = \"fire\" }, maximised = true },\n}",
    );
    let ts = &output.typescript;
    assert!(ts.contains("client.appId.startsWith(\"org.mozilla.\")"), "{ts}");
    assert!(ts.contains("client.title.endsWith(\"Notes\")"), "{ts}");
    assert!(ts.contains("client.title.includes(\"fire\")"), "{ts}");
}

#[test]
fn a_rule_that_matches_on_focus_or_workspace_is_left_out_whole() {
    let output = shell(
        "seat.data.window_rules = {\n    { match = { app_id = \"^a$\", focused = false }, maximised = true },\n    { match = { workspace_id = \"2\" }, maximised = true },\n}",
    );
    assert_eq!(places(&output), ["shell.lua:2", "shell.lua:3"]);
    assert!(
        output
            .report
            .unmapped
            .iter()
            .all(|item| item.reason.contains("a listener hears a window once")),
        "{:?}",
        output.report.unmapped
    );
    assert_eq!(output.report.converted.rules, 0);
    assert!(!output.typescript.contains("onAdded"), "{}", output.typescript);
}

#[test]
fn a_rule_with_a_pattern_that_has_no_equivalent_is_left_out_whole() {
    let output = shell(
        r#"seat.data.window_rules = {
    { match = { app_id = "%bxy" }, maximised = true },
    { match = { app_id = "^ok$" }, maximised = true },
    { match = { class = "x" }, maximised = true },
}"#,
    );
    assert_eq!(places(&output), ["shell.lua:2", "shell.lua:4"]);
    assert!(
        output.report.unmapped[0].reason.contains("%b"),
        "{:?}",
        output.report.unmapped
    );
    assert!(output.report.unmapped[0].reason.contains("rule at line 2"));
    assert!(output.report.unmapped[1].reason.contains("`class` has no equivalent"));
    assert_eq!(output.report.converted.rules, 1);
    assert!(output.typescript.contains("client.appId === \"ok\""));
    assert_eq!(
        output.typescript.matches("client.maximised = true;").count(),
        1,
        "a rule is never loosened:\n{}",
        output.typescript
    );
}

#[test]
fn a_rule_field_with_no_equivalent_only_narrows_the_rule_and_is_marked_in_the_file() {
    let output = shell(
        "seat.data.window_rules = {\n    { match = { app_id = \"^x$\" }, maximised = true, blur = true, opacity = 4 },\n}",
    );
    let ts = &output.typescript;
    assert_eq!(places(&output), ["shell.lua:2", "shell.lua:2"]);
    assert_eq!(output.report.converted.rules, 1);
    assert!(
        ts.contains("client.maximised = true;") && !ts.contains("client.blur"),
        "{ts}"
    );
    assert_eq!(ts.matches("// UNMAPPED (shell.lua:2)").count(), 2, "{ts}");
}

#[test]
fn secure_has_no_property_to_write_yet_and_the_rest_of_the_rule_stands() {
    let output =
        shell("seat.data.window_rules = {\n    { match = { title = \"^x$\" }, secure = true, fullscreen = false },\n}");
    assert_eq!(places(&output), ["shell.lua:2"]);
    let item = &output.report.unmapped[0];
    assert!(item.reason.contains("`secure` has no TypeScript home yet"), "{item:?}");
    assert_eq!(item.pointer, DROPPED);
    assert!(
        output.typescript.contains("client.fullscreen = false;"),
        "{}",
        output.typescript
    );
    assert_eq!(output.report.converted.rules, 1);
}

#[test]
fn a_rule_that_loses_all_its_actions_or_names_an_undeclared_workspace_is_left_out() {
    let output = shell(
        r#"seat.data.window_rules = {
    { match = { app_id = "^a$" }, blur = true },
    { match = { app_id = "^b$" }, workspace = { id = "nowhere" } },
    { match = { app_id = "^c$" }, workspace = { id = "2" } },
}"#,
    );
    assert_eq!(output.report.converted.rules, 1);
    assert!(
        output
            .report
            .unmapped
            .iter()
            .any(|item| item.reason.contains("does not declare")),
        "{:?}",
        output.report.unmapped
    );
    assert!(
        output
            .report
            .unmapped
            .iter()
            .any(|item| item.reason.contains("sets nothing")),
        "{:?}",
        output.report.unmapped
    );
}

#[test]
fn the_workspaces_a_config_declares_are_the_ones_a_rule_may_name() {
    let output = convert_with(
        "gnoblin.configure { workspaces = { { id = \"code\", name = \"Code\" } } }\n",
        &[],
        Some("seat.data.window_rules = { { match = { app_id = \"^e$\" }, workspace = { id = \"code\" } } }\n"),
    );
    assert_eq!(output.report.unmapped, Vec::new());
    assert_eq!(output.report.converted.rules, 1);
}

// --- What it cannot convert --------------------------------------------------------------------------------------

const UNMAPPED: &str = r#"local home = os.getenv("HOME")
gnoblin.configure({
    cursor = { size = 24, theme = compute_theme() },
    ["window-management"] = { ["focus-mode"] = "click" },
})
gnoblin.on("gnoblin.window.created", function(w)
    w:maximise()
end)
seat:bind({ ["<Super>x"] = function() end })
for i = 1, 3 do
    gnoblin.configure({ cursor = { size = i } })
end
if home then
    gnoblin.configure({ cursor = { size = 1 } })
end
local function helper() end
gnoblin.timers.after(1, helper)
require("extra")
gnoblin.configure(build())
"#;

#[test]
fn every_unmapped_construct_is_in_the_output_and_the_report() {
    let output = convert(UNMAPPED);
    assert_eq!(
        places(&output),
        [
            "init.lua:1",
            "init.lua:3",
            "init.lua:3",
            "init.lua:4",
            "init.lua:6",
            "init.lua:9",
            "init.lua:10",
            "init.lua:13",
            "init.lua:16",
            "init.lua:17",
            "init.lua:18",
            "init.lua:19",
        ],
        "{}",
        output.typescript
    );
    let ts = &output.typescript;
    assert_eq!(
        ts.matches("// UNMAPPED (init.lua:").count(),
        output.report.unmapped.len(),
        "{ts}"
    );
    for needle in [
        "// UNMAPPED (init.lua:1): local home = os.getenv(\"HOME\")",
        "// UNMAPPED (init.lua:3): size = 24",
        "// UNMAPPED (init.lua:3): theme = compute_theme()",
        "// UNMAPPED (init.lua:4): [\"focus-mode\"] = \"click\"",
        "// UNMAPPED (init.lua:6): gnoblin.on(\"gnoblin.window.created\", function(w)",
        "//     w:maximise()",
        "// end)",
        "// UNMAPPED (init.lua:9): seat:bind({ [\"<Super>x\"] = function() end })",
        "// UNMAPPED (init.lua:10): for i = 1, 3 do",
        "// UNMAPPED (init.lua:13): if home then",
        "// UNMAPPED (init.lua:16): local function helper() end",
        "// UNMAPPED (init.lua:17): gnoblin.timers.after(1, helper)",
        "// UNMAPPED (init.lua:18): require(\"extra\")",
        "// UNMAPPED (init.lua:19): gnoblin.configure(build())",
    ] {
        assert!(ts.contains(needle), "{needle}\n{ts}");
    }
    for item in &output.report.unmapped {
        assert!(item.pointer.starts_with("docs/migrating-from-lua.md#"), "{item:?}");
        assert!(!item.reason.is_empty() && !item.summary.is_empty(), "{item:?}");
    }
}

#[test]
fn a_computed_value_costs_only_its_own_field() {
    let output = convert("gnoblin.configure({ workspaces = { { id = \"a\", name = tonumber(\"1\") } } })");
    assert_eq!(places(&output), ["init.lua:1"]);
    assert_eq!(output.report.unmapped[0].summary, "name = tonumber(\"1\")");
    assert!(
        output
            .typescript
            .contains("gnoblin.workspaces.add(new Workspace({ id: \"a\", layout: new FloatingLayout() }));"),
        "{}",
        output.typescript
    );
}

#[test]
fn a_computed_local_is_reported_where_it_is_declared_and_where_it_is_used() {
    let output = convert("local size = os.time()\ngnoblin.configure { cursor = { size = size } }\n");
    assert_eq!(places(&output), ["init.lua:1", "init.lua:2"]);
}

#[test]
fn a_syntax_error_in_the_entry_file_is_an_error_and_in_another_file_is_one_item() {
    let entry = MigrateSource {
        name: "init.lua".to_owned(),
        text: "gnoblin.configure {\n    cursor = = 1\n}\n".to_owned(),
    };
    let error = migrate(&entry, &[], &MapLoader::new(&[])).expect_err("a syntax error");
    let MigrateError::Syntax { file, line, .. } = &error;
    assert_eq!((file.as_str(), *line), ("init.lua", 2));
    assert!(error.to_string().starts_with("init.lua:2:"), "{error}");

    let output = convert_with(
        "gnoblin.load(\"bad.lua\")\ngnoblin.configure { workspaces = { { id = \"a\", name = \"A\" } } }\n",
        &[("bad.lua", "x = = 1\n")],
        None,
    );
    assert_eq!(places(&output), ["bad.lua:1"]);
    assert!(output.report.unmapped[0].reason.contains("does not parse"));
    assert!(output.typescript.contains("new Workspace({ id: \"a\""));
}

#[test]
fn text_that_is_not_data_never_reaches_the_output_unmarked() {
    // A function body or a loop copied into the output would be code in a config; only comments may carry it.
    let output = convert(UNMAPPED);
    for line in output.typescript.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        for code in [
            "os.getenv",
            "function(",
            "compute_theme",
            "require(",
            "timers",
            "w:maximise",
            "helper",
        ] {
            assert!(!line.contains(code), "{line:?} carries Lua outside a comment");
        }
    }
}

#[test]
fn every_output_is_statements_against_objects() {
    let outputs = [
        convert_with(STARTER_INIT, STARTER_FILES, None),
        convert_with(EVERYTHING, &[], Some(EVERYTHING_SHELL)),
        convert(NO_HOME),
        convert(UNMAPPED),
    ];
    for output in &outputs {
        assert!(
            !output.typescript.contains("export default"),
            "the output is a program, not a default export:\n{}",
            output.typescript
        );
        // Every statement the converter writes is against an object: no `spawn(` action helper, no `keys:` table.
        assert!(
            !output.typescript.contains("\nkeys:") && !output.typescript.contains(": spawn("),
            "{}",
            output.typescript
        );
    }
}
