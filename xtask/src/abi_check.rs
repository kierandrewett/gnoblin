//! `cargo xtask abi-check` and `cargo xtask abi-header`: the ABI contract as a checked file.
//!
//! `crates/gnoblin-core-abi/abi.lock` records every exported symbol, constant, handle type, callback type, struct layout and
//! field offset of the C ABI, each with the ABI version that introduced it. `abi-check` regenerates the same manifest from
//! `gnoblin-core-abi`'s own declarations (`reflect`) and compares:
//!
//! - An entry the lock holds that is gone or different in the code is a **removal or change**. If the lock's `released`
//!   line covers the entry's version it is **breaking** and the check fails with no way to bless it short of a new major;
//!   before the release it is only a stale lock.
//! - An entry the code has that the lock lacks is **additive**: the lock is stale and `--update` records it, with the
//!   current ABI version as its `since`.
//! - A struct that starts with `size: u32` may grow (a field appended and the size larger); any other change to it, or a
//!   struct without the size word growing at all, is a change.
//!
//! The lock is plain text, one entry per line, `kind | key | detail | since`, so a reviewer reads the ABI diff in the pull
//! request. Type text is canonical (see `abi_c`), so the compiler's pretty-printing never shows up as a diff.
//!
//! The same command also verifies the golden shells of example 12 (`tests/modular/golden`, see `golden`): the stored binaries
//! built for ABI 1.0 match their recorded hash and still have the dynamic facts of a shell.
//!
//! `abi-header` renders `include/gnoblin/core.h` from the same declarations. Decision D18 keeps the ABI unstable until
//! 0.2.0 ships: the lock's `released none` means everything may still change, and the release commit sets it to `1.0`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use gnoblin_core_abi::reflect::Reflection;

use crate::abi_c::{CORE_HEADER, HeaderStyle, STD_HEADER, canonical, parse, render_header_with};

/// The core's lock file, relative to the workspace root.
pub const LOCK_PATH: &str = "crates/gnoblin-core-abi/abi.lock";

/// The std library's lock file, relative to the workspace root.
pub const STD_LOCK_PATH: &str = "crates/gnoblin-std-abi/abi.lock";

/// One library's C ABI: where its declarations come from and where its lock and header are.
pub struct AbiTarget {
    /// The name the command line and the messages use (`core`, `std`).
    pub name: &'static str,
    /// The lock file, relative to the workspace root.
    pub lock_path: &'static str,
    /// The crate that declares the ABI.
    pub crate_name: &'static str,
    /// The library the ABI belongs to.
    pub library: &'static str,
    /// The ABI of this build of the declaring crate.
    pub reflection: fn() -> Reflection,
    /// The C header generated from it.
    pub header: &'static HeaderStyle,
}

/// The ABI of `libgnoblin-core`.
pub const CORE: AbiTarget = AbiTarget {
    name: "core",
    lock_path: LOCK_PATH,
    crate_name: "gnoblin-core-abi",
    library: "libgnoblin-core",
    reflection: gnoblin_core_abi::reflect::reflection,
    header: &CORE_HEADER,
};

/// The ABI of `libgnoblin-std`.
pub const STD: AbiTarget = AbiTarget {
    name: "std",
    lock_path: STD_LOCK_PATH,
    crate_name: "gnoblin-std-abi",
    library: "libgnoblin-std",
    reflection: gnoblin_std_abi::reflection,
    header: &STD_HEADER,
};

/// Every ABI the workspace ships, core first.
pub const TARGETS: [&AbiTarget; 2] = [&CORE, &STD];

/// The test-only knobs of a target's ABI crate and whether this build has each on. A build with any of them on describes a
/// hypothetical ABI, so the lock is never compared with it.
fn test_features(target: &AbiTarget) -> [(&'static str, bool); 2] {
    if target.name == "core" {
        [
            ("next-minor", gnoblin_core_abi::NEXT_MINOR_FEATURE),
            ("next-major", gnoblin_core_abi::NEXT_MAJOR_FEATURE),
        ]
    } else {
        [
            ("next-minor", gnoblin_std_abi::NEXT_MINOR_FEATURE),
            ("next-major", gnoblin_std_abi::NEXT_MAJOR_FEATURE),
        ]
    }
}

const USAGE: &str = "\
usage: cargo xtask abi-check [--update]
       cargo xtask abi-header [--std] [--out FILE]";

/// An ABI version, `(major, minor)`.
pub type Version = (u32, u32);

fn version_text(version: Version) -> String {
    format!("{}.{}", version.0, version.1)
}

fn parse_version(text: &str) -> Result<Version, String> {
    let (major, minor) = text
        .split_once('.')
        .ok_or_else(|| format!("`{text}` is not a version like 1.0"))?;
    Ok((
        major
            .parse()
            .map_err(|_| format!("`{text}` is not a version like 1.0"))?,
        minor
            .parse()
            .map_err(|_| format!("`{text}` is not a version like 1.0"))?,
    ))
}

/// One line of the manifest: what the code declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub kind: &'static str,
    pub key: String,
    pub detail: String,
}

/// The manifest of an ABI crate build, in the order the lock lists it.
pub fn manifest(reflection: &Reflection) -> Result<Vec<ManifestEntry>, String> {
    let mut entries = vec![ManifestEntry {
        kind: "soname",
        key: "soname".to_owned(),
        detail: reflection.soname.to_owned(),
    }];
    for export in reflection.exports {
        let ty = parse(export.ty).map_err(|error| format!("export {}: {error}", export.symbol))?;
        entries.push(ManifestEntry {
            kind: "export",
            key: export.symbol.to_owned(),
            detail: canonical(&ty),
        });
    }
    for constant in reflection.consts {
        entries.push(ManifestEntry {
            kind: "const",
            key: constant.name.to_owned(),
            detail: format!("{} = {}", constant.ty, constant.value),
        });
    }
    for alias in reflection.aliases {
        let ty = parse(alias.ty).map_err(|error| format!("alias {}: {error}", alias.name))?;
        entries.push(ManifestEntry {
            kind: "alias",
            key: alias.name.to_owned(),
            detail: canonical(&ty),
        });
    }
    for callback in reflection.callbacks {
        let ty = parse(callback.ty).map_err(|error| format!("callback {}: {error}", callback.name))?;
        entries.push(ManifestEntry {
            kind: "callback",
            key: callback.name.to_owned(),
            detail: canonical(&ty),
        });
    }
    for layout in reflection.structs {
        entries.push(ManifestEntry {
            kind: "struct",
            key: layout.name.to_owned(),
            detail: format!(
                "size {} align {} {}",
                layout.size,
                layout.align,
                if layout.is_extensible() { "extensible" } else { "fixed" }
            ),
        });
        for field in layout.fields {
            let ty = parse(field.ty).map_err(|error| format!("field {}.{}: {error}", layout.name, field.name))?;
            entries.push(ManifestEntry {
                kind: "field",
                key: format!("{}.{}", layout.name, field.name),
                detail: format!("{} @{} +{}", canonical(&ty), field.offset, field.size),
            });
        }
    }
    Ok(entries)
}

/// One line of the lock: an entry and the version that introduced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockEntry {
    pub kind: String,
    pub key: String,
    pub detail: String,
    pub since: Version,
}

/// The parsed `abi.lock`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbiLock {
    /// The ABI version the lock was last written for.
    pub abi: Version,
    /// Entries introduced at or before this version are frozen; `None` before the first release.
    pub released: Option<Version>,
    pub entries: Vec<LockEntry>,
}

/// The comment at the top of a lock file.
fn lock_header(target: &AbiTarget) -> String {
    format!(
        "\
# The released C ABI of {library}: every export, constant, handle and callback type, and struct layout, each with the
# ABI version that introduced it. Generated by `cargo xtask abi-check --update`; reviewed like source.
#
# `cargo xtask abi-check` compares this file with crates/{crate_name}. An entry here that is missing or different in
# the code is a removal or change; if `released` covers its `since` it is a breaking change and fails with no way to bless
# it inside this major. A new entry in the code is additive and is recorded with `--update`. A struct that starts with
# `size: u32` is `extensible` and may grow; any other struct is `fixed`.
#
# `released none` means the ABI is still unstable (design/modular-framework.md decision D18): the release commit sets it
# to the version that ships, and from then on its entries are frozen.
",
        library = target.library,
        crate_name = target.crate_name
    )
}

/// Renders the core's lock text.
#[cfg(test)]
pub fn render_lock(lock: &AbiLock) -> String {
    render_lock_for(lock, &CORE)
}

/// Renders the lock text of `target`.
pub fn render_lock_for(lock: &AbiLock, target: &AbiTarget) -> String {
    let mut text = lock_header(target);
    let _ = writeln!(text, "abi {}", version_text(lock.abi));
    match lock.released {
        Some(version) => {
            let _ = writeln!(text, "released {}", version_text(version));
        }
        None => text.push_str("released none\n"),
    }
    let mut previous_kind = "";
    for entry in &lock.entries {
        if entry.kind != previous_kind {
            text.push('\n');
            previous_kind = &entry.kind;
        }
        let _ = writeln!(
            text,
            "{} | {} | {} | {}",
            entry.kind,
            entry.key,
            entry.detail,
            version_text(entry.since)
        );
    }
    text
}

/// Parses the core's lock text.
#[cfg(test)]
pub fn parse_lock(text: &str) -> Result<AbiLock, String> {
    parse_lock_at(text, LOCK_PATH)
}

/// Parses lock text read from `path`, which the error messages name.
pub fn parse_lock_at(text: &str, path: &str) -> Result<AbiLock, String> {
    let mut abi = None;
    let mut released = None;
    let mut released_seen = false;
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |message: String| format!("{path}:{}: {message}", index + 1);
        if let Some(version) = line.strip_prefix("abi ") {
            abi = Some(parse_version(version.trim()).map_err(at)?);
        } else if let Some(version) = line.strip_prefix("released ") {
            released_seen = true;
            released = match version.trim() {
                "none" => None,
                other => Some(parse_version(other).map_err(at)?),
            };
        } else {
            let parts: Vec<&str> = line.split(" | ").collect();
            let [kind, key, detail, since] = parts.as_slice() else {
                return Err(at("expected `kind | key | detail | since`".to_owned()));
            };
            entries.push(LockEntry {
                kind: (*kind).to_owned(),
                key: (*key).to_owned(),
                detail: (*detail).to_owned(),
                since: parse_version(since).map_err(at)?,
            });
        }
    }
    let abi = abi.ok_or_else(|| format!("{path}: no `abi` line"))?;
    if !released_seen {
        return Err(format!("{path}: no `released` line"));
    }
    Ok(AbiLock { abi, released, entries })
}

/// What the comparison found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AbiReport {
    /// Changes to released entries: not fixable by updating the lock.
    pub breaking: Vec<String>,
    /// Differences the lock can absorb with `--update`.
    pub stale: Vec<String>,
}

impl AbiReport {
    pub fn is_clean(&self) -> bool {
        self.breaking.is_empty() && self.stale.is_empty()
    }
}

/// `size N align A extensible|fixed` as its parts.
fn struct_parts(detail: &str) -> Option<(usize, usize, &str)> {
    let mut words = detail.split_whitespace();
    let (Some("size"), Some(size), Some("align"), Some(align), Some(kind)) =
        (words.next(), words.next(), words.next(), words.next(), words.next())
    else {
        return None;
    };
    Some((size.parse().ok()?, align.parse().ok()?, kind))
}

/// Files a difference about a lock entry: breaking when the release froze it, otherwise a stale lock.
fn record(report: &mut AbiReport, frozen: bool, since: Version, message: String) {
    if frozen {
        report
            .breaking
            .push(format!("{message} (released in ABI {})", version_text(since)));
    } else {
        report.stale.push(message);
    }
}

/// Compares the lock with the code.
pub fn diff(lock: &AbiLock, current: &[ManifestEntry]) -> AbiReport {
    let frozen = |since: Version| lock.released.is_some_and(|released| since <= released);
    let by_key: BTreeMap<(&str, &str), &ManifestEntry> = current
        .iter()
        .map(|entry| ((entry.kind, entry.key.as_str()), entry))
        .collect();
    let mut report = AbiReport::default();
    for entry in &lock.entries {
        let name = format!("{} {}", entry.kind, entry.key);
        match by_key.get(&(entry.kind.as_str(), entry.key.as_str())) {
            None => record(
                &mut report,
                frozen(entry.since),
                entry.since,
                format!("{name} was removed"),
            ),
            Some(now) if now.detail == entry.detail => {}
            Some(now) => {
                let growth = entry.kind == "struct"
                    && matches!(
                        (struct_parts(&entry.detail), struct_parts(&now.detail)),
                        (Some((old_size, old_align, "extensible")), Some((new_size, new_align, "extensible")))
                            if new_size >= old_size && new_align == old_align
                    );
                if growth {
                    report
                        .stale
                        .push(format!("{name} grew: `{}` is now `{}`", entry.detail, now.detail));
                } else {
                    let message = format!("{name} changed: was `{}`, is now `{}`", entry.detail, now.detail);
                    record(&mut report, frozen(entry.since), entry.since, message);
                }
            }
        }
    }
    let known: BTreeSet<(&str, &str)> = lock
        .entries
        .iter()
        .map(|entry| (entry.kind.as_str(), entry.key.as_str()))
        .collect();
    for entry in current {
        if !known.contains(&(entry.kind, entry.key.as_str())) {
            report
                .stale
                .push(format!("{} {} is new: `{}`", entry.kind, entry.key, entry.detail));
        }
    }
    report
}

/// The lock for `current`: existing entries keep their `since`, new ones get `version`.
pub fn updated(lock: &AbiLock, current: &[ManifestEntry], version: Version) -> AbiLock {
    let since: BTreeMap<(&str, &str), Version> = lock
        .entries
        .iter()
        .map(|entry| ((entry.kind.as_str(), entry.key.as_str()), entry.since))
        .collect();
    AbiLock {
        abi: version,
        released: lock.released,
        entries: current
            .iter()
            .map(|entry| LockEntry {
                kind: entry.kind.to_owned(),
                key: entry.key.clone(),
                detail: entry.detail.clone(),
                since: since.get(&(entry.kind, entry.key.as_str())).copied().unwrap_or(version),
            })
            .collect(),
    }
}

/// The first lock, for an ABI that has none yet.
pub fn initial(current: &[ManifestEntry], version: Version) -> AbiLock {
    updated(
        &AbiLock {
            abi: version,
            released: None,
            entries: Vec::new(),
        },
        current,
        version,
    )
}

/// Checks one library's lock against its declarations, or writes it with `update`.
fn check_target(root: &Path, update: bool, target: &AbiTarget) -> Result<bool, String> {
    let reflection = (target.reflection)();
    for (feature, on) in test_features(target) {
        if on {
            return Err(format!(
                "{} was built with its test-only `{feature}` feature; the lock describes the real ABI",
                target.crate_name
            ));
        }
    }
    let lock_path = target.lock_path;
    let current = manifest(&reflection)?;
    let version = (reflection.major, reflection.minor);
    let path = root.join(lock_path);
    let lock = match fs::read_to_string(&path) {
        Ok(text) => Some(parse_lock_at(&text, lock_path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let Some(lock) = lock else {
        if update {
            let first = initial(&current, version);
            fs::write(&path, render_lock_for(&first, target))
                .map_err(|error| format!("{}: {error}", path.display()))?;
            println!("[abi-check] wrote {} ({} entries)", lock_path, first.entries.len());
            return Ok(true);
        }
        eprintln!("[abi-check] {lock_path} does not exist; run `cargo xtask abi-check --update` to create it");
        return Ok(false);
    };
    let report = diff(&lock, &current);
    if report.is_clean() && lock.abi == version {
        println!(
            "[abi-check] {} entries match {lock_path} (ABI {})",
            current.len(),
            version_text(version)
        );
        return Ok(true);
    }
    for message in &report.breaking {
        println!("[abi-check] BREAKING: {message}");
    }
    if !report.breaking.is_empty() {
        eprintln!("[abi-check] a released ABI entry changed; that needs a new ABI major, not a lock update");
        return Ok(false);
    }
    if update {
        let next = updated(&lock, &current, version);
        fs::write(&path, render_lock_for(&next, target)).map_err(|error| format!("{}: {error}", path.display()))?;
        println!(
            "[abi-check] updated {lock_path}: {} difference(s) recorded",
            report.stale.len()
        );
        return Ok(true);
    }
    for message in &report.stale {
        println!("[abi-check] {message}");
    }
    if lock.abi != version {
        println!(
            "[abi-check] the lock says ABI {} and the code is ABI {}",
            version_text(lock.abi),
            version_text(version)
        );
    }
    eprintln!("[abi-check] {lock_path} is stale; run `cargo xtask abi-check --update` and commit it");
    Ok(false)
}

/// Checks the golden shells of example 12 against `golden.lock` and the ABI this build declares (see `golden`). They are
/// shipped binaries, so a core or std built with a test knob is not a build they are compared against.
fn check_golden(root: &Path) -> Result<bool, String> {
    let version = |target: &AbiTarget| {
        let reflection = (target.reflection)();
        (reflection.major, reflection.minor)
    };
    let (golden, problems) = crate::golden::problems(root, version(&CORE), version(&STD))?;
    if problems.is_empty() {
        println!(
            "[abi-check] {} golden shell(s) match {}/golden.lock (built for core ABI {}, std ABI {})",
            golden.shells.len(),
            crate::golden::DIR,
            version_text(golden.core_abi),
            version_text(golden.std_abi)
        );
        return Ok(true);
    }
    for problem in &problems {
        eprintln!("[abi-check] golden: {problem}");
    }
    Ok(false)
}

/// Checks every library's lock and the golden shells; every failure is reported, and the answer says whether there was none.
pub(crate) fn check(root: &Path, update: bool) -> Result<bool, String> {
    let mut all_match = true;
    for target in TARGETS {
        all_match &= check_target(root, update, target)?;
    }
    all_match &= check_golden(root)?;
    Ok(all_match)
}

/// ManifestEntry point for `cargo xtask abi-check`.
pub fn run_check(args: &[String]) -> ExitCode {
    let update = match args {
        [] => false,
        [flag] if flag == "--update" => true,
        _ => {
            eprintln!("[abi-check] unexpected arguments\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match check(Path::new("."), update) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("[abi-check] {message}");
            ExitCode::from(2)
        }
    }
}

/// ManifestEntry point for `cargo xtask abi-header`.
pub fn run_header(args: &[String]) -> ExitCode {
    let (target, rest) = match args {
        [flag, rest @ ..] if flag == "--std" => (&STD, rest),
        rest => (&CORE, rest),
    };
    let out = match rest {
        [] => None,
        [flag, path] if flag == "--out" => Some(path.clone()),
        _ => {
            eprintln!("[abi-header] unexpected arguments\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let header = match render_header_with(&(target.reflection)(), target.header) {
        Ok(header) => header,
        Err(message) => {
            eprintln!("[abi-header] {message}");
            return ExitCode::from(2);
        }
    };
    match out {
        Some(path) => {
            if let Some(parent) = Path::new(&path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                && let Err(error) = fs::create_dir_all(parent)
            {
                eprintln!("[abi-header] {}: {error}", parent.display());
                return ExitCode::from(2);
            }
            if let Err(error) = fs::write(&path, header) {
                eprintln!("[abi-header] {path}: {error}");
                return ExitCode::from(2);
            }
            println!("[abi-header] wrote {path}");
        }
        None => print!("{header}"),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &'static str, key: &str, detail: &str) -> ManifestEntry {
        ManifestEntry {
            kind,
            key: key.to_owned(),
            detail: detail.to_owned(),
        }
    }

    fn locked(entries: &[(&str, &str, &str, Version)], released: Option<Version>) -> AbiLock {
        AbiLock {
            abi: (1, 0),
            released,
            entries: entries
                .iter()
                .map(|(kind, key, detail, since)| LockEntry {
                    kind: (*kind).to_owned(),
                    key: (*key).to_owned(),
                    detail: (*detail).to_owned(),
                    since: *since,
                })
                .collect(),
        }
    }

    #[test]
    fn the_lock_round_trips_through_its_text() {
        let current = manifest(&(CORE.reflection)()).unwrap();
        let lock = initial(&current, (1, 0));
        let text = render_lock(&lock);
        assert_eq!(parse_lock(&text).unwrap(), lock);
    }

    #[test]
    fn a_lock_without_its_header_lines_is_refused() {
        assert!(parse_lock("export | a | b | 1.0\n").is_err());
        assert!(parse_lock("abi 1.0\nexport | a | b | 1.0\n").is_err());
        assert!(parse_lock("abi 1.0\nreleased none\nexport | a | b\n").is_err());
        assert!(parse_lock("abi one\nreleased none\n").is_err());
    }

    #[test]
    fn an_unchanged_abi_is_clean() {
        let current = vec![
            entry("export", "gnoblin_get_api", "fn()"),
            entry("const", "E_OK", "i32 = 0"),
        ];
        let lock = locked(
            &[
                ("export", "gnoblin_get_api", "fn()", (1, 0)),
                ("const", "E_OK", "i32 = 0", (1, 0)),
            ],
            Some((1, 0)),
        );
        assert!(diff(&lock, &current).is_clean());
    }

    #[test]
    fn an_added_entry_makes_the_lock_stale_not_breaking() {
        let current = vec![entry("const", "E_OK", "i32 = 0"), entry("const", "E_NEW", "i32 = -20")];
        let lock = locked(&[("const", "E_OK", "i32 = 0", (1, 0))], Some((1, 0)));
        let report = diff(&lock, &current);
        assert!(report.breaking.is_empty());
        assert_eq!(report.stale.len(), 1);
        assert!(report.stale[0].contains("E_NEW is new"));
    }

    #[test]
    fn removing_or_retyping_a_released_entry_is_breaking() {
        let lock = locked(
            &[
                ("field", "GnoblinApi.log", "unsafe extern \"C\" fn() @16 +8", (1, 0)),
                ("const", "E_OK", "i32 = 0", (1, 0)),
            ],
            Some((1, 0)),
        );
        let retyped = vec![
            entry("field", "GnoblinApi.log", "unsafe extern \"C\" fn(x: u32) @16 +8"),
            entry("const", "E_OK", "i32 = 0"),
        ];
        let report = diff(&lock, &retyped);
        assert_eq!(report.breaking.len(), 1, "{report:?}");
        assert!(report.breaking[0].contains("changed"));
        let removed = vec![entry("field", "GnoblinApi.log", "unsafe extern \"C\" fn() @16 +8")];
        let report = diff(&lock, &removed);
        assert_eq!(report.breaking.len(), 1);
        assert!(report.breaking[0].contains("E_OK was removed"));
        // Moving a field is a change of its offset.
        let moved = vec![
            entry("field", "GnoblinApi.log", "unsafe extern \"C\" fn() @24 +8"),
            entry("const", "E_OK", "i32 = 0"),
        ];
        assert_eq!(diff(&lock, &moved).breaking.len(), 1);
    }

    #[test]
    fn before_the_release_a_change_is_only_a_stale_lock() {
        let lock = locked(&[("const", "E_OK", "i32 = 0", (1, 0))], None);
        let report = diff(&lock, &[entry("const", "E_OK", "i32 = 1")]);
        assert!(report.breaking.is_empty());
        assert_eq!(report.stale.len(), 1);
    }

    #[test]
    fn entries_added_after_the_release_are_not_frozen_by_it() {
        // `E_LATE` came in 1.1; the freeze covers 1.0 only.
        let lock = locked(
            &[
                ("const", "E_OK", "i32 = 0", (1, 0)),
                ("const", "E_LATE", "i32 = -15", (1, 1)),
            ],
            Some((1, 0)),
        );
        let report = diff(&lock, &[entry("const", "E_OK", "i32 = 0")]);
        assert!(report.breaking.is_empty(), "{report:?}");
        assert_eq!(report.stale.len(), 1);
    }

    #[test]
    fn an_extensible_struct_may_grow_and_a_fixed_one_may_not() {
        let released = Some((1, 0));
        let lock = locked(
            &[
                ("struct", "GnoblinApi", "size 80 align 8 extensible", (1, 0)),
                ("struct", "GnoblinStr", "size 16 align 8 fixed", (1, 0)),
            ],
            released,
        );
        let grown = vec![
            entry("struct", "GnoblinApi", "size 88 align 8 extensible"),
            entry("struct", "GnoblinStr", "size 16 align 8 fixed"),
        ];
        let report = diff(&lock, &grown);
        assert!(report.breaking.is_empty(), "{report:?}");
        assert_eq!(report.stale.len(), 1);
        assert!(report.stale[0].contains("grew"));

        let shrunk = vec![
            entry("struct", "GnoblinApi", "size 72 align 8 extensible"),
            entry("struct", "GnoblinStr", "size 16 align 8 fixed"),
        ];
        assert_eq!(diff(&lock, &shrunk).breaking.len(), 1);
        let fixed_grown = vec![
            entry("struct", "GnoblinApi", "size 80 align 8 extensible"),
            entry("struct", "GnoblinStr", "size 24 align 8 fixed"),
        ];
        assert_eq!(diff(&lock, &fixed_grown).breaking.len(), 1);
        let lost_size_word = vec![
            entry("struct", "GnoblinApi", "size 88 align 8 fixed"),
            entry("struct", "GnoblinStr", "size 16 align 8 fixed"),
        ];
        assert_eq!(
            diff(&lock, &lost_size_word).breaking.len(),
            1,
            "extensible -> fixed is a change"
        );
    }

    #[test]
    fn update_keeps_old_since_versions_and_stamps_new_entries() {
        let lock = locked(&[("const", "E_OK", "i32 = 0", (1, 0))], Some((1, 0)));
        let current = vec![entry("const", "E_OK", "i32 = 0"), entry("const", "E_NEW", "i32 = -20")];
        let next = updated(&lock, &current, (1, 1));
        assert_eq!(next.abi, (1, 1));
        assert_eq!(next.released, Some((1, 0)));
        assert_eq!(next.entries[0].since, (1, 0));
        assert_eq!(next.entries[1].since, (1, 1));
        assert!(diff(&next, &current).is_clean());
    }

    #[test]
    fn the_real_abi_manifests_every_kind_and_every_field() {
        let reflection = (CORE.reflection)();
        let entries = manifest(&reflection).unwrap();
        for kind in ["soname", "export", "const", "alias", "callback", "struct", "field"] {
            assert!(entries.iter().any(|entry| entry.kind == kind), "no `{kind}` entry");
        }
        let fields: usize = reflection.structs.iter().map(|layout| layout.fields.len()).sum();
        assert_eq!(entries.iter().filter(|entry| entry.kind == "field").count(), fields);
        let api = entries
            .iter()
            .find(|entry| entry.kind == "struct" && entry.key == "GnoblinApi")
            .unwrap();
        assert!(api.detail.ends_with("extensible"), "{}", api.detail);
    }

    #[test]
    fn the_checked_in_locks_match_the_abi_crates() {
        // The same check `cargo xtask abi-check` runs, as a test, so `cargo test -p xtask` catches a stale lock too.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for target in TARGETS {
            let text = fs::read_to_string(root.join(target.lock_path)).unwrap_or_else(|error| {
                panic!("{}: {error}; run `cargo xtask abi-check --update`", target.lock_path);
            });
            let lock = parse_lock_at(&text, target.lock_path).unwrap();
            let current = manifest(&(target.reflection)()).unwrap();
            let report = diff(&lock, &current);
            assert!(report.is_clean(), "{}: {report:?}", target.name);
        }
    }

    #[test]
    fn the_std_abi_manifests_its_table_and_its_own_soname() {
        let entries = manifest(&(STD.reflection)()).unwrap();
        let soname = entries.iter().find(|entry| entry.kind == "soname").unwrap();
        assert_eq!(soname.detail, "libgnoblin-std.so.1");
        let table = entries
            .iter()
            .find(|entry| entry.kind == "struct" && entry.key == "GnoblinStdApi")
            .unwrap();
        assert!(table.detail.ends_with("extensible"), "{}", table.detail);
        let exports: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.kind == "export")
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(
            exports,
            [
                "gnoblin_std_abi_version",
                "gnoblin_std_get_api",
                "gnoblin_std_last_error",
                "gnoblin_std_init"
            ]
        );
    }

    #[test]
    fn the_two_headers_do_not_define_the_same_macro() {
        let core = render_header_with(&(CORE.reflection)(), CORE.header).unwrap();
        let std = render_header_with(&(STD.reflection)(), STD.header).unwrap();
        let defined = |text: &str| -> std::collections::BTreeSet<String> {
            text.lines()
                .filter_map(|line| line.strip_prefix("#define "))
                .filter_map(|rest| rest.split_whitespace().next())
                .map(str::to_owned)
                .collect()
        };
        let overlap: Vec<String> = defined(&core).intersection(&defined(&std)).cloned().collect();
        assert!(overlap.is_empty(), "both headers define {overlap:?}");
        assert!(std.contains("#include \"core.h\""));
        assert!(std.contains("#define GNOBLIN_STD_SONAME \"libgnoblin-std.so.1\""));
        assert!(std.trim_end().ends_with("#endif /* GNOBLIN_STD_H */"));
    }
}
