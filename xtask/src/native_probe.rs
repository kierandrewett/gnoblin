//! `cargo xtask native-probe`: the native libraries the workspace needs, checked against an image's root file system.
//!
//! `native-libs` proves the CI install list covers every `-sys` crate in the lock files. This is the next question the
//! design asks (design/rust-compositor.md, "Distribution and toolchain"): on each target image, which version of each
//! library does a build or a run get, and does it meet the floor the crates declare? The list of libraries and their
//! floors is data, `tests/native-probe/libraries.toml`, so the table in the design and the check read one source.
//!
//! The probe reads a root file system from outside, so it needs no compiler and no libc inside the image: an Alpine,
//! a Debian 11 and a Fedora 45 root are all just directories. It looks at three things and keeps them apart:
//!
//! - the `Version:` of each pkg-config file (`usr/lib*/pkgconfig`, `usr/share/pkgconfig`), which is what a build links;
//! - the shared objects and files present, which is what a run needs;
//! - the version of each package the table names, read from the image's own package database.
//!
//! It never guesses. A library with no pkg-config file, no shared object and no package is `missing`; a library that is
//! present and has no floor is `recorded`; a floor is only compared where `libraries.toml` states one, with the source
//! of the statement beside it.
//!
//! Subcommands: `list`, `packages FAMILY`, `family --root DIR`, `image --image ID`, `check --root DIR --image ID` (or `--versions FILE` for
//! Nix), `unprobed --image ID --reason TEXT` and `matrix [--check]`. `scripts/native-probe.sh` is the driver that makes the root file system.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::{Deserialize, Serialize};

use crate::native_libs;

const TABLE: &str = "tests/native-probe/libraries.toml";
const RESULTS: &str = "tests/native-probe/results";
const MATRIX: &str = "tests/native-probe/MATRIX.md";
const TARGETS: &str = "design/packaging/targets.json";

/// Images the design names that `targets.json` does not list: the apk packages are built in `alpine:edge`,
/// which is the only Alpine target.
const EXTRA_IMAGES: &[(&str, &str)] = &[("alpine-edge", "alpine:edge")];

/// The package families, in the order the table lists them.
const FAMILIES: &[&str] = &["apt", "dnf", "zypper", "pacman", "apk", "nix"];

#[derive(Debug, Deserialize)]
struct ProbeTable {
    library: Vec<Library>,
    #[serde(rename = "not-linked", default)]
    not_linked: Vec<NotLinked>,
}

#[derive(Debug, Deserialize)]
struct Library {
    id: String,
    kind: String,
    crates: Vec<String>,
    evidence: String,
    #[serde(rename = "pkg-config", default)]
    pkg_config: Vec<String>,
    #[serde(default)]
    sonames: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    floor: Option<String>,
    #[serde(rename = "floor-from")]
    floor_from: Option<String>,
    #[serde(rename = "floor-note")]
    floor_note: Option<String>,
    #[serde(rename = "mutter-floor")]
    mutter_floor: Option<String>,
    #[serde(rename = "mutter-floor-from")]
    mutter_floor_from: Option<String>,
    packages: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct NotLinked {
    id: String,
    evidence: String,
}

/// One probe of one image: what the file system holds for each library.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ProbeReport {
    image: ImageInfo,
    /// Who made the report and how: date, box, command lines and exit codes, as the driver passed them.
    #[serde(default)]
    run: BTreeMap<String, String>,
    #[serde(default)]
    library: Vec<FoundLibrary>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ImageInfo {
    /// The target id in `design/packaging/targets.json`.
    id: String,
    /// The image name the root file system came from.
    image: String,
    /// False when no root file system could be made; `reason` then says why and `library` is empty.
    probed: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    os_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    os_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    family: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct FoundLibrary {
    id: String,
    /// `ok`, `below-floor`, `recorded` or `missing`.
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    /// Where `version` was read: `pkg-config NAME` or `package NAME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    floor: Option<String>,
    /// `ok`, `below-floor` or `recorded` against the Mutter floor, when the table has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mutter: Option<String>,
    /// Shared objects and files found, relative to the root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    found: Vec<String>,
    /// Every package the table names for this family, with its installed version or `absent`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    packages: Vec<String>,
}

pub fn run(args: &[String]) -> ExitCode {
    match dispatch(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("native-probe: {error}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &[String]) -> Result<ExitCode, String> {
    let root = Path::new(".");
    let table = load_table(root)?;
    match args.first().map(String::as_str) {
        Some("list") => {
            print!("{}", render_table(&table));
            Ok(ExitCode::SUCCESS)
        }
        Some("packages") => {
            let family = args.get(1).ok_or("packages needs a family")?;
            for package in packages_of(&table, family)? {
                println!("{package}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => check_command(&table, &args[1..]),
        Some("family") => family_command(&args[1..]),
        Some("image") => {
            let id = option(args, "--image").ok_or("image needs --image ID")?;
            let (_, image) = targets(root)?
                .into_iter()
                .find(|(known, _)| known == id)
                .ok_or_else(|| format!("{id} is not a target"))?;
            println!("{image}");
            Ok(ExitCode::SUCCESS)
        }
        Some("unprobed") => unprobed_command(&args[1..]),
        Some("matrix") => matrix_command(&table, root, &args[1..]),
        _ => Err(
            "usage: native-probe list | packages FAMILY | family --root DIR | image --image ID \
                  | check (--root DIR | --versions FILE) --image ID [--image-name NAME] [--meta K=V].. [--deny] \
                  | unprobed --image ID --image-name NAME --reason TEXT | matrix [--check]"
                .to_owned(),
        ),
    }
}

fn load_table(root: &Path) -> Result<ProbeTable, String> {
    let text = fs::read_to_string(root.join(TABLE)).map_err(|e| format!("{TABLE}: {e}"))?;
    let table: ProbeTable = toml::from_str(&text).map_err(|e| format!("{TABLE}: {e}"))?;
    let problems = table_problems(&table);
    if problems.is_empty() {
        Ok(table)
    } else {
        Err(problems.join("\n"))
    }
}

/// What is wrong with the table itself: a floor with no source, a kind nobody defined, a family it forgot, and a
/// `-sys` crate `native-libs` classifies that no entry accounts for.
fn table_problems(table: &ProbeTable) -> Vec<String> {
    let mut problems = Vec::new();
    let mut ids = BTreeSet::new();
    for library in &table.library {
        if !ids.insert(library.id.as_str()) {
            problems.push(format!("{}: listed twice", library.id));
        }
        if !["link", "dlopen", "build", "run"].contains(&library.kind.as_str()) {
            problems.push(format!(
                "{}: kind `{}` is not one of link, dlopen, build, run",
                library.id, library.kind
            ));
        }
        if library.crates.is_empty() || library.evidence.trim().is_empty() {
            problems.push(format!("{}: names no crate or no evidence", library.id));
        }
        if library.floor.is_some() != library.floor_from.is_some() {
            problems.push(format!(
                "{}: a floor and the source that states it go together",
                library.id
            ));
        }
        if library.mutter_floor.is_some() != library.mutter_floor_from.is_some() {
            problems.push(format!("{}: a Mutter floor and its source go together", library.id));
        }
        for family in FAMILIES {
            if library.packages.get(*family).is_none_or(Vec::is_empty) {
                problems.push(format!("{}: no packages for {family}", library.id));
            }
        }
        if library.pkg_config.is_empty() && library.sonames.is_empty() && library.files.is_empty() {
            problems.push(format!("{}: nothing to look for", library.id));
        }
    }
    // Every `-sys` crate `native-libs` says links a system library is accounted for by an entry here.
    let accounted: BTreeSet<&str> = table
        .library
        .iter()
        .flat_map(|library| library.crates.iter().map(String::as_str))
        .collect();
    for sys in native_libs::native_crates() {
        if !accounted.contains(sys) {
            problems.push(format!(
                "{sys} is in native_libs::NATIVE but no entry of {TABLE} lists it in `crates`"
            ));
        }
    }
    problems
}

fn packages_of(table: &ProbeTable, family: &str) -> Result<Vec<String>, String> {
    if !FAMILIES.contains(&family) {
        return Err(format!("unknown family `{family}` (one of {})", FAMILIES.join(", ")));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for library in &table.library {
        for package in library.packages.get(family).into_iter().flatten() {
            if seen.insert(package.clone()) {
                out.push(package.clone());
            }
        }
    }
    Ok(out)
}

fn render_table(table: &ProbeTable) -> String {
    let mut out = String::new();
    out.push_str("library | kind | floor | mutter floor | crates\n");
    for library in &table.library {
        out.push_str(&format!(
            "{} | {} | {} | {} | {}\n",
            library.id,
            library.kind,
            library
                .floor
                .as_deref()
                .or(library.floor_note.as_deref().map(|_| "capability"))
                .unwrap_or("none declared"),
            library.mutter_floor.as_deref().unwrap_or("-"),
            library.crates.join(", ")
        ));
    }
    for entry in &table.not_linked {
        out.push_str(&format!("not linked: {} ({})\n", entry.id, entry.evidence));
    }
    out
}

// ---- reading a root file system -------------------------------------------------------------------------------

/// `NAME=value` pairs of an os-release file, with the quotes taken off.
fn parse_os_release(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().trim_matches('"').to_owned()))
        .collect()
}

/// The package family an `ID` and `ID_LIKE` belong to.
fn family_of(os: &BTreeMap<String, String>) -> Option<&'static str> {
    let words: Vec<&str> = os
        .get("ID")
        .into_iter()
        .chain(os.get("ID_LIKE"))
        .flat_map(|value| value.split_whitespace())
        .collect();
    let has = |names: &[&str]| words.iter().any(|word| names.contains(word));
    if has(&["debian", "ubuntu"]) {
        Some("apt")
    } else if has(&["fedora", "rhel", "centos", "rocky"]) {
        Some("dnf")
    } else if has(&["opensuse", "opensuse-leap", "opensuse-tumbleweed", "suse", "sles"]) {
        Some("zypper")
    } else if has(&["arch"]) {
        Some("pacman")
    } else if has(&["alpine"]) {
        Some("apk")
    } else {
        None
    }
}

/// The pkg-config directories of a root file system: the fixed ones, and every multiarch one under `usr/lib`.
fn pkgconfig_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [
        "usr/lib/pkgconfig",
        "usr/lib64/pkgconfig",
        "usr/share/pkgconfig",
        "lib/pkgconfig",
    ]
    .iter()
    .map(|dir| root.join(dir))
    .collect();
    for parent in ["usr/lib", "usr/lib64", "lib", "lib64"] {
        if let Ok(entries) = fs::read_dir(root.join(parent)) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("pkgconfig");
                if candidate.is_dir() {
                    dirs.push(candidate);
                }
            }
        }
    }
    dirs
}

/// The `Version:` of the pkg-config file for `module`, with `${name}` variables substituted.
fn pc_version(root: &Path, module: &str) -> Option<String> {
    for dir in pkgconfig_dirs(root) {
        let Ok(text) = fs::read_to_string(dir.join(format!("{module}.pc"))) else {
            continue;
        };
        return parse_pc_version(&text);
    }
    None
}

fn parse_pc_version(text: &str) -> Option<String> {
    let mut variables = BTreeMap::new();
    let mut version = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("Version:") {
            version = Some(value.trim().to_owned());
        } else if let Some((name, value)) = line.split_once('=')
            && !name.contains(char::is_whitespace)
            && !name.contains(':')
        {
            variables.insert(name.to_owned(), value.trim().to_owned());
        }
    }
    let mut version = version?;
    for _ in 0..4 {
        let before = version.clone();
        for (name, value) in &variables {
            version = version.replace(&format!("${{{name}}}"), value);
        }
        if version == before {
            break;
        }
    }
    (!version.is_empty() && !version.contains("${")).then_some(version)
}

/// The library directories a shared object can live in.
fn library_dirs(root: &Path) -> Vec<PathBuf> {
    // `lib` is a symlink to `usr/lib` on a merged-usr root, and listing both would report every file twice.
    let mut dirs: Vec<PathBuf> = ["usr/lib", "usr/lib64", "lib", "lib64"]
        .iter()
        .map(|dir| root.join(dir))
        .filter(|dir| !dir.is_symlink())
        .collect();
    for parent in ["usr/lib", "lib"] {
        if root.join(parent).is_symlink() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(root.join(parent)) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if entry.path().is_dir() && name.to_string_lossy().contains("-linux-") {
                    dirs.push(entry.path());
                }
            }
        }
    }
    dirs
}

/// Files under the library directories named `soname`, or starting with it when it ends in `-` or `.so`
/// (a family of names such as `libclang-18.so`). Paths come back relative to `root`.
fn find_soname(root: &Path, soname: &str) -> Vec<String> {
    let mut found = Vec::new();
    for dir in library_dirs(root) {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let hit = name == soname
                || name.starts_with(&format!("{soname}."))
                || (soname.ends_with('-') && name.starts_with(soname));
            if hit && let Ok(relative) = entry.path().strip_prefix(root) {
                found.push(relative.display().to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

fn find_file(root: &Path, relative: &str) -> Option<String> {
    let path = root.join(relative);
    (path.exists() || path.is_symlink()).then(|| relative.to_owned())
}

/// The upstream part of a package version: the epoch and the packaging revision taken off.
fn upstream(version: &str) -> String {
    let without_epoch = version.split_once(':').map_or(version, |(_, rest)| rest);
    let main = without_epoch.split(['-', '+', '~']).next().unwrap_or(without_epoch);
    main.to_owned()
}

/// Installed packages of a root file system, name to version, for the names asked about.
fn installed_versions(root: &Path, family: &str, names: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    match family {
        "apt" => {
            if let Ok(text) = fs::read_to_string(root.join("var/lib/dpkg/status")) {
                for (name, version) in parse_dpkg_status(&text) {
                    if names.contains(&name) {
                        out.insert(name, version);
                    }
                }
            }
        }
        "apk" => {
            if let Ok(text) = fs::read_to_string(root.join("lib/apk/db/installed")) {
                for (name, version) in parse_apk_db(&text) {
                    if names.contains(&name) {
                        out.insert(name, version);
                    }
                }
            }
        }
        "pacman" => {
            if let Ok(entries) = fs::read_dir(root.join("var/lib/pacman/local")) {
                for entry in entries.flatten() {
                    if let Some((name, version)) = parse_pacman_dir(&entry.file_name().to_string_lossy())
                        && names.contains(&name)
                    {
                        out.insert(name, version);
                    }
                }
            }
        }
        _ => {
            for name in names {
                if let Some(version) = rpm_version(root, name) {
                    out.insert(name.clone(), version);
                }
            }
        }
    }
    out
}

/// Packages `Status: install ok installed` of a dpkg status file, as `(name, version)`.
fn parse_dpkg_status(text: &str) -> Vec<(String, String)> {
    text.split("\n\n")
        .filter_map(|stanza| {
            let field = |name: &str| {
                stanza
                    .lines()
                    .find_map(|line| line.strip_prefix(name).and_then(|rest| rest.strip_prefix(':')))
                    .map(|value| value.trim().to_owned())
            };
            let status = field("Status")?;
            status.ends_with("installed").then_some(())?;
            Some((field("Package")?, field("Version")?))
        })
        .collect()
}

/// `P:` and `V:` lines of an apk installed database.
fn parse_apk_db(text: &str) -> Vec<(String, String)> {
    text.split("\n\n")
        .filter_map(|stanza| {
            let field = |prefix: &str| {
                stanza
                    .lines()
                    .find_map(|line| line.strip_prefix(prefix))
                    .map(str::to_owned)
            };
            Some((field("P:")?, field("V:")?))
        })
        .collect()
}

/// `name-version-release` of a pacman local database directory.
fn parse_pacman_dir(directory: &str) -> Option<(String, String)> {
    let mut parts = directory.rsplitn(3, '-');
    let release = parts.next()?;
    let version = parts.next()?;
    let name = parts.next()?;
    Some((name.to_owned(), format!("{version}-{release}")))
}

/// `rpm -q` against the root, with `--root` and, when that fails to start, `chroot`.
fn rpm_version(root: &Path, package: &str) -> Option<String> {
    let query = ["-q", "--qf", "%{VERSION}", package];
    let attempts = [
        Command::new("rpm").arg("--root").arg(root).args(query).output(),
        Command::new("chroot").arg(root).arg("rpm").args(query).output(),
    ];
    attempts
        .into_iter()
        .flatten()
        .find(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|version| !version.is_empty())
}

/// The numeric components of a version: `1.26.2` is `[1, 26, 2]`, `0.3.65rc1` is `[0, 3, 65, 1]`.
fn components(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Whether `version` is at least `floor`, comparing numeric components with a missing one as zero.
fn at_least(version: &str, floor: &str) -> bool {
    let (have, need) = (components(version), components(floor));
    for index in 0..have.len().max(need.len()) {
        let (a, b) = (
            have.get(index).copied().unwrap_or(0),
            need.get(index).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    true
}

/// The verdict of a version against a floor, or [`None`] when either is missing: nothing is judged from a guess.
fn judge(version: Option<&str>, floor: Option<&str>) -> Option<&'static str> {
    let (version, floor) = (version?, floor?);
    Some(if at_least(version, floor) { "ok" } else { "below-floor" })
}

/// Everything the root file system says about one library.
fn probe_library(root: &Path, family: &str, library: &Library, installed: &BTreeMap<String, String>) -> FoundLibrary {
    let mut found = Vec::new();
    for soname in &library.sonames {
        found.extend(find_soname(root, soname));
    }
    for file in &library.files {
        found.extend(find_file(root, file));
    }
    found.sort();
    found.dedup();

    let mut version = None;
    let mut via = None;
    for module in &library.pkg_config {
        if let Some(value) = pc_version(root, module) {
            version = Some(value);
            via = Some(format!("pkg-config {module}"));
            break;
        }
    }
    let wanted = library.packages.get(family).cloned().unwrap_or_default();
    let packages: Vec<String> = wanted
        .iter()
        .map(|name| match installed.get(name) {
            Some(value) => format!("{name} {value}"),
            None => format!("{name} absent"),
        })
        .collect();
    if version.is_none() {
        // A library with no pkg-config file (a run-time one, or an image without the development package) takes the
        // version of the first package that is installed: the one that carries the shared object.
        if let Some((name, value)) = wanted
            .iter()
            .find_map(|name| installed.get(name).map(|value| (name, value)))
        {
            version = Some(upstream(value));
            via = Some(format!("package {name}"));
        }
    }
    let present = version.is_some() || !found.is_empty();
    let floor = library.floor.clone();
    // A library that is there but whose version could not be read is `unjudged` when a floor wants one, and
    // `recorded` when none does.
    let verdict = |floor: Option<&str>| -> String {
        if !present {
            "missing".to_owned()
        } else {
            match (judge(version.as_deref(), floor), floor) {
                (Some(verdict), _) => verdict.to_owned(),
                (None, Some(_)) => "unjudged".to_owned(),
                (None, None) => "recorded".to_owned(),
            }
        }
    };
    let status = verdict(floor.as_deref());
    let mutter = library
        .mutter_floor
        .as_deref()
        .map(|mutter_floor| verdict(Some(mutter_floor)));
    FoundLibrary {
        id: library.id.clone(),
        status,
        mutter,
        version,
        via,
        floor,
        found,
        packages,
    }
}

// ---- the subcommands -------------------------------------------------------------------------------------------

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .map(String::as_str)
}

/// The `(id, image)` pairs of every target: `targets.json` first, then [`EXTRA_IMAGES`].
fn targets(root: &Path) -> Result<Vec<(String, String)>, String> {
    let text = fs::read_to_string(root.join(TARGETS)).map_err(|e| format!("{TARGETS}: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{TARGETS}: {e}"))?;
    let list = value
        .get("targets")
        .and_then(serde_json::Value::as_array)
        .ok_or("targets is not a list")?;
    let mut out = Vec::new();
    for target in list {
        let id = target
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or("a target has no id")?;
        let image = target
            .get("image")
            .and_then(serde_json::Value::as_str)
            .ok_or("a target has no image")?;
        out.push((id.to_owned(), image.to_owned()));
    }
    out.extend(
        EXTRA_IMAGES
            .iter()
            .map(|(id, image)| ((*id).to_owned(), (*image).to_owned())),
    );
    Ok(out)
}

/// `--meta K=V` pairs, as the run record a report carries.
fn meta(args: &[String]) -> BTreeMap<String, String> {
    let mut run = BTreeMap::new();
    for (index, arg) in args.iter().enumerate() {
        if arg == "--meta"
            && let Some((key, value)) = args.get(index + 1).and_then(|pair| pair.split_once('='))
        {
            run.insert(key.to_owned(), value.to_owned());
        }
    }
    run
}

/// The os-release of a root, from `etc` or `usr/lib`.
fn read_os_release(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = fs::read_to_string(root.join("etc/os-release"))
        .or_else(|_| fs::read_to_string(root.join("usr/lib/os-release")))
        .map_err(|e| format!("{}: no os-release: {e}", root.display()))?;
    Ok(parse_os_release(&text))
}

/// Lines of `attribute version`, which is how the driver reports what `nix eval` said.
fn parse_versions(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .map(|(attribute, version)| (attribute.to_owned(), version.trim().to_owned()))
        .filter(|(_, version)| !version.is_empty())
        .collect()
}

fn family_command(args: &[String]) -> Result<ExitCode, String> {
    let root = PathBuf::from(option(args, "--root").ok_or("family needs --root DIR")?);
    let os = read_os_release(&root)?;
    match family_of(&os) {
        Some(family) => {
            println!("{family}");
            Ok(ExitCode::SUCCESS)
        }
        None => Err(format!(
            "{}: the os-release names no package family I know",
            root.display()
        )),
    }
}

/// Probes one image: a root file system with `--root DIR`, or, for Nix, the versions the driver evaluated with
/// `--versions FILE --os-id ID --os-version V`.
fn check_command(table: &ProbeTable, args: &[String]) -> Result<ExitCode, String> {
    let id = option(args, "--image").ok_or("check needs --image ID")?;
    let image = match option(args, "--image-name") {
        Some(name) => name.to_owned(),
        None => targets(Path::new("."))?
            .into_iter()
            .find(|(known, _)| known == id)
            .map(|(_, image)| image)
            .ok_or_else(|| format!("{id} is not a target; pass --image-name"))?,
    };
    let (root, os, family, installed) = if let Some(file) = option(args, "--versions") {
        let text = fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
        let os = BTreeMap::from([
            ("ID".to_owned(), option(args, "--os-id").unwrap_or("nixos").to_owned()),
            (
                "VERSION_ID".to_owned(),
                option(args, "--os-version").unwrap_or_default().to_owned(),
            ),
        ]);
        // No file system to read: every library is judged by the version the driver evaluated for its attribute.
        (
            PathBuf::from("/nonexistent/native-probe"),
            os,
            "nix".to_owned(),
            parse_versions(&text),
        )
    } else {
        let root = PathBuf::from(option(args, "--root").ok_or("check needs --root DIR or --versions FILE")?);
        let os = read_os_release(&root)?;
        let family = option(args, "--family")
            .or_else(|| family_of(&os))
            .ok_or_else(|| format!("{id}: the os-release names no package family I know; pass --family"))?
            .to_owned();
        if !FAMILIES.contains(&family.as_str()) {
            return Err(format!("unknown family `{family}`"));
        }
        let names: BTreeSet<String> = packages_of(table, &family)?.into_iter().collect();
        let installed = installed_versions(&root, &family, &names);
        (root, os, family, installed)
    };
    let report = ProbeReport {
        image: ImageInfo {
            id: id.to_owned(),
            image,
            probed: true,
            reason: String::new(),
            os_id: os.get("ID").cloned().unwrap_or_default(),
            os_version: os.get("VERSION_ID").cloned().unwrap_or_default(),
            family: family.clone(),
        },
        run: meta(args),
        library: table
            .library
            .iter()
            .map(|library| probe_library(&root, &family, library, &installed))
            .collect(),
    };
    print!("{}", toml::to_string_pretty(&report).map_err(|e| e.to_string())?);
    let below = report.library.iter().any(|found| found.status == "below-floor");
    if below && args.iter().any(|arg| arg == "--deny") {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// The report of an image that could not be probed, with the reason as it was observed.
fn unprobed_command(args: &[String]) -> Result<ExitCode, String> {
    let id = option(args, "--image").ok_or("unprobed needs --image ID")?;
    let name = option(args, "--image-name").ok_or("unprobed needs --image-name NAME")?;
    let reason = option(args, "--reason").ok_or("unprobed needs --reason TEXT")?;
    let run = meta(args);
    let report = ProbeReport {
        image: ImageInfo {
            id: id.to_owned(),
            image: name.to_owned(),
            probed: false,
            reason: reason.to_owned(),
            os_id: String::new(),
            os_version: String::new(),
            family: String::new(),
        },
        run,
        library: Vec::new(),
    };
    print!("{}", toml::to_string_pretty(&report).map_err(|e| e.to_string())?);
    Ok(ExitCode::SUCCESS)
}

fn load_report(path: &Path) -> Result<ProbeReport, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The stored reports, by target id.
fn load_reports(root: &Path) -> Result<BTreeMap<String, ProbeReport>, String> {
    let mut out = BTreeMap::new();
    let Ok(entries) = fs::read_dir(root.join(RESULTS)) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "toml") {
            let report = load_report(&path)?;
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            if report.image.id != stem {
                return Err(format!(
                    "{}: names image `{}`, which is not its file name",
                    path.display(),
                    report.image.id
                ));
            }
            out.insert(stem, report);
        }
    }
    Ok(out)
}

fn matrix_command(table: &ProbeTable, root: &Path, args: &[String]) -> Result<ExitCode, String> {
    let reports = load_reports(root)?;
    let targets = targets(root)?;
    let known: BTreeSet<&str> = targets.iter().map(|(id, _)| id.as_str()).collect();
    for id in reports.keys() {
        if !known.contains(id.as_str()) {
            return Err(format!("{RESULTS}/{id}.toml: {id} is not a target"));
        }
    }
    let library_ids: BTreeSet<&str> = table.library.iter().map(|library| library.id.as_str()).collect();
    for report in reports.values() {
        for found in &report.library {
            if !library_ids.contains(found.id.as_str()) {
                return Err(format!(
                    "{}: names library `{}`, which the table does not",
                    report.image.id, found.id
                ));
            }
        }
    }
    let text = render_matrix(table, &targets, &reports);
    if args.iter().any(|arg| arg == "--check") {
        let stored = fs::read_to_string(root.join(MATRIX)).map_err(|e| format!("{MATRIX}: {e}"))?;
        if stored == text {
            println!("native-probe: {MATRIX} matches {} stored report(s)", reports.len());
            return Ok(ExitCode::SUCCESS);
        }
        eprintln!("native-probe: {MATRIX} is stale; run `cargo xtask native-probe matrix > {MATRIX}`");
        return Ok(ExitCode::FAILURE);
    }
    print!("{text}");
    Ok(ExitCode::SUCCESS)
}

/// One cell: the version, marked `!` below a floor and `?` when a floor wants a version that could not be read.
fn cell(report: Option<&ProbeReport>, id: &str) -> String {
    let Some(report) = report else {
        return "not probed".to_owned();
    };
    if !report.image.probed {
        return "not probed".to_owned();
    }
    let Some(found) = report.library.iter().find(|found| found.id == id) else {
        return "-".to_owned();
    };
    let version = found.version.as_deref().map(upstream).unwrap_or_default();
    match found.status.as_str() {
        "missing" => "-".to_owned(),
        "below-floor" => format!("{version}!"),
        "unjudged" => "?".to_owned(),
        _ if version.is_empty() => "present".to_owned(),
        _ => version,
    }
}

/// The matrix, one table per family of targets so each stays readable, and a list of what could not be probed.
fn render_matrix(table: &ProbeTable, targets: &[(String, String)], reports: &BTreeMap<String, ProbeReport>) -> String {
    let groups: &[(&str, &[&str])] = &[
        ("Fedora and Rocky", &["fedora-", "el-"]),
        ("Debian and Ubuntu", &["debian-", "ubuntu-"]),
        ("openSUSE", &["opensuse-"]),
        ("Arch, Alpine and NixOS", &["arch-", "alpine-", "nixos-"]),
    ];
    let mut out = String::new();
    out.push_str("<!-- Generated by `cargo xtask native-probe matrix`. Do not edit by hand. -->\n\n");
    out.push_str("# Native dependency probe\n\n");
    out.push_str(
        "Version of each native library on each target image, from the stored reports in `results/`. `!` marks a \
         version below the floor the crates declare, `?` a library present whose version could not be read, `-` a \
         library the image lacks after the install, `not probed` an image with no report or no root file system.\n\n",
    );
    for (title, prefixes) in groups {
        let members: Vec<&(String, String)> = targets
            .iter()
            .filter(|(id, _)| prefixes.iter().any(|prefix| id.starts_with(prefix)))
            .collect();
        if members.is_empty() {
            continue;
        }
        out.push_str(&format!("## {title}\n\n| library | floor |"));
        for (id, _) in &members {
            out.push_str(&format!(" {id} |"));
        }
        out.push_str("\n| --- | --- |");
        for _ in &members {
            out.push_str(" --- |");
        }
        out.push('\n');
        for library in &table.library {
            let floor = library.floor.as_deref().unwrap_or("-");
            out.push_str(&format!("| {} | {} |", library.id, floor));
            for (id, _) in &members {
                out.push_str(&format!(" {} |", cell(reports.get(id), &library.id)));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    let skipped: Vec<(&String, &str)> = targets
        .iter()
        .filter_map(|(id, _)| {
            reports
                .get(id)
                .filter(|report| !report.image.probed)
                .map(|report| (id, report.image.reason.as_str()))
        })
        .collect();
    out.push_str("## Images that could not be probed\n\n");
    if skipped.is_empty() {
        out.push_str("None.\n");
    }
    for (id, reason) in skipped {
        out.push_str(&format!("- `{id}`: {reason}\n"));
    }
    let unreported: Vec<&str> = targets
        .iter()
        .filter(|(id, _)| !reports.contains_key(id))
        .map(|(id, _)| id.as_str())
        .collect();
    if !unreported.is_empty() {
        out.push_str(&format!("\nNo report yet: {}.\n", unreported.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    #[test]
    fn the_table_parses_and_accounts_for_every_native_crate() {
        let table = load_table(&root()).expect("the table is well formed");
        assert!(table.library.iter().any(|library| library.id == "libinput"));
        assert!(!table.not_linked.is_empty());
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        assert!(at_least("1.26.2", "1.19"));
        assert!(at_least("1.19", "1.19"));
        assert!(at_least("1.19.0", "1.19"));
        assert!(!at_least("1.9", "1.19"), "1.9 is older than 1.19");
        assert!(at_least("0.3.65", "0.3.34"));
        assert!(!at_least("0.3.33", "0.3.34"));
        assert!(at_least("1.4.11", "1.4.9"), "11 is more than 9");
    }

    #[test]
    fn a_package_version_loses_its_epoch_and_revision() {
        assert_eq!(upstream("1:1.26.2-1ubuntu1"), "1.26.2");
        assert_eq!(upstream("1.4.11-r0"), "1.4.11");
        assert_eq!(upstream("1.19.0+dfsg-2~bpo11+1"), "1.19.0");
        assert_eq!(upstream("25.0.7"), "25.0.7");
    }

    #[test]
    fn a_pkg_config_version_may_name_a_variable() {
        assert_eq!(
            parse_pc_version("Name: x\nVersion: 1.21.0\n").as_deref(),
            Some("1.21.0")
        );
        assert_eq!(
            parse_pc_version("version=2.4\nVersion: ${version}.1\n").as_deref(),
            Some("2.4.1")
        );
        assert_eq!(
            parse_pc_version("Version: ${missing}\n"),
            None,
            "an unresolved variable is not a version"
        );
        assert_eq!(parse_pc_version("Name: x\n"), None);
    }

    #[test]
    fn dpkg_status_keeps_only_installed_packages() {
        let text = "Package: libinput-dev\nStatus: install ok installed\nVersion: 1.22.1-1\n\n\
                    Package: gone\nStatus: deinstall ok config-files\nVersion: 1.0\n\n";
        assert_eq!(
            parse_dpkg_status(text),
            vec![("libinput-dev".to_owned(), "1.22.1-1".to_owned())]
        );
    }

    #[test]
    fn apk_and_pacman_databases_are_read() {
        assert_eq!(
            parse_apk_db("P:libinput-dev\nV:1.28.1-r0\n\nP:other\nV:2-r1\n"),
            vec![
                ("libinput-dev".to_owned(), "1.28.1-r0".to_owned()),
                ("other".to_owned(), "2-r1".to_owned())
            ]
        );
        assert_eq!(
            parse_pacman_dir("libinput-1.28.1-1"),
            Some(("libinput".to_owned(), "1.28.1-1".to_owned()))
        );
        assert_eq!(
            parse_pacman_dir("vulkan-icd-loader-1.4.309-1"),
            Some(("vulkan-icd-loader".to_owned(), "1.4.309-1".to_owned()))
        );
    }

    #[test]
    fn os_release_names_the_family() {
        let ubuntu = parse_os_release("ID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"22.04\"\n");
        assert_eq!(family_of(&ubuntu), Some("apt"));
        assert_eq!(ubuntu.get("VERSION_ID").map(String::as_str), Some("22.04"));
        assert_eq!(
            family_of(&parse_os_release("ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"\n")),
            Some("zypper")
        );
        assert_eq!(
            family_of(&parse_os_release("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n")),
            Some("dnf")
        );
        assert_eq!(family_of(&parse_os_release("ID=alpine\n")), Some("apk"));
        assert_eq!(family_of(&parse_os_release("ID=arch\n")), Some("pacman"));
        assert_eq!(
            family_of(&parse_os_release("ID=nixos\n")),
            None,
            "a Nix root has no package family to read"
        );
    }

    /// A root file system built in a temporary directory, probed end to end.
    #[test]
    fn a_probe_reads_versions_and_shared_objects_from_a_root() {
        let directory = std::env::temp_dir().join(format!("native-probe-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let pc = directory.join("usr/lib/x86_64-linux-gnu/pkgconfig");
        fs::create_dir_all(&pc).expect("a pkgconfig directory");
        fs::write(pc.join("libinput.pc"), "Name: libinput\nVersion: 1.17.0\n").expect("a pc file");
        fs::write(pc.join("libpipewire-0.3.pc"), "Name: pw\nVersion: 1.4.2\n").expect("a pc file");
        let lib = directory.join("usr/lib/x86_64-linux-gnu");
        fs::write(lib.join("libpam.so.0"), "").expect("a shared object");
        fs::create_dir_all(directory.join("var/lib/dpkg")).expect("a dpkg directory");
        fs::write(
            directory.join("var/lib/dpkg/status"),
            "Package: libpam0g\nStatus: install ok installed\nVersion: 1.5.2-6\n\n",
        )
        .expect("a status file");
        let table = load_table(&root()).expect("the table");
        let names: BTreeSet<String> = packages_of(&table, "apt").expect("apt packages").into_iter().collect();
        let installed = installed_versions(&directory, "apt", &names);
        let find = |id: &str| {
            let library = table
                .library
                .iter()
                .find(|library| library.id == id)
                .expect("a library of the table");
            probe_library(&directory, "apt", library, &installed)
        };
        let libinput = find("libinput");
        assert_eq!(
            libinput.status, "below-floor",
            "1.17 is under the 1.19 the crates enable"
        );
        assert_eq!(libinput.mutter.as_deref(), Some("below-floor"));
        assert_eq!(libinput.version.as_deref(), Some("1.17.0"));
        assert_eq!(find("pipewire").status, "ok");
        assert_eq!(
            find("pipewire").mutter.as_deref(),
            Some("below-floor"),
            "1.4.2 is under Mutter's 1.4.11"
        );
        let pam = find("libpam");
        assert_eq!(
            pam.status, "recorded",
            "no floor, so the version is recorded and not judged"
        );
        assert_eq!(pam.version.as_deref(), Some("1.5.2"));
        assert_eq!(pam.via.as_deref(), Some("package libpam0g"));
        assert_eq!(find("libdrm").status, "missing", "nothing of libdrm is in this root");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_stored_reports_match_the_matrix() {
        let root = root();
        let table = load_table(&root).expect("the table");
        let reports = load_reports(&root).expect("the stored reports parse");
        let targets = targets(&root).expect("the targets");
        let stored = fs::read_to_string(root.join(MATRIX)).expect("the matrix is stored");
        assert_eq!(
            stored,
            render_matrix(&table, &targets, &reports),
            "run `cargo xtask native-probe matrix > {MATRIX}`"
        );
    }
}
