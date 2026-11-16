//! `gnoblinctl check` on TypeScript and JavaScript modules, and `gnoblinctl types init`,
//! driven through the real parse, the real command functions and the real exit codes on
//! temporary directories. Neither command opens a connection, so the connection here is one
//! that fails if it is ever used.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblinctl::{ConnectFailure, Connection};

/// A connection that was never meant to be opened: a command that needs no session must not ask for one.
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

/// A fresh directory under the temporary directory, removed when the test ends.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "gnoblinctl-js-check-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create the test directory");
        Self(path)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the parent directory");
        }
        fs::write(&path, text).expect("write the test file");
        path
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Run one command line, returning its exit code and both streams.
fn run(args: &[OsString]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = gnoblinctl::run(args.iter().cloned(), NoSession, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("the command writes UTF-8"),
        String::from_utf8(err).expect("the command writes UTF-8"),
    )
}

fn check(file: &Path, extra: &[&str]) -> (u8, String, String) {
    let mut args = vec![OsString::from("check"), file.as_os_str().to_owned()];
    args.extend(extra.iter().map(OsString::from));
    run(&args)
}

#[test]
fn a_clean_init_with_an_enum_and_a_relative_import_checks_ok() {
    let dir = Dir::new();
    dir.write("x.ts", "export const limit: number = 3;\n");
    let init = dir.write(
        "shell.ts",
        "import { limit } from './x.ts';\nenum Mode {\n    Tile,\n    Float,\n}\nexport const mode: Mode = Mode.Tile;\nexport const n: number = limit;\n",
    );
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("loads in the dry-run shell"), "{out}");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn a_syntax_error_reports_the_file_line_and_column_and_exits_1() {
    let dir = Dir::new();
    let init = dir.write("shell.ts", "const ok = 1;\nconst broken = ;\n");
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    assert!(err.contains("shell.ts:2:16"), "{err}");
    assert!(err.contains("error:"), "{err}");
    assert!(err.contains("hint:"), "{err}");
}

#[test]
fn a_missing_import_names_the_specifier_and_the_importing_file() {
    let dir = Dir::new();
    dir.write(
        "lib/helper.ts",
        "import { gone } from './gone.ts';\nexport const helper = gone;\n",
    );
    let init = dir.write(
        "shell.ts",
        "import { helper } from './lib/helper.ts';\nexport const x = helper;\n",
    );
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("./gone.ts"), "{err}");
    assert!(err.contains("helper.ts:1:22"), "{err}");
}

#[test]
fn a_top_level_throw_is_reported_at_the_line_that_threw() {
    let dir = Dir::new();
    let init = dir.write(
        "shell.ts",
        "enum Mode {\n    A,\n}\nconst mode: Mode = Mode.A;\nthrow new Error('boom ' + mode);\n",
    );
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("shell.ts:5:"), "{err}");
    assert!(err.contains("boom 0"), "{err}");
}

#[test]
fn an_import_cycle_that_reads_a_binding_before_it_exists_fails() {
    let dir = Dir::new();
    dir.write(
        "a.ts",
        "import { b } from './b.ts';\nexport const a = 1;\nexport const sum = a + b;\n",
    );
    dir.write("b.ts", "import { a } from './a.ts';\nexport const b = a + 1;\n");
    let init = dir.write("shell.ts", "import './b.ts';\n");
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("a.ts:3:"), "{err}");
}

#[test]
fn an_endless_top_level_loop_is_stopped_by_the_budget() {
    let dir = Dir::new();
    let init = dir.write("shell.ts", "while (true) {}\n");
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("budget"), "{err}");
}

#[test]
fn a_spec_style_init_loads_in_the_dry_run_host() {
    let dir = Dir::new();
    let init = dir.write(
        "shell.ts",
        "import { FloatingLayout, MasterStackLayout, Workspace } from 'gnoblin-std';\n\nconst input = gnoblin.input.primary;\ngnoblin.workspaces.add(new Workspace({ id: 'web', name: 'Web', layout: new MasterStackLayout() }));\ngnoblin.workspaces.add(new Workspace({ id: 'main', layout: new FloatingLayout() }));\ngnoblin.clients.style.cornerRadius = 12;\ninput.bind('Super+Return', () => {\n    gnoblin.spawn(['foot']);\n});\n",
    );
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("loads in the dry-run shell"), "{out}");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn a_binding_the_shell_would_refuse_is_refused_here() {
    // The stub host accepted anything; the dry-run host runs the shell's own checks.
    let dir = Dir::new();
    let init = dir.write(
        "shell.ts",
        "gnoblin.input.primary.bind('Super+k', () => {});\ngnoblin.input.primary.bind('Super+k', () => {});\n",
    );
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("conflicts with") && err.contains("Super+k"), "{err}");
}

#[test]
fn an_import_the_shell_does_not_provide_is_still_refused() {
    let dir = Dir::new();
    let init = dir.write("shell.ts", "import _ from 'lodash';\nexport default _;\n");
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("lodash"), "{err}");
}

#[test]
fn a_name_the_gnoblin_module_does_not_export_is_refused_with_the_name() {
    let dir = Dir::new();
    let init = dir.write(
        "shell.ts",
        "import { NoSuchLayout } from 'gnoblin-core'; import { Workspace } from 'gnoblin-std';\nexport const w = [Workspace, NoSuchLayout];\n",
    );
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("NoSuchLayout"), "{err}");
}

#[test]
fn a_file_that_reads_live_state_sees_an_empty_world_and_loads() {
    // The dry-run host has a root and a seat and nothing else, as the shell has at first boot: counting windows,
    // walking a collection and asking for the focused window all work, and answer that there is none.
    let dir = Dir::new();
    let init = dir.write(
        "shell.ts",
        "const seen: string[] = [];\nfor (const client of gnoblin.clients) {\n    seen.push(client.title);\n}\nconst focused = gnoblin.input.primary.focusedClient;\nif (focused === null && gnoblin.clients.length === 0 && seen.length === 0) {\n    gnoblin.input.primary.bind('Super+k', () => {\n        gnoblin.spawn(['foot']);\n    });\n}\n",
    );
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
}

#[test]
fn a_file_that_does_not_exist_is_a_problem_not_a_usage_error() {
    let dir = Dir::new();
    let (code, _, err) = check(&dir.path().join("absent.ts"), &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("absent.ts"), "{err}");
}

#[test]
fn json_output_carries_the_position_of_a_problem() {
    let dir = Dir::new();
    let init = dir.write("shell.ts", "const ok = 1;\nconst broken = ;\n");
    let (code, out, _) = check(&init, &["--json"]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["ok"], false);
    assert_eq!(document["problems"][0]["kind"], "syntax");
    assert_eq!(document["problems"][0]["line"], 2);
    assert_eq!(document["problems"][0]["column"], 16);
}

fn validate(file: &Path, extra: &[&str]) -> (u8, String, String) {
    let mut args = vec![
        OsString::from("config"),
        OsString::from("validate"),
        file.as_os_str().to_owned(),
    ];
    args.extend(extra.iter().map(OsString::from));
    run(&args)
}

#[test]
fn config_validate_checks_a_typescript_config_as_check_does() {
    let dir = Dir::new();
    dir.write("x.ts", "export const limit: number = 3;\n");
    let init = dir.write(
        "shell.ts",
        "import { limit } from './x.ts';\nexport const n: number = limit;\n",
    );
    let (code, out, err) = validate(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert_eq!(out, check(&init, &[]).1, "validate is the same in-process check");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn config_validate_refuses_a_broken_typescript_config_with_its_position() {
    let dir = Dir::new();
    let init = dir.write("shell.ts", "const ok = 1;\nconst broken = ;\n");
    let (code, out, err) = validate(&init, &[]);
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    assert!(err.contains("shell.ts:2:16"), "{err}");
    assert_eq!(err, check(&init, &[]).2, "the same report as `check`");
}

#[test]
fn config_validate_accepts_a_javascript_config_and_reports_a_throw_in_json() {
    let dir = Dir::new();
    let good = dir.write("shell.js", "export const n = 1;\n");
    let (code, out, err) = validate(&good, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    let bad = dir.write("bad.js", "throw new Error('boom');\n");
    let (code, out, err) = validate(&bad, &["--json"]);
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["problems"][0]["kind"], "exception");
}

#[test]
fn config_validate_of_a_missing_typescript_file_is_a_problem() {
    let dir = Dir::new();
    let (code, _, err) = validate(&dir.path().join("shell.ts"), &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn a_file_that_is_not_a_module_keeps_the_configuration_check() {
    let dir = Dir::new();
    let config = dir.write("config.json", "{}");
    let (code, out, err) = check(&config, &[]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "valid\n");
}

#[test]
fn types_is_refused_for_a_file_the_runtime_does_not_load() {
    let dir = Dir::new();
    let config = dir.write("config.json", "{}");
    let (code, out, err) = check(&config, &["--types"]);
    assert_eq!(code, 2, "stdout: {out}\nstderr: {err}");
    assert!(err.contains("--types"), "{err}");
}

#[test]
fn types_init_writes_the_real_sources_and_a_second_run_keeps_an_edited_tsconfig() {
    let dir = Dir::new();
    let target = dir.path().join("config");
    let args = [
        OsString::from("types"),
        OsString::from("init"),
        target.as_os_str().to_owned(),
    ];

    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("library refreshed"), "{out}");
    assert!(out.contains("gnoblin.d.ts refreshed"), "{out}");
    assert!(out.contains("tsconfig.json created"), "{out}");
    let declarations = fs::read_to_string(target.join(".gnoblin/gnoblin.d.ts")).expect("the declarations are written");
    assert!(
        declarations.contains("declare module \"gnoblin-core/schema/tree\""),
        "{declarations}"
    );
    assert!(
        !declarations.contains("declare module \"gnoblin\""),
        "the library is not declared a second time"
    );
    let tsconfig = fs::read_to_string(target.join("tsconfig.json")).expect("the tsconfig is written");
    assert!(tsconfig.contains("./.gnoblin/tsconfig.json"), "{tsconfig}");

    // The editor reads the library's own files: byte for byte what the repository holds, except the package entries,
    // which are the repository's index plus the host's exports.
    let library = target.join(".gnoblin/lib");
    let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js");
    for file in [
        "gnoblin-std/scene/node.ts",
        "gnoblin-std/scene/camera.ts",
        "gnoblin-std/scene.ts",
        "gnoblin-std/host/workspace.ts",
        "gnoblin-std/extensions/xr/index.ts",
        "gnoblin-core/host/client.ts",
    ] {
        assert_eq!(
            fs::read_to_string(library.join(file)).expect("the source is written"),
            fs::read_to_string(sources.join(file)).expect("the source is in the repository"),
            "{file}"
        );
    }
    for package in ["gnoblin-core", "gnoblin-std"] {
        let index = fs::read_to_string(library.join(package).join("index.ts")).expect("the index is written");
        let original =
            fs::read_to_string(sources.join(package).join("index.ts")).expect("the index is in the repository");
        assert_eq!(index, format!("{original}\nexport * from \"{package}/host\";\n"));
    }

    fs::write(target.join("tsconfig.json"), "{\"mine\": true}\n").expect("the user edits the tsconfig");
    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("tsconfig.json kept"), "{out}");
    assert!(
        out.contains("does not extend .gnoblin/tsconfig.json"),
        "a tsconfig that cannot find the types is said so: {out}"
    );
    assert_eq!(
        fs::read_to_string(target.join("tsconfig.json")).expect("the tsconfig is still there"),
        "{\"mine\": true}\n"
    );
}

#[test]
fn types_init_upgrades_the_files_an_earlier_release_wrote() {
    let dir = Dir::new();
    let target = dir.path().join("config");
    let legacy = serde_json::json!({
        "compilerOptions": {
            "target": "es2023", "lib": ["es2023"], "module": "esnext", "moduleResolution": "bundler",
            "strict": true, "noEmit": true, "isolatedModules": true, "allowImportingTsExtensions": true,
            "types": [], "paths": { "gnoblin": ["./gnoblin.d.ts"] },
        },
        "files": ["./gnoblin.d.ts"],
        "include": ["./**/*.ts"],
    });
    fs::create_dir_all(&target).expect("the config directory");
    fs::write(target.join("tsconfig.json"), legacy.to_string()).expect("the old tsconfig");
    fs::write(
        target.join("gnoblin.d.ts"),
        "// gnoblin.d.ts: the types of the Gnoblin JavaScript and TypeScript API.\n",
    )
    .expect("the old declarations");

    let (code, out, err) = run(&[
        OsString::from("types"),
        OsString::from("init"),
        target.as_os_str().to_owned(),
    ]);

    assert_eq!(code, 0, "{err}");
    assert!(out.contains("tsconfig.json replaced"), "{out}");
    assert!(
        !target.join("gnoblin.d.ts").exists(),
        "the old declarations would clash"
    );
    let tsconfig = fs::read_to_string(target.join("tsconfig.json")).expect("the tsconfig");
    assert!(tsconfig.contains("./.gnoblin/tsconfig.json"), "{tsconfig}");
}

#[test]
fn types_init_reports_where_it_wrote_as_json() {
    let dir = Dir::new();
    let target = dir.path().join("cfg");
    let args = [
        OsString::from("--json"),
        OsString::from("types"),
        OsString::from("init"),
        target.as_os_str().to_owned(),
    ];
    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["tsconfig_created"], true);
    assert_eq!(document["tsconfig_linked"], true);
    let expected = target.join(".gnoblin/gnoblin.d.ts");
    assert_eq!(document["types"], expected.to_str().expect("the path is UTF-8"));
    let library = target.join(".gnoblin/lib");
    assert_eq!(document["library"], library.to_str().expect("the path is UTF-8"));
    assert!(
        document["library_files"].as_u64().is_some_and(|count| count > 100),
        "{document}"
    );
}

#[test]
fn types_init_into_a_path_that_is_a_file_fails_with_a_hint() {
    let dir = Dir::new();
    let file = dir.write("not-a-dir", "x");
    let args = [
        OsString::from("types"),
        OsString::from("init"),
        file.as_os_str().to_owned(),
    ];
    let (code, _, err) = run(&args);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("hint:"), "{err}");
}

// --- `config init` ---------------------------------------------------------------------------------------------

fn config_init(target: &Path, extra: &[&str]) -> (u8, String, String) {
    let mut args = vec![
        OsString::from("config"),
        OsString::from("init"),
        target.as_os_str().to_owned(),
    ];
    args.extend(extra.iter().map(OsString::from));
    run(&args)
}

#[test]
fn config_init_writes_the_starter_and_the_editor_files_and_prints_three_next_steps() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    let (code, out, err) = config_init(&target, &[]);
    assert_eq!(code, 0, "{err}");
    let init = target.join("shell.ts");
    assert_eq!(
        fs::read_to_string(&init).expect("shell.ts is written"),
        gnoblin_js::STARTER_CONFIG,
        "the starter is the one the runtime ships"
    );
    assert!(target.join(".gnoblin/gnoblin.d.ts").is_file(), "types init ran");
    assert!(target.join("tsconfig.json").is_file(), "types init ran");
    assert!(out.contains("shell.ts written"), "{out}");
    assert!(out.contains("gnoblin.d.ts refreshed"), "{out}");
    let steps: Vec<&str> = out
        .lines()
        .filter(|line| line.starts_with("  ") && line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(steps.len(), 3, "{out}");
    assert!(steps[0].contains("1. Open") && steps[0].contains("shell.ts"), "{out}");
    assert!(
        steps[1].contains("2. Check it with `gnoblinctl check --types") && steps[1].contains("shell.ts"),
        "{out}"
    );
    assert!(steps[2].contains("3. Save it") && steps[2].contains("reloads"), "{out}");
}

#[test]
fn config_init_does_not_overwrite_a_config_unless_forced() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    assert_eq!(config_init(&target, &[]).0, 0);
    fs::write(target.join("shell.ts"), "// mine\n").expect("the user edits shell.ts");

    let (code, out, err) = config_init(&target, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(err.contains("shell.ts already exists"), "{err}");
    assert!(err.contains("--force"), "the hint names the way out: {err}");
    assert_eq!(
        fs::read_to_string(target.join("shell.ts")).expect("shell.ts"),
        "// mine\n",
        "a refused run touched nothing"
    );

    let (code, out, err) = config_init(&target, &["--force"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("shell.ts replaced"), "{out}");
    assert_eq!(
        fs::read_to_string(target.join("shell.ts")).expect("shell.ts"),
        gnoblin_js::STARTER_CONFIG
    );
}

#[test]
fn config_init_beside_an_init_lua_says_the_lua_file_is_no_longer_read() {
    let dir = Dir::new();
    dir.write("gnoblin/init.lua", "gnoblin.configure({})\n");
    let target = dir.path().join("gnoblin");
    let (code, out, err) = config_init(&target, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("init.lua is still there"), "{out}");
    assert!(out.contains("migrating-from-lua.md"), "{out}");
    assert!(target.join("shell.ts").exists(), "the starter is written");
    assert!(target.join("init.lua").exists(), "the Lua file is left alone");
}

#[test]
fn config_init_leaves_a_shell_js_alone_because_the_starter_would_be_equally_preferred_with_it() {
    let dir = Dir::new();
    dir.write("gnoblin/shell.js", "export default {};\n");
    let target = dir.path().join("gnoblin");
    let (code, _, err) = config_init(&target, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("shell.js already exists"), "{err}");
    assert!(!target.join("shell.ts").exists(), "nothing was written");
}

#[test]
fn config_init_leaves_any_other_shell_file_alone() {
    for name in ["shell.py", "shell.so", "shell.mts"] {
        let dir = Dir::new();
        dir.write(&format!("gnoblin/{name}"), "x\n");
        let target = dir.path().join("gnoblin");
        let (code, _, err) = config_init(&target, &[]);
        assert_eq!(code, 1, "{name}: {err}");
        assert!(err.contains(&format!("{name} already exists")), "{name}: {err}");
        assert!(err.contains("equally preferred"), "{name}: {err}");
        assert!(!target.join("shell.ts").exists(), "{name}: nothing was written");
    }
}

#[test]
fn config_init_then_check_round_trips_under_the_new_name_and_discovery_finds_it() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    let (code, _, err) = config_init(&target, &[]);
    assert_eq!(code, 0, "{err}");
    let written = target.join("shell.ts");
    assert!(written.is_file(), "the starter is named shell.ts");
    assert!(
        !target.join("init.ts").exists(),
        "nothing is written under the old name"
    );
    let (code, out, err) = check(&written, &[]);
    assert_eq!(code, 0, "{out}{err}");
    let environment = gnoblin_config::Environment {
        xdg_config_home: Some(dir.path().to_path_buf()),
        xdg_data_dirs: Some(OsString::from("/nonexistent/share")),
        ..gnoblin_config::Environment::default()
    };
    let located = gnoblin_config::locate(&environment).expect("discovery finds the written starter");
    assert_eq!(located.path(), written.as_path());
    assert!(located.exists());
}

#[test]
fn config_init_reports_what_it_wrote_as_json() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    let (code, out, err) = config_init(&target, &["--json"]);
    assert_eq!(code, 0, "{err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["init"], target.join("shell.ts").to_str().expect("UTF-8"));
    assert_eq!(document["replaced"], false);
    assert_eq!(document["tsconfig_created"], true);
    assert_eq!(document["next"].as_array().map(Vec::len), Some(3));
}

/// The starter with every example switched on: a comment of the form `// code` whose code ends the way a line of the
/// config does is uncommented, and prose stays.
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
fn the_starter_passes_check_as_written_and_with_every_example_switched_on() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    assert_eq!(config_init(&target, &[]).0, 0);
    let written = target.join("shell.ts");
    let on = dir.write("on/shell.ts", &switched_on(gnoblin_js::STARTER_CONFIG));

    for file in [&written, &on] {
        // The in-process check: the file runs in the dry-run host, with no TypeScript compiler involved.
        let (code, out, err) = check(file, &[]);
        assert_eq!(code, 0, "{}: stdout {out}\nstderr {err}", file.display());

        // `--types` runs a TypeScript compiler when there is one. Without one it says so and exits 0, which is all a
        // machine with no compiler can show; with one, exit 0 is the declarations accepting every line.
        let (code, out, err) = check(file, &["--types"]);
        assert_eq!(code, 0, "{}: stdout {out}\nstderr {err}", file.display());
        if format!("{out}{err}").contains("type check skipped") {
            eprintln!(
                "no TypeScript compiler on this machine: --types skipped {}",
                file.display()
            );
        }
    }
}

#[test]
fn a_wrong_property_or_a_wrong_value_is_a_type_error_when_a_compiler_is_installed() {
    let dir = Dir::new();
    let wrong_key = dir.write("key/shell.ts", "gnoblin.clients.style.cornerRadus = 12;\n");
    let wrong_value = dir.write(
        "value/shell.ts",
        "import { Workspace } from \"gnoblin-std\";\ngnoblin.workspaces.add(new Workspace({ id: \"a\", layout: \"spiral\" }));\n",
    );
    for (file, named) in [(&wrong_key, "cornerRadus"), (&wrong_value, "spiral")] {
        let (code, out, err) = check(file, &["--types"]);
        let text = format!("{out}{err}");
        if text.contains("type check skipped") {
            // No compiler here: the declarations cannot be asked, and the load-time errors are tested above.
            eprintln!(
                "no TypeScript compiler on this machine: {} was not type-checked",
                file.display()
            );
            return;
        }
        assert_eq!(code, 1, "{named}: stdout {out}\nstderr {err}");
        assert!(text.contains(named), "the compiler names what is wrong: {text}");
    }
}
