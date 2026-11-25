//! `cargo xtask dist-check`: the static and link-level checks of an installed (or staged) Gnoblin tree.
//!
//! The modular framework makes dynamic linking part of the product, so "it builds" is not enough: the installed core must
//! carry the right SONAME and export exactly the ABI, the launcher must record the core once and find it with no
//! `LD_LIBRARY_PATH`, a shell must link the core and never carry a copy of it, and the development files must let a C program
//! link and run. This command is those checks, over files read
//! with `elf.rs` (cross-checked against `readelf` and `nm` by its tests). The run-time half (one core mapping in a live
//! process) is a test of the sessions, not a static check.
//!
//! ```text
//! cargo xtask dist-check --prefix /usr --destdir /tmp/stage [--libdir lib64] [--shell FILE]... [--std-shell FILE]... [--run]
//! cargo xtask dist-check --staged [--profile release] [--run]
//! cargo xtask dist-check --core target/debug/libgnoblin_core_impl.so --shell target/debug/libshell_min.so
//! ```
//!
//! With `--prefix` the command finds the core, the launcher, the std library, the SONAME links, the header and the
//! pkg-config file where `dist install` puts them; it also checks the tree against its install manifest and the layout of
//! section 11, resolves every gnoblin library the launcher needs the way the loader would (run path, then the tree's own
//! library directory for a `/usr` prefix), and checks every shell found in `<libdir>/gnoblin/shells`. With `--staged` it
//! checks the build tree `cargo xtask stage` made: `target/<profile>/gnoblin` and `libgnoblin-core.so.1`. `--run` also runs
//! the launcher from its own tree with `LD_LIBRARY_PATH` cleared. Individual files can be named; each flag checks that file
//! only. The glibc floor of every file is printed; `--max-glibc X.Y` fails a file that needs more.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use gnoblin_core_abi as abi;

use crate::elf::{self, Elf};

/// The SONAME of the std library, which a std shell and nothing else of the core's tier links.
pub const STD_SONAME: &str = "libgnoblin-std.so.1";

const USAGE: &str = "\
usage: cargo xtask dist-check [--prefix P [--destdir D] [--libdir NAME]] [--staged [--profile NAME]] [--core FILE]
                              [--launcher FILE] [--std FILE] [--shell FILE]... [--std-shell FILE]... [--run]
                              [--max-glibc X.Y]";

fn is_core(name: &str) -> bool {
    name == abi::SONAME
}

fn is_std(name: &str) -> bool {
    name == STD_SONAME || name.starts_with("libgnoblin_std") || name.starts_with("libgnoblin-std")
}

fn core_exports() -> BTreeSet<String> {
    abi::export_names().map(str::to_owned).collect()
}

/// The core library: SONAME, `NODELETE`, and exactly the exported symbols of `exports.txt`.
pub fn check_core(elf: &Elf) -> Vec<String> {
    let mut problems = Vec::new();
    if elf.soname.as_deref() != Some(abi::SONAME) {
        problems.push(format!("SONAME is {:?}, expected `{}`", elf.soname, abi::SONAME));
    }
    if !elf.is_nodelete() {
        problems.push("is not linked with `-z nodelete` (FLAGS_1 lacks NODELETE): a retired shell generation's code would not stay mapped".to_owned());
    }
    let expected = core_exports();
    let defined: BTreeSet<String> = elf.defined.iter().cloned().collect();
    for missing in expected.difference(&defined) {
        problems.push(format!("does not define `{missing}`, which exports.txt lists"));
    }
    for extra in defined.difference(&expected) {
        let why = if extra.starts_with("__rust") || extra.starts_with("rust_") {
            "a Rust runtime symbol leaked into the dynamic symbol table"
        } else if extra == "gnoblin_test_main" {
            "the test driver's export: a shipped core is built without the `test-driver` feature"
        } else {
            "it is not in exports.txt"
        };
        problems.push(format!("defines `{extra}`: {why}"));
    }
    if let Some(other) = elf
        .needed
        .iter()
        .find(|needed| needed.starts_with("libgnoblin") && !is_core(needed))
    {
        problems.push(format!(
            "needs `{other}`: the core must not depend on another gnoblin library"
        ));
    }
    problems
}

/// The launcher: the core recorded exactly once, no std, and no export of its own that could interpose on the core's.
pub fn check_launcher(elf: &Elf) -> Vec<String> {
    let mut problems = Vec::new();
    let cores = elf.needed.iter().filter(|needed| is_core(needed)).count();
    if cores != 1 {
        problems.push(format!(
            "NEEDED lists `{}` {cores} times, expected exactly once (NEEDED: {:?})",
            abi::SONAME,
            elf.needed
        ));
    }
    if let Some(std) = elf.needed.iter().find(|needed| is_std(needed)) {
        problems.push(format!("needs `{std}`: the launcher links core only"));
    }
    let defined: Vec<&String> = elf.defined.iter().filter(|name| name.starts_with("gnoblin_")).collect();
    if !defined.is_empty() {
        problems.push(format!(
            "defines {defined:?}: a launcher that exports the core's names would take the calls of every shell"
        ));
    }
    problems
}

/// What every shell must satisfy: no copy of the core, one entry symbol, and only symbols the core exports.
fn check_shell_common(elf: &Elf) -> Vec<String> {
    let mut problems = Vec::new();
    if !elf.needed.iter().any(|needed| is_core(needed)) {
        problems.push(format!(
            "does not need `{}`: it must link the installed core (NEEDED: {:?})",
            abi::SONAME,
            elf.needed
        ));
    }
    let defined_core: Vec<&String> = elf.defined.iter().filter(|name| name.starts_with("gnoblin_")).collect();
    if !defined_core.is_empty() {
        problems.push(format!(
            "defines {defined_core:?}: the shell carries its own copy of the core instead of linking the installed one"
        ));
    }
    let entry_only: Vec<&String> = elf
        .defined
        .iter()
        .filter(|name| name.as_str() != abi::SHELL_ENTRY_SYMBOL)
        .collect();
    if !entry_only.is_empty() {
        problems.push(format!(
            "exports {entry_only:?}: a shell exports only `{}`",
            abi::SHELL_ENTRY_SYMBOL
        ));
    }
    if !elf.defined.iter().any(|name| name == abi::SHELL_ENTRY_SYMBOL) {
        problems.push(format!(
            "does not export `{}`: it is not a gnoblin shell",
            abi::SHELL_ENTRY_SYMBOL
        ));
    }
    let exports = core_exports();
    for name in elf.undefined.iter().filter(|name| name.starts_with("gnoblin_")) {
        if !exports.contains(name)
            && !(name.starts_with("gnoblin_std_") && elf.needed.iter().any(|needed| is_std(needed)))
        {
            problems.push(format!("references `{name}`, which the core does not export"));
        }
    }
    problems
}

/// A core-only shell: the core, and nothing of the std tier.
pub fn check_core_only_shell(elf: &Elf) -> Vec<String> {
    let mut problems = check_shell_common(elf);
    if let Some(std) = elf.needed.iter().find(|needed| is_std(needed)) {
        problems.push(format!("needs `{std}`: a core-only shell must not load std"));
    } else if let Some(other) = elf
        .needed
        .iter()
        .find(|needed| needed.starts_with("libgnoblin") && !is_core(needed))
    {
        problems.push(format!(
            "needs `{other}`: a core-only shell links the core and no other gnoblin library"
        ));
    }
    problems
}

/// A std shell: std and the core.
pub fn check_std_shell(elf: &Elf) -> Vec<String> {
    let mut problems = check_shell_common(elf);
    if !elf.needed.iter().any(|needed| needed == STD_SONAME) {
        problems.push(format!("does not need `{STD_SONAME}` (NEEDED: {:?})", elf.needed));
    }
    problems
}

/// The std library: its own SONAME, `NODELETE`, exactly the exports of its `exports.txt`, and the core it builds on (and nothing else
/// of ours), never the reverse.
pub fn check_std_library(elf: &Elf) -> Vec<String> {
    let mut problems = Vec::new();
    if elf.soname.as_deref() != Some(STD_SONAME) {
        problems.push(format!("SONAME is {:?}, expected `{STD_SONAME}`", elf.soname));
    }
    if !elf.is_nodelete() {
        problems.push("is not linked with `-z nodelete` (FLAGS_1 lacks NODELETE): a retired shell generation's registrations point into it".to_owned());
    }
    if !elf.needed.iter().any(|needed| is_core(needed)) {
        problems.push(format!("does not need `{}` (NEEDED: {:?})", abi::SONAME, elf.needed));
    }
    if let Some(other) = elf
        .needed
        .iter()
        .find(|needed| needed.starts_with("libgnoblin") && !is_core(needed))
    {
        problems.push(format!(
            "needs `{other}`: the std library builds on the core and on nothing else of ours"
        ));
    }
    let expected: BTreeSet<String> = gnoblin_std_abi::export_names().map(str::to_owned).collect();
    let defined: BTreeSet<String> = elf.defined.iter().cloned().collect();
    for missing in expected.difference(&defined) {
        problems.push(format!("does not define `{missing}`, which its exports.txt lists"));
    }
    for extra in defined.difference(&expected) {
        let why = if extra.starts_with("__rust") || extra.starts_with("rust_") {
            "a Rust runtime symbol leaked into the dynamic symbol table"
        } else if extra.starts_with("gnoblin_") && !extra.starts_with("gnoblin_std_") {
            "a copy of a core symbol: the library must call the installed core, not carry one"
        } else {
            "it is not in exports.txt"
        };
        problems.push(format!("defines `{extra}`: {why}"));
    }
    problems
}

/// What a language runtime must satisfy (`libgnoblin-runtime-<language>.so`, which the loader opens by file name when a script of
/// its language is selected): a module and not a library (no SONAME, so the loader's private copy cannot be taken for another
/// library), the one export `gnoruntime_entry_v1`, the core it calls through the table and nothing else of ours (a runtime never
/// loads std: a script that wants it reaches it through the installed library), and no symbol the core does not export. A runtime
/// may need the system library of its language (`libpython3.N.so`), which is not ours.
pub fn check_runtime(elf: &Elf) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(soname) = &elf.soname {
        problems.push(format!(
            "has the SONAME `{soname}`: a runtime is a module the loader opens by path, and a SONAME would let the dynamic loader take it for a library"
        ));
    }
    if !elf.needed.iter().any(|needed| is_core(needed)) {
        problems.push(format!(
            "does not need `{}`: it calls the core through the table and one export (NEEDED: {:?})",
            abi::SONAME,
            elf.needed
        ));
    }
    if let Some(other) = elf
        .needed
        .iter()
        .find(|needed| needed.starts_with("libgnoblin") && !is_core(needed))
    {
        problems.push(format!(
            "needs `{other}`: a runtime links the core and no other gnoblin library"
        ));
    }
    let extra: Vec<&String> = elf
        .defined
        .iter()
        .filter(|name| name.as_str() != abi::RUNTIME_ENTRY_SYMBOL)
        .collect();
    if !extra.is_empty() {
        problems.push(format!(
            "exports {extra:?}: a runtime exports only `{}` (a version script hides the rest)",
            abi::RUNTIME_ENTRY_SYMBOL
        ));
    }
    if !elf.defined.iter().any(|name| name == abi::RUNTIME_ENTRY_SYMBOL) {
        problems.push(format!(
            "does not export `{}`: it is not a gnoblin runtime",
            abi::RUNTIME_ENTRY_SYMBOL
        ));
    }
    let exports = core_exports();
    for name in elf.undefined.iter().filter(|name| name.starts_with("gnoblin_")) {
        if !exports.contains(name) {
            problems.push(format!("references `{name}`, which the core does not export"));
        }
    }
    problems
}

/// The newest glibc symbol version any of `files` requires, against an optional ceiling.
pub fn check_glibc(files: &[(String, Elf)], ceiling: Option<(u32, u32)>) -> (Option<(u32, u32)>, Vec<String>) {
    let newest = files.iter().filter_map(|(_, elf)| elf.glibc).max();
    let mut problems = Vec::new();
    if let Some(ceiling) = ceiling {
        for (name, elf) in files {
            if let Some(needs) = elf.glibc.filter(|needs| *needs > ceiling) {
                problems.push(format!(
                    "{name} needs glibc {}.{}, newer than the ceiling {}.{}",
                    needs.0, needs.1, ceiling.0, ceiling.1
                ));
            }
        }
    }
    (newest, problems)
}

/// The directories the dynamic loader searches for a library `file` needs, with `LD_LIBRARY_PATH` unset: the file's run
/// path (`DT_RUNPATH`, else `DT_RPATH`) with `$ORIGIN` expanded to the directory the file really sits in, then `system_dirs`
/// (the directories the loader searches by default, which for a staged `/usr` tree are its own library directory).
pub fn search_dirs(file: &Path, elf: &Elf, system_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let origin = fs::canonicalize(file)
        .ok()
        .and_then(|real| real.parent().map(Path::to_path_buf))
        .unwrap_or_default()
        .display()
        .to_string();
    let mut dirs = Vec::new();
    if let Some(path) = elf.runpath.as_ref().or(elf.rpath.as_ref()) {
        for entry in path.split(':').filter(|entry| !entry.is_empty()) {
            dirs.push(PathBuf::from(
                entry.replace("${ORIGIN}", &origin).replace("$ORIGIN", &origin),
            ));
        }
    }
    dirs.extend(system_dirs.iter().cloned());
    dirs
}

/// Resolves one NEEDED entry of `file` the way the loader would: the first search directory that holds a file of that name,
/// which must carry the name as its SONAME. Returns the path found, or what was searched.
pub fn resolve_needed(file: &Path, elf: &Elf, name: &str, system_dirs: &[PathBuf]) -> Result<PathBuf, String> {
    let dirs = search_dirs(file, elf, system_dirs);
    for dir in &dirs {
        let candidate = dir.join(name);
        if !candidate.exists() {
            continue;
        }
        let found = elf::read(&candidate).map_err(|error| format!("{}: {error}", candidate.display()))?;
        if found.soname.as_deref() != Some(name) {
            return Err(format!(
                "`{name}` resolves to {}, whose SONAME is {:?}",
                candidate.display(),
                found.soname
            ));
        }
        return Ok(fs::canonicalize(&candidate).unwrap_or(candidate));
    }
    Err(format!(
        "`{name}` is found in none of the directories the loader would search for {} (run path {:?}, searched {dirs:?}); \
         install with a libdir the run path names (`lib`, `lib64`), or set GNOBLIN_RPATH when building",
        file.display(),
        elf.runpath.as_ref().or(elf.rpath.as_ref())
    ))
}

/// Resolves every gnoblin library `file` needs. Returns what resolved (name and path, for the report) and the failures;
/// libraries of the system (libc, libwayland) are the loader's own business, which `ldd` covers.
pub fn check_resolution(file: &Path, elf: &Elf, system_dirs: &[PathBuf]) -> (Vec<(String, PathBuf)>, Vec<String>) {
    let mut resolved = Vec::new();
    let mut problems = Vec::new();
    for name in elf.needed.iter().filter(|needed| needed.starts_with("libgnoblin")) {
        match resolve_needed(file, elf, name, system_dirs) {
            Ok(path) => resolved.push((name.clone(), path)),
            Err(problem) => problems.push(problem),
        }
    }
    (resolved, problems)
}

/// The paths the install layout puts in a tree that carries the core, which the manifest
/// check cannot supply: it only proves that what is listed is there.
pub fn check_layout(root: &Path, libdir: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for file in ["bin/gnoblin", "bin/gnoblinctl", "include/gnoblin/core.h"] {
        if !root.join(file).is_file() {
            problems.push(format!("{file} is missing"));
        }
    }
    for dir in [
        "share/gnoblin/shells".to_owned(),
        format!("{libdir}/gnoblin/shells"),
        "share/gnoblin/js/gnoblin-core".to_owned(),
        "share/gnoblin/js/gnoblin-std".to_owned(),
    ] {
        if !root.join(&dir).is_dir() {
            problems.push(format!("{dir}/ is missing"));
        }
    }
    if !root.join("share/gnoblin/default-shell/shell.ts").is_file() {
        problems.push(
            "share/gnoblin/default-shell/shell.ts is missing: the starter is the packaged default shell (design section 7.2, tier 7)"
                .to_owned(),
        );
    }
    for file in [
        "share/gnoblin/types/gnoblin.d.ts",
        "share/gnoblin/types/tsconfig.json",
        "share/gnoblin/types/gnoblin-core/index.ts",
        "share/gnoblin/types/gnoblin-std/index.ts",
    ] {
        if !root.join(file).is_file() {
            problems.push(format!(
                "{file} is missing: the declarations of a TypeScript shell (design section 11)"
            ));
        }
    }
    // The configuration maps the packages to the installed library: every path it names must be there.
    if let Ok(text) = fs::read_to_string(root.join("share/gnoblin/types/tsconfig.json")) {
        for package in ["gnoblin-core", "gnoblin-std"] {
            let wanted = format!("./{package}/index.ts");
            if !text.contains(&wanted) {
                problems.push(format!(
                    "share/gnoblin/types/tsconfig.json does not map \"{package}\" to {wanted}"
                ));
            }
            if !text.contains(&format!("../js/{package}/*"))
                || !root.join(format!("share/gnoblin/js/{package}")).is_dir()
            {
                problems.push(format!(
                    "share/gnoblin/types/tsconfig.json maps the submodules of \"{package}\" to a directory that is not installed"
                ));
            }
            let host = format!("export * from \"{package}/host\";");
            let entry = root.join(format!("share/gnoblin/types/{package}/index.ts"));
            if fs::read_to_string(&entry).is_ok_and(|text| !text.contains(&host)) {
                problems.push(format!(
                    "share/gnoblin/types/{package}/index.ts does not export its host module"
                ));
            }
        }
    }
    for package in ["gnoblin-core", "gnoblin-std"] {
        let index = format!("share/gnoblin/js/{package}/index.ts");
        if !root.join(&index).is_file() {
            problems.push(format!(
                "{index} is missing: the stock JavaScript library is installed as data"
            ));
        }
    }
    // Nothing generated is committed, so these exist only because `dist install` rendered them: the modules the editors open beside
    // the sources that import them, and the stubs of the Python package.
    for file in [
        "share/gnoblin/js/gnoblin-core/generated/abi.ts",
        "share/gnoblin/js/gnoblin-core/generated/schema.ts",
        "share/gnoblin/js/gnoblin-std/generated/classes.ts",
        "share/gnoblin/js/gnoblin-std/generated/objects.d.ts",
        "share/gnoblin/types/python/gnoblin/_generated/classes.pyi",
        "share/gnoblin/types/python/gnoblin/std.pyi",
        "share/gnoblin/types/python/gnoblin/objects/std.pyi",
    ] {
        if !root.join(file).is_file() {
            problems.push(format!(
                "{file} is missing: it is rendered by `cargo xtask dist` from the schema and the ABI crates, and committed nowhere"
            ));
        }
    }
    // The Python package is embedded in the runtime and installed as no source: what an editor reads is the stub package, whose
    // hand-written half is the runtime crate's own.
    for file in [
        "share/gnoblin/types/python/gnoblin/__init__.pyi",
        "share/gnoblin/types/python/gnoblin/_base.pyi",
        "share/gnoblin/types/python/gnoblin/py.typed",
        "share/gnoblin/types/python/_gnoblin_core.pyi",
    ] {
        if !root.join(file).is_file() {
            problems.push(format!(
                "{file} is missing: it is the runtime's own stub, installed by `cargo xtask dist`"
            ));
        }
    }
    problems
}

/// The language runtimes in `dir` (`libgnoblin-runtime-<language>.so`, which sit directly in the core's `gnoblin/` directory), sorted.
pub fn installed_runtimes(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libgnoblin-runtime-") && name.ends_with(".so"))
        })
        .collect();
    found.sort();
    found
}

/// The shared objects under `dir` that a shell package installed, found recursively and sorted.
pub fn installed_shells(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "so") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Runs the launcher as a user would after an install, with the loader's own environment cleared, and returns what went wrong.
/// The `--version` run proves the dynamic link works with no `LD_LIBRARY_PATH` (the run path alone finds the core); the
/// `--role compositor --help` run proves the role dispatch starts from the install tree.
pub fn check_runs(launcher: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for arguments in [&["--version"][..], &["--role", "compositor", "--help"][..]] {
        let output = Command::new(launcher)
            .args(arguments)
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .output();
        match output {
            Ok(output) if output.status.success() && !output.stdout.is_empty() => {}
            Ok(output) => problems.push(format!(
                "`{} {}` exited with {} and printed {} bytes; stderr: {}",
                launcher.display(),
                arguments.join(" "),
                output.status,
                output.stdout.len(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => problems.push(format!(
                "cannot run `{} {}`: {error}",
                launcher.display(),
                arguments.join(" ")
            )),
        }
    }
    problems
}

/// Loads `shell` with the installed launcher as a session would (`gnoblin --role check SHELL`), with the loader's own environment
/// and `GNOBLIN_STD_DIR` cleared, and returns what went wrong. A shell has no run path of its own, so this is what proves the
/// install finds the libraries a shell needs (the core, and std for a std shell) from wherever the tree was installed.
pub fn check_shell_loads(launcher: &Path, shell: &Path) -> Vec<String> {
    let output = Command::new(launcher)
        .args(["--role", "check"])
        .arg(shell)
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .env_remove("GNOBLIN_STD_DIR")
        .output();
    match output {
        Ok(output) if output.status.success() && output.stdout.starts_with(b"ok") => Vec::new(),
        Ok(output) => vec![format!(
            "`{} --role check {}` exited with {}: {}",
            launcher.display(),
            shell.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout).trim()
        )],
        Err(error) => vec![format!(
            "cannot run `{} --role check {}`: {error}",
            launcher.display(),
            shell.display()
        )],
    }
}

/// The symlink chain of a versioned library: `<stem>.so.<major>.<minor>.<patch>` is the file, `<stem>.so.<major>` points at
/// it, and `<stem>.so` points at that, all relative links in one directory.
pub fn check_links(dir: &Path, stem: &str, major: u32, minor: u32, patch: u32) -> Vec<String> {
    let mut problems = Vec::new();
    let versioned = format!("{stem}.so.{major}.{minor}.{patch}");
    let soname = format!("{stem}.so.{major}");
    let linker = format!("{stem}.so");
    match fs::symlink_metadata(dir.join(&versioned)) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => problems.push(format!("{} is not a regular file", dir.join(&versioned).display())),
        Err(error) => problems.push(format!("{}: {error}", dir.join(&versioned).display())),
    }
    for (link, target) in [(&soname, &versioned), (&linker, &soname)] {
        let path = dir.join(link);
        match fs::read_link(&path) {
            Ok(found) if found == Path::new(target) => {}
            Ok(found) => problems.push(format!(
                "{} points at `{}`, expected `{target}`",
                path.display(),
                found.display()
            )),
            Err(error) => problems.push(format!("{} is not a symlink to `{target}`: {error}", path.display())),
        }
    }
    problems
}

/// The fields of a pkg-config file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PkgConfig {
    pub variables: Vec<(String, String)>,
    pub fields: Vec<(String, String)>,
}

impl PkgConfig {
    fn variable(&self, name: &str) -> Option<&str> {
        self.variables
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn field(&self, name: &str) -> Option<String> {
        let raw = &self.fields.iter().find(|(key, _)| key == name)?.1;
        Some(self.expand(raw))
    }

    /// Replaces `${name}` with the variable's own expanded value.
    fn expand(&self, text: &str) -> String {
        let mut result = text.to_owned();
        for _ in 0..8 {
            let Some(start) = result.find("${") else { break };
            let Some(end) = result[start..].find('}') else { break };
            let name = result[start + 2..start + end].to_owned();
            let value = self.variable(&name).unwrap_or("").to_owned();
            result.replace_range(start..start + end + 1, &value);
        }
        result
    }
}

/// Parses `name=value` variables and `Key: value` fields.
pub fn parse_pc(text: &str) -> Result<PkgConfig, String> {
    let mut pc = PkgConfig::default();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let colon = line.find(':');
        let equals = line.find('=');
        match (colon, equals) {
            (Some(colon), equals) if equals.is_none_or(|equals| colon < equals) => {
                pc.fields
                    .push((line[..colon].trim().to_owned(), line[colon + 1..].trim().to_owned()));
            }
            (_, Some(equals)) => pc
                .variables
                .push((line[..equals].trim().to_owned(), line[equals + 1..].trim().to_owned())),
            _ => return Err(format!("line {}: expected `name=value` or `Key: value`", index + 1)),
        }
    }
    Ok(pc)
}

/// The development package's pkg-config file for the core.
pub fn check_pc(pc: &PkgConfig, libdir_name: &str) -> Vec<String> {
    check_pc_for(pc, libdir_name, &CORE_LIBRARY)
}

/// The pkg-config file of `library`: its name and version, what it links and includes, and the module it requires.
pub fn check_pc_for(pc: &PkgConfig, libdir_name: &str, library: &Library) -> Vec<String> {
    let mut problems = Vec::new();
    let (major, minor, patch) = library.version;
    let version = format!("{major}.{minor}.{patch}");
    if pc.field("Name").as_deref() != Some(library.module) {
        problems.push(format!("Name is {:?}, expected `{}`", pc.field("Name"), library.module));
    }
    if pc.field("Version").as_deref() != Some(version.as_str()) {
        problems.push(format!("Version is {:?}, expected `{version}`", pc.field("Version")));
    }
    let link = format!("-l{}", library.module);
    match pc.field("Libs") {
        Some(libs) if libs.contains(&link) && libs.contains(&format!("-L{}", pc.expand("${libdir}"))) => {}
        other => problems.push(format!("Libs is {other:?}, expected `-L${{libdir}} {link}`")),
    }
    match pc.field("Cflags") {
        Some(cflags) if cflags.contains(&pc.expand("-I${includedir}")) => {}
        other => problems.push(format!("Cflags is {other:?}, expected `-I${{includedir}}`")),
    }
    if pc.field("Requires").as_deref() != library.requires {
        problems.push(format!(
            "Requires is {:?}, expected {:?}",
            pc.field("Requires"),
            library.requires
        ));
    }
    let libdir = pc.expand("${libdir}");
    if !libdir.ends_with(&format!("/{libdir_name}")) {
        problems.push(format!(
            "libdir is `{libdir}`, expected a path ending in `/{libdir_name}`"
        ));
    }
    problems
}

/// Runs `ldd` and returns the libraries it could not find.
fn ldd_missing(path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("ldd")
        .arg(path)
        .env_remove("LD_LIBRARY_PATH")
        .output()
        .map_err(|error| format!("`ldd` is needed for this check: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .filter(|line| line.contains("not found"))
        .map(|line| line.trim().to_owned())
        .collect())
}

/// Builds a C program against the installed development files through `pkg-config` and runs it.
///
/// Returns problems, or `Ok(None)`-style notes through the second vector when a tool is missing (a missing tool is
/// reported, never silent).
pub fn check_pkg_config_link(root: &Path, libdir_name: &str, scratch: &Path) -> (Vec<String>, Vec<String>) {
    check_pkg_config_link_for(root, libdir_name, scratch, &CORE_LIBRARY)
}

/// One development library: the pkg-config module a consumer asks for, the header it includes, the function that reports the
/// version, and the version the install should report.
pub struct Library {
    pub module: &'static str,
    pub header: &'static str,
    pub version_fn: &'static str,
    pub version: (u32, u32, u32),
    /// The module its `.pc` requires, if any.
    pub requires: Option<&'static str>,
}

/// The core's development files.
pub const CORE_LIBRARY: Library = Library {
    module: "gnoblin-core",
    header: "gnoblin/core.h",
    version_fn: "gnoblin_abi_version",
    version: (abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH),
    requires: None,
};

/// The std library's development files.
pub const STD_LIBRARY: Library = Library {
    module: "gnoblin-std",
    header: "gnoblin/std.h",
    version_fn: "gnoblin_std_abi_version",
    version: (
        gnoblin_std_abi::ABI_MAJOR,
        gnoblin_std_abi::ABI_MINOR,
        gnoblin_std_abi::ABI_PATCH,
    ),
    requires: Some("gnoblin-core"),
};

/// [`check_pkg_config_link`] for `library`.
pub fn check_pkg_config_link_for(
    root: &Path,
    libdir_name: &str,
    scratch: &Path,
    library: &Library,
) -> (Vec<String>, Vec<String>) {
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let pc_dir = root.join(libdir_name).join("pkgconfig");
    let flags = Command::new("pkg-config")
        .arg(format!("--define-variable=prefix={}", root.display()))
        .args(["--cflags", "--libs", library.module])
        .env("PKG_CONFIG_PATH", &pc_dir)
        .env("PKG_CONFIG_LIBDIR", &pc_dir)
        .output();
    let flags = match flags {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Ok(output) => {
            problems.push(format!(
                "`pkg-config --cflags --libs {}` failed: {}",
                library.module,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            return (problems, notes);
        }
        Err(error) => {
            notes.push(format!(
                "skipped the pkg-config link check: `pkg-config` is not available ({error})"
            ));
            return (problems, notes);
        }
    };
    let source = scratch.join("probe.c");
    let program = scratch.join("probe");
    let text = format!(
        "#include <{}>\n#include <stdio.h>\n\
         int main(void) {{\n  uint32_t major = 0, minor = 0, patch = 0;\n  {}(&major, &minor, &patch);\n  \
         printf(\"%u.%u.%u\\n\", major, minor, patch);\n  return 0;\n}}\n",
        library.header, library.version_fn
    );
    if let Err(error) = fs::write(&source, text) {
        problems.push(format!("cannot write {}: {error}", source.display()));
        return (problems, notes);
    }
    let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    let compile = Command::new(&compiler)
        .arg("-o")
        .arg(&program)
        .arg(&source)
        .args(flags.split_whitespace())
        .output();
    match compile {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            problems.push(format!(
                "a C program using `pkg-config --cflags --libs {}` did not compile and link: {}",
                library.module,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            return (problems, notes);
        }
        Err(error) => {
            notes.push(format!(
                "skipped the pkg-config link check: the C compiler `{compiler}` is not available ({error})"
            ));
            return (problems, notes);
        }
    }
    match Command::new(&program)
        .env("LD_LIBRARY_PATH", root.join(libdir_name))
        .output()
    {
        Ok(output) if output.status.success() => {
            let printed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let (major, minor, patch) = library.version;
            let expected = format!("{major}.{minor}.{patch}");
            if printed != expected {
                problems.push(format!(
                    "the probe program printed ABI {printed}, the installed {} should be {expected}",
                    library.module
                ));
            }
        }
        Ok(output) => problems.push(format!(
            "the probe program failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => problems.push(format!("cannot run the probe program: {error}")),
    }
    (problems, notes)
}

/// What to check.
#[derive(Debug, Default, PartialEq, Eq)]
struct CheckOptions {
    prefix: Option<String>,
    destdir: PathBuf,
    libdir: String,
    core: Option<PathBuf>,
    launcher: Option<PathBuf>,
    std: Option<PathBuf>,
    shells: Vec<PathBuf>,
    std_shells: Vec<PathBuf>,
    max_glibc: Option<(u32, u32)>,
    /// Check the build tree `cargo xtask stage` made: the launcher and the staged core in `<target>/<profile>`.
    staged: bool,
    /// The directory under the target directory `--staged` reads.
    profile: String,
    target_dir: PathBuf,
    /// Run the launcher the way a user would (see [`check_runs`]).
    run: bool,
}

fn parse_args(args: &[String]) -> Result<CheckOptions, String> {
    let mut options = CheckOptions {
        libdir: "lib".to_owned(),
        profile: "debug".to_owned(),
        target_dir: std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from),
        ..CheckOptions::default()
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().cloned().ok_or_else(|| format!("`{flag}` needs a value"));
        match flag.as_str() {
            "--prefix" => options.prefix = Some(value()?),
            "--destdir" => options.destdir = PathBuf::from(value()?),
            "--libdir" => options.libdir = value()?,
            "--core" => options.core = Some(PathBuf::from(value()?)),
            "--launcher" => options.launcher = Some(PathBuf::from(value()?)),
            "--std" => options.std = Some(PathBuf::from(value()?)),
            "--shell" => options.shells.push(PathBuf::from(value()?)),
            "--std-shell" => options.std_shells.push(PathBuf::from(value()?)),
            "--staged" => options.staged = true,
            "--profile" => options.profile = value()?,
            "--run" => options.run = true,
            "--max-glibc" => {
                let text = value()?;
                let (major, minor) = text
                    .split_once('.')
                    .ok_or_else(|| format!("`{text}` is not a glibc version like 2.31"))?;
                options.max_glibc = Some((
                    major
                        .parse()
                        .map_err(|_| format!("`{text}` is not a glibc version like 2.31"))?,
                    minor
                        .parse()
                        .map_err(|_| format!("`{text}` is not a glibc version like 2.31"))?,
                ));
            }
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    if options.prefix.is_none()
        && !options.staged
        && options.core.is_none()
        && options.launcher.is_none()
        && options.std.is_none()
        && options.shells.is_empty()
        && options.std_shells.is_empty()
    {
        return Err("nothing to check: give `--prefix`, `--staged`, or at least one file".to_owned());
    }
    if options.run && options.prefix.is_none() && !options.staged && options.launcher.is_none() {
        return Err("`--run` needs a launcher: give `--prefix`, `--staged` or `--launcher`".to_owned());
    }
    Ok(options)
}

/// Collects the outcome of each check and prints it.
#[derive(Default)]
struct CheckOutcome {
    failures: usize,
}

impl CheckOutcome {
    fn report(&mut self, what: &str, problems: &[String]) {
        if problems.is_empty() {
            println!("[dist-check] ok    {what}");
        } else {
            for problem in problems {
                println!("[dist-check] FAIL  {what}: {problem}");
            }
            self.failures += problems.len();
        }
    }

    fn note(what: &str) {
        println!("[dist-check] note  {what}");
    }
}

fn read_elf(path: &Path, outcome: &mut CheckOutcome, files: &mut Vec<(String, Elf)>) -> Option<Elf> {
    match elf::read(path) {
        Ok(elf) => {
            files.push((path.display().to_string(), elf.clone()));
            Some(elf)
        }
        Err(error) => {
            outcome.report(&path.display().to_string(), &[error]);
            None
        }
    }
}

fn run_checks(options: &CheckOptions) -> Result<usize, String> {
    let mut outcome = CheckOutcome::default();
    let mut files: Vec<(String, Elf)> = Vec::new();
    let root = options
        .prefix
        .as_ref()
        .map(|prefix| crate::dist::install_root(&options.destdir, prefix));
    let libdir = root.as_ref().map(|root| root.join(&options.libdir));
    let staged_dir = options.staged.then(|| options.target_dir.join(&options.profile));
    // A distribution prefix needs no run path: the loader searches `/usr/<libdir>` itself. Any other prefix is found only
    // through the binary's own run path, so the resolution check gets no system directory for it.
    let system_dirs: Vec<PathBuf> = match (&libdir, options.prefix.as_deref()) {
        (Some(libdir), Some("/usr")) => vec![libdir.clone()],
        _ => Vec::new(),
    };

    let core_path = options
        .core
        .clone()
        .or_else(|| libdir.as_ref().map(|libdir| libdir.join(abi::SONAME)))
        .or_else(|| staged_dir.as_ref().map(|dir| dir.join(abi::SONAME)));
    if let Some(path) = &core_path
        && let Some(core) = read_elf(path, &mut outcome, &mut files)
    {
        outcome.report(&format!("core {}", path.display()), &check_core(&core));
    }
    if let (Some(libdir), Some(_)) = (&libdir, &core_path) {
        let problems = check_links(
            libdir,
            "libgnoblin-core",
            abi::ABI_MAJOR,
            abi::ABI_MINOR,
            abi::ABI_PATCH,
        );
        outcome.report(&format!("SONAME links in {}", libdir.display()), &problems);
    }

    let launcher_path = options
        .launcher
        .clone()
        .or_else(|| {
            root.as_ref()
                .map(|root| root.join("bin/gnoblin"))
                .filter(|path| core_path.is_some() && path.exists())
        })
        .or_else(|| staged_dir.as_ref().map(|dir| dir.join("gnoblin")));
    if let Some(path) = &launcher_path {
        let mut resolved = Vec::new();
        if let Some(launcher) = read_elf(path, &mut outcome, &mut files) {
            outcome.report(&format!("launcher {}", path.display()), &check_launcher(&launcher));
            let (found, problems) = check_resolution(path, &launcher, &system_dirs);
            for (name, found) in &found {
                println!("[dist-check] info  {name} resolves to {}", found.display());
            }
            outcome.report(
                &format!("loader resolution of {} with LD_LIBRARY_PATH unset", path.display()),
                &problems,
            );
            resolved = found;
        }
        // The loader's own answer for everything else the launcher needs. A gnoblin library the model above resolved
        // through the staged `/usr` tree is not on this machine's search path, so `ldd` cannot see it.
        match ldd_missing(path) {
            Ok(missing) => {
                let missing: Vec<String> = missing
                    .into_iter()
                    .filter(|line| !resolved.iter().any(|(name, _)| line.starts_with(name.as_str())))
                    .collect();
                outcome.report(&format!("ldd {}", path.display()), &missing);
            }
            Err(error) => outcome.report(&format!("ldd {}", path.display()), &[error]),
        }
        if options.run {
            outcome.report(&format!("{} runs from its own tree", path.display()), &check_runs(path));
            for shell in options.shells.iter().chain(&options.std_shells) {
                outcome.report(
                    &format!("{} loads {}", path.display(), shell.display()),
                    &check_shell_loads(path, shell),
                );
            }
        }
    }

    let std_path = options
        .std
        .clone()
        .or_else(|| {
            libdir
                .as_ref()
                .map(|libdir| libdir.join(STD_SONAME))
                .filter(|path| path.exists())
        })
        .or_else(|| {
            staged_dir
                .as_ref()
                .map(|dir| dir.join(STD_SONAME))
                .filter(|path| path.exists())
        });
    if let Some(path) = &std_path
        && let Some(std) = read_elf(path, &mut outcome, &mut files)
    {
        outcome.report(&format!("std {}", path.display()), &check_std_library(&std));
        // A library in a private prefix must find the core through its own run path; one in the library directory of a
        // distribution prefix needs none.
        let (_, problems) = check_resolution(path, &std, &system_dirs);
        outcome.report(&format!("loader resolution of {}", path.display()), &problems);
        if let Some(libdir) = libdir.as_ref().filter(|libdir| path.starts_with(libdir)) {
            let problems = check_links(
                libdir,
                "libgnoblin-std",
                gnoblin_std_abi::ABI_MAJOR,
                gnoblin_std_abi::ABI_MINOR,
                gnoblin_std_abi::ABI_PATCH,
            );
            outcome.report(&format!("std SONAME links in {}", libdir.display()), &problems);
        }
    }
    for path in &options.shells {
        if let Some(shell) = read_elf(path, &mut outcome, &mut files) {
            outcome.report(
                &format!("core-only shell {}", path.display()),
                &check_core_only_shell(&shell),
            );
        }
    }
    for path in &options.std_shells {
        if let Some(shell) = read_elf(path, &mut outcome, &mut files) {
            outcome.report(&format!("std shell {}", path.display()), &check_std_shell(&shell));
        }
    }
    // Shells a package installed into the core's module directory. They are loaded by a process that already holds the
    // core, so they need no run path; they must still link it and carry no copy of it.
    if let (Some(libdir), Some(_)) = (&libdir, &core_path) {
        for path in installed_shells(&libdir.join("gnoblin/shells")) {
            if let Some(shell) = read_elf(&path, &mut outcome, &mut files) {
                let problems = if shell.needed.iter().any(|needed| is_std(needed)) {
                    check_std_shell(&shell)
                } else {
                    check_core_only_shell(&shell)
                };
                outcome.report(&format!("installed shell {}", path.display()), &problems);
            }
        }
    }

    // The language runtimes the install carries (`<libdir>/gnoblin/libgnoblin-runtime-*.so`) and the ones a build tree has staged
    // (`<profile>/runtimes/`, `cargo xtask stage`).
    let runtime_dirs = libdir
        .iter()
        .map(|libdir| libdir.join("gnoblin"))
        .chain(staged_dir.iter().map(|dir| dir.join(crate::stage::RUNTIMES_DIR)));
    for dir in runtime_dirs {
        for path in installed_runtimes(&dir) {
            if let Some(runtime) = read_elf(&path, &mut outcome, &mut files) {
                outcome.report(&format!("runtime {}", path.display()), &check_runtime(&runtime));
            }
        }
    }

    if let (Some(root), true) = (&root, core_path.is_some()) {
        let (problems, listed) = crate::dist::manifest_problems(root)?;
        outcome.report(&format!("install manifest, {listed} paths"), &problems);
        outcome.report(
            "install layout (design section 11)",
            &check_layout(root, &options.libdir),
        );

        let header = root.join("include/gnoblin/core.h");
        let expected = crate::abi_c::render_header(&gnoblin_core_abi::reflect::reflection())?;
        let problems = match fs::read_to_string(&header) {
            Ok(text) if text == expected => Vec::new(),
            Ok(_) => vec![format!(
                "{} differs from the header `cargo xtask abi-header` generates",
                header.display()
            )],
            Err(error) => vec![format!("{}: {error}", header.display())],
        };
        outcome.report(&format!("header {}", header.display()), &problems);

        let pc_path = root.join(&options.libdir).join("pkgconfig/gnoblin-core.pc");
        let problems = match fs::read_to_string(&pc_path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse_pc(&text))
        {
            Ok(pc) => check_pc(&pc, &options.libdir),
            Err(error) => vec![format!("{}: {error}", pc_path.display())],
        };
        outcome.report(&format!("pkg-config {}", pc_path.display()), &problems);

        let scratch = std::env::temp_dir().join(format!("gnoblin-dist-check-{}", std::process::id()));
        fs::create_dir_all(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))?;
        let (problems, notes) = check_pkg_config_link(root, &options.libdir, &scratch);
        let _ = fs::remove_dir_all(&scratch);
        for note in notes {
            CheckOutcome::note(&note);
        }
        outcome.report(
            "a C program linked through pkg-config runs and reports the ABI version",
            &problems,
        );

        // The std library, when the install carries it: the same development files, and a pkg-config module that pulls the core in.
        if std_path
            .as_ref()
            .is_some_and(|path| libdir.as_ref().is_some_and(|dir| path.starts_with(dir)))
        {
            let header = root.join("include/gnoblin/std.h");
            let expected = crate::abi_c::render_header_with(&gnoblin_std_abi::reflection(), &crate::abi_c::STD_HEADER)?;
            let problems = match fs::read_to_string(&header) {
                Ok(text) if text == expected => Vec::new(),
                Ok(_) => vec![format!(
                    "{} differs from the header `cargo xtask abi-header` generates",
                    header.display()
                )],
                Err(error) => vec![format!("{}: {error}", header.display())],
            };
            outcome.report(&format!("header {}", header.display()), &problems);

            let pc_path = root.join(&options.libdir).join("pkgconfig/gnoblin-std.pc");
            let problems = match fs::read_to_string(&pc_path)
                .map_err(|error| error.to_string())
                .and_then(|text| parse_pc(&text))
            {
                Ok(pc) => check_pc_for(&pc, &options.libdir, &STD_LIBRARY),
                Err(error) => vec![format!("{}: {error}", pc_path.display())],
            };
            outcome.report(&format!("pkg-config {}", pc_path.display()), &problems);

            let scratch = std::env::temp_dir().join(format!("gnoblin-dist-check-std-{}", std::process::id()));
            fs::create_dir_all(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))?;
            let (problems, notes) = check_pkg_config_link_for(root, &options.libdir, &scratch, &STD_LIBRARY);
            let _ = fs::remove_dir_all(&scratch);
            for note in notes {
                CheckOutcome::note(&note);
            }
            outcome.report(
                "a C program linked through pkg-config against the std library runs and reports its ABI version",
                &problems,
            );
        }
    }

    // The glibc floor: the newest symbol version each file needs, so a build on a newer machine than the oldest target
    // shows up as a number before it shows up as a loader error on the target. `--max-glibc` turns it into a failure.
    for (name, elf) in &files {
        if let Some((major, minor)) = elf.glibc {
            println!("[dist-check] info  glibc {major}.{minor} is the newest symbol version {name} needs");
        }
    }
    let (newest, problems) = check_glibc(&files, options.max_glibc);
    match newest {
        Some((major, minor)) => println!("[dist-check] info  minimum glibc for these files: {major}.{minor}"),
        None => println!("[dist-check] info  no glibc symbol versions required"),
    }
    outcome.report("glibc ceiling", &problems);
    Ok(outcome.failures)
}

/// Entry point for `cargo xtask dist-check`.
pub fn run(args: &[String]) -> ExitCode {
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("[dist-check] {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run_checks(&options) {
        Ok(0) => {
            println!("[dist-check] all checks passed");
            ExitCode::SUCCESS
        }
        Ok(failures) => {
            eprintln!("[dist-check] {failures} problem(s)");
            ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("[dist-check] {message}");
            ExitCode::from(2)
        }
    }
}

/// Checks an installed tree, as the tests of `dist install` do on what it wrote. Returns the number of failed checks.
#[cfg(test)]
pub(crate) fn check_installed(destdir: &Path, prefix: &str, libdir: &str) -> Result<usize, String> {
    run_checks(&CheckOptions {
        prefix: Some(prefix.to_owned()),
        destdir: destdir.to_owned(),
        libdir: libdir.to_owned(),
        ..CheckOptions::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{ENTRY, cc, exports_source, good_core, good_std, scratch, shell};

    #[test]
    fn a_good_core_passes_and_each_defect_is_named() {
        let dir = scratch("core");
        let core = good_core(&dir);
        assert_eq!(check_core(&elf::read(&core).unwrap()), Vec::<String>::new());

        // A Rust runtime symbol leaked, and the test driver's export left in.
        cc(
            &dir,
            "leaky.c",
            &exports_source("void __rust_alloc(void) {}\nvoid gnoblin_test_main(void) {}\n", ""),
            &[
                "-shared",
                "-fPIC",
                "-Wl,-soname,libgnoblin-core.so.1",
                "-Wl,-z,nodelete",
                "-o",
                "leaky.so",
            ],
        );
        let problems = check_core(&elf::read(&dir.join("leaky.so")).unwrap());
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("`__rust_alloc`") && problem.contains("Rust runtime symbol")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("`gnoblin_test_main`") && problem.contains("test-driver")),
            "{problems:?}"
        );

        // A missing export, no NODELETE, the wrong SONAME.
        cc(
            &dir,
            "short.c",
            &exports_source("", "gnoblin_catalog_hash"),
            &["-shared", "-fPIC", "-Wl,-soname,libother.so.1", "-o", "short.so"],
        );
        let problems = check_core(&elf::read(&dir.join("short.so")).unwrap());
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("does not define `gnoblin_catalog_hash`")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|problem| problem.contains("nodelete")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|problem| problem.contains("SONAME")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_core_only_shell_passes_and_a_copy_of_the_core_is_caught() {
        let dir = scratch("shell");
        good_core(&dir);
        let good = shell(&dir, "good.so", ENTRY, &[]);
        assert_eq!(check_core_only_shell(&elf::read(&good).unwrap()), Vec::<String>::new());

        // The negative control: a shell that defines the core's symbols itself.
        let copy = shell(
            &dir,
            "copy.so",
            &format!("{ENTRY}void gnoblin_abi_version(void) {{}}\n"),
            &[],
        );
        let problems = check_core_only_shell(&elf::read(&copy).unwrap());
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("carries its own copy of the core")),
            "{problems:?}"
        );

        // A shell that exports more than the entry symbol, and one that is not a shell at all.
        let extra = shell(&dir, "extra.so", &format!("{ENTRY}void helper(void) {{}}\n"), &[]);
        assert!(
            check_core_only_shell(&elf::read(&extra).unwrap())
                .iter()
                .any(|problem| problem.contains("exports"))
        );
        let plain = shell(&dir, "plain.so", "void other(void) {}\n", &[]);
        let problems = check_core_only_shell(&elf::read(&plain).unwrap());
        assert!(
            problems.iter().any(|problem| problem.contains("not a gnoblin shell")),
            "{problems:?}"
        );

        // A shell that does not link the core records no NEEDED for it.
        cc(&dir, "bare.c", ENTRY, &["-shared", "-fPIC", "-o", "bare.so"]);
        let problems = check_core_only_shell(&elf::read(&dir.join("bare.so")).unwrap());
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("must link the installed core")),
            "{problems:?}"
        );
    }

    #[test]
    fn std_is_required_of_a_std_shell_and_forbidden_of_a_core_only_one() {
        let dir = scratch("std");
        good_core(&dir);
        // A stand-in std library: its own SONAME and exports, linked against the core.
        good_std(&dir);
        let std = elf::read(&dir.join("libgnoblin-std.so")).unwrap();
        assert_eq!(check_std_library(&std), Vec::<String>::new());

        let with_std = shell(
            &dir,
            "with_std.so",
            &format!(
                "{ENTRY}extern void gnoblin_std_get_api(void);\n__attribute__((visibility(\"hidden\"))) void *use(void) {{ return (void *)gnoblin_std_get_api; }}\n"
            ),
            &["-lgnoblin-std"],
        );
        let elf = elf::read(&with_std).unwrap();
        assert_eq!(check_std_shell(&elf), Vec::<String>::new(), "{:?}", elf.needed);
        assert!(
            check_core_only_shell(&elf)
                .iter()
                .any(|problem| problem.contains("must not load std"))
        );

        let core_only = shell(&dir, "core_only.so", ENTRY, &[]);
        assert!(
            check_std_shell(&elf::read(&core_only).unwrap())
                .iter()
                .any(|problem| problem.contains(STD_SONAME))
        );

        // A std library that does not link the core is wrong.
        cc(
            &dir,
            "lone.c",
            "void f(void) {}\n",
            &["-shared", "-fPIC", "-Wl,-soname,libgnoblin-std.so.1", "-o", "lone.so"],
        );
        assert!(
            check_std_library(&elf::read(&dir.join("lone.so")).unwrap())
                .iter()
                .any(|problem| problem.contains("does not need"))
        );

        // A std library that carries a core symbol, defines something outside its exports, and is not NODELETE is wrong three ways.
        cc(
            &dir,
            "leaky_std.c",
            "extern int gnoblin_get_api(unsigned, unsigned, const void **);\nvoid gnoblin_std_get_api(void) { gnoblin_get_api(1, 0, 0); }\n\
             void gnoblin_get_node(void) {}\nvoid helper(void) {}\n",
            &[
                "-shared",
                "-fPIC",
                "-L",
                ".",
                "-lgnoblin-core",
                "-Wl,-soname,libgnoblin-std.so.1",
                "-o",
                "leaky_std.so",
            ],
        );
        let problems = check_std_library(&elf::read(&dir.join("leaky_std.so")).unwrap());
        assert!(problems.iter().any(|p| p.contains("NODELETE")), "{problems:?}");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("gnoblin_get_node") && p.contains("copy of a core symbol")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("helper") && p.contains("not in exports.txt")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("does not define `gnoblin_std_abi_version`")),
            "{problems:?}"
        );
    }

    #[test]
    fn the_launcher_must_record_the_core_exactly_once() {
        let dir = scratch("launcher");
        good_core(&dir);
        let main = "extern void gnoblin_abi_version(void);\nint main(void) { gnoblin_abi_version(); return 0; }\n";
        cc(&dir, "ok.c", main, &["-L", ".", "-lgnoblin-core", "-o", "ok"]);
        assert_eq!(
            check_launcher(&elf::read(&dir.join("ok")).unwrap()),
            Vec::<String>::new()
        );
        // Linked twice by name, the linker still records one NEEDED; a launcher that does not link it records none.
        cc(&dir, "none.c", "int main(void) { return 0; }\n", &["-o", "none"]);
        let problems = check_launcher(&elf::read(&dir.join("none")).unwrap());
        assert!(
            problems.iter().any(|problem| problem.contains("0 times")),
            "{problems:?}"
        );
    }

    #[test]
    fn the_soname_link_chain_is_checked() {
        let dir = scratch("links");
        let (major, minor, patch) = (abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH);
        let versioned = format!("libgnoblin-core.so.{major}.{minor}.{patch}");
        fs::write(dir.join(&versioned), b"x").unwrap();
        std::os::unix::fs::symlink(&versioned, dir.join(format!("libgnoblin-core.so.{major}"))).unwrap();
        std::os::unix::fs::symlink(format!("libgnoblin-core.so.{major}"), dir.join("libgnoblin-core.so")).unwrap();
        assert_eq!(
            check_links(&dir, "libgnoblin-core", major, minor, patch),
            Vec::<String>::new()
        );

        // A linker name that points straight at the file skips the SONAME link: a build against it records the wrong name.
        fs::remove_file(dir.join("libgnoblin-core.so")).unwrap();
        std::os::unix::fs::symlink(&versioned, dir.join("libgnoblin-core.so")).unwrap();
        let problems = check_links(&dir, "libgnoblin-core", major, minor, patch);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("expected `libgnoblin-core.so.1`")),
            "{problems:?}"
        );

        // A regular file where a link belongs.
        fs::remove_file(dir.join(format!("libgnoblin-core.so.{major}"))).unwrap();
        fs::write(dir.join(format!("libgnoblin-core.so.{major}")), b"x").unwrap();
        assert!(
            check_links(&dir, "libgnoblin-core", major, minor, patch)
                .iter()
                .any(|problem| problem.contains("not a symlink"))
        );
        assert!(!check_links(&dir.join("missing"), "libgnoblin-core", major, minor, patch).is_empty());
    }

    #[test]
    fn a_pkg_config_file_is_parsed_with_variable_expansion_and_checked() {
        let text = format!(
            "prefix=/usr\nlibdir=${{prefix}}/lib64\nincludedir=${{prefix}}/include\n\nName: gnoblin-core\nDescription: x\nVersion: {}.{}.{}\nLibs: -L${{libdir}} -lgnoblin-core\nCflags: -I${{includedir}}\n",
            abi::ABI_MAJOR,
            abi::ABI_MINOR,
            abi::ABI_PATCH
        );
        let pc = parse_pc(&text).unwrap();
        assert_eq!(pc.expand("${libdir}"), "/usr/lib64");
        assert_eq!(check_pc(&pc, "lib64"), Vec::<String>::new());
        assert!(check_pc(&pc, "lib").iter().any(|problem| problem.contains("libdir")));

        let wrong = parse_pc("prefix=/usr\nlibdir=${prefix}/lib\nincludedir=${prefix}/include\nName: other\nVersion: 9.9.9\nLibs: -lfoo\nCflags:\n").unwrap();
        let problems = check_pc(&wrong, "lib");
        assert!(problems.iter().any(|problem| problem.contains("Name")));
        assert!(problems.iter().any(|problem| problem.contains("Version")));
        assert!(problems.iter().any(|problem| problem.contains("Libs")));
        assert!(parse_pc("this is not a pc file").is_err());
    }

    #[test]
    fn the_glibc_ceiling_names_the_file_that_breaks_it() {
        let old = Elf {
            glibc: Some((2, 17)),
            ..Elf::default()
        };
        let new = Elf {
            glibc: Some((2, 35)),
            ..Elf::default()
        };
        let files = vec![("a.so".to_owned(), old), ("b.so".to_owned(), new)];
        let (newest, problems) = check_glibc(&files, Some((2, 31)));
        assert_eq!(newest, Some((2, 35)));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("b.so needs glibc 2.35"));
        assert!(check_glibc(&files, None).1.is_empty());
    }

    #[test]
    fn arguments_need_something_to_check_and_reject_the_unknown() {
        let words = |text: &[&str]| text.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&words(&["--frob"])).is_err());
        assert!(parse_args(&words(&["--shell"])).is_err());
        assert!(parse_args(&words(&["--max-glibc", "two"])).is_err());
        let options = parse_args(&words(&[
            "--prefix",
            "/usr",
            "--libdir",
            "lib64",
            "--shell",
            "a.so",
            "--max-glibc",
            "2.31",
        ]))
        .unwrap();
        assert_eq!(options.libdir, "lib64");
        assert_eq!(options.max_glibc, Some((2, 31)));
        assert_eq!(options.shells, vec![PathBuf::from("a.so")]);
    }

    /// A tree like an install: `root/bin/<name>` linked against a fixture core and carrying `rpath`, and the core in
    /// `root/<libdir>/libgnoblin-core.so.1`. Returns the root and the launcher.
    fn tree(name: &str, rpath: &str, libdir: &str) -> (PathBuf, PathBuf) {
        let dir = scratch(name);
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let root = dir.join("root");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join(libdir)).unwrap();
        fs::copy(&core, root.join(libdir).join(abi::SONAME)).unwrap();
        let built = crate::test_fixtures::launcher(&dir, "gnoblin", &core_dir, rpath);
        let installed = root.join("bin/gnoblin");
        fs::copy(built, &installed).unwrap();
        (root, installed)
    }

    #[test]
    fn the_core_resolves_through_the_run_path_and_a_miss_names_what_was_searched() {
        let (root, launcher) = tree("resolve-ok", "$ORIGIN/../lib", "lib");
        let elf = elf::read(&launcher).unwrap();
        let (found, problems) = check_resolution(&launcher, &elf, &[]);
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, abi::SONAME);
        assert_eq!(
            found[0].1,
            fs::canonicalize(root.join("lib").join(abi::SONAME)).unwrap()
        );

        // A run path that names the wrong directory: a private prefix whose libdir is not `lib`.
        let (root, launcher) = tree("resolve-miss", "$ORIGIN/../lib", "lib/x86_64-linux-gnu");
        let elf = elf::read(&launcher).unwrap();
        let (found, problems) = check_resolution(&launcher, &elf, &[]);
        assert!(found.is_empty());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains(abi::SONAME) && problems[0].contains("GNOBLIN_RPATH") && problems[0].contains("lib"),
            "{problems:?}"
        );
        // The same tree under a distribution prefix is found through the loader's default directories.
        let default_dir = root.join("lib/x86_64-linux-gnu");
        let (found, problems) = check_resolution(&launcher, &elf, &[default_dir]);
        assert_eq!((found.len(), problems.len()), (1, 0));
    }

    #[test]
    fn a_library_found_under_the_core_name_must_carry_its_soname() {
        let (root, launcher) = tree("resolve-soname", "$ORIGIN/../lib", "lib");
        cc(
            &root,
            "wrong.c",
            "void other(void) {}\n",
            &[
                "-shared",
                "-fPIC",
                "-Wl,-soname,libother.so.1",
                "-o",
                "lib/libgnoblin-core.so.1",
            ],
        );
        let elf = elf::read(&launcher).unwrap();
        let (_, problems) = check_resolution(&launcher, &elf, &[]);
        assert!(
            problems.iter().any(|problem| problem.contains("whose SONAME is")),
            "{problems:?}"
        );
    }

    #[test]
    fn origin_is_the_directory_the_file_really_sits_in() {
        let (root, launcher) = tree("origin", "$ORIGIN/../lib:${ORIGIN}/x:", "lib");
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&launcher, &alias).unwrap();
        let elf = elf::read(&alias).unwrap();
        let dirs = search_dirs(&alias, &elf, &[PathBuf::from("/system")]);
        let bin = fs::canonicalize(root.join("bin")).unwrap();
        assert_eq!(
            dirs,
            [bin.join("../lib"), bin.join("x"), PathBuf::from("/system")],
            "the loader expands $ORIGIN from the resolved path; an empty entry is dropped"
        );
    }

    #[test]
    fn a_launcher_that_exports_core_names_is_caught() {
        let dir = scratch("launcher-exports");
        cc(
            &dir,
            "exports.c",
            "int main(void) { return 0; }\nvoid gnoblin_get_api(void) {}\n",
            &["-rdynamic", "-o", "exports"],
        );
        let problems = check_launcher(&elf::read(&dir.join("exports")).unwrap());
        assert!(
            problems.iter().any(|problem| problem.contains("gnoblin_get_api")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_core_only_shell_may_not_link_another_gnoblin_library() {
        let dir = scratch("shell-other-lib");
        good_core(&dir);
        cc(
            &dir,
            "libgnoblin-extras.c",
            "void extras(void) {}\n",
            &[
                "-shared",
                "-fPIC",
                "-Wl,-soname,libgnoblin-extras.so.1",
                "-o",
                "libgnoblin-extras.so",
            ],
        );
        let with = shell(
            &dir,
            "with.so",
            &format!(
                "{ENTRY}extern void extras(void);\n__attribute__((visibility(\"hidden\"))) void *use(void) {{ return (void *)extras; }}\n"
            ),
            &["-lgnoblin-extras"],
        );
        let problems = check_core_only_shell(&elf::read(&with).unwrap());
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("libgnoblin-extras.so.1")),
            "{problems:?}"
        );
    }

    #[test]
    fn the_launcher_is_run_with_the_loader_environment_cleared() {
        let dir = scratch("runs");
        cc(
            &dir,
            "ok.c",
            "#include <stdio.h>\nint main(void) { puts(\"gnoblin\"); return 0; }\n",
            &["-o", "ok"],
        );
        assert_eq!(check_runs(&dir.join("ok")), Vec::<String>::new());
        cc(&dir, "bad.c", "int main(void) { return 3; }\n", &["-o", "bad"]);
        let problems = check_runs(&dir.join("bad"));
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("--version") && problems[1].contains("--role compositor --help"));
        assert_eq!(check_runs(&dir.join("missing")).len(), 2);
    }

    #[test]
    fn the_layout_of_section_11_is_listed_when_missing() {
        let empty = scratch("layout-empty");
        let problems = check_layout(&empty, "lib64");
        for expected in [
            "bin/gnoblin is missing",
            "include/gnoblin/core.h is missing",
            "lib64/gnoblin/shells/ is missing",
            "share/gnoblin/shells/ is missing",
            "share/gnoblin/js/gnoblin-core/index.ts is missing",
            "share/gnoblin/js/gnoblin-std/index.ts is missing",
            "share/gnoblin/js/gnoblin-core/generated/abi.ts is missing",
            "share/gnoblin/js/gnoblin-std/generated/classes.ts is missing",
            "share/gnoblin/types/python/gnoblin/std.pyi is missing",
            "share/gnoblin/types/gnoblin.d.ts is missing",
            "share/gnoblin/types/tsconfig.json is missing",
            "share/gnoblin/types/gnoblin-core/index.ts is missing",
            "share/gnoblin/types/gnoblin-std/index.ts is missing",
        ] {
            assert!(
                problems.iter().any(|problem| problem.starts_with(expected)),
                "{expected}: {problems:?}"
            );
        }
    }

    #[test]
    fn shells_in_the_module_directory_are_found_below_it() {
        let dir = scratch("installed-shells");
        fs::create_dir_all(dir.join("tiler")).unwrap();
        fs::write(dir.join("tiler/libtiler.so"), b"x").unwrap();
        fs::write(dir.join("libplain.so"), b"x").unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(
            installed_shells(&dir),
            [dir.join("libplain.so"), dir.join("tiler/libtiler.so")]
        );
        assert!(installed_shells(&dir.join("missing")).is_empty());
    }

    #[test]
    fn a_staged_tree_is_checked_through_its_origin_run_path() {
        let dir = scratch("staged");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let debug = dir.join("target/debug");
        fs::create_dir_all(&debug).unwrap();
        fs::copy(&core, debug.join("libgnoblin_core_impl.so")).unwrap();
        std::os::unix::fs::symlink("libgnoblin_core_impl.so", debug.join(abi::SONAME)).unwrap();
        let built = crate::test_fixtures::launcher(&dir, "gnoblin", &core_dir, "$ORIGIN");
        fs::copy(built, debug.join("gnoblin")).unwrap();
        let options = CheckOptions {
            staged: true,
            profile: "debug".to_owned(),
            target_dir: dir.join("target"),
            libdir: "lib".to_owned(),
            ..CheckOptions::default()
        };
        assert_eq!(run_checks(&options).unwrap(), 0);

        // Without the staged link the launcher has nothing to resolve to, and the check says so.
        fs::remove_file(debug.join(abi::SONAME)).unwrap();
        assert!(run_checks(&options).unwrap() >= 2);
    }

    #[test]
    fn the_staged_and_run_flags_parse() {
        let words = |text: &[&str]| text.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
        let options = parse_args(&words(&["--staged", "--profile", "release", "--run"])).unwrap();
        assert!(options.staged && options.run);
        assert_eq!(options.profile, "release");
        assert!(parse_args(&words(&["--run"])).is_err(), "--run needs a launcher to run");
        assert!(parse_args(&words(&["--run", "--launcher", "bin/gnoblin"])).is_ok());
    }
}
