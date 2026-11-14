//! `gnoblinctl config migrate`, driven through the real parse, the real command and the real exit codes on temporary
//! directories. The converted file is loaded with the shell's own stock `defineConfig`, and its plan is compared with
//! the document and the bindings the Lua starter makes. Nothing here runs Lua: the Lua starter's document is listed by
//! hand from `data/default-config`, which is the file the converter was written against.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_js::{JsConfig, JsGeneration, STARTER_CONFIG, stock_modules};
use gnoblinctl::{ConnectFailure, Connection};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A connection that was never meant to be opened: the command needs no session.
struct NoSession;

impl Connection for NoSession {
    type Transport = UnixTransport;

    fn connect(self) -> Result<Client<UnixTransport>, ConnectFailure> {
        Err(ConnectFailure::Socket {
            path: None,
            reason: "this command needs no session".to_owned(),
        })
    }
}

fn run(args: &[&str]) -> (u8, String, String) {
    run_os(&args.iter().map(OsString::from).collect::<Vec<_>>())
}

fn run_os(args: &[OsString]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = gnoblinctl::run(args.iter().cloned(), NoSession, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("the command writes UTF-8"),
        String::from_utf8(err).expect("the command writes UTF-8"),
    )
}

/// `config migrate FILE` and the extra arguments.
fn migrate(file: &Path, extra: &[&str]) -> (u8, String, String) {
    let mut args = vec![
        OsString::from("config"),
        OsString::from("migrate"),
        file.as_os_str().to_owned(),
    ];
    args.extend(extra.iter().map(OsString::from));
    run_os(&args)
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the parent directory");
    }
    fs::write(&path, text).expect("write the fixture");
    path
}

/// The Lua starter as the converter's own tests keep it.
fn starter_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gnoblin-config-migrate/tests/fixtures/starter")
}

/// A copy of the Lua starter in a directory of its own.
fn copied_starter() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("a temp dir");
    for name in [
        "init.lua",
        "config/10-windows.lua",
        "config/20-input.lua",
        "config/30-session.lua",
        "config/40-shortcuts.lua",
    ] {
        let text = fs::read_to_string(starter_dir().join(name)).expect("a starter file");
        write(dir.path(), name, &text);
    }
    let init = dir.path().join("init.lua");
    (dir, init)
}

// --- Loading the converted file ---------------------------------------------------------------------------------

/// Evaluates `init.ts` in `dir` with the stock library and a host that registers nothing, and returns what `probe`
/// leaves in `globalThis.out` as JSON. `defineConfig` runs every check it runs in a session, so a file that loads is
/// a file the shell accepts.
fn evaluate(dir: &Path, probe: &str) -> Value {
    let root = fs::canonicalize(dir).expect("the directory exists");
    fs::write(
        root.join("probe.ts"),
        format!(
            "import \"harness\";\nimport config from \"./init.ts\";\nimport {{ desugarConfig, mergeConfig, defaultConfig }} from \"gnoblin\";\n\
             const replacer = (_key: string, value: unknown) => value instanceof RegExp ? String(value) : value;\n{probe}\n"
        ),
    )
    .expect("write the probe");
    let mut jsconfig = JsConfig::new(&root).embed("harness", "globalThis.gnoblin = { __register() {} };");
    for (name, source) in stock_modules() {
        jsconfig = jsconfig.embed(name, source);
    }
    let generation = JsGeneration::new(&jsconfig, 1).expect("a generation");
    generation
        .eval_module(Path::new("probe.ts"))
        .unwrap_or_else(|error| panic!("the converted file does not load: {error:?}"));
    let text = generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, String>("out"))
        .expect("the probe set out");
    serde_json::from_str(&text).expect("the probe wrote JSON")
}

/// The plan of the config on its own: the document patch, the workspaces, the bindings, the rules, autostart.
fn plan(dir: &Path) -> Value {
    evaluate(
        dir,
        "const plan = desugarConfig(config);\n\
         globalThis.out = JSON.stringify({\n\
             document: plan.document,\n\
             workspaces: plan.workspaces.map((w) => ({ id: w.id, name: w.name })),\n\
             bindings: plan.bindings.map((b) => ({ trigger: b.trigger, action: b.action })),\n\
             rules: plan.rules,\n\
             autostart: plan.autostart,\n\
             alerts: plan.alerts,\n\
             workspaceSets: plan.workspaceSets,\n\
         }, replacer);",
    )
}

/// The effective bindings and workspaces: the config laid over the stock config, as the shell runs it.
fn effective(dir: &Path) -> Value {
    evaluate(
        dir,
        "const plan = desugarConfig(mergeConfig(defaultConfig, config));\n\
         const bindings = plan.bindings.map((b) => ({ trigger: b.trigger, action: b.action }));\n\
         bindings.sort((a, b) => a.trigger < b.trigger ? -1 : 1);\n\
         globalThis.out = JSON.stringify({\n\
             bindings,\n\
             workspaces: plan.workspaces.map((w) => ({ id: w.id, name: w.name })),\n\
         }, replacer);",
    )
}

/// The document the Lua starter makes: `data/default-config/config/{10,20,30}-*.lua` listed by hand, sections as the
/// document names them.
fn lua_starter_document() -> Value {
    json!({
        "window-management": {
            "focus-mode": "click",
            "focus-new-windows": "allow",
            "raise-on-click": true,
            "auto-raise": false,
            "auto-raise-delay": 500,
            "check-alive-timeout": 5000,
            "focus-change-on-pointer-rest": false,
            "edge-tiling": true,
            "attach-modal-dialogs": true,
            "constrain-drag-to-work-area": true,
            "action-double-click-titlebar": "toggle-maximize",
            "action-middle-click-titlebar": "none",
            "action-right-click-titlebar": "menu",
        },
        "cursor": {},
        "input": {
            "mouse": {},
            "touchpad": {},
            "trackball": {},
            "pointing-stick": {},
            "keyboard": {},
        },
        "compositor": {
            "locate-pointer": false,
            "locate-pointer-key": "Control_L",
            "visual-bell": false,
            "audible-bell": true,
            "visual-bell-type": "fullscreen-flash",
        },
        "protocols": {},
        "layer-shell": {
            "preserve-active-window": true,
        },
    })
}

/// The seven shortcuts of `40-shortcuts.lua`: binding and command, in the converted spelling of the binding.
fn lua_starter_shortcuts() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "XF86AudioRaiseVolume",
            vec!["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
        ),
        (
            "XF86AudioLowerVolume",
            vec!["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
        ),
        (
            "XF86AudioMute",
            vec!["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
        ),
        (
            "XF86AudioMicMute",
            vec!["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
        ),
        ("XF86AudioPlay", vec!["playerctl", "play-pause"]),
        ("XF86AudioNext", vec!["playerctl", "next"]),
        ("XF86AudioPrev", vec!["playerctl", "previous"]),
    ]
}

// --- The shipped starter ----------------------------------------------------------------------------------------

#[test]
fn the_starter_converts_to_a_config_that_loads_and_makes_the_lua_starters_document_and_bindings() {
    let (dir, init) = copied_starter();
    let (code, out, err) = migrate(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    let ts = dir.path().join("init.ts");
    assert!(ts.is_file(), "init.ts is written beside init.lua");

    // (a) the in-process check the command line has: the module graph, and `defineConfig`'s own validation.
    let (check_code, check_out, check_err) = run(&["check", ts.to_str().expect("UTF-8")]);
    assert_eq!(check_code, 0, "{check_out}{check_err}");
    let (types_code, types_out, types_err) = run(&["check", ts.to_str().expect("UTF-8"), "--types"]);
    assert_eq!(types_code, 0, "{types_out}{types_err}");

    // (b) the plan is the Lua starter's: its document, its four workspaces and its seven shortcuts.
    let planned = plan(dir.path());
    assert_eq!(planned["document"], lua_starter_document());
    let workspaces: Vec<(String, String)> = planned["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .map(|w| {
            (
                w["id"].as_str().unwrap_or_default().to_owned(),
                w["name"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = (1..=4).map(|n| (n.to_string(), format!("Workspace {n}"))).collect();
    assert_eq!(workspaces, expected);
    let mut bindings: Vec<(String, Vec<String>)> = planned["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .map(|b| {
            assert_eq!(b["action"]["kind"], "spawn", "{b}");
            let argv = b["action"]["argv"]
                .as_array()
                .expect("argv")
                .iter()
                .map(|a| a.as_str().unwrap_or_default().to_owned())
                .collect();
            (b["trigger"].as_str().unwrap_or_default().to_owned(), argv)
        })
        .collect();
    bindings.sort();
    let mut shortcuts: Vec<(String, Vec<String>)> = lua_starter_shortcuts()
        .into_iter()
        .map(|(trigger, argv)| (trigger.to_owned(), argv.into_iter().map(str::to_owned).collect()))
        .collect();
    shortcuts.sort();
    assert_eq!(bindings, shortcuts);
    assert_eq!(planned["rules"], json!([]));
    assert_eq!(planned["autostart"], json!([]));
}

#[test]
fn the_converted_starter_runs_the_same_bindings_and_workspaces_as_the_typescript_starter() {
    let (dir, init) = copied_starter();
    assert_eq!(migrate(&init, &[]).0, 0);
    let starter = TempDir::new().expect("a temp dir");
    write(starter.path(), "init.ts", STARTER_CONFIG);
    // The TypeScript starter is the stock config as written, so what the session runs for it is the stock config.
    assert_eq!(effective(dir.path()), effective(starter.path()));
}

#[test]
fn the_converted_starter_is_the_converters_golden() {
    let (dir, init) = copied_starter();
    assert_eq!(migrate(&init, &[]).0, 0);
    let golden = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../gnoblin-config-migrate/tests/fixtures/starter/init.ts.golden"),
    )
    .expect("the golden file");
    assert_eq!(fs::read_to_string(dir.path().join("init.ts")).expect("init.ts"), golden);
}

// --- Everything that converts -----------------------------------------------------------------------------------

const EVERYTHING: &str = r#"
gnoblin.configure {
    ["window-management"] = { ["focus-mode"] = "hover", ["auto-raise-delay"] = 5 * 100 },
    cursor = { size = 28 },
    protocols = { ["wlr-screencopy"] = false },
    workspaces = { { id = "web", name = "Web" }, { id = "dev", name = "Development" } },
    autostart = {
        bar = { command = { "waybar" } },
        agent = { command = { "foot", "--server" }, when = "on_login" },
        off = { command = { "never" }, enable = false },
    },
    shortcuts = {
        terminal = { binding = "<Super>Return", command = { "foot" } },
        browser = { binding = "<Super><Shift>b", command = "firefox" },
        mute = { binding = "XF86AudioMute", command = { "wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle" } },
    },
    ["touchpad-gestures"] = {
        { name = "term", gesture = "swipe", fingers = 3, path = { { x = 0, y = 0 }, { x = 1, y = 0 } }, command = { "foot" } },
    },
}
"#;

const EVERYTHING_SHELL: &str = r#"
seat.data.window_rules = {
    { match = { app_id = "^org%.example%.Editor$" }, workspace = { id = "dev" }, maximised = true },
    { match = { title = "Picture.in.Picture" }, secure = true },
    { match = { app_id = "^org%.x%.(%w+)$", focused = false }, opacity = 0.9 },
}
seat.data.alerts = false
seat.edge_tiling = true
seat.drag_constraint = "minimum_strip"
seat.data.workspace_sets = { { name = "work", entries = { { workspace = "web" }, { workspace = "dev", output = "DP-1" } } } }
"#;

#[test]
fn a_config_of_every_construct_loads_with_defineconfig_and_means_what_the_lua_said() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", EVERYTHING);
    write(dir.path(), "shell.lua", EVERYTHING_SHELL);
    let (code, out, err) = migrate(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    let planned = plan(dir.path());

    assert_eq!(
        planned["document"]["window-management"],
        json!({ "focus-mode": "hover", "auto-raise-delay": 500 })
    );
    assert_eq!(planned["document"]["cursor"], json!({ "size": 28 }));
    assert_eq!(planned["document"]["protocols"], json!({ "wlr-screencopy": false }));
    assert_eq!(planned["document"]["touchpad-gestures"][0]["name"], "term");
    assert_eq!(
        planned["document"]["touchpad-gestures"][0]["path"],
        json!([{ "x": 0, "y": 0 }, { "x": 1, "y": 0 }])
    );
    assert_eq!(
        planned["workspaces"],
        json!([{ "id": "web", "name": "Web" }, { "id": "dev", "name": "Development" }])
    );
    assert_eq!(planned["autostart"], json!([["foot", "--server"], ["waybar"]]));
    assert_eq!(planned["alerts"], json!(false));

    let triggers: Vec<(&str, Value)> = planned["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .map(|b| (b["trigger"].as_str().unwrap_or_default(), b["action"]["argv"].clone()))
        .collect();
    assert_eq!(
        triggers,
        [
            ("Super+Return", json!(["foot"])),
            ("Super+Shift+b", json!(["firefox"])),
            (
                "XF86AudioMute",
                json!(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
            ),
        ]
    );

    let rules = planned["rules"].as_array().expect("rules");
    assert_eq!(rules.len(), 3);
    assert_eq!(
        rules[0],
        json!({ "appId": "org.example.Editor", "workspace": "dev", "maximised": true })
    );
    assert_eq!(rules[1], json!({ "title": "/Picture.in.Picture/s", "secure": true }));
    assert_eq!(rules[2]["appId"], "/^org\\.x\\.[A-Za-z0-9]+$/");
    assert_eq!(rules[2]["focused"], false);
    assert_eq!(rules[2]["style"], json!({ "opacity": 0.9 }));
    assert_eq!(
        planned["workspaceSets"],
        json!([{ "name": "work", "entries": [{ "workspace": "web" }, { "workspace": "dev", "output": "DP-1" }] }])
    );
}

// --- What it cannot convert -------------------------------------------------------------------------------------

const UNMAPPED_INIT: &str = r#"gnoblin.configure({
    cursor = { size = 24, theme = compute_theme() },
})
gnoblin.on("gnoblin.window.created", function(w)
    w:maximise()
end)
gnoblin.configure({
    shortcuts = {
        a = { binding = "<Super>a", command = { "foot" } },
        b = { binding = "<Super>Return", command = { "x" }, trigger = "release" },
        c = { binding = "<Super>a", command = { "dup" } },
    },
})
for i = 1, 3 do
    gnoblin.configure({ cursor = { size = i } })
end
"#;

#[test]
fn unmapped_constructs_exit_1_are_listed_with_their_lines_and_the_output_still_loads() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", UNMAPPED_INIT);
    let (code, out, err) = migrate(&init, &[]);
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    for needle in [
        "ok  init.ts written",
        "Converted",
        "Left out (5)",
        "init.lua:2  theme = compute_theme()",
        "init.lua:4  gnoblin.on(\"gnoblin.window.created\", function(w) ...",
        "init.lua:10  b = { binding = \"<Super>Return\", command = {",
        "init.lua:11  binding = \"<Super>a\"",
        "init.lua:14  for i = 1, 3 do ...",
        "docs/migrating-from-lua.md#what-the-converter-cannot-do",
        "docs/migrating-from-lua.md#what-each-construct-becomes",
        "Next:",
        "1. Review ",
        "2. Check it with `gnoblinctl check --types ",
        "3. Log out and back in.",
    ] {
        assert!(out.contains(needle), "{needle}\n{out}");
    }
    let ts = fs::read_to_string(dir.path().join("init.ts")).expect("init.ts");
    assert!(
        ts.contains("// UNMAPPED (init.lua:4): gnoblin.on(\"gnoblin.window.created\", function(w)"),
        "{ts}"
    );
    assert!(ts.contains("// UNMAPPED (init.lua:14): for i = 1, 3 do"), "{ts}");
    // Nothing of the left-out code is in the file as code.
    assert!(!ts.contains("\nfor ") && !ts.contains("\nfunction"), "{ts}");
    // The file still loads with defineConfig and keeps what could convert.
    let planned = plan(dir.path());
    assert_eq!(planned["document"]["cursor"], json!({ "size": 24 }));
    assert_eq!(planned["bindings"].as_array().map(Vec::len), Some(1));
    let (check_code, check_out, check_err) = run(&["check", dir.path().join("init.ts").to_str().expect("UTF-8")]);
    assert_eq!(check_code, 0, "{check_out}{check_err}");
}

// --- The command ------------------------------------------------------------------------------------------------

#[test]
fn it_writes_the_editor_files_and_prints_three_next_steps() {
    let (dir, init) = copied_starter();
    let (code, out, _) = migrate(&init, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(dir.path().join("gnoblin.d.ts").is_file(), "types init ran");
    assert!(dir.path().join("tsconfig.json").is_file(), "types init ran");
    assert!(out.contains("gnoblin.d.ts refreshed"), "{out}");
    assert!(out.contains("tsconfig.json created"), "{out}");
    let steps: Vec<&str> = out
        .lines()
        .filter(|line| line.starts_with("  ") && line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(steps.len(), 3, "{out}");
    assert!(steps[0].contains("1. Review") && steps[0].contains("init.ts"), "{out}");
    assert!(
        steps[1].contains("2. Check it with `gnoblinctl check --types") && steps[1].contains("init.ts"),
        "{out}"
    );
    assert!(
        steps[2].contains("3. Log out and back in") && steps[2].contains("init.lua"),
        "{out}"
    );
    assert!(
        out.contains("settings") && out.contains("19"),
        "the counts are printed: {out}"
    );
}

#[test]
fn it_refuses_to_overwrite_unless_forced() {
    let (dir, init) = copied_starter();
    assert_eq!(migrate(&init, &[]).0, 0);
    let ts = dir.path().join("init.ts");
    fs::write(&ts, "// mine\n").expect("edit init.ts");
    let (code, out, err) = migrate(&init, &[]);
    assert_eq!(code, 2, "{out}{err}");
    assert!(err.contains("already exists, so nothing was written"), "{err}");
    assert!(err.contains("hint:") && err.contains("--force"), "{err}");
    assert_eq!(
        fs::read_to_string(&ts).expect("init.ts"),
        "// mine\n",
        "nothing was written"
    );
    let (forced, out, err) = migrate(&init, &["--force"]);
    assert_eq!(forced, 0, "{out}{err}");
    assert!(fs::read_to_string(&ts).expect("init.ts").contains("defineConfig"));
}

#[test]
fn it_does_not_write_an_init_ts_beside_an_init_js() {
    let (dir, init) = copied_starter();
    write(dir.path(), "init.js", "export default {};\n");
    let (code, _, err) = migrate(&init, &[]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("init.js") && err.contains("read instead of it"), "{err}");
    assert!(!dir.path().join("init.ts").exists());
    assert_eq!(migrate(&init, &["--force"]).0, 0);
}

#[test]
fn output_goes_where_dash_o_says() {
    let (dir, init) = copied_starter();
    let target = dir.path().join("out/new.ts");
    let (code, out, err) = migrate(&init, &["-o", target.to_str().expect("UTF-8")]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(target.is_file());
    assert!(!dir.path().join("init.ts").exists(), "the default is not written");
    assert!(
        dir.path().join("out/gnoblin.d.ts").is_file(),
        "the editor files go beside the output"
    );
}

#[test]
fn dry_run_prints_typescript_and_writes_nothing() {
    let (dir, init) = copied_starter();
    let (code, out, err) = migrate(&init, &["--dry-run"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("// init.lua converted to TypeScript"), "{out}");
    assert!(out.contains("export default defineConfig({"), "{out}");
    assert!(
        !out.contains("Converted"),
        "standard output is the TypeScript alone: {out}"
    );
    assert!(
        err.contains("dry run: nothing was written") && err.contains("Converted"),
        "{err}"
    );
    assert!(!dir.path().join("init.ts").exists());
    assert!(!dir.path().join("gnoblin.d.ts").exists());
    // `--dry-run` does not trip over an init.ts that is there.
    fs::write(dir.path().join("init.ts"), "// mine\n").expect("write");
    assert_eq!(migrate(&init, &["--dry-run"]).0, 0);
}

#[test]
fn dry_run_with_unmapped_items_exits_1() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", UNMAPPED_INIT);
    let (code, out, err) = migrate(&init, &["--dry-run"]);
    assert_eq!(code, 1, "{err}");
    assert!(out.contains("UNMAPPED (init.lua:14)"), "{out}");
    assert!(err.contains("Left out (5)"), "{err}");
}

#[test]
fn a_missing_file_and_a_syntax_error_exit_2_with_a_hint() {
    let dir = TempDir::new().expect("a temp dir");
    let (code, _, err) = migrate(&dir.path().join("nope.lua"), &[]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("is not a file") && err.contains("hint:"), "{err}");
    let init = write(dir.path(), "init.lua", "gnoblin.configure {\n    cursor = = 1\n}\n");
    let (code, _, err) = migrate(&init, &[]);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("init.lua:2:") && err.contains("does not parse as Lua"),
        "{err}"
    );
    assert!(err.contains("hint:"), "{err}");
    assert!(!dir.path().join("init.ts").exists());
}

#[test]
fn a_bad_command_line_exits_2() {
    let (code, _, err) = run(&["config", "migrate", "--bogus"]);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn json_output_carries_the_report_and_the_three_steps() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", UNMAPPED_INIT);
    let (code, out, err) = run_os(&[
        OsString::from("--json"),
        OsString::from("config"),
        OsString::from("migrate"),
        init.as_os_str().to_owned(),
    ]);
    assert_eq!(code, 1, "{err}");
    let document: Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["written"], true);
    assert_eq!(document["init"], dir.path().join("init.ts").to_str().expect("UTF-8"));
    assert_eq!(document["unmapped"].as_array().map(Vec::len), Some(5));
    assert_eq!(document["unmapped"][0]["file"], "init.lua");
    assert_eq!(document["unmapped"][0]["line"], 2);
    assert!(
        document["unmapped"][0]["pointer"]
            .as_str()
            .expect("pointer")
            .starts_with("docs/migrating-from-lua.md#")
    );
    assert_eq!(document["converted"]["keys"], 1);
    assert_eq!(document["next"].as_array().map(Vec::len), Some(3));
    assert_eq!(document["load_problem"], Value::Null);
}

#[test]
fn a_glob_load_is_sorted_and_relative_to_the_file_that_makes_it() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", "gnoblin.load(\"conf.d/*.lua\")\n");
    write(
        dir.path(),
        "conf.d/20-b.lua",
        "gnoblin.configure { cursor = { size = 20 } }\ngnoblin.load(\"deeper/x.lua\")\n",
    );
    write(
        dir.path(),
        "conf.d/10-a.lua",
        "gnoblin.configure { cursor = { size = 10, theme = \"a\" } }\n",
    );
    write(
        dir.path(),
        "conf.d/deeper/x.lua",
        "gnoblin.configure { cursor = { theme = \"deep\" } }\n",
    );
    write(dir.path(), "conf.d/readme.md", "not lua\n");
    let (code, out, err) = migrate(&init, &["--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let document: Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        document["sources"],
        json!(["init.lua", "conf.d/10-a.lua", "conf.d/20-b.lua", "conf.d/deeper/x.lua"])
    );
    // Load order decides: 20-b sets the size after 10-a, and the file it loads sets the theme last.
    assert_eq!(
        plan(dir.path())["document"]["cursor"],
        json!({ "size": 20, "theme": "deep" })
    );
}
