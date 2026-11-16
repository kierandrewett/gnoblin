//! `cargo xtask dist-check`: the static and link-level checks of an installed (or staged) Gnoblin tree.
//!
//! The modular framework makes dynamic linking part of the product, so "it builds" is not enough: the installed core must
//! carry the right SONAME and export exactly the ABI, the launcher must record the core once, a shell must link the core and
//! never carry a copy of it, and the development files must let a C program link and run. `design/modular-framework.md`
//! section 11 lists the checks; this is that list as a command, over files read with `elf.rs` (cross-checked against
//! `readelf` and `nm` by its tests). The run-time half (one core mapping in a live process) is a test of the sessions, not a
//! static check.
//!
//! ```text
//! cargo xtask dist-check --prefix /usr --destdir /tmp/stage [--libdir lib64] [--shell FILE]... [--std-shell FILE]...
//! cargo xtask dist-check --core target/debug/libgnoblin_core_impl.so --shell target/debug/libmyshell.so
//! ```
//!
//! With `--prefix` the command finds the core, the launcher, the std library, the SONAME links, the header and the
//! pkg-config file where `dist install` puts them. Individual files can also be named; each flag checks that file only.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use gnoblin_core_abi as abi;

use crate::elf::{self, Elf};

/// The SONAME of the std library, which a std shell and nothing else of the core's tier links.
pub const STD_SONAME: &str = "libgnoblin-std.so.1";

const USAGE: &str = "\
usage: cargo xtask dist-check [--prefix P [--destdir D] [--libdir NAME]] [--core FILE] [--launcher FILE] [--std FILE]
                              [--shell FILE]... [--std-shell FILE]... [--max-glibc X.Y]";

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

/// The launcher: the core recorded exactly once, and no std.
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

/// The std library: its own SONAME, and the core it builds on, never the reverse.
pub fn check_std_library(elf: &Elf) -> Vec<String> {
    let mut problems = Vec::new();
    if elf.soname.as_deref() != Some(STD_SONAME) {
        problems.push(format!("SONAME is {:?}, expected `{STD_SONAME}`", elf.soname));
    }
    if !elf.needed.iter().any(|needed| is_core(needed)) {
        problems.push(format!("does not need `{}` (NEEDED: {:?})", abi::SONAME, elf.needed));
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
    let mut problems = Vec::new();
    let version = format!("{}.{}.{}", abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH);
    if pc.field("Name").as_deref() != Some("gnoblin-core") {
        problems.push(format!("Name is {:?}, expected `gnoblin-core`", pc.field("Name")));
    }
    if pc.field("Version").as_deref() != Some(version.as_str()) {
        problems.push(format!("Version is {:?}, expected `{version}`", pc.field("Version")));
    }
    match pc.field("Libs") {
        Some(libs) if libs.contains("-lgnoblin-core") && libs.contains(&format!("-L{}", pc.expand("${libdir}"))) => {}
        other => problems.push(format!("Libs is {other:?}, expected `-L${{libdir}} -lgnoblin-core`")),
    }
    match pc.field("Cflags") {
        Some(cflags) if cflags.contains(&pc.expand("-I${includedir}")) => {}
        other => problems.push(format!("Cflags is {other:?}, expected `-I${{includedir}}`")),
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
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let pc_dir = root.join(libdir_name).join("pkgconfig");
    let flags = Command::new("pkg-config")
        .arg(format!("--define-variable=prefix={}", root.display()))
        .args(["--cflags", "--libs", "gnoblin-core"])
        .env("PKG_CONFIG_PATH", &pc_dir)
        .env("PKG_CONFIG_LIBDIR", &pc_dir)
        .output();
    let flags = match flags {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Ok(output) => {
            problems.push(format!(
                "`pkg-config --cflags --libs gnoblin-core` failed: {}",
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
    let text = "#include <gnoblin/core.h>\n#include <stdio.h>\n\
                int main(void) {\n  uint32_t major = 0, minor = 0, patch = 0;\n  gnoblin_abi_version(&major, &minor, &patch);\n  \
                printf(\"%u.%u.%u\\n\", major, minor, patch);\n  return 0;\n}\n";
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
                "a C program using `pkg-config --cflags --libs gnoblin-core` did not compile and link: {}",
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
            let expected = format!("{}.{}.{}", abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH);
            if printed != expected {
                problems.push(format!(
                    "the probe program printed ABI {printed}, the installed core should be {expected}"
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
}

fn parse_args(args: &[String]) -> Result<CheckOptions, String> {
    let mut options = CheckOptions {
        libdir: "lib".to_owned(),
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
        && options.core.is_none()
        && options.launcher.is_none()
        && options.std.is_none()
        && options.shells.is_empty()
        && options.std_shells.is_empty()
    {
        return Err("nothing to check: give `--prefix`, or at least one file".to_owned());
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

    let core_path = options
        .core
        .clone()
        .or_else(|| libdir.as_ref().map(|libdir| libdir.join(abi::SONAME)));
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

    let launcher_path = options.launcher.clone().or_else(|| {
        root.as_ref()
            .map(|root| root.join("bin/gnoblin"))
            .filter(|path| core_path.is_some() && path.exists())
    });
    if let Some(path) = &launcher_path {
        if let Some(launcher) = read_elf(path, &mut outcome, &mut files) {
            outcome.report(&format!("launcher {}", path.display()), &check_launcher(&launcher));
        }
        match ldd_missing(path) {
            Ok(missing) => outcome.report(&format!("ldd {}", path.display()), &missing),
            Err(error) => outcome.report(&format!("ldd {}", path.display()), &[error]),
        }
    }

    let std_path = options.std.clone().or_else(|| {
        libdir
            .as_ref()
            .map(|libdir| libdir.join(STD_SONAME))
            .filter(|path| path.exists())
    });
    if let Some(path) = &std_path
        && let Some(std) = read_elf(path, &mut outcome, &mut files)
    {
        outcome.report(&format!("std {}", path.display()), &check_std_library(&std));
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

    if let (Some(root), true) = (&root, core_path.is_some()) {
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
    use crate::test_fixtures::{ENTRY, cc, exports_source, good_core, scratch, shell};

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
        // A stand-in std library: its own SONAME, linked against the core.
        cc(
            &dir,
            "std.c",
            "extern int gnoblin_get_api(unsigned, unsigned, const void **);\nvoid gnoblin_std_get_api(void) { gnoblin_get_api(1, 0, 0); }\n",
            &[
                "-shared",
                "-fPIC",
                "-L",
                ".",
                "-lgnoblin-core",
                "-Wl,-soname,libgnoblin-std.so.1",
                "-o",
                "libgnoblin-std.so",
            ],
        );
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
}
