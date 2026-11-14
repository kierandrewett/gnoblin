//! The converter over whole configs: the shipped starter, a config of every construct, a config of constructs it cannot
//! convert, the names, the shortcuts and the rules. Nothing here runs Lua or the TypeScript runtime; the `gnoblinctl`
//! tests load the output with the runtime's own `defineConfig`.

use std::collections::BTreeMap;

use gnoblin_config_migrate::{
    ConfigSourceLoader, MigrateError, MigrateSource, MigrationOutput, glob_matches, is_glob, migrate, resolve_load_path,
};

/// The Lua starter as it shipped, `data/default-config` at the time the converter was written.
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

// --- The shipped starter -----------------------------------------------------------------------------------------

#[test]
fn the_starter_converts_with_nothing_left_out() {
    let output = convert_with(STARTER_INIT, STARTER_FILES, None);
    assert_eq!(output.report.unmapped, Vec::new());
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
    assert_eq!(counts.settings, 19);
    assert_eq!(counts.workspaces, 4);
    assert_eq!(counts.keys, 7);
    assert_eq!(
        counts.autostart + counts.rules + counts.workspace_sets + counts.options,
        0
    );
    assert!(!output.typescript.contains("UNMAPPED"));
}

#[test]
fn the_starter_is_this_typescript() {
    let output = convert_with(STARTER_INIT, STARTER_FILES, None);
    assert_eq!(output.typescript, include_str!("fixtures/starter/init.ts.golden"));
}

#[test]
fn the_starter_copy_matches_the_shipped_one_while_that_exists() {
    // `data/default-config` goes with the Lua shell; until then the fixture must not drift from it.
    let shipped = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/default-config");
    if !shipped.is_dir() {
        return;
    }
    let read = |name: &str| std::fs::read_to_string(shipped.join(name)).expect("a shipped starter file");
    assert_eq!(read("init.lua"), STARTER_INIT);
    for (name, text) in STARTER_FILES {
        assert_eq!(&read(name), text, "{name}");
    }
}

// --- Every construct that converts -------------------------------------------------------------------------------

const EVERYTHING: &str = r#"
-- Sections, kebab and plain, nested tables, every scalar.
local delay = 5 * 100
gnoblin.configure {
    ["window-management"] = {
        ["focus-mode"] = "hover",
        ["auto-raise"] = true,
        ["auto-raise-delay"] = delay,
        ["edge-tiling"] = false,
    },
    cursor = { size = 28, theme = "Adwaita" .. "-dark" },
    protocols = { ["wlr-screencopy"] = false },
    ["touchpad-gestures"] = {
        { name = "term", gesture = "swipe", fingers = 3, path = { { x = 0, y = 0 }, { x = 1, y = 0 } }, command = { "foot" } },
    },
}
gnoblin.configure({
    workspaces = {
        { id = "web", name = "Web" },
        { id = "dev", name = "Development" },
    },
    autostart = {
        bar = { command = { "waybar" } },
        agent = { command = { "foot", "--server" }, when = "on_login" },
        off = { command = { "never" }, enable = false },
    },
    shortcuts = {
        terminal = { binding = "<Super>Return", command = { "foot" } },
        browser = { binding = "<Super><Shift>b", command = "firefox" },
        mute = { binding = "XF86AudioMute", command = { "wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle" } },
        off = { binding = "<Super>x", command = { "xterm" }, enable = false },
    },
})
"#;

#[test]
fn a_config_of_every_construct_converts_with_nothing_left_out() {
    let output = convert(EVERYTHING);
    assert_eq!(output.report.unmapped, Vec::new(), "{}", output.typescript);
    let ts = &output.typescript;
    assert!(ts.contains("import { defineConfig, spawn } from \"gnoblin\";"), "{ts}");
    assert!(ts.contains("export default defineConfig({"), "{ts}");
    assert!(ts.contains("windowManagement: {"), "{ts}");
    assert!(ts.contains("focusMode: \"hover\","), "{ts}");
    assert!(ts.contains("autoRaiseDelay: 500,"), "{ts}");
    assert!(ts.contains("edgeTiling: false,"), "{ts}");
    assert!(ts.contains("theme: \"Adwaita-dark\","), "{ts}");
    assert!(ts.contains("protocols: {\n        wlrScreencopy: false,"), "{ts}");
    assert!(ts.contains("touchpadGestures: ["), "{ts}");
    assert!(ts.contains("{ id: \"web\", name: \"Web\" }"), "{ts}");
    assert!(ts.contains("\"Super+Return\": spawn(\"foot\"),"), "{ts}");
    assert!(ts.contains("\"Super+Shift+b\": spawn(\"firefox\"),"), "{ts}");
    assert!(
        ts.contains("XF86AudioMute: spawn(\"wpctl\", \"set-mute\", \"@DEFAULT_AUDIO_SINK@\", \"toggle\"),"),
        "{ts}"
    );
    assert!(
        !ts.contains("xterm") && !ts.contains("never"),
        "a disabled entry is left out: {ts}"
    );
    // Autostart is read in name order, as the document holds it; a one-word command is a plain string.
    let agent = ts.find("[\"foot\", \"--server\"]").expect("the argv");
    let bar = ts.find("\"waybar\"").expect("the program");
    assert!(agent < bar, "{ts}");
    let counts = &output.report.converted;
    assert_eq!((counts.workspaces, counts.keys, counts.autostart), (2, 3, 2));
    assert_eq!(
        output.report.notes.len(),
        1,
        "the disabled shortcut is noted: {:?}",
        output.report.notes
    );
}

#[test]
fn a_comment_names_each_file_the_config_came_from() {
    let output = convert_with(STARTER_INIT, STARTER_FILES, None);
    for (name, _) in STARTER_FILES {
        assert!(
            output.typescript.contains(&format!("// From {name}.")),
            "{name}\n{}",
            output.typescript
        );
    }
    assert!(
        output
            .typescript
            .contains("// Sources, in load order: init.lua, config/10-windows.lua")
    );
}

#[test]
fn several_configure_calls_and_files_merge_like_the_documents_layers() {
    let first = "gnoblin.configure { cursor = { size = 24, theme = \"a\" }, workspaces = { { id = \"1\", name = \"One\" } } }\ngnoblin.load(\"more.lua\")\n";
    let more = "gnoblin.configure { cursor = { size = 48 }, workspaces = { { id = \"2\", name = \"Two\" } } }\n";
    let output = convert_with(first, &[("more.lua", more)], None);
    let ts = &output.typescript;
    assert!(
        ts.contains("size: 48,") && ts.contains("theme: \"a\","),
        "tables merge by key: {ts}"
    );
    assert!(
        ts.contains("id: \"2\"") && !ts.contains("id: \"1\""),
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
            ("config/sub/b.lua", "gnoblin.configure { cursor = { size = 30 } }\n"),
        ],
        None,
    );
    assert_eq!(output.report.unmapped, Vec::new());
    assert!(output.typescript.contains("size: 30,"));
    assert_eq!(output.report.sources, ["init.lua", "config/a.lua", "config/sub/b.lua"]);
}

// --- Names -------------------------------------------------------------------------------------------------------

#[test]
fn kebab_and_camel_keys_both_come_out_as_the_name_the_runtime_reads_back() {
    let output = convert(
        r#"gnoblin.configure {
            ["window-management"] = { ["action-double-click-titlebar"] = "toggle-maximize" },
            ["layer-shell"] = { ["preserve-active-window"] = true },
            ["input-sources"] = {},
            appearance = { ["color-scheme"] = "prefer-dark" },
            input = { ["pointing-stick"] = {}, touchpad = { ["tap-to-click"] = true } },
        }"#,
    );
    let ts = &output.typescript;
    assert_eq!(output.report.unmapped, Vec::new(), "{ts}");
    for expected in [
        "windowManagement: {",
        "actionDoubleClickTitlebar: \"toggle-maximize\",",
        "layerShell: {",
        "preserveActiveWindow: true,",
        "inputSources: {},",
        "appearance: {",
        "colorScheme: \"prefer-dark\",",
        "pointingStick: {},",
        "tapToClick: true,",
    ] {
        assert!(ts.contains(expected), "{expected}\n{ts}");
    }
    assert!(
        !ts.contains("window-management") && !ts.contains("preserve-active-window"),
        "{ts}"
    );
}

#[test]
fn a_key_that_would_not_read_back_keeps_its_spelling_and_a_capital_is_noted() {
    let output = convert(r#"gnoblin.configure { monitors = { ["DP-1"] = { scale = 2 }, ["edge_x"] = 1 } }"#);
    let ts = &output.typescript;
    assert!(ts.contains("\"DP-1\": {"), "{ts}");
    assert!(ts.contains("edge_x: 1,"), "{ts}");
    assert_eq!(output.report.notes.len(), 1, "{:?}", output.report.notes);
    assert!(output.report.notes[0].contains("DP-1") && output.report.notes[0].contains("lower case"));
}

#[test]
fn an_unknown_or_carried_section_is_left_out_with_a_reason() {
    let output = convert(
        "gnoblin.configure {\n    cursor = { size = 24 },\n    animations = { x = 1 },\n    typo = {},\n    shell = \"x\",\n}\n",
    );
    assert_eq!(places(&output), ["init.lua:3", "init.lua:4", "init.lua:5"]);
    assert!(output.report.unmapped[0].reason.contains("read by nothing"));
    assert!(output.report.unmapped[1].reason.contains("not a section"));
    assert!(output.report.unmapped[2].reason.contains("init.ts"));
    assert!(output.typescript.contains("size: 24,"));
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
    assert!(ts.contains("\"Ctrl+Tab\": spawn(\"x\"),"), "{ts}");
    assert!(ts.contains("\"Super+Shift+q\": spawn(\"y\", \"--z\"),"), "{ts}");
    assert!(ts.contains("XF86AudioMute: spawn(\"m\"),"), "{ts}");
    assert!(ts.contains("\"Super+k\": spawn(\"k\"),"), "{ts}");
    assert!(
        !ts.contains("foot") || ts.contains("// "),
        "a release shortcut is not a key: {ts}"
    );
    let reasons: Vec<&str> = output.report.unmapped.iter().map(|item| item.reason.as_str()).collect();
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
        output.typescript.contains("\"Super+t\": spawn(\"alacritty\"),"),
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
}

// --- Window rules ------------------------------------------------------------------------------------------------

#[test]
fn window_rules_in_a_shell_file_become_rules() {
    let output = shell(
        r#"seat.data.window_rules = {
    { match = { app_id = "^org%.example%.Editor$" }, workspace = { id = "3" }, maximised = true },
    { match = { app_id = "^pavucontrol$", focused = false }, opacity = 0.9 },
    { match = { title = "Picture.in.Picture" }, secure = true, fullscreen = false },
    { match = { workspace_id = "2" }, maximised = false },
    { match = { app_id = "^org%.x%.(%w+)$" }, workspace = "1" },
}"#,
    );
    let ts = &output.typescript;
    assert_eq!(output.report.unmapped, Vec::new(), "{ts}");
    assert!(ts.contains("appId: \"org.example.Editor\","), "{ts}");
    assert!(ts.contains("workspace: \"3\","), "{ts}");
    assert!(ts.contains("maximised: true"), "{ts}");
    assert!(ts.contains("style: {\n"), "{ts}");
    assert!(ts.contains("opacity: 0.9,"), "{ts}");
    assert!(ts.contains("title: /Picture.in.Picture/s,"), "{ts}");
    assert!(ts.contains("onWorkspace: \"2\","), "{ts}");
    assert!(ts.contains("appId: /^org\\.x\\.[A-Za-z0-9]+$/,"), "{ts}");
    assert!(ts.contains("// From shell.lua."), "{ts}");
    assert_eq!(output.report.converted.rules, 5);
    assert_eq!(output.report.sources, ["init.lua", "shell.lua"]);
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
    assert!(output.typescript.contains("appId: \"ok\","));
    assert_eq!(
        output.typescript.matches("maximised: true").count(),
        1,
        "a rule is never loosened:\n{}",
        output.typescript
    );
}

#[test]
fn a_rule_field_with_no_equivalent_only_narrows_the_rule() {
    let output = shell(
        "seat.data.window_rules = {\n    { match = { app_id = \"^x$\" }, maximised = true, blur = true, opacity = 4 },\n}",
    );
    assert_eq!(places(&output), ["shell.lua:2", "shell.lua:2"]);
    assert_eq!(output.report.converted.rules, 1);
    assert!(output.typescript.contains("maximised: true") && !output.typescript.contains("blur:"));
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

#[test]
fn seat_settings_of_a_shell_file_become_their_keys() {
    let output = shell(
        "seat.data.alerts = false\nseat.edge_tiling = true\nseat.drag_constraint = \"minimum_strip\"\nseat.data.prompts = false\nseat.drag_constraint = \"other\"\n",
    );
    let ts = &output.typescript;
    assert!(
        ts.contains("alerts: false,")
            && ts.contains("edgeTiling: true,")
            && ts.contains("dragConstraint: \"minimum_strip\","),
        "{ts}"
    );
    assert_eq!(places(&output), ["shell.lua:4", "shell.lua:5"]);
    assert_eq!(output.report.converted.options, 3);
}

#[test]
fn workspace_sets_are_checked_against_the_declared_workspaces() {
    let output = convert_with(
        "gnoblin.configure { workspaces = { { id = \"a\", name = \"A\" }, { id = \"b\", name = \"B\" } } }\n",
        &[],
        Some(
            r#"seat.data.workspace_sets = {
    { name = "work", entries = { { workspace = "a", output = "DP-1" }, { workspace = "b" } } },
    { name = "bad", entries = { { workspace = "zzz" } } },
}"#,
        ),
    );
    assert_eq!(output.report.converted.workspace_sets, 1);
    assert!(output.typescript.contains("workspaceSets: ["), "{}", output.typescript);
    assert!(output.typescript.contains("output: \"DP-1\""), "{}", output.typescript);
    assert_eq!(places(&output), ["shell.lua:3"]);
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
        ts.matches("UNMAPPED (init.lua:").count(),
        output.report.unmapped.len(),
        "{ts}"
    );
    for needle in [
        "// UNMAPPED (init.lua:1): local home = os.getenv(\"HOME\")",
        "// UNMAPPED (init.lua:3): theme = compute_theme()",
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
    // The settings around the computed one still convert, and nothing inside a left-out block leaks out.
    assert!(ts.contains("size: 24,") && ts.contains("focusMode: \"click\","), "{ts}");
    assert!(!ts.contains("size: 1,") && !ts.contains("maximise()\n    }"), "{ts}");
    for item in &output.report.unmapped {
        assert!(item.pointer.starts_with("docs/migrating-from-lua.md#"), "{item:?}");
        assert!(!item.reason.is_empty() && !item.summary.is_empty(), "{item:?}");
    }
}

#[test]
fn a_computed_value_costs_only_its_own_field() {
    let output = convert("gnoblin.configure({ cursor = { size = tonumber(\"24\"), theme = \"x\" } })");
    assert_eq!(places(&output), ["init.lua:1"]);
    assert_eq!(output.report.unmapped[0].summary, "size = tonumber(\"24\")");
    assert!(output.typescript.contains("theme: \"x\","));
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
        "gnoblin.load(\"bad.lua\")\ngnoblin.configure { cursor = { size = 24 } }\n",
        &[("bad.lua", "x = = 1\n")],
        None,
    );
    assert_eq!(places(&output), ["bad.lua:1"]);
    assert!(output.report.unmapped[0].reason.contains("does not parse"));
    assert!(output.typescript.contains("size: 24,"));
}

#[test]
fn text_that_is_not_data_never_reaches_the_output_unmarked() {
    // A function body or a loop copied into the output would be code in a config; only comments may carry it.
    let output = convert(UNMAPPED);
    for line in output.typescript.lines() {
        let line = line.trim();
        assert!(
            line.is_empty()
                || line.starts_with("//")
                || line.starts_with("import ")
                || line.starts_with("export ")
                || line.ends_with('{')
                || line.ends_with(',')
                || line.ends_with("});")
                || line == "}"
                || line == "},",
            "{line:?} is not data or a comment"
        );
    }
}

#[test]
fn an_empty_config_converts_to_an_empty_define_config() {
    let output = convert("-- nothing\n");
    assert!(
        output.typescript.contains("export default defineConfig({\n});"),
        "{}",
        output.typescript
    );
    assert!(output.typescript.contains("import { defineConfig } from \"gnoblin\";"));
    assert_eq!(output.report.unmapped, Vec::new());
}
