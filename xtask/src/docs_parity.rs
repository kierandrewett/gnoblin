//! The docs-parity gate: a name that a shell-api or events page documents must be implemented.
//!
//! The implemented surface is the generated stub `docs/public/gnoblin.d.lua` (its functions and
//! object methods) plus the object signals the schema declares, each as `gnoblin.<object>.<signal>`.
//! A documented name that is not implemented must be listed in `design/docs-parity/allow.toml` with
//! the slice that will implement it, or `docs wrong`. The gate fails on an unlisted name, and on a
//! listed name that is now implemented or no longer documented, so the list cannot go stale.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde::Deserialize;

const STUB: &str = "docs/public/gnoblin.d.lua";
const SCHEMA_DIR: &str = "crates/gnoblin-schema/schema";
const DERIVED_FILE: &str = "crates/gnoblin-lua/derived_events.toml";
const ALLOW: &str = "design/docs-parity/allow.toml";
const DOC_DIRS: [&str; 2] = ["docs/shell-api", "docs/events"];
/// Methods of Lua's string library. A documented `s:find(` is string code, not a gnoblin name.
const LUA_STRING_METHODS: [&str; 10] = [
    "find", "sub", "gsub", "match", "gmatch", "format", "len", "rep", "byte", "lower",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowFile {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

/// A reviewed exception: a documented name that is not implemented yet.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AllowEntry {
    /// `gnoblin.a.b` for a dotted name, or `:method` for an object method.
    pub name: String,
    /// The documented page, relative to the repository root.
    pub doc: String,
    pub reason: String,
    #[serde(rename = "owner-slice")]
    pub owner_slice: String,
}

/// The implemented names. Dotted paths and events are matched whole; methods are matched by name
/// across every object class, because the stub does not tie a method to a class in a documented
/// reference.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Implemented {
    pub paths: BTreeSet<String>,
    pub events: BTreeSet<String>,
    pub methods: BTreeSet<String>,
}

impl Implemented {
    /// A dotted name is provided when it is an implemented path or event, or a namespace that
    /// holds one (`gnoblin.windows` for `gnoblin.windows.list`). A method is matched by name.
    fn provides(&self, name: &str) -> bool {
        if name.starts_with(':') {
            return self.methods.contains(name);
        }
        let prefix = format!("{name}.");
        self.paths.contains(name)
            || self.events.contains(name)
            || self.paths.iter().any(|path| path.starts_with(&prefix))
            || self.events.iter().any(|event| event.starts_with(&prefix))
    }
}

/// One documented occurrence of a name, with the page and line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub name: String,
    pub doc: String,
    pub line: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DocsReport {
    /// Distinct documented names that the surface does not provide.
    pub missing: usize,
    /// Of those, how many an allow entry covers.
    pub allowed: usize,
    /// Documented names not implemented and not listed; each one fails the gate.
    pub unlisted: Vec<Mention>,
    /// Allow entries that are implemented now or no longer documented; each one fails the gate.
    pub stale: Vec<String>,
    /// Distinct `(name, doc)` pairs in the documentation.
    pub documented: usize,
}

/// Read the functions the generated stub declares. A `Class:method` head is an object method; a
/// dotted head is a path.
pub fn parse_stub(text: &str) -> Implemented {
    let mut implemented = Implemented::default();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("function ") else {
            continue;
        };
        let Some(head) = rest.split('(').next() else {
            continue;
        };
        match head.rsplit_once(':') {
            Some((_, method)) => {
                implemented.methods.insert(format!(":{method}"));
            }
            None => {
                implemented.paths.insert(head.to_owned());
            }
        }
    }
    implemented
}

/// The `gnoblin.<object>.<signal>` names that the schema's objects emit. Each file is one TOML
/// document with an `object` array; an object may declare `signal` tables.
pub fn schema_events(documents: &[toml::Value]) -> BTreeSet<String> {
    let mut events = BTreeSet::new();
    for document in documents {
        let Some(objects) = document.get("object").and_then(toml::Value::as_array) else {
            continue;
        };
        for object in objects {
            let Some(name) = object.get("name").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(signals) = object.get("signal").and_then(toml::Value::as_array) else {
                continue;
            };
            for signal in signals {
                if let Some(signal) = signal.get("name").and_then(toml::Value::as_str) {
                    events.insert(format!("gnoblin.{name}.{signal}"));
                }
            }
        }
    }
    events
}

/// Every dotted `gnoblin.` name and every `:method(` call in one page. A `gnoblin.` name must not
/// follow a path or identifier character, so `docs/gnoblin.d.lua` and `https://` URLs are not
/// names; file suffixes are dropped.
pub fn scan_doc(doc: &str, text: &str) -> Vec<Mention> {
    let mut mentions = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut search = 0;
        while let Some(found) = line[search..].find("gnoblin.") {
            let start = search + found;
            search = start + "gnoblin.".len();
            // A name inside a longer identifier, path or URL is not a name we check.
            let preceded = start > 0 && (is_name_char(bytes[start - 1]) || bytes[start - 1] == b'/');
            if preceded {
                continue;
            }
            let mut end = search;
            while end < bytes.len() && (is_name_char(bytes[end]) || bytes[end] == b'.' || bytes[end] == b'-') {
                end += 1;
            }
            let token = line[start..end].trim_end_matches(['.', '-']);
            search = end;
            let file_suffix = [".lua", ".md", ".toml", ".json"]
                .iter()
                .any(|suffix| token.ends_with(suffix));
            if token.len() > "gnoblin.".len() && !file_suffix {
                mentions.push(Mention {
                    name: token.to_owned(),
                    doc: doc.to_owned(),
                    line: index + 1,
                });
            }
        }
        for (colon, _) in line.match_indices(':') {
            let after = &bytes[colon + 1..];
            if !after.first().is_some_and(|c| c.is_ascii_lowercase() || *c == b'_') {
                continue;
            }
            let ident_len = after
                .iter()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || **c == b'_')
                .count();
            if after.get(ident_len) != Some(&b'(') {
                continue;
            }
            let before = colon.checked_sub(1).map(|at| bytes[at]);
            if !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b')') {
                continue;
            }
            let method = std::str::from_utf8(&after[..ident_len]).unwrap_or_default();
            if !LUA_STRING_METHODS.contains(&method) {
                mentions.push(Mention {
                    name: format!(":{method}"),
                    doc: doc.to_owned(),
                    line: index + 1,
                });
            }
        }
    }
    mentions
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Compare the documented names with the implemented surface and the allow list.
pub fn reconcile(mentions: &[Mention], implemented: &Implemented, allow: &[AllowEntry]) -> DocsReport {
    let mut first: BTreeMap<(String, String), usize> = BTreeMap::new();
    for mention in mentions {
        first
            .entry((mention.name.clone(), mention.doc.clone()))
            .or_insert(mention.line);
    }
    let mut report = DocsReport {
        documented: first.len(),
        ..DocsReport::default()
    };
    for ((name, doc), line) in &first {
        if implemented.provides(name) {
            continue;
        }
        report.missing += 1;
        if allow.iter().any(|entry| &entry.name == name && &entry.doc == doc) {
            report.allowed += 1;
        } else {
            report.unlisted.push(Mention {
                name: name.clone(),
                doc: doc.clone(),
                line: *line,
            });
        }
    }
    for entry in allow {
        let key = (entry.name.clone(), entry.doc.clone());
        if implemented.provides(&entry.name) {
            report.stale.push(format!(
                "{} in {} is implemented now; remove its allow entry",
                entry.name, entry.doc
            ));
        } else if !first.contains_key(&key) {
            report.stale.push(format!(
                "{} in {} is no longer documented; remove its allow entry",
                entry.name, entry.doc
            ));
        }
    }
    report
}

fn load_allow(root: &Path) -> Result<Vec<AllowEntry>, String> {
    let path = root.join(ALLOW);
    let text = fs::read_to_string(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let parsed: AllowFile = toml::from_str(&text).map_err(|error| format!("invalid TOML in {ALLOW}: {error}"))?;
    for entry in &parsed.allow {
        if entry.reason.trim().is_empty() {
            return Err(format!("{ALLOW}: `{}` in {} has no reason", entry.name, entry.doc));
        }
        let slice = entry.owner_slice.as_str();
        let numbered = slice
            .strip_prefix('S')
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
        if !numbered && slice != "docs wrong" {
            return Err(format!(
                "{ALLOW}: `{}` in {} has owner-slice `{slice}`, not an S-number or `docs wrong`",
                entry.name, entry.doc
            ));
        }
    }
    Ok(parsed.allow)
}

fn load_schema(root: &Path) -> Result<Vec<toml::Value>, String> {
    let dir = root.join(SCHEMA_DIR);
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .map_err(|error| format!("could not read {}: {error}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let text =
                fs::read_to_string(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
            toml::from_str(&text).map_err(|error| format!("invalid TOML in {}: {error}", path.display()))
        })
        .collect()
}

/// The events the host raises from a property change, as `gnoblin.<object>.<signal>`. The
/// manifest is the one `gnoblin-lua`'s build reads, so the gate and the host cannot disagree.
fn load_derived(root: &Path) -> Result<BTreeSet<String>, String> {
    let path = root.join(DERIVED_FILE);
    let events =
        gnoblin_schema::derived::load(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    Ok(events
        .iter()
        .map(|event| format!("gnoblin.{}.{}", event.object, event.signal))
        .collect())
}

fn load_docs(root: &Path) -> Result<Vec<Mention>, String> {
    let mut mentions = Vec::new();
    for dir in DOC_DIRS {
        let mut pages: Vec<_> = fs::read_dir(root.join(dir))
            .map_err(|error| format!("could not read {dir}: {error}"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        pages.sort();
        for page in pages {
            let relative = page
                .strip_prefix(root)
                .unwrap_or(&page)
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&page).map_err(|error| format!("could not read {relative}: {error}"))?;
            mentions.extend(scan_doc(&relative, &text));
        }
    }
    Ok(mentions)
}

/// Run the gate from the repository root. Prints one line per failure and exits non-zero on any.
pub fn run() -> ExitCode {
    let root = Path::new(".");
    let outcome = (|| -> Result<DocsReport, String> {
        let stub = fs::read_to_string(root.join(STUB)).map_err(|error| format!("could not read {STUB}: {error}"))?;
        let mut implemented = parse_stub(&stub);
        implemented.events = schema_events(&load_schema(root)?);
        implemented.events.extend(load_derived(root)?);
        let allow = load_allow(root)?;
        Ok(reconcile(&load_docs(root)?, &implemented, &allow))
    })();
    let report = match outcome {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[docs-parity] {error}");
            return ExitCode::from(2);
        }
    };
    for mention in &report.unlisted {
        println!(
            "[docs-parity] unlisted {}:{} {}",
            mention.doc, mention.line, mention.name
        );
    }
    for stale in &report.stale {
        println!("[docs-parity] stale {stale}");
    }
    println!(
        "[docs-parity] {} documented names, {} not implemented ({} allowed, {} unlisted), {} stale",
        report.documented,
        report.missing,
        report.allowed,
        report.unlisted.len(),
        report.stale.len()
    );
    if report.unlisted.is_empty() && report.stale.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn implemented() -> Implemented {
        parse_stub(
            "function gnoblin.window_rule(rule) end\nfunction window:set_workspace(w) end\nfunction gnoblin.input.grab(o) end\n",
        )
    }

    fn entry(name: &str, doc: &str) -> AllowEntry {
        AllowEntry {
            name: name.into(),
            doc: doc.into(),
            reason: "why".into(),
            owner_slice: "docs wrong".into(),
        }
    }

    #[test]
    fn a_method_call_and_a_dotted_name_are_found_but_urls_and_paths_are_not() {
        let mentions = scan_doc(
            "docs/shell-api/x.md",
            "Call window:move_to_workspace(w) and `gnoblin.window.created`.\nSee https://example.com/gnoblin.a and docs/gnoblin.d.lua.\n",
        );
        let mut names: Vec<_> = mentions.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, [":move_to_workspace", "gnoblin.window.created"], "{mentions:?}");
        assert_eq!(mentions[0].line, 1);
    }

    #[test]
    fn a_hyphenated_event_name_is_read_whole() {
        let mentions = scan_doc("docs/events/a.md", "Subscribe to `gnoblin.session.lock-requested`.\n");
        let names: Vec<_> = mentions.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["gnoblin.session.lock-requested"], "{mentions:?}");
    }

    #[test]
    fn a_documented_name_absent_from_the_surface_is_unlisted() {
        let mentions = scan_doc("docs/events/a.md", "Use `gnoblin.input.gesture`.\n");
        let report = reconcile(&mentions, &implemented(), &[]);
        assert_eq!(report.missing, 1);
        assert_eq!(report.unlisted.len(), 1, "{report:?}");
        assert!(report.stale.is_empty());
    }

    #[test]
    fn an_allow_entry_for_a_missing_name_covers_it() {
        let mentions = scan_doc("docs/events/a.md", "Use `gnoblin.input.gesture`.\n");
        let allow = [entry("gnoblin.input.gesture", "docs/events/a.md")];
        let report = reconcile(&mentions, &implemented(), &allow);
        assert_eq!((report.missing, report.allowed), (1, 1));
        assert!(report.unlisted.is_empty() && report.stale.is_empty(), "{report:?}");
    }

    #[test]
    fn an_allow_entry_that_became_implemented_is_stale() {
        let mentions = scan_doc("docs/shell-api/a.md", "Call `gnoblin.input.grab(o)`.\n");
        let allow = [entry("gnoblin.input.grab", "docs/shell-api/a.md")];
        let report = reconcile(&mentions, &implemented(), &allow);
        assert_eq!(report.stale.len(), 1, "{report:?}");
        assert!(report.stale[0].contains("implemented now"));
    }

    #[test]
    fn an_allow_entry_for_a_name_no_longer_documented_is_stale() {
        let allow = [entry(":gone", "docs/shell-api/a.md")];
        let report = reconcile(&[], &implemented(), &allow);
        assert_eq!(report.stale.len(), 1, "{report:?}");
        assert!(report.stale[0].contains("no longer documented"));
    }

    #[test]
    fn schema_signals_become_object_events() {
        let document: toml::Value =
            toml::from_str("[[object]]\nname = \"window\"\n[[object.signal]]\nname = \"closed\"\n").unwrap();
        assert_eq!(
            schema_events(&[document]),
            BTreeSet::from(["gnoblin.window.closed".to_owned()])
        );
    }

    #[test]
    fn derived_events_count_as_implemented() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the xtask lives in the repository");
        let mut surface = implemented();
        surface.events = load_derived(root).expect("the derived manifest reads");
        let mentions = scan_doc("docs/events/a.md", "Subscribe to `gnoblin.window.focused`.\n");
        let report = reconcile(&mentions, &surface, &[]);
        assert!(report.unlisted.is_empty() && report.stale.is_empty(), "{report:?}");
    }

    #[test]
    fn a_derived_event_missing_from_the_manifest_is_unlisted() {
        let mentions = scan_doc("docs/events/a.md", "Subscribe to `gnoblin.window.focused`.\n");
        let report = reconcile(&mentions, &implemented(), &[]);
        assert_eq!(report.unlisted.len(), 1, "{report:?}");
    }

    #[test]
    fn a_namespace_holding_an_implemented_path_is_provided() {
        let mentions = scan_doc("docs/shell-api/a.md", "Use `gnoblin.input`, not a method.\n");
        let report = reconcile(&mentions, &implemented(), &[]);
        assert!(report.unlisted.is_empty(), "{report:?}");
    }

    #[test]
    fn lua_string_methods_are_not_gnoblin_names() {
        let mentions = scan_doc("docs/shell-api/a.md", "local i = text:find(\"x\") and s:sub(1, 2)\n");
        assert!(mentions.is_empty(), "{mentions:?}");
    }
}
