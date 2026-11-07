//! The verbs that are not object actions: `status`, `version`, `ping`,
//! `capabilities` and `config`.
//!
//! The object verbs are generated from the schema because an object's members are
//! the schema's to declare. These are different: each one asks the session a fixed
//! question, so they are written by hand and print the answer in the same two forms
//! as everything else, text for a person and `--json` for a script. Decision 5 of
//! the runtime surface plan keeps this set to what is built; a verb is added only
//! with the member that answers it.
//!
//! `config validate` is the one verb that never opens a connection: it checks a
//! file on disk with the same parser the runtime uses, so it works with no session.

use std::io::Write;
use std::path::{Path, PathBuf};

use gnoblin_client::{Capabilities, Capability, Client, Transport};
use gnoblin_config::document::Config;
use gnoblin_model::{Filter, ObjectView, Snapshot, TypeKey, Value};
use serde_json::{Value as Json, json};

use crate::commands::Failure;

/// One hand-written verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verb {
    /// Exit 0 when the compositor answers the handshake.
    Ping,
    /// The session state, the revision and the lock.
    Status,
    /// The Gnoblin version and the IPC version this connection speaks.
    Version,
    /// Every capability, and whether this connection holds it.
    Capabilities,
    /// The configuration document the compositor last accepted.
    ConfigGet,
    /// Ask the runtime to re-read the configuration in place.
    ConfigReload,
    /// Check a configuration document on disk, without connecting.
    ConfigValidate { path: PathBuf },
}

/// Run a verb that needs the compositor.
///
/// `ConfigValidate` is also accepted here so the match is total, and it runs the
/// same local check as [`validate`]; the binary routes it past the connection.
pub(crate) fn execute<T: Transport>(
    client: &mut Client<T>,
    verb: &Verb,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let outcome = match verb {
        Verb::Ping => ping(json, out),
        Verb::Status => status(client, json, out),
        Verb::Version => version(client, json, out),
        Verb::Capabilities => capabilities(client, json, out),
        Verb::ConfigGet => config_get(client, json, out),
        Verb::ConfigReload => config_reload(client, json, out),
        Verb::ConfigValidate { path } => validate(path, json, out),
    };
    match outcome {
        Ok(code) => code,
        Err(failure) => failure.report(err, json),
    }
}

/// `ping`: reaching this point means the handshake succeeded.
fn ping(json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let result = if json {
        writeln!(out, "{}", json!({"ping": "pong"}))
    } else {
        writeln!(out, "pong")
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// `status`: the session's state, read from one snapshot.
fn status<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let snapshot = client.subscribe(Filter::All).map_err(Failure::Client)?;
    let report = StatusReport::from_snapshot(&snapshot);
    let result = if json {
        writeln!(out, "{}", report.json())
    } else {
        write!(out, "{}", report.text())
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// What `status` reports. The lock is absent from a compositor that publishes no
/// lock object; that is reported as unavailable rather than as unlocked.
#[derive(Debug, PartialEq, Eq)]
struct StatusReport {
    revision: u64,
    lock_available: bool,
    lock_state: Option<&'static str>,
}

impl StatusReport {
    fn from_snapshot(snapshot: &Snapshot) -> StatusReport {
        let lock = find(snapshot, "lock");
        let lock_state = lock.and_then(|view| match view.property("locked") {
            Some(Value::Bool(true)) => Some("locked"),
            Some(Value::Bool(false)) => Some("unlocked"),
            _ => None,
        });
        StatusReport {
            revision: snapshot.revision().get(),
            lock_available: lock.is_some(),
            lock_state,
        }
    }

    fn json(&self) -> Json {
        json!({
            "state": "running",
            "revision": self.revision,
            "lock_available": self.lock_available,
            "lock_state": self.lock_state,
        })
    }

    fn text(&self) -> String {
        let lock = match (self.lock_available, self.lock_state) {
            (false, _) => "unavailable".to_owned(),
            (true, Some(state)) => state.to_owned(),
            (true, None) => "unknown".to_owned(),
        };
        format!("state     running\nrevision  {}\nlock      {lock}\n", self.revision)
    }
}

/// `version`: the Gnoblin build and the IPC version this connection speaks.
fn version<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let snapshot = client.subscribe(Filter::All).map_err(Failure::Client)?;
    let session = find(&snapshot, "session").ok_or_else(|| Failure::Refusal {
        code: "no-session".to_owned(),
        message: "the compositor publishes no session object, so it has no version to report".to_owned(),
    })?;
    let gnoblin = match session.property("version") {
        Some(Value::Text(version)) => version.clone(),
        _ => {
            return Err(Failure::Refusal {
                code: "protocol".to_owned(),
                message: "the session object has no text `version` property".to_owned(),
            });
        }
    };
    let ipc = client.version().to_string();
    let result = if json {
        writeln!(out, "{}", json!({"gnoblin": gnoblin, "ipc": ipc}))
    } else {
        writeln!(out, "gnoblin {gnoblin}\nipc {ipc}")
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// `capabilities`: every capability, granted or not, in the plan's order.
fn capabilities<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let granted = client.capabilities();
    let rows = capability_rows(granted);
    let result = if json {
        let list: Vec<Json> = rows
            .iter()
            .map(|(name, held)| json!({"name": name, "granted": held}))
            .collect();
        writeln!(out, "{}", Json::Array(list))
    } else {
        write!(out, "{}", capability_table(&rows))
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// Each capability's name with whether `granted` holds it.
fn capability_rows(granted: Capabilities) -> Vec<(&'static str, bool)> {
    Capability::ALL
        .into_iter()
        .map(|capability| (capability.name(), granted.contains(capability)))
        .collect()
}

/// The text table: one capability per line, `yes` or `no` for whether it is held.
fn capability_table(rows: &[(&'static str, bool)]) -> String {
    let width = rows.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let mut text = format!("{:<width$}  granted\n", "capability");
    for (name, held) in rows {
        let answer = if *held { "yes" } else { "no" };
        text.push_str(&format!("{name:<width$}  {answer}\n"));
    }
    text
}

/// `config get`: the document the compositor last accepted, with its revision.
///
/// Nothing committed is a normal state (a fresh session), so it is reported as
/// `null` in JSON and as a sentence in text, with exit 0.
fn config_get<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let current = client.configuration().map_err(Failure::Client)?;
    let result = match current {
        None => {
            if json {
                writeln!(out, "null")
            } else {
                writeln!(out, "no configuration has been committed")
            }
        }
        Some((revision, bytes)) => {
            let document = parse_committed(&bytes)?;
            if json {
                writeln!(out, "{}", json!({"revision": revision, "document": document}))
            } else {
                let pretty = serde_json::to_string_pretty(&document).map_err(|error| Failure::Refusal {
                    code: "invalid-document".to_owned(),
                    message: error.to_string(),
                })?;
                writeln!(out, "revision {revision}\n{pretty}")
            }
        }
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// `config reload`: ask the runtime to re-read the document in place. A document it
/// refuses leaves the last accepted one in force, and the refusal comes back as the
/// compositor's `CONFIG_REJECTED` error with the diagnostic, so it reaches
/// [`Failure::report`] like any other refused request.
fn config_reload<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let revision = client.reload_config().map_err(Failure::Client)?;
    let result = if json {
        writeln!(out, "{}", json!({"revision": revision}))
    } else {
        writeln!(out, "reloaded: revision {revision}")
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// The committed document as JSON. The compositor stores it opaquely, but the
/// runtime writes JSON, so anything else means a runtime this build cannot read.
fn parse_committed(bytes: &[u8]) -> Result<Json, Failure> {
    serde_json::from_slice(bytes).map_err(|error| Failure::Refusal {
        code: "invalid-document".to_owned(),
        message: format!("the committed configuration is not JSON: {error}"),
    })
}

/// `config validate FILE`: parse a configuration document with the runtime's own
/// parser. Deprecated spellings are reported as warnings; they do not fail it.
pub(crate) fn validate(path: &Path, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let bytes = std::fs::read(path).map_err(|error| Failure::Refusal {
        code: "io".to_owned(),
        message: format!("cannot read {}: {error}", path.display()),
    })?;
    let config = Config::from_json_bytes(&bytes).map_err(|message| Failure::Refusal {
        code: "invalid-config".to_owned(),
        message,
    })?;
    let warnings: Vec<String> = config.deprecations().iter().map(ToString::to_string).collect();
    let result = if json {
        writeln!(out, "{}", json!({"valid": true, "warnings": warnings}))
    } else {
        let mut text = String::from("valid\n");
        for warning in &warnings {
            text.push_str(&format!("warning: {warning}\n"));
        }
        write!(out, "{text}")
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// The first live object of a schema type, by its command name.
fn find<'a>(snapshot: &'a Snapshot, name: &str) -> Option<&'a ObjectView> {
    let key = TypeKey::gnoblin(name);
    snapshot.objects().iter().find(|view| view.ty() == &key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_table_names_every_capability_and_its_grant() {
        let rows = capability_rows(Capabilities::NONE);
        assert_eq!(rows.len(), Capability::ALL.len());
        assert!(rows.iter().all(|(_, held)| !held), "nothing is granted by NONE");
        let table = capability_table(&rows);
        assert!(table.starts_with("capability"), "{table}");
        let width = "offer-request".len();
        assert!(table.contains(&format!("{:<width$}  no\n", "offer-request")), "{table}");
        assert!(table.contains(&format!("{:<width$}  no\n", "portal-input")), "{table}");
    }

    #[test]
    fn a_granted_capability_is_marked_yes() {
        let granted = Capabilities::of(&[Capability::CommitConfig]);
        let table = capability_table(&capability_rows(granted));
        let width = "commit-config".len();
        assert!(
            table.contains(&format!("{:<width$}  yes\n", "commit-config")),
            "{table}"
        );
        assert!(table.contains(&format!("{:<width$}  no\n", "offer-request")), "{table}");
    }

    #[test]
    fn a_committed_document_must_be_json() {
        assert_eq!(parse_committed(br#"{"a": 1}"#).expect("JSON parses"), json!({"a": 1}));
        let Err(Failure::Refusal { code, .. }) = parse_committed(b"not json") else {
            panic!("a non-JSON document is refused");
        };
        assert_eq!(code, "invalid-document");
    }

    /// Write `bytes` to a file unique to this test and run `validate` on it.
    fn validate_bytes(name: &str, bytes: &[u8]) -> (Result<u8, Failure>, String) {
        let path = std::env::temp_dir().join(format!("gnoblinctl-validate-{}-{name}.json", std::process::id()));
        std::fs::write(&path, bytes).expect("write the test document");
        let mut out = Vec::new();
        let result = validate(&path, false, &mut out);
        let _ = std::fs::remove_file(&path);
        (result, String::from_utf8(out).expect("UTF-8 output"))
    }

    #[test]
    fn a_valid_document_validates_with_no_warnings() {
        let (result, out) = validate_bytes("valid", b"{}");
        assert_eq!(result.expect("an empty document is valid"), 0);
        assert_eq!(out, "valid\n");
    }

    #[test]
    fn a_deprecated_spelling_warns_but_still_validates() {
        let (result, out) = validate_bytes("deprecated", br#"{"window-management": {"focus-mode": "sloppy"}}"#);
        assert_eq!(result.expect("a deprecated spelling is not a failure"), 0);
        assert!(out.starts_with("valid\n"), "{out}");
        assert!(out.contains("warning: window-management.focus-mode"), "{out}");
    }

    #[test]
    fn a_document_that_does_not_parse_is_refused() {
        let (result, _) = validate_bytes("invalid", b"not json");
        let Err(Failure::Refusal { code, .. }) = result else {
            panic!("an unparseable document is refused");
        };
        assert_eq!(code, "invalid-config");
    }

    #[test]
    fn an_absent_lock_is_unavailable_not_unlocked() {
        let report = StatusReport {
            revision: 7,
            lock_available: false,
            lock_state: None,
        };
        assert_eq!(
            report.json(),
            json!({"state": "running", "revision": 7, "lock_available": false, "lock_state": null})
        );
        assert!(report.text().contains("lock      unavailable"), "{}", report.text());
    }
}
