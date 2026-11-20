//! The surface lock: what proves the public surface of the catalog is stable now that no generated file is committed.
//!
//! The generator renders the surface of every catalog object as sorted lines, one per member, and
//! `crates/gnoblin-schema/schema/lock/<namespace>.lock` records them (one file per namespace, so two lanes that add objects
//! to different namespaces edit different files):
//!
//! ```text
//! object | tree | window | base node | 0.1.0
//! member | tree | window.title | property string rw | 0.1.0
//! member | tree | window.close | method () | 0.1.0
//! member | tree | window.moved | signal (x:int, y:int) | 0.1.0
//! ```
//!
//! The comparison is the one `abi-check` makes for the C ABI. An entry the lock holds that is gone or different in the
//! catalog is a removal or change: when the lock's `released` line covers the entry's `since` it is breaking and the check fails
//! with no way to bless it, and before the release it is only a stale lock. An entry the catalog has that the lock lacks is
//! additive: the lock is stale and `--update` records it. The generated SDKs of every language follow the catalog, so a lock
//! that holds proves they keep every released member.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use gnoblin_schema::{Method, ObjectType, Property, Signal, Version};

use super::model::SdkSpec;

/// The directory the lock files live in, relative to the workspace root.
pub const LOCK_DIR: &str = "crates/gnoblin-schema/schema/lock";

/// One line of a surface lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceEntry {
    /// `object` or `member`.
    pub kind: &'static str,
    /// `<object>` or `<object>.<member>`.
    pub key: String,
    /// The shape: the type and access of a property, the parameters of a method, the fields of a signal.
    pub detail: String,
    /// The version that added it.
    pub since: Version,
}

/// What the comparison of one lock found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LockReport {
    /// Changes to released entries: not fixable by updating the lock.
    pub breaking: Vec<String>,
    /// Differences the lock can absorb with `--update`.
    pub stale: Vec<String>,
}

impl LockReport {
    /// True when the lock and the catalog agree.
    pub fn is_clean(&self) -> bool {
        self.breaking.is_empty() && self.stale.is_empty()
    }
}

fn property_detail(property: &Property) -> String {
    let mut text = format!(
        "property {} {}",
        property.ty,
        if property.readonly { "ro" } else { "rw" }
    );
    if property.animated {
        text.push_str(",animated");
    }
    if property.secret {
        text.push_str(",secret");
    }
    text
}

fn method_detail(method: &Method) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|field| format!("{}{}:{}", field.name, if field.optional { "?" } else { "" }, field.ty))
        .collect();
    format!(
        "{} ({}){}",
        if method.function { "function" } else { "method" },
        params.join(", "),
        if method.secret { " secret" } else { "" }
    )
}

fn signal_detail(signal: &Signal) -> String {
    let fields: Vec<String> = signal
        .fields
        .iter()
        .map(|field| format!("{}:{}", field.name, field.ty))
        .collect();
    format!(
        "signal ({}){}",
        fields.join(", "),
        if signal.secret { " secret" } else { "" }
    )
}

fn object_entries(namespace: &str, object: &ObjectType, entries: &mut Vec<SurfaceEntry>) {
    let mut detail = String::new();
    if let Some(base) = &object.base {
        let _ = write!(detail, "base {base}");
    } else {
        detail.push_str("base none");
    }
    if object.record {
        detail.push_str(" record");
    }
    if object.secret {
        detail.push_str(" secret");
    }
    let _ = namespace;
    entries.push(SurfaceEntry {
        kind: "object",
        key: object.name.clone(),
        detail,
        since: object.since,
    });
    for property in &object.properties {
        entries.push(SurfaceEntry {
            kind: "member",
            key: format!("{}.{}", object.name, property.name),
            detail: property_detail(property),
            since: property.since,
        });
    }
    for method in &object.methods {
        entries.push(SurfaceEntry {
            kind: "member",
            key: format!("{}.{}", object.name, method.name),
            detail: method_detail(method),
            since: method.since,
        });
    }
    for signal in &object.signals {
        entries.push(SurfaceEntry {
            kind: "member",
            key: format!("{}.{}", object.name, signal.name),
            detail: signal_detail(signal),
            since: signal.since,
        });
    }
}

/// The surface of every namespace of the catalog: the entries of its lock, sorted by kind, then key.
pub fn surface_lines(spec: &SdkSpec) -> BTreeMap<String, Vec<SurfaceEntry>> {
    let mut surface = BTreeMap::new();
    for namespace in spec.catalog.namespaces() {
        let mut entries = Vec::new();
        for object in spec.catalog.objects(&namespace) {
            object_entries(namespace.as_str(), object, &mut entries);
        }
        entries.sort_by(|a, b| (a.kind, &a.key).cmp(&(b.kind, &b.key)));
        surface.insert(namespace.as_str().to_owned(), entries);
    }
    surface
}

fn header(namespace: &str) -> String {
    format!(
        "\
# The public surface of the `{namespace}` namespace of the schema catalog: every object and member, with the version that
# added it. Written by `cargo xtask gen-sdks --update`; reviewed like source.
#
# `cargo xtask gen-sdks --check` compares this file with crates/gnoblin-schema/schema. An entry here that is missing or
# different in the catalog is a removal or change; when `released` covers its `since` it is a breaking change and fails with
# no way to bless it inside this major. A new entry in the catalog is additive and is recorded with `--update`.
#
# `released none` means the surface is still unstable (design/modular-framework.md, decision D18): the release commit sets it
# to the version that ships, and from then on its entries are frozen.
"
    )
}

/// A lock file, parsed.
struct ParsedLock {
    released: Option<Version>,
    entries: Vec<SurfaceEntry>,
}

fn parse_lock(text: &str, path: &str) -> Result<ParsedLock, String> {
    let mut released = None;
    let mut released_seen = false;
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |message: String| format!("{path}:{}: {message}", index + 1);
        if let Some(version) = line.strip_prefix("released ") {
            released_seen = true;
            released = match version.trim() {
                "none" => None,
                other => Some(Version::parse(other).map_err(|error| at(error.to_string()))?),
            };
            continue;
        }
        let parts: Vec<&str> = line.split(" | ").collect();
        let [kind, key, detail, since] = parts.as_slice() else {
            return Err(at("expected `kind | key | detail | since`".to_owned()));
        };
        let kind = match *kind {
            "object" => "object",
            "member" => "member",
            other => return Err(at(format!("unknown kind `{other}`"))),
        };
        entries.push(SurfaceEntry {
            kind,
            key: (*key).to_owned(),
            detail: (*detail).to_owned(),
            since: Version::parse(since).map_err(|error| at(error.to_string()))?,
        });
    }
    if !released_seen {
        return Err(format!("{path}: no `released` line"));
    }
    Ok(ParsedLock { released, entries })
}

fn render_lock(namespace: &str, released: Option<Version>, entries: &[SurfaceEntry]) -> String {
    let mut text = header(namespace);
    match released {
        Some(version) => {
            let _ = writeln!(text, "released {version}");
        }
        None => text.push_str("released none\n"),
    }
    let mut previous = "";
    for entry in entries {
        if entry.kind != previous {
            text.push('\n');
            previous = entry.kind;
        }
        let _ = writeln!(
            text,
            "{} | {} | {} | {}",
            entry.kind, entry.key, entry.detail, entry.since
        );
    }
    text
}

/// Compares one lock's entries with the catalog's.
fn diff(lock: &ParsedLock, current: &[SurfaceEntry]) -> LockReport {
    let frozen = |since: Version| lock.released.is_some_and(|released| since <= released);
    let by_key: BTreeMap<(&str, &str), &SurfaceEntry> = current
        .iter()
        .map(|entry| ((entry.kind, entry.key.as_str()), entry))
        .collect();
    let mut report = LockReport::default();
    let mut record = |frozen: bool, since: Version, message: String| {
        if frozen {
            report.breaking.push(format!("{message} (released in {since})"));
        } else {
            report.stale.push(message);
        }
    };
    for entry in &lock.entries {
        let name = format!("{} {}", entry.kind, entry.key);
        match by_key.get(&(entry.kind, entry.key.as_str())) {
            None => record(frozen(entry.since), entry.since, format!("{name} was removed")),
            Some(now) if now.detail == entry.detail && now.since == entry.since => {}
            Some(now) => record(
                frozen(entry.since),
                entry.since,
                format!(
                    "{name} changed: was `{}` since {}, is now `{}` since {}",
                    entry.detail, entry.since, now.detail, now.since
                ),
            ),
        }
    }
    let known: std::collections::BTreeSet<(&str, &str)> = lock
        .entries
        .iter()
        .map(|entry| (entry.kind, entry.key.as_str()))
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

fn lock_path(root: &Path, namespace: &str) -> std::path::PathBuf {
    root.join(LOCK_DIR).join(format!("{namespace}.lock"))
}

/// Compares every namespace's lock under `root` with the catalog `spec` holds.
///
/// A namespace with no lock file is all stale (`--update` writes the first one), and a lock file for a namespace the catalog
/// no longer has is stale too.
///
/// # Errors
///
/// A lock file cannot be read or parsed.
pub fn check_surface_lock(spec: &SdkSpec, root: &Path) -> Result<LockReport, String> {
    let mut report = LockReport::default();
    let surface = surface_lines(spec);
    for (namespace, entries) in &surface {
        let path = lock_path(root, namespace);
        let shown = format!("{LOCK_DIR}/{namespace}.lock");
        let Ok(text) = fs::read_to_string(&path) else {
            report.stale.push(format!(
                "{shown} does not exist; `cargo xtask gen-sdks --update` writes it"
            ));
            continue;
        };
        let lock = parse_lock(&text, &shown)?;
        let found = diff(&lock, entries);
        let clean = found.is_clean();
        report
            .breaking
            .extend(found.breaking.into_iter().map(|line| format!("{namespace}: {line}")));
        report
            .stale
            .extend(found.stale.into_iter().map(|line| format!("{namespace}: {line}")));
        if clean && text != render_lock(namespace, lock.released, entries) {
            report.stale.push(format!("{shown} is not the text `--update` writes"));
        }
    }
    if let Ok(dir) = fs::read_dir(root.join(LOCK_DIR)) {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(namespace) = name.strip_suffix(".lock")
                && !surface.contains_key(namespace)
            {
                report.stale.push(format!(
                    "{LOCK_DIR}/{name} is for a namespace the catalog does not have"
                ));
            }
        }
    }
    Ok(report)
}

/// The text of every namespace's lock as `--update` writes it: the `released` line of an existing lock is kept.
///
/// # Errors
///
/// An existing lock cannot be parsed.
pub fn render_surface_locks(spec: &SdkSpec, root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut locks = Vec::new();
    for (namespace, entries) in surface_lines(spec) {
        let released = match fs::read_to_string(lock_path(root, &namespace)) {
            Ok(text) => parse_lock(&text, &format!("{LOCK_DIR}/{namespace}.lock"))?.released,
            Err(_) => None,
        };
        locks.push((
            format!("{LOCK_DIR}/{namespace}.lock"),
            render_lock(&namespace, released, &entries),
        ));
    }
    Ok(locks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &'static str, key: &str, detail: &str, since: &str) -> SurfaceEntry {
        SurfaceEntry {
            kind,
            key: key.to_owned(),
            detail: detail.to_owned(),
            since: Version::parse(since).expect("a version"),
        }
    }

    fn lock(released: Option<&str>, entries: Vec<SurfaceEntry>) -> ParsedLock {
        ParsedLock {
            released: released.map(|text| Version::parse(text).expect("a version")),
            entries,
        }
    }

    #[test]
    fn a_released_member_that_goes_is_breaking_and_an_unreleased_one_is_only_stale() {
        let current = vec![entry("member", "window.title", "property string rw", "0.1.0")];
        let held = lock(
            Some("0.1.0"),
            vec![
                entry("member", "window.title", "property string rw", "0.1.0"),
                entry("member", "window.gone", "method ()", "0.1.0"),
                entry("member", "window.later", "method ()", "0.2.0"),
            ],
        );
        let report = diff(&held, &current);
        assert_eq!(report.breaking.len(), 1, "{report:?}");
        assert!(report.breaking[0].contains("window.gone"));
        assert_eq!(report.stale.len(), 1, "{report:?}");
        assert!(report.stale[0].contains("window.later"));
    }

    #[test]
    fn a_new_member_is_stale_until_the_lock_records_it() {
        let current = vec![
            entry("member", "window.title", "property string rw", "0.1.0"),
            entry("member", "window.added", "method ()", "0.1.0"),
        ];
        let held = lock(
            None,
            vec![entry("member", "window.title", "property string rw", "0.1.0")],
        );
        let report = diff(&held, &current);
        assert!(report.breaking.is_empty());
        assert_eq!(report.stale.len(), 1);
        assert!(report.stale[0].contains("is new"));
    }

    #[test]
    fn a_changed_type_of_a_released_member_is_breaking() {
        let current = vec![entry("member", "window.title", "property int rw", "0.1.0")];
        let held = lock(
            Some("0.1.0"),
            vec![entry("member", "window.title", "property string rw", "0.1.0")],
        );
        assert_eq!(diff(&held, &current).breaking.len(), 1);
    }

    #[test]
    fn a_lock_renders_and_parses_back_to_the_same_entries() {
        let entries = vec![
            entry("object", "window", "base node", "0.1.0"),
            entry("member", "window.title", "property string rw", "0.1.0"),
        ];
        let text = render_lock("tree", None, &entries);
        let parsed = parse_lock(&text, "x.lock").expect("parses");
        assert_eq!(parsed.entries, entries);
        assert_eq!(parsed.released, None);
        assert!(
            parse_lock("member | a | b | 0.1.0\n", "x.lock").is_err(),
            "a lock needs a released line"
        );
    }
}
