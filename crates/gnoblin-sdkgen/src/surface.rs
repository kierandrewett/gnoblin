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

use gnoblin_schema::{CallbackDecl, EnumDecl, Method, ObjectKind, ObjectType, Property, Signal, Version};

use super::model::{MODEL_DIRS, SdkSpec};

/// The directory the lock files live in, relative to the workspace root.
pub const LOCK_DIR: &str = "crates/gnoblin-schema/schema/lock";

/// One line of a surface lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceEntry {
    /// `object`, `member`, `enum` or `callback`.
    pub kind: &'static str,
    /// `<object>`, `<object>.<member>`, or the name of an enum or callback.
    pub key: String,
    /// The shape: the type and access of a property, the parameters of a method, the fields of a signal, the values of an enum.
    pub detail: String,
    /// The version that added it.
    pub since: Version,
}

/// One lock file and what it records: a namespace of the compositor's schema, or one module of a namespace a library serves.
pub struct LockUnit {
    /// The lock's path, relative to the workspace root.
    pub path: String,
    /// What the lock describes, for its header.
    pub title: String,
    /// Where the catalog files it covers are, for its header.
    pub source: String,
    /// The entries, sorted by kind, then key.
    pub entries: Vec<SurfaceEntry>,
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
    if property.optional {
        text.push_str(",optional");
    }
    if property.secret {
        text.push_str(",secret");
    }
    if property.extras.inert.is_some() {
        text.push_str(",inert");
    }
    if let Some(abi) = &property.extras.abi {
        let _ = write!(text, ",abi={abi}");
    }
    text
}

fn method_detail(method: &Method) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|field| format!("{}{}:{}", field.name, if field.optional { "?" } else { "" }, field.ty))
        .collect();
    let mut text = format!(
        "{} ({})",
        if method.function { "function" } else { "method" },
        params.join(", ")
    );
    if let Some(returns) = &method.returns {
        let _ = write!(text, " -> {returns}{}", if method.optional { "?" } else { "" });
    }
    if method.required {
        text.push_str(" required");
    }
    if method.native_default {
        text.push_str(" native-default");
    }
    if method.extras.inert.is_some() {
        text.push_str(" inert");
    }
    if let Some(abi) = &method.extras.abi {
        let _ = write!(text, " abi={abi}");
    }
    if method.secret {
        text.push_str(" secret");
    }
    text
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

fn enum_entry(declared: &EnumDecl) -> SurfaceEntry {
    SurfaceEntry {
        kind: "enum",
        key: declared.name.clone(),
        detail: format!("values {}", declared.values.join(" ")),
        since: declared.since,
    }
}

fn callback_entry(declared: &CallbackDecl) -> SurfaceEntry {
    let params: Vec<String> = declared
        .params
        .iter()
        .map(|field| format!("{}{}:{}", field.name, if field.optional { "?" } else { "" }, field.ty))
        .collect();
    let mut detail = format!("({})", params.join(", "));
    if let Some(returns) = &declared.returns {
        let _ = write!(detail, " -> {returns}");
    }
    SurfaceEntry {
        kind: "callback",
        key: declared.name.clone(),
        detail,
        since: declared.since,
    }
}

fn object_entries(namespace: &str, object: &ObjectType, entries: &mut Vec<SurfaceEntry>) {
    let mut detail = String::new();
    if let Some(base) = &object.base {
        let _ = write!(detail, "base {base}");
    } else {
        detail.push_str("base none");
    }
    match object.kind {
        ObjectKind::Record => detail.push_str(" record"),
        ObjectKind::Interface => detail.push_str(" interface"),
        ObjectKind::Service => detail.push_str(" service"),
        ObjectKind::Handle => {}
    }
    if object.retained {
        detail.push_str(" retained");
    }
    if let Some(ctor) = &object.ctor {
        let _ = write!(detail, " ctor({})", ctor.join(", "));
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

/// The surface of every namespace of the compositor's schema: the entries of its lock, sorted by kind, then key.
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

/// The locks of the namespaces a library serves: one per module (a file of the catalog directory), so two lanes that add classes
/// to different modules edit different locks.
pub fn module_units(spec: &SdkSpec) -> Vec<LockUnit> {
    let mut units: BTreeMap<String, LockUnit> = BTreeMap::new();
    for schema in spec.objects.schemas() {
        let origin = Path::new(&schema.origin);
        let Some(dir) = MODEL_DIRS.iter().find(|dir| origin.starts_with(spec.root.join(dir))) else {
            continue;
        };
        let Some(stem) = origin.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let path = format!("{dir}/lock/{stem}.lock");
        let unit = units.entry(path.clone()).or_insert_with(|| LockUnit {
            path,
            title: format!("the `{stem}` module of the `{}` namespace", schema.namespace),
            source: (*dir).to_owned(),
            entries: Vec::new(),
        });
        for object in &schema.objects {
            object_entries(schema.namespace.as_str(), object, &mut unit.entries);
        }
        unit.entries.extend(schema.enums.iter().map(enum_entry));
        unit.entries.extend(schema.callbacks.iter().map(callback_entry));
    }
    let mut units: Vec<LockUnit> = units.into_values().collect();
    for unit in &mut units {
        unit.entries.sort_by(|a, b| (a.kind, &a.key).cmp(&(b.kind, &b.key)));
    }
    units
}

/// The header of a lock. `title` names what the lock describes and `source` the files it covers; for a namespace of the compositor's
/// schema the words are the ones the first locks were written with, so they do not change.
fn header(title: &str, source: &str) -> String {
    format!(
        "\
# The public surface of {title}: every object and member, with the version that
# added it. Written by `cargo xtask gen-sdks --update`; reviewed like source.
#
# `cargo xtask gen-sdks --check` compares this file with {source}. An entry here that is missing or
# different in the catalog is a removal or change; when `released` covers its `since` it is a breaking change and fails with
# no way to bless it inside this major. A new entry in the catalog is additive and is recorded with `--update`.
#
# `released none` means the surface is still unstable (design/modular-framework.md, decision D18): the release commit sets it
# to the version that ships, and from then on its entries are frozen.
"
    )
}

/// The header of the lock of a namespace of the compositor's schema.
fn namespace_header(namespace: &str) -> String {
    header(
        &format!("the `{namespace}` namespace of the schema catalog"),
        "crates/gnoblin-schema/schema",
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
            "enum" => "enum",
            "callback" => "callback",
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

fn render_lock(header: String, released: Option<Version>, entries: &[SurfaceEntry]) -> String {
    let mut text = header;
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

/// Every lock the catalogs call for: one per namespace of the compositor's schema, one per module of a namespace a library serves.
fn units(spec: &SdkSpec) -> Vec<LockUnit> {
    let mut units: Vec<LockUnit> = surface_lines(spec)
        .into_iter()
        .map(|(namespace, entries)| LockUnit {
            path: format!("{LOCK_DIR}/{namespace}.lock"),
            title: namespace,
            source: String::new(),
            entries,
        })
        .collect();
    units.extend(module_units(spec));
    units
}

/// The header a unit's lock starts with.
fn unit_header(unit: &LockUnit) -> String {
    if unit.source.is_empty() {
        namespace_header(&unit.title)
    } else {
        header(&unit.title, &unit.source)
    }
}

/// Compares every lock under `root` with the catalogs `spec` holds.
///
/// A namespace or module with no lock file is all stale (`--update` writes the first one), and a lock file for one the catalog no
/// longer has is stale too.
///
/// # Errors
///
/// A lock file cannot be read or parsed.
pub fn check_surface_lock(spec: &SdkSpec, root: &Path) -> Result<LockReport, String> {
    let mut report = LockReport::default();
    let units = units(spec);
    for unit in &units {
        let shown = &unit.path;
        let Ok(text) = fs::read_to_string(root.join(&unit.path)) else {
            report.stale.push(format!(
                "{shown} does not exist; `cargo xtask gen-sdks --update` writes it"
            ));
            continue;
        };
        let lock = parse_lock(&text, shown)?;
        let found = diff(&lock, &unit.entries);
        let clean = found.is_clean();
        report
            .breaking
            .extend(found.breaking.into_iter().map(|line| format!("{shown}: {line}")));
        report
            .stale
            .extend(found.stale.into_iter().map(|line| format!("{shown}: {line}")));
        if clean && text != render_lock(unit_header(unit), lock.released, &unit.entries) {
            report.stale.push(format!("{shown} is not the text `--update` writes"));
        }
    }
    let mut dirs = vec![LOCK_DIR.to_owned()];
    dirs.extend(MODEL_DIRS.iter().map(|dir| format!("{dir}/lock")));
    for dir in dirs {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{dir}/{name}");
            if name.ends_with(".lock") && !units.iter().any(|unit| unit.path == path) {
                report
                    .stale
                    .push(format!("{path} is for a namespace or module the catalog does not have"));
            }
        }
    }
    Ok(report)
}

/// The text of every lock as `--update` writes it: the `released` line of an existing lock is kept.
///
/// # Errors
///
/// An existing lock cannot be parsed.
pub fn render_surface_locks(spec: &SdkSpec, root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut locks = Vec::new();
    for unit in units(spec) {
        let released = match fs::read_to_string(root.join(&unit.path)) {
            Ok(text) => parse_lock(&text, &unit.path)?.released,
            Err(_) => None,
        };
        let text = render_lock(unit_header(&unit), released, &unit.entries);
        locks.push((unit.path, text));
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
        let text = render_lock(namespace_header("tree"), None, &entries);
        let parsed = parse_lock(&text, "x.lock").expect("parses");
        assert_eq!(parsed.entries, entries);
        assert_eq!(parsed.released, None);
        assert!(
            parse_lock("member | a | b | 0.1.0\n", "x.lock").is_err(),
            "a lock needs a released line"
        );
    }
}
