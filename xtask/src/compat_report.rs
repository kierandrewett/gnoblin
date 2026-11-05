//! `cargo xtask compat-report`: every compat adapter's declared support matrix.
//!
//! The frame that owns the format is `gnoblin-compat-core`, but the
//! `compat-host` dependency rule keeps xtask from linking any compat crate, so
//! this report reads the `support-matrix.json` each adapter checks in beside its
//! `Cargo.toml` and that the adapter itself parses with
//! `SupportMatrix::from_json` at start-up. The declaration is therefore the
//! reviewed artifact, the adapter's behaviour and this report read the same file.
//!
//! Two readers share one schema without sharing code, so both are pinned to the
//! frame's own fixture: the frame's test asserts that its renderer writes
//! `fixtures/matrix.json` and `fixtures/matrix.txt` byte for byte, and the test
//! here asserts that this printer reproduces those same bytes from that same
//! manifest. A schema change therefore fails on both sides.
//!
//! Validation here is deliberately light: required fields, known support words,
//! a note or reason where one is required. The frame is what honours a
//! declaration, so it is the frame that rejects a repeated member; this report
//! exists so a manifest is printable before anything is built.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Map, Value};

/// Where adapters live. Every directory under it with a manifest is reported.
const ADAPTER_ROOT: &str = "crates/compat";

/// The manifest an adapter checks in beside its crate root.
const MANIFEST: &str = "support-matrix.json";

/// Print every declared support matrix, as text or as JSON.
pub fn run(json: bool) -> ExitCode {
    let adapters = match collect(Path::new(ADAPTER_ROOT)) {
        Ok(adapters) => adapters,
        Err(message) => {
            eprintln!("[compat-report] {message}");
            return ExitCode::FAILURE;
        }
    };
    if json {
        println!("{}", report_json(&adapters));
        return ExitCode::SUCCESS;
    }
    println!(
        "[compat-report] {} adapter(s) declared under {ADAPTER_ROOT}",
        adapters.len()
    );
    for adapter in &adapters {
        println!("[compat-report] {} {}", adapter.crate_name, adapter.path.display());
        let text = adapter.render_text();
        if text.is_empty() {
            println!("  (no interfaces declared)");
        } else {
            print!("{text}");
        }
    }
    ExitCode::SUCCESS
}

/// One adapter, read from its checked-in manifest.
struct Adapter {
    crate_name: String,
    path: PathBuf,
    interfaces: Vec<Interface>,
    raw: Value,
}

impl Adapter {
    /// The frame's text rendering of this adapter's declaration.
    fn render_text(&self) -> String {
        let mut out = String::new();
        for interface in &self.interfaces {
            writeln!(out, "interface {} version {}", interface.name, interface.version)
                .expect("writing to a String cannot fail");
            for member in &interface.members {
                write!(out, "  {} {} {}", member.kind, member.name, member.support)
                    .expect("writing to a String cannot fail");
                match &member.detail {
                    Some(detail) => writeln!(out, ": {detail}").expect("writing to a String cannot fail"),
                    None => writeln!(out).expect("writing to a String cannot fail"),
                }
            }
        }
        out
    }
}

struct Interface {
    name: String,
    version: u64,
    members: Vec<Member>,
}

struct Member {
    kind: String,
    name: String,
    support: String,
    /// The note of a `partial` member, or the reason of an `unsupported` one.
    detail: Option<String>,
}

/// Every adapter under `root`, in crate-name order.
fn collect(root: &Path) -> Result<Vec<Adapter>, String> {
    let entries = fs::read_dir(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let mut adapters = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
        let directory = entry.path();
        if !directory.is_dir() {
            continue;
        }
        let manifest = directory.join(MANIFEST);
        if !manifest.is_file() {
            continue;
        }
        let crate_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unnamed>")
            .to_owned();
        let text = fs::read_to_string(&manifest).map_err(|error| format!("{}: {error}", manifest.display()))?;
        let (interfaces, raw) = parse_manifest(&manifest.display().to_string(), &text)?;
        adapters.push(Adapter {
            crate_name,
            path: manifest,
            interfaces,
            raw,
        });
    }
    adapters.sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    Ok(adapters)
}

fn parse_manifest(label: &str, text: &str) -> Result<(Vec<Interface>, Value), String> {
    let document: Value = serde_json::from_str(text).map_err(|error| format!("{label}: {error}"))?;
    let object = document
        .as_object()
        .ok_or_else(|| error(label, "<document>", "expected an object"))?;
    for key in object.keys() {
        if key != "interfaces" {
            return Err(error(label, "<document>", &format!("unknown key `{key}`")));
        }
    }
    let entries = object
        .get("interfaces")
        .and_then(Value::as_array)
        .ok_or_else(|| error(label, "interfaces", "expected an array"))?;
    let mut interfaces = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        interfaces.push(parse_interface(label, &format!("interfaces[{index}]"), entry)?);
    }
    Ok((interfaces, document))
}

fn parse_interface(label: &str, path: &str, value: &Value) -> Result<Interface, String> {
    let object = value
        .as_object()
        .ok_or_else(|| error(label, path, "expected an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "name" | "version" | "members") {
            return Err(error(label, path, &format!("unknown key `{key}`")));
        }
    }
    let name = string_field(label, path, object, "name")?.to_owned();
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| error(label, &format!("{path}.version"), "expected a non-negative integer"))?;
    let entries = object
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| error(label, &format!("{path}.members"), "expected an array"))?;
    let mut members = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        members.push(parse_member(label, &format!("{path}.members[{position}]"), entry)?);
    }
    Ok(Interface { name, version, members })
}

fn parse_member(label: &str, path: &str, value: &Value) -> Result<Member, String> {
    let object = value
        .as_object()
        .ok_or_else(|| error(label, path, "expected an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "kind" | "name" | "support" | "note" | "reason") {
            return Err(error(label, path, &format!("unknown key `{key}`")));
        }
    }
    let kind = string_field(label, path, object, "kind")?.to_owned();
    if !matches!(kind.as_str(), "method" | "property" | "signal") {
        return Err(error(
            label,
            &format!("{path}.kind"),
            &format!("unknown member kind `{kind}`"),
        ));
    }
    let name = string_field(label, path, object, "name")?.to_owned();
    let support = string_field(label, path, object, "support")?.to_owned();
    let note = optional_text(label, path, object, "note")?;
    let reason = optional_text(label, path, object, "reason")?;
    let detail = match support.as_str() {
        "supported" => {
            if note.is_some() || reason.is_some() {
                return Err(error(
                    label,
                    path,
                    &format!("`{name}` is supported, so it takes neither a note nor a reason"),
                ));
            }
            None
        }
        "partial" => Some(note.ok_or_else(|| {
            error(
                label,
                &format!("{path}.note"),
                &format!("`{name}` is partial, so it needs a note"),
            )
        })?),
        "unsupported" => Some(reason.ok_or_else(|| {
            error(
                label,
                &format!("{path}.reason"),
                &format!("`{name}` is unsupported, so it needs a reason"),
            )
        })?),
        other => {
            return Err(error(
                label,
                &format!("{path}.support"),
                &format!("unknown support `{other}`; expected `supported`, `partial` or `unsupported`"),
            ));
        }
    };
    Ok(Member {
        kind,
        name,
        support,
        detail,
    })
}

fn string_field<'a>(label: &str, path: &str, object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| error(label, &format!("{path}.{key}"), "expected a string"))
}

fn optional_text(label: &str, path: &str, object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| error(label, &format!("{path}.{key}"), "expected a string"))?;
            if text.trim().is_empty() {
                return Err(error(
                    label,
                    &format!("{path}.{key}"),
                    "an empty explanation is not a declaration",
                ));
            }
            Ok(Some(text.to_owned()))
        }
    }
}

fn error(label: &str, path: &str, message: &str) -> String {
    format!("{label}: {path}: {message}")
}

fn report_json(adapters: &[Adapter]) -> String {
    let mut entries = Vec::with_capacity(adapters.len());
    for adapter in adapters {
        let mut object = Map::new();
        object.insert("crate".to_owned(), Value::String(adapter.crate_name.clone()));
        object.insert("manifest".to_owned(), Value::String(adapter.path.display().to_string()));
        object.insert("support_matrix".to_owned(), adapter.raw.clone());
        entries.push(Value::Object(object));
    }
    let mut root = Map::new();
    root.insert("adapters".to_owned(), Value::Array(entries));
    serde_json::to_string_pretty(&Value::Object(root)).expect("a JSON value always serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame's own fixture. `gnoblin-compat-core`'s test asserts that its
    /// renderers write exactly these bytes, so reading them here checks this
    /// printer against the frame's schema rather than against a copy of it.
    const FIXTURE_JSON: &str = include_str!("../../crates/compat/gnoblin-compat-core/fixtures/matrix.json");
    const FIXTURE_TEXT: &str = include_str!("../../crates/compat/gnoblin-compat-core/fixtures/matrix.txt");

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/compat/gnoblin-compat-core/fixtures")
    }

    #[test]
    fn the_printer_reproduces_the_frame_text_rendering() {
        let (interfaces, raw) = parse_manifest("matrix.json", FIXTURE_JSON).expect("the frame fixture parses");
        let adapter = Adapter {
            crate_name: "gnoblin-compat-fixture".to_owned(),
            path: PathBuf::from("matrix.json"),
            interfaces,
            raw,
        };
        assert_eq!(adapter.render_text(), FIXTURE_TEXT);
    }

    #[test]
    fn an_example_adapter_is_found_and_printed() {
        let adapters = collect(&fixtures().join("adapters")).expect("the example adapter collects");
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].crate_name, "gnoblin-compat-example");
        let text = adapters[0].render_text();
        assert!(
            text.starts_with("interface org.gnome.Shell.Introspect version 1\n"),
            "{text}"
        );
        assert!(text.contains("  method GetWindows supported"), "{text}");
        assert!(text.contains("partial: only the application id is filled in"), "{text}");
    }

    #[test]
    fn a_directory_without_manifests_reports_nothing() {
        assert!(collect(&fixtures()).expect("the fixture root reads").is_empty());
    }

    #[test]
    fn an_empty_report_is_the_empty_set() {
        assert_eq!(report_json(&[]), "{\n  \"adapters\": []\n}");
    }

    #[test]
    fn a_malformed_manifest_fails_with_its_path_and_reason() {
        let error = match collect(&fixtures().join("broken")) {
            Ok(adapters) => panic!("the broken manifest was accepted as {} adapter(s)", adapters.len()),
            Err(error) => error,
        };
        assert!(error.contains("support-matrix.json"), "{error}");
        assert!(error.contains("mostly"), "{error}");
    }

    #[test]
    fn the_json_report_keeps_each_matrix_as_its_manifest_wrote_it() {
        let adapters = collect(&fixtures().join("adapters")).expect("the example adapter collects");
        let report: Value = serde_json::from_str(&report_json(&adapters)).expect("the report is JSON");
        let entries = report["adapters"].as_array().expect("adapters is an array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["crate"], Value::String("gnoblin-compat-example".to_owned()));
        assert!(entries[0]["manifest"].as_str().is_some());
        assert_eq!(
            entries[0]["support_matrix"]["interfaces"][0]["members"][1]["name"],
            Value::String("GetWindows".to_owned())
        );
    }
}
