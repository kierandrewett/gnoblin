//! Reading a shared object the way `design/modular-framework.md` section 10 asks it to be validated: `readelf -d` for what it needs,
//! `nm -D` for what it defines and wants, `ldd` for where the loader finds it. The sessions that run a native shell and the
//! upgrade suite share these, so the same facts are asserted of a golden binary, of an example and of a library in a prefix.
//!
//! The tools are binutils and glibc's `ldd`; a test that needs one fails, never skips, when it is missing.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::native_shell::repo_root;

/// Runs a binutils (or coreutils) tool on `path` and returns its standard output. `LD_LIBRARY_PATH` is cleared so `ldd` shows what
/// the run path alone resolves.
pub fn tool(name: &str, args: &[&str], path: &Path) -> String {
    tool_with(name, args, path, &[])
}

pub fn tool_with(name: &str, args: &[&str], path: &Path, env: &[(&str, &Path)]) -> String {
    let mut command = Command::new(name);
    command.args(args).arg(path).env_remove("LD_LIBRARY_PATH");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
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

/// What `readelf -d` says of a file.
#[derive(Debug, Default)]
pub struct Dynamic {
    pub needed: Vec<String>,
    pub soname: Option<String>,
    pub runpath: Option<String>,
}

fn bracketed(line: &str) -> Option<String> {
    let start = line.find('[')?;
    let end = line.rfind(']')?;
    Some(line[start + 1..end].to_owned())
}

pub fn dynamic(path: &Path) -> Dynamic {
    let mut dynamic = Dynamic::default();
    for line in tool("readelf", &["-d", "-W"], path).lines() {
        if line.contains("(NEEDED)") {
            dynamic.needed.extend(bracketed(line));
        } else if line.contains("(SONAME)") {
            dynamic.soname = bracketed(line);
        } else if line.contains("(RUNPATH)") || line.contains("(RPATH)") {
            dynamic.runpath = bracketed(line);
        }
    }
    dynamic
}

/// The dynamic symbols of `path`; `which` is `--defined-only` or `--undefined-only`.
pub fn symbols(path: &Path, which: &str) -> BTreeSet<String> {
    tool("nm", &["-D", which], path)
        .lines()
        .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
        .collect()
}

/// The SONAME of the core library, as a shell records it.
pub fn is_core(name: &str) -> bool {
    name == "libgnoblin-core.so.1"
}

/// The SONAME of the std library, as a std shell records it.
pub fn is_std(name: &str) -> bool {
    name == "libgnoblin-std.so.1"
}

/// The dependency facts of a shell, core-only or std (`design/modular-framework.md` 11): it links the installed core, and std when it
/// uses std, and no other library of ours; it defines nothing but `gnoshell_entry_v1` (no copy of the compositor, the core or std);
/// every `gnoblin_` symbol it needs is one the core (or std) exports.
#[track_caller]
pub fn assert_shell_dependencies(shell: &Path, uses_std: bool) {
    let facts = dynamic(shell);
    assert_eq!(
        facts.needed.iter().filter(|name| is_core(name)).count(),
        1,
        "{} names the core exactly once: {facts:?}",
        shell.display()
    );
    assert_eq!(
        facts.needed.iter().filter(|name| is_std(name)).count(),
        usize::from(uses_std),
        "{} names std exactly when it uses std: {facts:?}",
        shell.display()
    );
    let ours: Vec<&String> = facts.needed.iter().filter(|name| name.contains("gnoblin")).collect();
    assert_eq!(
        ours.len(),
        1 + usize::from(uses_std),
        "no library of ours but the core (and std): {facts:?}"
    );
    assert_eq!(
        facts.runpath, None,
        "a shell carries no run path: the prefix finds its libraries"
    );
    assert_eq!(
        symbols(shell, "--defined-only"),
        BTreeSet::from(["gnoshell_entry_v1".to_owned()]),
        "{} defines only the shell entry",
        shell.display()
    );
    let core_exports: BTreeSet<String> = fs::read_to_string(repo_root().join("crates/gnoblin-core-abi/exports.txt"))
        .expect("the core's export list")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    for name in symbols(shell, "--undefined-only")
        .into_iter()
        .filter(|name| name.starts_with("gnoblin_"))
    {
        assert!(
            core_exports.contains(&name) || (uses_std && name.starts_with("gnoblin_std_")),
            "{} needs `{name}`, which neither the core nor std exports",
            shell.display()
        );
    }
}

/// Where `ldd` says `name` resolves, with the path made canonical (`$ORIGIN/../lib` is printed as written).
pub fn resolved(ldd: &str, name: &str) -> Option<PathBuf> {
    ldd.lines().find_map(|line| {
        let (library, rest) = line.trim().split_once(" => ")?;
        (library == name)
            .then(|| rest.rsplit_once(" (").map_or(rest, |(path, _)| path))
            .and_then(|path| fs::canonicalize(path).ok())
    })
}
