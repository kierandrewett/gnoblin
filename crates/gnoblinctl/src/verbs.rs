//! The verbs that are not object actions: `status`, `version`, `ping` and `config`.
//!
//! The object verbs are generated from the schema because an object's members are
//! the schema's to declare. These are different: each one asks the session a fixed
//! question, so they are written by hand and print the answer in the same two forms
//! as everything else, text for a person and `--json` for a script. Decision 5 of
//! the runtime surface plan keeps this set to what is built; a verb is added only
//! with the member that answers it.
//!
//! `config validate` is the one verb that never opens a connection: it checks a file on
//! disk. An `init.ts` or `init.js` goes to the in-process check `gnoblinctl check` runs (`js.rs`, routed
//! in `lib.rs`); an `init.lua` is evaluated by the runtime's own host against stock schemas, so a
//! file that validates here is one the compositor accepts; a JSON document goes through the
//! configuration layer's parser. None needs a session.

use std::io::Write;
use std::path::{Path, PathBuf};

use gnoblin_client::wire::NoticeKind;
use gnoblin_client::{Client, ClientError, Event, SessionLock, Transport};
use gnoblin_config::{SearchPaths, document::Config};
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
    /// Take the session lock and wait until the cover is up.
    Lock,
    /// The Gnoblin version and the IPC version this connection speaks.
    Version,
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
        Verb::Lock => lock(client, json, out),
        Verb::Version => version(client, json, out),
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
    // The lock is the tree's `lock` node. A compositor with none, or one that does not let this connection
    // observe it, reports the lock as unavailable: never as unlocked.
    let lock = match client.watch_lock() {
        Ok(state) => Some(state),
        Err(ClientError::Refused { code, .. }) if code == "not-found" || code == "not-owner" => None,
        Err(error) => return Err(Failure::Client(error)),
    };
    let report = StatusReport::new(&snapshot, lock);
    let result = if json {
        writeln!(out, "{}", report.json())
    } else {
        write!(out, "{}", report.text())
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// How long `lock` waits for the cover before it reports a failure.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// `lock`: the `lock` call on the tree's lock node, then a read until the node says `locked`. The call has no answer of
/// its own, so a lock that falls back to `not-locked`, or never confirms, is a failure: a script that locks and then acts
/// must not take a queued request for a protected screen. A session that is already locked is `ok`.
fn lock<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let unconfirmed = |message: &str| Failure::Refusal {
        code: "not-locked".to_owned(),
        message: message.to_owned(),
    };
    let state = client.watch_lock().map_err(Failure::Client)?;
    if state != SessionLock::Locked {
        if state == SessionLock::NotLocked {
            client.lock_session().map_err(Failure::Client)?;
        }
        // A transport with no read deadline (the in-process one) is read to its end instead.
        let _ = client.set_read_timeout(Some(LOCK_WAIT));
        loop {
            match client.next_event() {
                Ok(Event::Lock(SessionLock::Locked)) => break,
                Ok(Event::Lock(SessionLock::NotLocked)) => {
                    return Err(unconfirmed("the compositor did not take the lock"));
                }
                Ok(_) => {}
                Err(ClientError::Idle | ClientError::Disconnected) => {
                    return Err(unconfirmed("the compositor did not confirm the lock"));
                }
                Err(error) => return Err(Failure::Client(error)),
            }
        }
    }
    let result = if json {
        writeln!(out, "{}", json!({"lock_state": "locked"}))
    } else {
        writeln!(out, "locked")
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// What `status` reports. The lock is absent from a compositor that publishes no
/// lock node; that is reported as unavailable rather than as unlocked.
#[derive(Debug, PartialEq, Eq)]
struct StatusReport {
    revision: u64,
    lock_available: bool,
    lock_state: Option<&'static str>,
}

impl StatusReport {
    /// The report for `snapshot` and the lock node's state, `None` when the tree has none. `locked` is the
    /// whole embargo: a lock that is still `pending` already hides the desktop.
    fn new(snapshot: &Snapshot, lock: Option<SessionLock>) -> StatusReport {
        StatusReport {
            revision: snapshot.revision().get(),
            lock_available: lock.is_some(),
            lock_state: lock.map(|state| if state.covers() { "locked" } else { "unlocked" }),
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
///
/// An accepted document can still contain settings the running session does not apply: ones a
/// process reads only when it starts, and ones nothing reads. They are listed after the revision,
/// each on its own line, so a reload never passes silently for a change that did not happen.
fn config_reload<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let reloaded = client.reload_config().map_err(Failure::Client)?;
    let of_kind = |kind| {
        reloaded
            .notices
            .iter()
            .filter(|notice| notice.kind == kind)
            .map(|notice| notice.text.as_str())
            .collect::<Vec<_>>()
    };
    let result = if json {
        writeln!(
            out,
            "{}",
            json!({
                "revision": reloaded.revision,
                "restart_required": of_kind(NoticeKind::RestartRequired),
                "no_effect": of_kind(NoticeKind::NoEffect),
            })
        )
    } else {
        writeln!(out, "reloaded: revision {}", reloaded.revision).and_then(|()| {
            reloaded
                .notices
                .iter()
                .try_for_each(|notice| writeln!(out, "not applied: {}", notice.text))
        })
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

/// `config validate FILE`: check a configuration document the way the runtime would.
/// Deprecated spellings are reported as warnings; they do not fail it.
pub(crate) fn validate(path: &Path, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let warnings = if path.extension().is_some_and(|extension| extension == "lua") {
        validate_lua(path)?
    } else {
        validate_json(path)?
    };
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

/// Evaluate an `init.lua` with the host the runtime uses, so a refused file is refused for the
/// same reasons the compositor would give. Validation runs against an empty object graph; see
/// `gnoblin_lua::check` for what that does not check.
fn validate_lua(path: &Path) -> Result<Vec<String>, Failure> {
    // `run_config` treats an absent file as "no configuration" (the runtime then starts with
    // defaults). That is right for the compositor, but the user named this file, so its
    // absence is a refusal here.
    if let Err(error) = std::fs::metadata(path) {
        return Err(Failure::Refusal {
            code: "io".to_owned(),
            message: format!("cannot read {}: {error}", path.display()),
        });
    }
    let paths = SearchPaths {
        home: None,
        config_home: None,
        override_path: Some(path.to_path_buf()),
    };
    let check = gnoblin_lua::check::check(&paths).map_err(|message| Failure::Refusal {
        code: "validation-unavailable".to_owned(),
        message,
    })?;
    if !check.problems.is_empty() {
        return Err(Failure::Refusal {
            code: "invalid-config".to_owned(),
            message: check.problems.join("\n"),
        });
    }
    Ok(check.warnings)
}

/// Parse a JSON configuration document with the configuration layer's own parser.
fn validate_json(path: &Path) -> Result<Vec<String>, Failure> {
    let bytes = std::fs::read(path).map_err(|error| Failure::Refusal {
        code: "io".to_owned(),
        message: format!("cannot read {}: {error}", path.display()),
    })?;
    let config = Config::from_json_bytes(&bytes).map_err(|message| Failure::Refusal {
        code: "invalid-config".to_owned(),
        message,
    })?;
    Ok(config.deprecations().iter().map(ToString::to_string).collect())
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
    fn a_committed_document_must_be_json() {
        assert_eq!(parse_committed(br#"{"a": 1}"#).expect("JSON parses"), json!({"a": 1}));
        let Err(Failure::Refusal { code, .. }) = parse_committed(b"not json") else {
            panic!("a non-JSON document is refused");
        };
        assert_eq!(code, "invalid-document");
    }

    /// Write `bytes` to a file unique to this test, named with `extension`, and run `validate` on it.
    fn validate_bytes(name: &str, extension: &str, bytes: &[u8]) -> (Result<u8, Failure>, String) {
        let path = std::env::temp_dir().join(format!("gnoblinctl-validate-{}-{name}.{extension}", std::process::id()));
        std::fs::write(&path, bytes).expect("write the test document");
        let mut out = Vec::new();
        let result = validate(&path, false, &mut out);
        let _ = std::fs::remove_file(&path);
        (result, String::from_utf8(out).expect("UTF-8 output"))
    }

    #[test]
    fn a_valid_document_validates_with_no_warnings() {
        let (result, out) = validate_bytes("valid", "json", b"{}");
        assert_eq!(result.expect("an empty document is valid"), 0);
        assert_eq!(out, "valid\n");
    }

    #[test]
    fn a_deprecated_spelling_warns_but_still_validates() {
        let (result, out) = validate_bytes(
            "deprecated",
            "json",
            br#"{"window-management": {"focus-mode": "sloppy"}}"#,
        );
        assert_eq!(result.expect("a deprecated spelling is not a failure"), 0);
        assert!(out.starts_with("valid\n"), "{out}");
        assert!(out.contains("warning: window-management.focus-mode"), "{out}");
    }

    #[test]
    fn a_document_that_does_not_parse_is_refused() {
        let (result, _) = validate_bytes("invalid", "json", b"not json");
        let Err(Failure::Refusal { code, .. }) = result else {
            panic!("an unparseable document is refused");
        };
        assert_eq!(code, "invalid-config");
    }

    /// A good init.lua, in the form the repo's default configuration uses, validates.
    #[test]
    fn a_good_init_lua_validates_as_the_runtime_loads_it() {
        let (result, out) = validate_bytes(
            "lua-good",
            "lua",
            br#"gnoblin.configure { ["window-management"] = { ["focus-mode"] = "click" } }"#,
        );
        assert_eq!(result.expect("a good init.lua validates"), 0);
        assert_eq!(out, "valid\n");
    }

    /// An init.lua the Lua host cannot evaluate is refused with the runtime's own problem.
    #[test]
    fn a_bad_init_lua_is_refused_with_the_runtime_error() {
        let (result, _) = validate_bytes("lua-bad", "lua", b"gnoblin.configure {");
        let Err(Failure::Refusal { code, message }) = result else {
            panic!("an init.lua that does not evaluate is refused");
        };
        assert_eq!(code, "invalid-config");
        assert!(!message.is_empty(), "the refusal names the problem");
    }

    /// `validate` names the file the user gave; a missing one is refused, not reported valid.
    /// (The runtime starts with defaults when `GNOBLIN_CONFIG` names no file; validating a
    /// path the user typed must not do the same.)
    #[test]
    fn a_missing_lua_file_is_refused_not_reported_valid() {
        let path = std::env::temp_dir().join(format!("gnoblinctl-validate-{}-absent.lua", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut out = Vec::new();
        let Err(Failure::Refusal { code, message }) = validate(&path, false, &mut out) else {
            panic!("a missing init.lua is refused");
        };
        assert_eq!(code, "io");
        assert!(message.contains("cannot read"), "{message}");
        assert!(out.is_empty(), "nothing is printed as valid");
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
