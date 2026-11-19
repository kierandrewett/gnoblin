//! `gnoblinctl config migrate`, driven through the real parse, the real command and the real exit codes on temporary
//! directories. The converted file is run in the shell's dry-run host (the shell over an inert tree: a root and a
//! seat), and the world it builds (workspaces, bindings, what each binding does, the started services) is compared
//! with the world the stock starter builds and with what the Lua said. Nothing here runs Lua: the Lua starter is the
//! fixture of `gnoblin-config-migrate`, which is `data/default-config` as it shipped.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_js::{JsShell, JsShellConfig};
use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorValue, SnapshotNode, TreeIncoming, TreeLink, TreeTx,
};
use gnoblinctl::{ConnectFailure, Connection};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A core with the std library in this process, the shell's guest: the rig the launcher's tests share.
#[path = "../../gnoblin/tests/support/core_rig.rs"]
mod core_rig;

/// The starter, `data/js/starter/shell.ts`: the packaged default shell.
const STARTER_CONFIG: &str = include_str!("../../../data/js/starter/shell.ts");

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

/// The files of the Lua starter.
const STARTER_FILES: [&str; 5] = [
    "init.lua",
    "config/10-windows.lua",
    "config/20-input.lua",
    "config/30-session.lua",
    "config/40-shortcuts.lua",
];

/// A copy of the Lua starter in a directory of its own.
fn copied_starter() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("a temp dir");
    for name in STARTER_FILES {
        let text = fs::read_to_string(starter_dir().join(name)).expect("a starter file");
        write(dir.path(), name, &text);
    }
    let init = dir.path().join("init.lua");
    (dir, init)
}

/// A config with nothing the converter has to leave out: two workspaces and a shortcut.
const CLEAN: &str = "gnoblin.configure({\n    workspaces = { { id = \"a\", name = \"A\" }, { id = \"b\", name = \"B\" } },\n    shortcuts = { term = { binding = \"<Super>Return\", command = { \"foot\" } } },\n})\n";

/// A directory holding `CLEAN` as `init.lua`.
fn clean_dir() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", CLEAN);
    (dir, init)
}

/// Every file under `dir` and its text, so a test can say nothing was touched.
fn tree(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path.strip_prefix(root).expect("under the root").display().to_string();
                out.insert(name, fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

// --- The dry-run host --------------------------------------------------------------------------------------------

/// The root of an inert tree, and its one seat.
const ROOT: u64 = 1;
const SEAT: u64 = 2;

/// A compositor with nothing in it: the root node and a seat, as the dry run of `gnoblinctl check` boots the shell (`gnoblin-core-impl`, `check_role.rs`).
#[derive(Default)]
struct InertTree {
    queue: Vec<TreeIncoming>,
}

impl TreeLink for InertTree {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        self.queue.push(TreeIncoming::Attached(Attached {
            epoch: 1,
            root: ROOT,
            serial: 0,
        }));
        self.queue.push(TreeIncoming::Snapshot {
            serial: 0,
            chunk: 0,
            last: true,
            nodes: vec![
                SnapshotNode {
                    id: ROOT,
                    parent: 0,
                    index: 0,
                    class: "node".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: Vec::new(),
                },
                SnapshotNode {
                    id: SEAT,
                    parent: ROOT,
                    index: 0,
                    class: "seat".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: vec![("name".to_owned(), MirrorValue::Text("seat0".to_owned()))],
                },
            ],
        });
        Ok(())
    }

    fn transact(&mut self, _tx: &TreeTx) -> Result<(), LinkError> {
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.queue))
    }
}

/// Runs before the config: it records every key the config binds, every listener it attaches and every program it
/// asks to start, so the probe can press the keys and say what each did.
const PRE: &str = r#"const g = globalThis as any;
g.bound = [];
g.spawned = [];
g.listeners = [];
const input = gnoblin.input.primary;
const realBind = input.bind.bind(input);
(input as any).bind = (trigger: string, action: unknown) => {
    g.bound.push({ trigger, action });
    return realBind(trigger, action as never);
};
const clients: any = gnoblin.clients;
const realAdd = clients.addListener.bind(clients);
clients.addListener = (listener: unknown) => {
    g.listeners.push(listener);
    return realAdd(listener);
};
(gnoblin as any).spawn = (argv: string[]) => {
    g.spawned.push(argv);
    return undefined;
};
"#;

/// Runs after the config: the world it built, as JSON in `globalThis.out`. `EXTRA` is a block of statements that may
/// add to `result`.
fn probe(extra: &str) -> String {
    format!(
        r#"import {{ windowMenu, notRespondingAlerts }} from "gnoblin-std";
const g = globalThis as any;
const input = gnoblin.input.primary;
const workspaces = gnoblin.workspaces.map((w: any) => ({{ id: w.id, name: w.name, layout: w.layout.constructor.name }}));
const seat = [...input.bindings].map((b: any) => `${{b.constructor.name}}:${{b.trigger}}`).sort();
const bindings = g.bound.map(({{ trigger, action }}: any) => {{
    g.spawned.length = 0;
    let error: string | null = null;
    if (typeof action === "function") {{
        try {{
            action({{}});
        }} catch (e) {{
            error = String(e);
        }}
    }}
    return {{
        trigger,
        action: typeof action === "function" ? "function" : action.constructor.name,
        spawned: [...g.spawned],
        error,
    }};
}});
bindings.sort((a: any, b: any) => (a.trigger < b.trigger ? -1 : 1));
const result: any = {{ workspaces, seat, menu: windowMenu.started, alert: notRespondingAlerts.started, bindings }};
{extra}
g.out = JSON.stringify(result);
"#
    )
}

/// The world a config builds in the dry-run host: its workspaces, the keys it binds and what each key does when it is
/// pressed (the programs it starts), the claimed bindings, and whether the window menu and the alert are started.
/// `init` is the text of `shell.ts`.
fn world(init: &str, extra: &str) -> Value {
    let dir = TempDir::new().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the directory exists");
    fs::write(root.join("pre.ts"), PRE).expect("pre.ts is written");
    fs::write(root.join("probe.ts"), probe(extra)).expect("probe.ts is written");
    let mut config = JsShellConfig::new(root.clone())
        .with_stock(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js"));
    config.js.stage_budget = Duration::from_secs(10);
    config.entry_budget = Duration::from_secs(10);
    config.frame_budget = Duration::from_secs(10);
    // The shell runs over a core with the std library loaded, as in a session; the rig is dropped after the shell and gives the core up
    // to the next test of this process.
    let _turn = core_rig::Turn::wait();
    let rig = core_rig::CoreRig::new(std::sync::Arc::clone(&config.clock)).expect("a core with the std library");
    config.js.natives.push(rig.std_native());
    let link = rig
        .take_over(Box::<InertTree>::default())
        .expect("the core takes the link");
    let mut shell = JsShell::new(link, config).expect("the shell boots");
    fs::write(root.join("shell.ts"), init).expect("shell.ts is written");
    // One entry that runs the three in order: a second entry would be a second load, and a load that declares no
    // workspace gets the one floating `desktop` the shell keeps a script from being without.
    fs::write(
        root.join("entry.ts"),
        "import \"./pre.ts\";\nimport \"./shell.ts\";\nimport \"./probe.ts\";\n",
    )
    .expect("entry.ts is written");
    let diagnostics = shell.load_entry(Path::new("entry.ts")).expect("the config loads");
    rig.dispatch(&shell);
    assert!(
        diagnostics.is_empty(),
        "the config loaded with diagnostics: {diagnostics:?}"
    );
    let text = shell
        .eval("globalThis.out")
        .expect("the probe is read")
        .expect("the probe set out");
    serde_json::from_str(&text).expect("the probe wrote JSON")
}

/// The binding of `trigger` in a world.
fn binding<'a>(world: &'a Value, trigger: &str) -> &'a Value {
    world["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .find(|binding| binding["trigger"] == trigger)
        .unwrap_or_else(|| panic!("{trigger} is not bound in {world:#}"))
}

/// The ids of the workspaces of a world, in order.
fn workspace_ids(world: &Value) -> Vec<&str> {
    world["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .map(|workspace| workspace["id"].as_str().expect("an id"))
        .collect()
}

// --- The shipped starter ----------------------------------------------------------------------------------------

#[test]
fn the_starter_converts_to_a_file_that_loads_and_reports_each_setting_it_has_no_home_for() {
    let (dir, init) = copied_starter();
    let before = tree(dir.path());
    let (code, out, err) = migrate(&init, &[]);
    // The Lua starter's `window-management`, `compositor` and `layer-shell` hold 19 settings the specification has no
    // home for, so the file is written and the exit code says it is not complete.
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("Left out (19)"), "{out}");
    let ts = dir.path().join("shell.ts");
    assert!(ts.is_file(), "shell.ts is written beside init.lua");
    let text = fs::read_to_string(&ts).expect("shell.ts");
    assert_eq!(
        text.lines().next(),
        Some("// INCOMPLETE: 19 unmapped settings, see comments")
    );

    // The source is never modified: the Lua files read back exactly as they were, and only shell.ts and the editor
    // files are new.
    let after = tree(dir.path());
    for (name, original) in &before {
        assert_eq!(after.get(name), Some(original), "{name} was changed");
    }
    let new: Vec<&String> = after.keys().filter(|name| !before.contains_key(*name)).collect();
    // The editor files are the generated `.gnoblin` directory (the library sources, the declarations) and the tsconfig.
    let beside: Vec<&str> = new
        .iter()
        .map(|name| name.as_str())
        .filter(|name| !name.starts_with(".gnoblin/"))
        .collect();
    assert_eq!(beside, ["shell.ts", "tsconfig.json"]);
    for generated in [
        ".gnoblin/gnoblin.d.ts",
        ".gnoblin/tsconfig.json",
        ".gnoblin/lib/gnoblin-core/index.ts",
    ] {
        assert!(
            new.iter().any(|name| name.as_str() == generated),
            "{generated} is written: {new:?}"
        );
    }

    // The file loads in the dry-run host, which is what `check` runs.
    let (check_code, check_out, check_err) = run(&["check", ts.to_str().expect("UTF-8")]);
    assert_eq!(check_code, 0, "{check_out}{check_err}");
    let (types_code, types_out, types_err) = run(&["check", ts.to_str().expect("UTF-8"), "--types"]);
    assert_eq!(types_code, 0, "{types_out}{types_err}");
}

#[test]
fn the_converted_starter_builds_the_world_the_stock_starter_builds() {
    let (dir, init) = copied_starter();
    assert_eq!(migrate(&init, &[]).0, 1);
    let converted = fs::read_to_string(dir.path().join("shell.ts")).expect("shell.ts");

    let starter = world(STARTER_CONFIG, "");
    let migrated = world(&converted, "");

    // The stock world is real: four floating workspaces, the window menu and the alert, and twenty keys that each
    // did something when pressed (the media keys start their program).
    assert_eq!(workspace_ids(&starter), ["1", "2", "3", "4"], "{starter:#}");
    assert_eq!(starter["menu"], true);
    assert_eq!(starter["alert"], true);
    assert_eq!(starter["bindings"].as_array().map(Vec::len), Some(20), "{starter:#}");
    assert_eq!(
        binding(&starter, "XF86AudioMute")["spawned"],
        json!([["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]])
    );

    // The same worlds: the starter file and the converted Lua starter.
    assert_eq!(migrated, starter, "the converted Lua starter is the starter");
}

#[test]
fn the_converted_starter_is_the_converters_golden() {
    let (dir, init) = copied_starter();
    assert_eq!(migrate(&init, &[]).0, 1);
    let golden = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../gnoblin-config-migrate/tests/fixtures/starter/shell.ts.golden"),
    )
    .expect("the golden file");
    assert_eq!(
        fs::read_to_string(dir.path().join("shell.ts")).expect("shell.ts"),
        golden
    );
}

// --- Everything that converts -----------------------------------------------------------------------------------

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
    },
})
"#;

const EVERYTHING_SHELL: &str = r#"
seat.data.window_rules = {
    { match = { app_id = "^org%.example%.Editor$" }, workspace = { id = "dev" }, maximised = true },
    { match = { title = "Picture.in.Picture" }, fullscreen = true },
    { match = { app_id = "^pavucontrol$" }, opacity = 0.9 },
    { match = { app_id = "^org%.x%.(%w+)$" }, workspace = "web", maximised = false },
}
seat.data.alerts = false
"#;

/// Presses nothing: it hands a window-shaped object to each listener the config attached and records what the listener
/// wrote to it and where it moved it, for five windows. The stock `Decorations` listener (`startDecorations`, which the
/// starter calls) is not the config's: it frames a window by the node the compositor made for it, and a hand-made object
/// has none, so it is left out rather than fed an object it rightly refuses.
const RULES_PROBE: &str = r#"const moved: any[] = [];
(gnoblin.workspaces as any).get = (id: string) => ({
    clients: {
        add: (client: any) => {
            moved.push([id, client.appId]);
        },
    },
});
const cases = [
    ["org.example.Editor", ""],
    ["foot", "one Picture in Picture"],
    ["org.x.abc", ""],
    ["org.x.abc.d", ""],
    ["other", ""],
];
result.rules = cases.map(([appId, title]) => {
    const log: string[] = [];
    moved.length = 0;
    const client: any = {
        appId,
        title,
        style: {},
        set maximised(value: boolean) {
            log.push(`maximised=${value}`);
        },
        set fullscreen(value: boolean) {
            log.push(`fullscreen=${value}`);
        },
    };
    for (const listener of g.listeners.filter((l: any) => l.constructor.name !== "Decorations")) {
        listener.onAdded?.(client, gnoblin.clients);
    }
    return { appId, title, log, moved: [...moved], opacity: client.style.opacity ?? null };
});
"#;

#[test]
fn a_config_of_every_construct_loads_and_means_what_the_lua_said() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", EVERYTHING);
    write(dir.path(), "shell.lua", EVERYTHING_SHELL);
    let (code, out, err) = migrate(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    let ts = fs::read_to_string(dir.path().join("shell.ts")).expect("shell.ts");
    assert!(!ts.contains("INCOMPLETE"), "{ts}");
    let (check_code, check_out, check_err) = run(&["check", dir.path().join("shell.ts").to_str().expect("UTF-8")]);
    assert_eq!(check_code, 0, "{check_out}{check_err}");

    let world = world(&ts, RULES_PROBE);
    // The declared workspaces replace the stock four, and the alert is off as `seat.data.alerts = false` said.
    assert_eq!(workspace_ids(&world), ["web", "dev"], "{world:#}");
    assert_eq!(world["workspaces"][1]["name"], "Development");
    assert_eq!(world["workspaces"][0]["layout"], "FloatingLayout");
    assert_eq!(world["menu"], true);
    assert_eq!(world["alert"], false);

    // Each shortcut is a key that starts its program; the stock mute key gave way to the config's.
    assert_eq!(binding(&world, "Super+Return")["spawned"], json!([["foot"]]));
    assert_eq!(binding(&world, "Super+Shift+b")["spawned"], json!([["firefox"]]));
    assert_eq!(
        binding(&world, "XF86AudioMute")["spawned"],
        json!([["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]])
    );
    let mutes = world["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .filter(|binding| binding["trigger"] == "XF86AudioMute")
        .count();
    assert_eq!(mutes, 1, "{world:#}");

    // The rules run on a window as it is added: the listener moves and maximises the editor, makes the picture in
    // picture window fullscreen, moves a match of the pattern, and leaves the rest alone. The opacity rule is the
    // style of a selection, so no window is written to.
    let rules = world["rules"].as_array().expect("rules");
    assert_eq!(
        rules[0],
        json!({ "appId": "org.example.Editor", "title": "", "log": ["maximised=true"], "moved": [["dev", "org.example.Editor"]], "opacity": null })
    );
    assert_eq!(rules[1]["log"], json!(["fullscreen=true"]));
    assert_eq!(rules[1]["moved"], json!([]));
    assert_eq!(rules[2]["log"], json!(["maximised=false"]));
    assert_eq!(rules[2]["moved"], json!([["web", "org.x.abc"]]));
    assert_eq!(rules[3]["log"], json!([]), "the pattern is anchored: {rules:#?}");
    assert_eq!(rules[3]["moved"], json!([]));
    assert_eq!(rules[4]["log"], json!([]));
    assert_eq!(rules[4]["opacity"], Value::Null);
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
        "ok  shell.ts written",
        "Converted",
        "Left out (6)",
        "init.lua:2  size = 24",
        "init.lua:2  theme = compute_theme()",
        "init.lua:4  gnoblin.on(\"gnoblin.window.created\", function(w) ...",
        "init.lua:10  b = { binding = \"<Super>Return\", command = {",
        "init.lua:11  binding = \"<Super>a\"",
        "init.lua:14  for i = 1, 3 do ...",
        "docs/migrating-from-lua.md#settings-with-no-typescript-home-yet",
        "docs/migrating-from-lua.md#what-the-converter-cannot-do",
        "docs/migrating-from-lua.md#what-each-construct-becomes",
        "Next:",
        "1. Review ",
        "2. Check it with `gnoblinctl check --types ",
        "3. Log out and back in.",
    ] {
        assert!(out.contains(needle), "{needle}\n{out}");
    }
    let ts = fs::read_to_string(dir.path().join("shell.ts")).expect("shell.ts");
    assert_eq!(
        ts.lines().next(),
        Some("// INCOMPLETE: 6 unmapped settings, see comments")
    );
    assert!(
        ts.contains("// UNMAPPED (init.lua:4): gnoblin.on(\"gnoblin.window.created\", function(w)"),
        "{ts}"
    );
    assert!(ts.contains("// UNMAPPED (init.lua:14): for i = 1, 3 do"), "{ts}");
    // Nothing of the left-out code is in the file as code.
    for line in ts.lines().filter(|line| !line.trim_start().starts_with("//")) {
        for lua in ["function(w)", "compute_theme", "w:maximise", "end)"] {
            assert!(!line.contains(lua), "{line:?} carries Lua outside a comment");
        }
    }
    // The file still loads and keeps what could convert: the one shortcut, over the stock desktop.
    let (check_code, check_out, check_err) = run(&["check", dir.path().join("shell.ts").to_str().expect("UTF-8")]);
    assert_eq!(check_code, 0, "{check_out}{check_err}");
    let world = world(&ts, "");
    assert_eq!(binding(&world, "Super+a")["spawned"], json!([["foot"]]));
    assert!(
        !world["bindings"]
            .as_array()
            .expect("bindings")
            .iter()
            .any(|binding| binding["trigger"] == "Super+Return"),
        "a shortcut on key release is not a key: {world:#}"
    );
    assert_eq!(workspace_ids(&world), ["1", "2", "3", "4"]);
}

// --- The command ------------------------------------------------------------------------------------------------

#[test]
fn it_writes_the_editor_files_and_prints_three_next_steps() {
    let (dir, init) = clean_dir();
    let (code, out, _) = migrate(&init, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(dir.path().join(".gnoblin/gnoblin.d.ts").is_file(), "types init ran");
    assert!(dir.path().join("tsconfig.json").is_file(), "types init ran");
    assert!(out.contains("gnoblin.d.ts refreshed"), "{out}");
    assert!(out.contains("tsconfig.json created"), "{out}");
    let steps: Vec<&str> = out
        .lines()
        .filter(|line| line.starts_with("  ") && line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(steps.len(), 3, "{out}");
    assert!(steps[0].contains("1. Review") && steps[0].contains("shell.ts"), "{out}");
    assert!(
        steps[1].contains("2. Check it with `gnoblinctl check --types") && steps[1].contains("shell.ts"),
        "{out}"
    );
    assert!(
        steps[2].contains("3. Log out and back in") && steps[2].contains("init.lua"),
        "{out}"
    );
    // The counts are printed: two workspaces and one key.
    assert!(
        out.contains("workspaces") && out.contains("keys"),
        "the counts are printed: {out}"
    );
}

#[test]
fn it_refuses_to_overwrite_unless_forced_and_force_touches_only_init_ts() {
    let (dir, init) = clean_dir();
    assert_eq!(migrate(&init, &[]).0, 0);
    let ts = dir.path().join("shell.ts");
    fs::write(&ts, "// mine\n").expect("edit shell.ts");
    let (code, out, err) = migrate(&init, &[]);
    assert_eq!(code, 2, "{out}{err}");
    assert!(err.contains("already exists, so nothing was written"), "{err}");
    assert!(err.contains("hint:") && err.contains("--force"), "{err}");
    assert_eq!(
        fs::read_to_string(&ts).expect("shell.ts"),
        "// mine\n",
        "nothing was written"
    );
    let lua = fs::read_to_string(&init).expect("init.lua");
    let (forced, out, err) = migrate(&init, &["--force"]);
    assert_eq!(forced, 0, "{out}{err}");
    let written = fs::read_to_string(&ts).expect("shell.ts");
    assert!(written.contains("gnoblin.workspaces.add(new Workspace("), "{written}");
    assert_eq!(
        fs::read_to_string(&init).expect("init.lua"),
        lua,
        "the source is never modified"
    );
}

#[test]
fn it_does_not_write_an_init_ts_beside_an_init_js() {
    let (dir, init) = clean_dir();
    write(dir.path(), "shell.js", "export default {};\n");
    let (code, _, err) = migrate(&init, &[]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("shell.js") && err.contains("equally preferred"), "{err}");
    assert!(!dir.path().join("shell.ts").exists());
    assert_eq!(migrate(&init, &["--force"]).0, 0);
}

#[test]
fn output_goes_where_dash_o_says() {
    let (dir, init) = clean_dir();
    let target = dir.path().join("out/new.ts");
    let (code, out, err) = migrate(&init, &["-o", target.to_str().expect("UTF-8")]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(target.is_file());
    assert!(!dir.path().join("shell.ts").exists(), "the default is not written");
    assert!(
        dir.path().join("out/.gnoblin/gnoblin.d.ts").is_file(),
        "the editor files go beside the output"
    );
}

#[test]
fn dry_run_prints_typescript_and_writes_nothing() {
    let (dir, init) = clean_dir();
    let (code, out, err) = migrate(&init, &["--dry-run"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("// init.lua converted to TypeScript"), "{out}");
    assert!(
        out.contains(
            "gnoblin.workspaces.add(new Workspace({ id: \"a\", name: \"A\", layout: new FloatingLayout() }));"
        ),
        "{out}"
    );
    assert!(
        !out.contains("Converted"),
        "standard output is the TypeScript alone: {out}"
    );
    assert!(
        err.contains("dry run: nothing was written") && err.contains("Converted"),
        "{err}"
    );
    assert!(!dir.path().join("shell.ts").exists());
    assert!(!dir.path().join(".gnoblin").exists());
    // `--dry-run` does not trip over a shell.ts that is there.
    fs::write(dir.path().join("shell.ts"), "// mine\n").expect("write");
    assert_eq!(migrate(&init, &["--dry-run"]).0, 0);
}

#[test]
fn dry_run_with_unmapped_items_exits_1_and_the_file_says_it_is_incomplete() {
    let dir = TempDir::new().expect("a temp dir");
    let init = write(dir.path(), "init.lua", UNMAPPED_INIT);
    let (code, out, err) = migrate(&init, &["--dry-run"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        out.starts_with("// INCOMPLETE: 6 unmapped settings, see comments\n"),
        "{out}"
    );
    assert!(out.contains("UNMAPPED (init.lua:14)"), "{out}");
    assert!(err.contains("Left out (6)"), "{err}");
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
    assert!(!dir.path().join("shell.ts").exists());
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
    assert_eq!(document["init"], dir.path().join("shell.ts").to_str().expect("UTF-8"));
    assert_eq!(document["unmapped"].as_array().map(Vec::len), Some(6));
    assert_eq!(document["unmapped"][0]["file"], "init.lua");
    assert_eq!(document["unmapped"][0]["line"], 2);
    assert!(
        document["unmapped"][0]["pointer"]
            .as_str()
            .expect("pointer")
            .starts_with("docs/migrating-from-lua.md#")
    );
    assert!(
        document["unmapped"][0]["reason"]
            .as_str()
            .expect("reason")
            .contains("no TypeScript home yet")
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
        "gnoblin.configure { workspaces = { { id = \"b\", name = \"B\" } } }\ngnoblin.load(\"deeper/x.lua\")\n",
    );
    write(
        dir.path(),
        "conf.d/10-a.lua",
        "gnoblin.configure { workspaces = { { id = \"a\", name = \"A\" } } }\n",
    );
    write(
        dir.path(),
        "conf.d/deeper/x.lua",
        "gnoblin.configure { workspaces = { { id = \"deep\", name = \"Deep\" } } }\n",
    );
    write(dir.path(), "conf.d/readme.md", "not lua\n");
    let (code, out, err) = migrate(&init, &["--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let document: Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        document["sources"],
        json!(["init.lua", "conf.d/10-a.lua", "conf.d/20-b.lua", "conf.d/deeper/x.lua"])
    );
    // Load order decides: each file declares the workspaces again and the last list stands, so the file the second
    // file loads wins.
    let ts = fs::read_to_string(dir.path().join("shell.ts")).expect("shell.ts");
    assert_eq!(workspace_ids(&world(&ts, "")), ["deep"]);
}
