//! The runtime role: the Lua host as its own process.
//!
//! Design/rust-compositor.md, D1: "separate process" — the compositor runs no Lua, so a
//! fault in a script is a restart of this process rather than the end of the session.
//! This module is the role `gnoblin --role runtime` plays:
//!
//! 1. Adopt the privileged connection the guardian passed, and connect. It is the owner's
//!    connection, so the runtime may commit configuration; the runtime is the only process that
//!    knows what a configuration document means, so it is the only one that does.
//! 2. Locate, evaluate and validate `init.lua` through the configuration layer.
//! 3. Commit the validated document, and report the revision it was given to the guardian.
//! 4. Report readiness, then serve: pump the host's handlers on the caller's clock until
//!    the connection ends or the guardian asks it to stop.
//!
//! # Why a rejected document is not committed
//!
//! The compositor keeps the last accepted document for as long as it runs, so a runtime
//! that refuses its files leaves the session on the settings it already had. The
//! alternative — committing a document that failed validation — would replace a working
//! configuration with a broken one, which is the failure mode "a rejected configuration
//! loses the last-good revision" describes.
//!
//! # Failure modes first
//!
//! 1. **The guardian did not pass a connection, or passed one this process cannot use.**
//!    The role refuses at once with the variable named, so the guardian's log says what is
//!    wrong instead of the child hanging:
//!    `tests::the_role_refuses_without_a_privileged_connection`.
//! 2. **A configuration document fails validation.** The problems are reported, nothing is
//!    committed, and the process exits non-zero, so the guardian's restart ladder and its
//!    circuit breaker bound the attempts:
//!    `tests::a_rejected_document_is_reported_and_not_committed`.
//! 3. **The compositor refuses the commit.** The revision never arrives and the role exits
//!    non-zero with the compositor's code, while the last accepted document stays in force:
//!    `tests::a_refused_commit_ends_the_role`.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::shell_role::{ShellSlot, connect_session_shell, installed_stock_dir};
use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{
    Client, ClientError, Environment, Inherited, Reporter, Transport, inherited_stream, privilege, report,
};
use gnoblin_config::{Config, Problem, SearchPaths, Value, enabled_autostart, salvage};
use gnoblin_proto::Version;
use gnoblin_types::Tick;

use crate::error::{HostError, ScriptError};
use crate::host::{Host, ReloadOutcome, Reloaded, RunReport};
use gnoblin_lua_sandbox::Limits;

use crate::log;

/// How long one pump waits for a frame before it looks at its own signals.
///
/// The pump has to come back so the role can notice the guardian's `SIGTERM` and exit
/// inside the grace period. It is short because the only cost of a wake-up that finds
/// nothing is one read that timed out.
pub const PUMP_TIMEOUT: Duration = Duration::from_millis(100);

/// A test-only hook: the runtime exits at once, on every start.
///
/// The guardian's circuit breaker exists for a child that crashes as fast as it starts, and a
/// real-process test needs such a child without building one. Setting this variable makes the
/// role fail immediately, which is what `tests/guardian.rs` in the session binary asks for. It
/// is read once, here, and nothing else in the workspace looks at it.
pub const TEST_CRASH_VARIABLE: &str = "GNOBLIN_TEST_RUNTIME_CRASH";

/// How many handlers one run of the pump may execute before the loop yields.
///
/// A script that answers its own signals could otherwise keep one pump busy for ever; the
/// pump returns and the loop is entered again, so the process still notices a shutdown.
const PUMP_HANDLERS: usize = 256;

/// Why the runtime role could not run.
#[derive(Debug)]
pub enum RoleError {
    /// The privileged connection the guardian passes was missing or unusable.
    Connection(Inherited),
    /// The handshake with the compositor failed.
    Connect(String),
    /// The configuration could not be located or evaluated.
    Config(String),
    /// The configuration document did not validate, and the problems are here.
    Invalid(String),
    /// The compositor refused the commit.
    Commit(String),
    /// The connection failed while the host was serving.
    Serving(String),
}

impl std::fmt::Display for RoleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoleError::Connection(error) => write!(formatter, "the runtime needs its privileged connection: {error}"),
            RoleError::Connect(reason)
            | RoleError::Config(reason)
            | RoleError::Invalid(reason)
            | RoleError::Commit(reason)
            | RoleError::Serving(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for RoleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RoleError::Connection(error) => Some(error),
            _ => None,
        }
    }
}

/// How the role runs, for a caller that is not the process itself.
///
/// `env` and `paths` are passed in so a test drives the shipped role over a socketpair and
/// a configuration directory of its own, without a process-global environment.
pub struct RoleRunner {
    /// The environment to read the inherited descriptors from.
    pub env: Environment,
    /// Where the configuration lives.
    pub paths: SearchPaths,
    /// The budgets the sandbox runs under.
    pub limits: Limits,
}

impl RoleRunner {
    /// The role as the process finds it: the real environment, the real search path and
    /// the default sandbox budgets.
    pub fn from_environment() -> RoleRunner {
        RoleRunner {
            env: Box::new(|name| std::env::var(name).ok()),
            paths: SearchPaths::from_env(),
            limits: Limits::default(),
        }
    }

    /// Run the role until the session ends.
    ///
    /// `stop` is asked between pumps: the guardian's `SIGTERM` sets a flag rather than
    /// interrupting a read, so the role leaves its handlers and its connection in a state
    /// the compositor can drop cleanly.
    ///
    /// # Errors
    ///
    /// [`RoleError`] for each documented failure mode.
    pub fn run(self, stop: impl Fn() -> bool) -> Result<(), RoleError> {
        if (self.env)(TEST_CRASH_VARIABLE).is_some() {
            return Err(RoleError::Serving(format!(
                "{TEST_CRASH_VARIABLE} is set, so the runtime exits at once"
            )));
        }
        let mut reporter = Reporter::new(&self.env);
        let stream = inherited_stream(privilege::FD_VARIABLE, &self.env)
            .map_err(RoleError::Connection)?
            .ok_or(RoleError::Connection(Inherited::Absent))?;
        let transport = UnixTransport::from_stream(stream).map_err(|error| {
            RoleError::Connection(Inherited::NotASocket {
                number: -1,
                reason: error.to_string(),
            })
        })?;
        // The deadline is on from the first read: every wait this role does after the
        // handshake is a wait for a frame that may not come, and a read that times out is
        // "nothing yet" rather than a failure.
        let _ = transport.read_timeout(Some(PUMP_TIMEOUT));
        let client = Client::connect(transport, version()).map_err(|error| RoleError::Connect(error.to_string()))?;
        let mut host = Host::new(client, self.limits).map_err(|error| RoleError::Config(error.to_string()))?;

        let loaded = host
            .run_config(&self.paths)
            .map_err(|error| RoleError::Config(error.to_string()))?;
        let Accepted { revision, .. } = commit(&mut host, &loaded, &mut reporter).map_err(|failed| failed.error)?;
        log::info!(area = "runtime", path = %loaded.path.path().display(),
            revision,
            "configuration committed"
        );
        // Readiness is reported after the commit, because a running host with no
        // configuration is not usable: the guardian waits for this line before it starts
        // anything that reads the document.
        reporter.ready(&[("GNOBLIN_CONFIG_REVISION".to_owned(), revision.to_string())]);

        let installed = installed_stock_dir(&self.env);
        let user_dir = loaded.path.path().parent().map(Path::to_path_buf);
        let mut shell = ShellSlot::from_env(&self.env, &installed, |dir| {
            connect_session_shell(&self.env, dir, user_dir.as_deref())
        });
        serve(&mut host, &stop, &self.paths, &mut reporter, &mut shell)
    }
}

/// Why a commit did not go through.
struct Failed {
    error: RoleError,
    /// Whether the role can keep serving: the document was refused (by the autostart check or
    /// by the compositor), which leaves the last accepted one in force. A lost connection is not.
    refusal: bool,
}

/// What the compositor accepted from a commit.
struct Accepted {
    /// The revision now in force.
    revision: u64,
    /// The problems of the sections the commit dropped.
    warnings: Vec<String>,
    /// The settings the compositor accepted without applying them, one sentence each.
    not_applied: Vec<String>,
}

/// Commit the validated document and tell the guardian the revision, returning the revision,
/// the warnings of the sections that were dropped and the settings the compositor did not apply.
///
/// The enabled autostart entries go out as their own lines before `config-accepted`, so the
/// guardian has them by the time it learns the revision was accepted. A rejected revision
/// sends none, so the guardian keeps running what the last accepted revision started.
fn commit<T: Transport + 'static>(
    host: &mut Host<T>,
    report: &RunReport,
    reporter: &mut Reporter,
) -> Result<Accepted, Failed> {
    // A configuration that fails validation is not all-or-nothing at login: the sections that
    // validate are kept, and the dropped ones are logged as `ignored` once the revision is
    // accepted. Only a document with nothing left to keep is refused whole.
    let (document, autostart, dropped) = match &report.config {
        Ok(config) => match enabled_autostart(config) {
            Ok(entries) => (
                report.document.to_json().to_string().into_bytes(),
                owned_autostart(entries),
                Vec::new(),
            ),
            Err(problem) => {
                return Err(Failed {
                    error: refuse(report, reporter, &problem),
                    refusal: true,
                });
            }
        },
        Err(problems) => match keep_what_validates(report.document.clone()) {
            Some((kept, dropped)) => match enabled_autostart(&kept) {
                Ok(entries) => (
                    kept.document().to_json().to_string().into_bytes(),
                    owned_autostart(entries),
                    dropped,
                ),
                Err(problem) => {
                    return Err(Failed {
                        error: refuse(report, reporter, &problem),
                        refusal: true,
                    });
                }
            },
            None => {
                return Err(Failed {
                    error: refuse(report, reporter, &problems.to_string()),
                    refusal: true,
                });
            }
        },
    };
    // A refused revision logs its script errors inside the refusal reason, so they are logged
    // here only when the revision goes on.
    report.log_errors();
    let committed = match host.client_mut().commit_config_reporting(&document) {
        Ok(committed) => committed,
        Err(error) => {
            reporter.write(&report::config_rejected(0, &error.to_string()));
            return Err(Failed {
                refusal: matches!(error, ClientError::Refused { .. }),
                error: RoleError::Commit(error.to_string()),
            });
        }
    };
    let revision = committed.revision;
    for problem in &dropped {
        log::warn!(area = "config", "ignored {problem}");
    }
    for notice in &committed.notices {
        log::warn!(area = "config", "not applied: {}", notice.text);
    }
    for (name, command) in &autostart {
        reporter.write(&report::autostart(revision, name, command));
    }
    reporter.write(&report::config_accepted(revision));
    Ok(Accepted {
        revision,
        warnings: dropped.iter().map(ToString::to_string).collect(),
        not_applied: committed.notices.into_iter().map(|notice| notice.text).collect(),
    })
}

/// The document login keeps when the whole file fails validation: the sections that validate,
/// and the problems of the sections that were dropped.
///
/// `None` when no section survives. A document with nothing left would replace the last
/// accepted settings with the empty configuration, so the caller refuses the revision instead.
fn keep_what_validates(document: Value) -> Option<(Config, Vec<Problem>)> {
    let salvaged = salvage(document);
    if salvaged.config.is_empty() {
        return None;
    }
    let dropped = salvaged
        .report
        .map(|report| report.problems().to_vec())
        .unwrap_or_default();
    Some((salvaged.config, dropped))
}

/// The autostart entries as owned values, so they outlive the configuration they came from.
fn owned_autostart(entries: Vec<(&str, &[String])>) -> Vec<(String, Vec<String>)> {
    entries
        .into_iter()
        .map(|(name, command)| (name.to_owned(), command.to_vec()))
        .collect()
}

/// Refuse a configuration revision: log why, tell the guardian, and return the error.
fn refuse(report: &RunReport, reporter: &mut Reporter, problems: &str) -> RoleError {
    let mut parts: Vec<String> = report.errors.iter().map(ScriptError::stop_notice).collect();
    parts.push(problems.to_owned());
    let reason = format!(
        "the configuration in {} was not committed, and the last accepted one stays in force: {}",
        report.path.path().display(),
        parts.join("; ")
    );
    // The last accepted document stays in force, so the log is where a refused revision's
    // reason survives.
    log::error!(area = "runtime", "{reason}");
    reporter.write(&report::config_rejected(0, &reason));
    RoleError::Invalid(reason)
}

/// Serve the handlers until the guardian asks the role to stop or the connection ends.
///
/// A reload the compositor asked for is acted on before the next pump, so no handler runs
/// against a document that is about to be replaced.
fn serve<T: Transport + 'static>(
    host: &mut Host<T>,
    stop: &impl Fn() -> bool,
    paths: &SearchPaths,
    reporter: &mut Reporter,
    shell: &mut ShellSlot,
) -> Result<(), RoleError> {
    let start = Instant::now();
    while !stop() {
        if host.take_reload_request() {
            reload(host, paths, reporter)?;
        }
        if let Err(ended) = shell.tick(start.elapsed().as_secs_f64()) {
            log::error!(area = "runtime", "{ended}");
        }
        let now = Tick::new(u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX));
        match host.pump(now) {
            Ok(report) => {
                for error in &report.errors {
                    log::warn!(area = "runtime", %error, "handler failed");
                }
                if report.handlers >= PUMP_HANDLERS {
                    // A handler that wakes another handler could otherwise spin inside one
                    // pump; the next turn is the same work with a chance to notice a stop.
                    continue;
                }
            }
            Err(HostError::Client(ClientError::Idle)) => {}
            Err(HostError::Client(ClientError::Disconnected)) => return Ok(()),
            Err(error) => return Err(RoleError::Serving(error.to_string())),
        }
    }
    Ok(())
}

/// Run the configuration again and commit it, or refuse it and keep the last accepted one.
///
/// A refusal is reported to the compositor, which answers the caller of `config reload`. The
/// role keeps serving either way: a bad edit must not take down the runtime that holds the
/// last good document.
fn reload<T: Transport + 'static>(
    host: &mut Host<T>,
    paths: &SearchPaths,
    reporter: &mut Reporter,
) -> Result<(), RoleError> {
    // A reload that cannot even run, such as the file having been removed, is still a
    // refusal: the compositor is waiting on an answer, and the runtime keeps its last good document.
    let outcome = match host.reload_config(paths) {
        Ok(outcome) => outcome,
        Err(error) => ReloadOutcome::Rejected(format!(
            "the reload could not run, and the last accepted configuration stays in force: {error}"
        )),
    };
    match outcome {
        ReloadOutcome::Accepted(loaded) => match commit(host, &loaded, reporter) {
            Ok(Accepted {
                revision,
                warnings,
                not_applied,
            }) => {
                host.keep_reload();
                host.announce_reload(loaded.path.path(), revision, &warnings, &not_applied);
                host.answer_reload(Ok(Reloaded {
                    revision,
                    warnings: &warnings,
                    not_applied: &not_applied,
                }));
                log::info!(area = "runtime", path = %loaded.path.path().display(), revision, "configuration reloaded");
            }
            // The document passed Lua and validation but not the commit: the compositor or the
            // autostart check refused a setting. The handlers of the last accepted file come
            // back, the caller is told why, and the role keeps serving what it had.
            Err(Failed { error, refusal: true }) => {
                let reason = error.to_string();
                log::error!(area = "runtime", "{reason}");
                host.revert_reload(loaded.path.path(), &reason)
                    .map_err(|error| RoleError::Serving(error.to_string()))?;
                host.answer_reload(Err(&reason));
                host.client_mut()
                    .reject_config(&reason)
                    .map_err(|error| RoleError::Commit(error.to_string()))?;
            }
            Err(Failed { error, .. }) => return Err(error),
        },
        ReloadOutcome::Rejected(reason) => {
            log::error!(area = "runtime", "{reason}");
            host.answer_reload(Err(&reason));
            reporter.write(&report::config_rejected(0, &reason));
            host.client_mut()
                .reject_config(&reason)
                .map_err(|error| RoleError::Commit(error.to_string()))?;
        }
    }
    Ok(())
}

/// The IPC version this build advertises, from the schema the bindings were generated from.
pub(crate) fn version() -> Version {
    Version::parse(env!("GNOBLIN_IPC_VERSION")).expect("the build script wrote the schema's own version")
}

#[cfg(test)]
mod tests {
    use super::keep_what_validates;
    use gnoblin_config::Value;

    fn json_document(text: &str) -> Value {
        Value::from_json(&serde_json::from_str(text).expect("the test document is JSON")).expect("a table")
    }

    /// The failure this prevents: one misspelled section costs the user every other setting
    /// at login. The valid cursor section is kept, and the invalid compositor section is the
    /// one named as dropped.
    #[test]
    fn login_keeps_the_sections_that_validate_and_names_the_one_it_drops() {
        let document = json_document(r#"{"compositor": {"blur2": "on"}, "cursor": {"size": 32}}"#);
        let (kept, dropped) = keep_what_validates(document).expect("the cursor section survives");
        assert_eq!(kept.cursor.as_ref().expect("cursor").size, Some(32));
        assert!(kept.compositor.is_none(), "the invalid section is not kept");
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        assert_eq!(dropped[0].path.to_string(), "compositor.blur2");
    }

    /// The failure this prevents: a document with nothing valid left replaces the last
    /// accepted settings with an empty configuration. Such a document is refused whole.
    #[test]
    fn login_refuses_a_document_with_nothing_left_to_keep() {
        let document = json_document(r#"{"compositor": {"blur2": "on"}}"#);
        assert!(keep_what_validates(document).is_none());
    }
}
