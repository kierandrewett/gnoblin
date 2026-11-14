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
        "init.ts",
        "import { limit } from './x.ts';\nenum Mode {\n    Tile,\n    Float,\n}\nexport const mode: Mode = Mode.Tile;\nexport const n: number = limit;\n",
    );
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("2 files"), "{out}");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn a_syntax_error_reports_the_file_line_and_column_and_exits_1() {
    let dir = Dir::new();
    let init = dir.write("init.ts", "const ok = 1;\nconst broken = ;\n");
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    assert!(err.contains("init.ts:2:16"), "{err}");
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
        "init.ts",
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
        "init.ts",
        "enum Mode {\n    A,\n}\nconst mode: Mode = Mode.A;\nthrow new Error('boom ' + mode);\n",
    );
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("init.ts:5:"), "{err}");
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
    let init = dir.write("init.ts", "import './b.ts';\n");
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("a.ts:3:"), "{err}");
}

#[test]
fn an_endless_top_level_loop_is_stopped_by_the_budget() {
    let dir = Dir::new();
    let init = dir.write("init.ts", "while (true) {}\n");
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("budget"), "{err}");
}

#[test]
fn the_shell_modules_and_the_global_do_not_need_a_host() {
    let dir = Dir::new();
    let init = dir.write(
        "init.ts",
        "import { defineConfig, FloatingLayout } from 'gnoblin';\nimport { grid } from 'gnoblin/layouts';\ngnoblin.on('start', () => {});\nclass Mine extends FloatingLayout {}\nexport default defineConfig({ layout: new Mine(), grid });\n",
    );
    let (code, out, err) = check(&init, &[]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
}

#[test]
fn an_import_the_shell_does_not_provide_is_still_refused() {
    let dir = Dir::new();
    let init = dir.write("init.ts", "import _ from 'lodash';\nexport default _;\n");
    let (code, _, err) = check(&init, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("lodash"), "{err}");
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
    let init = dir.write("init.ts", "const ok = 1;\nconst broken = ;\n");
    let (code, out, _) = check(&init, &["--json"]);
    assert_eq!(code, 1);
    let document: serde_json::Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["ok"], false);
    assert_eq!(document["problems"][0]["kind"], "syntax");
    assert_eq!(document["problems"][0]["line"], 2);
    assert_eq!(document["problems"][0]["column"], 16);
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
fn types_init_writes_both_files_and_a_second_run_keeps_an_edited_tsconfig() {
    let dir = Dir::new();
    let target = dir.path().join("config");
    let args = [
        OsString::from("types"),
        OsString::from("init"),
        target.as_os_str().to_owned(),
    ];

    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("gnoblin.d.ts refreshed"), "{out}");
    assert!(out.contains("tsconfig.json created"), "{out}");
    let declarations = fs::read_to_string(target.join("gnoblin.d.ts")).expect("gnoblin.d.ts is written");
    assert!(declarations.contains("gnoblin"), "{declarations}");
    assert!(target.join("tsconfig.json").is_file());

    fs::write(target.join("tsconfig.json"), "{\"mine\": true}\n").expect("the user edits the tsconfig");
    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("tsconfig.json kept"), "{out}");
    assert_eq!(
        fs::read_to_string(target.join("tsconfig.json")).expect("the tsconfig is still there"),
        "{\"mine\": true}\n"
    );
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
    let expected = target.join("gnoblin.d.ts");
    assert_eq!(document["types"], expected.to_str().expect("the path is UTF-8"));
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
    let init = target.join("init.ts");
    assert_eq!(
        fs::read_to_string(&init).expect("init.ts is written"),
        gnoblin_js::STARTER_CONFIG,
        "the starter is the one the runtime ships"
    );
    assert!(target.join("gnoblin.d.ts").is_file(), "types init ran");
    assert!(target.join("tsconfig.json").is_file(), "types init ran");
    assert!(out.contains("init.ts written"), "{out}");
    assert!(out.contains("gnoblin.d.ts refreshed"), "{out}");
    let steps: Vec<&str> = out
        .lines()
        .filter(|line| line.starts_with("  ") && line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(steps.len(), 3, "{out}");
    assert!(steps[0].contains("1. Open") && steps[0].contains("init.ts"), "{out}");
    assert!(
        steps[1].contains("2. Check it with `gnoblinctl check --types") && steps[1].contains("init.ts"),
        "{out}"
    );
    assert!(steps[2].contains("3. Save it") && steps[2].contains("reloads"), "{out}");
}

#[test]
fn config_init_does_not_overwrite_a_config_unless_forced() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    assert_eq!(config_init(&target, &[]).0, 0);
    fs::write(target.join("init.ts"), "// mine\n").expect("the user edits init.ts");

    let (code, out, err) = config_init(&target, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(err.contains("init.ts already exists"), "{err}");
    assert!(err.contains("--force"), "the hint names the way out: {err}");
    assert_eq!(
        fs::read_to_string(target.join("init.ts")).expect("init.ts"),
        "// mine\n",
        "a refused run touched nothing"
    );

    let (code, out, err) = config_init(&target, &["--force"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("init.ts replaced"), "{out}");
    assert_eq!(
        fs::read_to_string(target.join("init.ts")).expect("init.ts"),
        gnoblin_js::STARTER_CONFIG
    );
}

#[test]
fn config_init_leaves_an_init_js_alone_because_the_starter_would_shadow_it() {
    let dir = Dir::new();
    dir.write("gnoblin/init.js", "export default {};\n");
    let target = dir.path().join("gnoblin");
    let (code, _, err) = config_init(&target, &[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("init.js already exists"), "{err}");
    assert!(!target.join("init.ts").exists(), "nothing was written");
}

#[test]
fn config_init_reports_what_it_wrote_as_json() {
    let dir = Dir::new();
    let target = dir.path().join("gnoblin");
    let (code, out, err) = config_init(&target, &["--json"]);
    assert_eq!(code, 0, "{err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("--json prints JSON");
    assert_eq!(document["init"], target.join("init.ts").to_str().expect("UTF-8"));
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
    let written = target.join("init.ts");
    let on = dir.write("on/init.ts", &switched_on(gnoblin_js::STARTER_CONFIG));

    for file in [&written, &on] {
        // The in-process check: the module graph loads, with no TypeScript compiler involved.
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
fn a_wrong_key_or_a_wrong_value_is_a_type_error_when_a_compiler_is_installed() {
    let dir = Dir::new();
    let wrong_key = dir.write(
        "key/init.ts",
        "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ autostrt: [\"waybar\"] });\n",
    );
    let wrong_value = dir.write(
        "value/init.ts",
        "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ autostart: [5] });\n",
    );
    for (file, named) in [(&wrong_key, "autostrt"), (&wrong_value, "CommandLine")] {
        let (code, out, err) = check(file, &["--types"]);
        let text = format!("{out}{err}");
        if text.contains("type check skipped") {
            // No compiler here: the declarations cannot be asked, and the load-time errors are tested in gnoblin-js.
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
