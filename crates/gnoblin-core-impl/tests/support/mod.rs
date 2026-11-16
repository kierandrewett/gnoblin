//! What the core's integration tests share: building the fixtures with nested `cargo` runs, running the launcher fixture's
//! scenarios, and reading an ELF file's dynamic section and symbols with the binutils.
#![allow(dead_code, unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use gnoblin_core_abi as abi;

pub const LAUNCHER: &str = "gnoblin-fixture-launcher";
pub const CORE_FILE: &str = "libgnoblin-core.so.1";

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// Runs `cargo build` for the workspace into `target_dir`, with `args`, and returns cargo's stderr on failure.
pub fn cargo_build(target_dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<(), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let root = repo_root();
    let output = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .envs(env.iter().copied())
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo build {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn copy_to(from: &Path, to: &Path) -> PathBuf {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::copy(from, to).unwrap_or_else(|error| panic!("cannot copy {} to {}: {error}", from.display(), to.display()));
    to.to_owned()
}

/// Runs one scenario of the driver in `dir` and returns its output.
pub fn scenario(dir: &Path, args: &[&Path]) -> Output {
    scenario_with_env(dir, args, &[])
}

/// Like [`scenario`], with `extra` set in the launcher's environment.
pub fn scenario_with_env(dir: &Path, args: &[&Path], extra: &[(&str, &Path)]) -> Output {
    let run_dir = dir.join("run");
    if !run_dir.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&run_dir)
            .unwrap();
    }
    fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700)).unwrap();
    Command::new(dir.join(LAUNCHER))
        .current_dir(dir)
        .args(args)
        .env_remove("LD_LIBRARY_PATH")
        .env("XDG_RUNTIME_DIR", &run_dir)
        .env("GNOBLIN_LOG", "info")
        .env("GNOBLIN_LOG_FORMAT", "plain")
        .env("RUST_BACKTRACE", "0")
        .envs(extra.iter().copied())
        .output()
        .expect("the launcher runs")
}

pub fn words(arguments: &[&str]) -> Vec<PathBuf> {
    arguments.iter().map(PathBuf::from).collect()
}

pub fn run(dir: &Path, arguments: &[&str], modules: &[&Path]) -> (String, String, i32) {
    run_with_env(dir, arguments, modules, &[])
}

/// Like [`run`], with `extra` set in the launcher's environment.
pub fn run_with_env(
    dir: &Path,
    arguments: &[&str],
    modules: &[&Path],
    extra: &[(&str, &Path)],
) -> (String, String, i32) {
    let names = words(arguments);
    let mut all: Vec<&Path> = names.iter().map(PathBuf::as_path).collect();
    all.extend(modules.iter().copied());
    let output = scenario_with_env(dir, &all, extra);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// The value of `key` on the first output line that starts with `prefix`.
pub fn field(stdout: &str, prefix: &str, key: &str) -> Option<String> {
    let line = stdout.lines().find(|line| line.starts_with(prefix))?;
    let marker = format!("{key}=");
    line.split_whitespace()
        .find_map(|word| word.strip_prefix(marker.as_str()))
        .map(str::to_owned)
}

pub fn num(stdout: &str, prefix: &str, key: &str) -> i64 {
    field(stdout, prefix, key)
        .unwrap_or_else(|| panic!("no `{key}` on a `{prefix}` line in:\n{stdout}"))
        .parse()
        .unwrap_or_else(|error| panic!("`{key}` on `{prefix}` is not a number: {error}\n{stdout}"))
}

// ---- ELF inspection, through the binutils the spike used --------------------------------------------------------------

// `LD_LIBRARY_PATH` is removed: cargo puts `target/<profile>` on it for a test run, and that directory holds the staged
// `libgnoblin-core.so.1` of the workspace build, which `ldd` would then report instead of the one beside the launcher.
pub fn tool(name: &str, args: &[&str], path: &Path) -> String {
    let output = Command::new(name)
        .args(args)
        .arg(path)
        .env_remove("LD_LIBRARY_PATH")
        .output()
        .unwrap_or_else(|error| panic!("`{name}` is needed for these tests (binutils): {error}"));
    assert!(
        output.status.success(),
        "{name} failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[derive(Debug, Default)]
pub struct Dynamic {
    pub needed: Vec<String>,
    pub soname: Option<String>,
    pub runpath: Option<String>,
    pub flags_1: String,
}

pub fn bracketed(line: &str) -> Option<String> {
    let start = line.find('[')?;
    let end = line.rfind(']')?;
    Some(line[start + 1..end].to_owned())
}

pub fn dynamic(path: &Path) -> Dynamic {
    let text = tool("readelf", &["-d", "-W"], path);
    let mut dynamic = Dynamic::default();
    for line in text.lines() {
        if line.contains("(NEEDED)") {
            dynamic.needed.extend(bracketed(line));
        } else if line.contains("(SONAME)") {
            dynamic.soname = bracketed(line);
        } else if line.contains("(RUNPATH)") {
            dynamic.runpath = bracketed(line);
        } else if line.contains("(FLAGS_1)") {
            dynamic.flags_1 = line.split("Flags:").nth(1).unwrap_or("").trim().to_owned();
        }
    }
    dynamic
}

pub fn defined_symbols(path: &Path) -> BTreeSet<String> {
    tool("nm", &["-D", "--defined-only"], path)
        .lines()
        .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
        .collect()
}

pub fn undefined_symbols(path: &Path) -> BTreeSet<String> {
    tool("nm", &["-D", "--undefined-only"], path)
        .lines()
        .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
        .collect()
}

pub fn exports() -> BTreeSet<String> {
    abi::export_names().map(str::to_owned).collect()
}
