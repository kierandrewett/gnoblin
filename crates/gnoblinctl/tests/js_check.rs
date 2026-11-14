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
