//! The host functions of `gnoblin.session`, `gnoblin.process`, `gnoblin.on`, `gnoblin.version` and
//! `gnoblin.runtime`, as `globalThis.__gnoblinHost.api`.
//!
//! Every function takes and returns plain strings (JSON) and numbers, so the logic here is testable without an
//! engine and the script's module (`data/js/gnoblin/session.ts`) owns every typed shape. A function that can fail
//! answers `{"error":{"code":..,"message":..}}`; the module turns it into the error the script sees.
//!
//! Nothing here runs a script. What happens outside the script's thread (a process ending, the compositor finishing
//! a call, a signal) is collected by [`JsHostApi::poll`] once a frame, and the shell hands the batch to the module,
//! which runs the listeners as queued calls under the usual budgets.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use rquickjs::{Ctx, Function, Object};
use serde_json::{Map, Value as Json, json};

use crate::bindings::SharedState;
use crate::host_process::{JsExit, JsProcessError, JsProcesses, JsSpawnOptions};
use crate::host_session::{JsSessionEvent, JsSessionLink, JsSessionVerb};

/// The `gnoblin.version` field when the build sidecar does not supply it.
const UNKNOWN: &str = "unknown";

/// What the shell holds for the host API: the session connection, if it has one, and the processes it started.
#[derive(Default)]
pub(crate) struct JsHostApi {
    pub(crate) session: Option<Box<dyn JsSessionLink>>,
    pub(crate) processes: JsProcesses,
}

fn refusal(code: &str, message: &str) -> String {
    json!({ "error": { "code": code, "message": message } }).to_string()
}

fn process_refusal(error: &JsProcessError) -> String {
    refusal(error.code, &error.message)
}

impl JsHostApi {
    /// Everything that happened outside the script since the last frame, as the events the module takes.
    pub(crate) fn poll(&mut self) -> Vec<Json> {
        let mut events = Vec::new();
        if let Some(session) = self.session.as_mut() {
            for event in session.poll() {
                events.push(match event {
                    JsSessionEvent::Finished { ticket, outcome } => match outcome {
                        Ok(value) => json!({ "kind": "op", "ticket": ticket, "ok": true, "value": value }),
                        Err(error) => json!({
                            "kind": "op",
                            "ticket": ticket,
                            "ok": false,
                            "error": { "code": error.code, "message": error.message },
                        }),
                    },
                    JsSessionEvent::Signal { name, object, fields } => {
                        json!({ "kind": "signal", "name": name, "object": object, "fields": Json::Object(fields) })
                    }
                    JsSessionEvent::Lost(message) => json!({ "kind": "lost", "message": message }),
                });
            }
        }
        for (id, exit) in self.processes.drain() {
            events.push(exit_event(id, &exit));
        }
        events
    }

    fn spawn(&mut self, argv: &str, options: &str) -> String {
        let argv: Vec<String> = match serde_json::from_str(argv) {
            Ok(argv) => argv,
            Err(error) => return refusal("invalid_argument", &format!("argv is not a list of strings: {error}")),
        };
        let options: JsSpawnOptions = match serde_json::from_str(options) {
            Ok(options) => options,
            Err(error) => return refusal("invalid_argument", &format!("the options are not valid: {error}")),
        };
        match self.processes.spawn(&argv, &options) {
            Ok(started) => json!({ "id": started.id, "pid": started.pid }).to_string(),
            Err(error) => process_refusal(&error),
        }
    }

    fn begin(&mut self, verb: &str) -> String {
        let Some(verb) = JsSessionVerb::named(verb) else {
            return refusal("invalid_argument", &format!("no session verb named {verb:?}"));
        };
        let Some(session) = self.session.as_mut() else {
            return refusal("unavailable", "the shell has no session connection");
        };
        match session.begin(verb) {
            Ok(ticket) => json!({ "ticket": ticket }).to_string(),
            Err(error) => refusal("unavailable", &error.0),
        }
    }

    fn status(&self) -> Option<String> {
        let status = self.session.as_ref()?.status();
        let mut record = Map::new();
        record.insert("lockAvailable".to_owned(), json!(status.lock_available));
        record.insert("revision".to_owned(), json!(status.revision));
        if status.lock_available
            && let Some(state) = status.lock_state
        {
            record.insert("lockState".to_owned(), json!(state));
        }
        Some(Json::Object(record).to_string())
    }
}

fn exit_event(id: u64, exit: &JsExit) -> Json {
    json!({
        "kind": "exit",
        "id": id,
        "exitCode": exit.exit_code,
        "signal": exit.signal,
        "timedOut": exit.timed_out,
        "stdout": exit.stdout,
        "stderr": exit.stderr,
        "stdoutTruncated": exit.stdout_truncated,
        "stderrTruncated": exit.stderr_truncated,
    })
}

/// Every schema signal, as `{"name": "namespace.object.signal", "fields": ["field", ..]}`, in catalog order.
///
/// The catalog is the checked-in schema the compositor and the Lua host are built from, so the names a script may
/// subscribe to are the ones the compositor can raise. It is parsed once per process.
fn signal_table() -> &'static str {
    static TABLE: LazyLock<String> = LazyLock::new(|| {
        let Ok(catalog) = gnoblin_js_types::stock_catalog() else {
            return "[]".to_owned();
        };
        let mut signals = Vec::new();
        for namespace in catalog.namespaces() {
            for object in catalog.objects(&namespace) {
                for signal in &object.signals {
                    let fields: Vec<&str> = signal.fields.iter().map(|field| field.name.as_str()).collect();
                    signals.push(json!({
                        "name": format!("{namespace}.{}.{}", object.name, signal.name),
                        "fields": fields,
                    }));
                }
            }
        }
        Json::Array(signals).to_string()
    });
    &TABLE
}

// ---- gnoblin.version ---------------------------------------------------------------------------------------------

/// The `[version]` values of a `version.ini` text. Other sections, blank lines and `#` or `;` comments are ignored.
fn version_values(text: &str) -> Vec<(String, String)> {
    let mut inside = false;
    let mut values = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            inside = section.trim() == "version";
        } else if inside && let Some((key, value)) = line.split_once('=') {
            values.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    values
}

/// A git remote with its credentials, query and fragment removed. A URL keeps its scheme, host, port and path; the
/// scp-like spelling has no scheme, so what precedes its first `@` goes.
fn public_remote(remote: &str) -> String {
    if remote.is_empty() {
        return UNKNOWN.to_owned();
    }
    let Some((scheme, rest)) = remote.split_once("://") else {
        return remote.split_once('@').map_or(remote, |(_, host)| host).to_owned();
    };
    let (authority, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let path = path.split(['?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}{path}")
}

/// Where the sidecar may be, best first: the override, beside the executable's prefix, then the data directories.
fn sidecar_paths(
    override_path: Option<&str>,
    executable: Option<&Path>,
    data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = override_path
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(prefix) = executable.and_then(Path::parent).and_then(Path::parent) {
        paths.push(prefix.join("share/gnoblin/version.ini"));
    }
    let dirs = data_dirs.filter(|dirs| !dirs.is_empty()).unwrap_or("/usr/local/share/:/usr/share/");
    paths.extend(
        dirs.split(':')
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(dir).join("gnoblin/version.ini")),
    );
    paths
}

/// The build identity as JSON: `gnoblin`, `gnome`, `mutter`, `js`, `api`, `gitRemote`, `gitSha`, `buildId`.
/// A field the sidecar does not give is `"unknown"`.
fn version_record(sidecar: &str) -> String {
    let values = version_values(sidecar);
    let field = |key: &str| {
        values
            .iter()
            .find(|(name, value)| name == key && !value.is_empty())
            .map_or_else(|| UNKNOWN.to_owned(), |(_, value)| value.clone())
    };
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    json!({
        "gnoblin": field("gnoblin"),
        "gnome": field("gnome"),
        "mutter": field("mutter"),
        "js": "quickjs-ng",
        "api": format!("{major}.{minor}.{patch}"),
        "gitRemote": public_remote(&values.iter().find(|(name, _)| name == "git_remote").map_or("", |(_, value)| value.as_str())),
        "gitSha": field("git_sha"),
        "buildId": field("build_id"),
    })
    .to_string()
}

fn read_version() -> String {
    let env = |name: &str| std::env::var(name).ok();
    let executable = std::env::current_exe().ok();
    let paths = sidecar_paths(
        env("GNOBLIN_VERSION_METADATA_FILE").as_deref(),
        executable.as_deref(),
        env("XDG_DATA_DIRS").as_deref(),
    );
    let text = paths
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    version_record(&text)
}

// ---- the host functions -----------------------------------------------------------------------------------------

/// Installs `host.api`. Each closure keeps a handle on the shell's state, which owns no script value.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let api = Object::new(ctx.clone())?;

    let shared = state.clone();
    api.set(
        "spawn",
        Function::new(ctx.clone(), move |argv: String, options: String| -> String {
            shared.borrow_mut().api.spawn(&argv, &options)
        })?,
    )?;

    let shared = state.clone();
    api.set(
        "kill",
        Function::new(ctx.clone(), move |id: f64, signal: String| -> Option<String> {
            let mut state = shared.borrow_mut();
            // An id is a small integer a script was handed; anything else names no process.
            let id = if id.is_finite() && id >= 0.0 { id as u64 } else { 0 };
            state.api.processes.kill(id, &signal).err().map(|error| process_refusal(&error))
        })?,
    )?;

    let shared = state.clone();
    api.set(
        "begin",
        Function::new(ctx.clone(), move |verb: String| -> String {
            shared.borrow_mut().api.begin(&verb)
        })?,
    )?;

    let shared = state.clone();
    api.set(
        "status",
        Function::new(ctx.clone(), move || -> Option<String> { shared.borrow().api.status() })?,
    )?;

    api.set("version", Function::new(ctx.clone(), read_version)?)?;
    api.set("signals", Function::new(ctx.clone(), || signal_table().to_owned())?)?;

    host.set("api", api)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{public_remote, sidecar_paths, signal_table, version_record};

    #[test]
    fn a_remote_loses_its_credentials_query_and_fragment() {
        assert_eq!(
            public_remote("https://user:token@git.example.org:8443/a/b.git?x=1#f"),
            "https://git.example.org:8443/a/b.git"
        );
        assert_eq!(public_remote("ssh://git@host/repo.git"), "ssh://host/repo.git");
        assert_eq!(public_remote("git@host:owner/repo.git"), "host:owner/repo.git");
        assert_eq!(public_remote(""), "unknown");
        assert_eq!(public_remote("/local/path"), "/local/path");
    }

    #[test]
    fn the_record_reads_only_the_version_section_and_marks_the_rest_unknown() {
        let record: serde_json::Value = serde_json::from_str(&version_record(
            "[other]\ngnoblin=0\n[version]\n# note\ngnoblin = 1.2.3\ngit_sha=abc\ngit_remote=https://u:p@h/r\n[more]\ngnome=49\n",
        ))
        .expect("the record is JSON");
        assert_eq!(record["gnoblin"], "1.2.3");
        assert_eq!(record["gitSha"], "abc");
        assert_eq!(record["gitRemote"], "https://h/r");
        assert_eq!(record["gnome"], "unknown", "a key of another section is not read");
        assert_eq!(record["mutter"], "unknown");
        assert_eq!(record["js"], "quickjs-ng");
        assert!(record["api"].as_str().is_some_and(|api| api.split('.').count() == 3));
    }

    #[test]
    fn the_sidecar_is_looked_for_at_the_override_then_the_prefix_then_the_data_dirs() {
        let paths = sidecar_paths(Some("/o/version.ini"), Some(Path::new("/usr/bin/gnoblin")), Some("/a:/b"));
        let paths: Vec<&str> = paths.iter().map(|path| path.to_str().unwrap_or("")).collect();
        assert_eq!(
            paths,
            [
                "/o/version.ini",
                "/usr/share/gnoblin/version.ini",
                "/a/gnoblin/version.ini",
                "/b/gnoblin/version.ini"
            ]
        );
    }

    #[test]
    fn the_signal_table_names_the_schemas_signals_with_their_fields() {
        let table: serde_json::Value = serde_json::from_str(signal_table()).expect("the table is JSON");
        let bell = table
            .as_array()
            .and_then(|all| all.iter().find(|entry| entry["name"] == "gnoblin.session.bell"))
            .expect("the session bell is declared");
        assert_eq!(bell["fields"], serde_json::json!(["count"]));
    }
}
