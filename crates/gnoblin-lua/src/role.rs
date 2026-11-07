//! The runtime role: the Lua host as its own process.
//!
//! Design/rust-compositor.md, D1: "separate process" — the compositor runs no Lua, so a
//! fault in a script is a restart of this process rather than the end of the session.
//! This module is the role `gnoblin --role runtime` plays:
//!
//! 1. Adopt the privileged connection the guardian passed, and connect. The handshake
//!    grants `commit-config` and `input-policy`; the runtime is the only process that
//!    knows what a configuration document means, so it is the only one allowed to commit
//!    one.
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

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{
    Client, ClientError, Environment, Inherited, Reporter, Transport, inherited_stream, privilege, report,
};
use gnoblin_config::{Config, SearchPaths, extensions::load_extensions};
use gnoblin_proto::Version;
use gnoblin_schema::Schema;
use gnoblin_types::Tick;

use crate::error::HostError;
use crate::extensions::ExtensionRegistry;
use crate::host::{Host, ReloadOutcome, RunReport};
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
    /// The stock schemas. An extension may not reuse their namespaces.
    pub stock: Vec<Schema>,
}

impl RoleRunner {
    /// The role as the process finds it: the real environment, the real search path and
    /// the default sandbox budgets, and the stock schemas the extensions are checked against.
    pub fn from_environment(stock: Vec<Schema>) -> RoleRunner {
        RoleRunner {
            env: Box::new(|name| std::env::var(name).ok()),
            paths: SearchPaths::from_env(),
            limits: Limits::default(),
            stock,
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
        bind_configured_extensions(&mut host, &loaded, &self.stock)?;
        let revision = commit(&mut host, &loaded, &mut reporter)?;
        log::info!(area = "runtime", path = %loaded.path.path().display(),
            revision,
            "configuration committed"
        );
        // Readiness is reported after the commit, because a running host with no
        // configuration is not usable: the guardian waits for this line before it starts
        // anything that reads the document.
        reporter.ready(&[("GNOBLIN_CONFIG_REVISION".to_owned(), revision.to_string())]);

        serve(&mut host, &stop, &self.paths, &self.stock, &mut reporter)
    }
}

/// Commit the validated document and tell the guardian the revision.
///
/// The enabled autostart entries go out as their own lines before `config-accepted`, so the
/// guardian has them by the time it learns the revision was accepted. A rejected revision
/// sends none, so the guardian keeps running what the last accepted revision started.
fn commit<T: Transport + 'static>(
    host: &mut Host<T>,
    report: &RunReport,
    reporter: &mut Reporter,
) -> Result<u64, RoleError> {
    let (document, autostart) = match &report.config {
        Ok(config) => match enabled_autostart(config) {
            Ok(entries) => (report.document.to_json().to_string().into_bytes(), entries),
            Err(problem) => return Err(refuse(report, reporter, &problem)),
        },
        Err(problems) => return Err(refuse(report, reporter, &problems.to_string())),
    };
    let revision = match host.client_mut().commit_config(&document) {
        Ok(revision) => revision,
        Err(error) => {
            reporter.write(&report::config_rejected(0, &error.to_string()));
            return Err(RoleError::Commit(error.to_string()));
        }
    };
    for (name, command) in &autostart {
        reporter.write(&report::autostart(revision, name, command));
    }
    reporter.write(&report::config_accepted(revision));
    Ok(revision)
}

/// Refuse a configuration revision: log why, tell the guardian, and return the error.
fn refuse(report: &RunReport, reporter: &mut Reporter, problems: &str) -> RoleError {
    let reason = format!(
        "the configuration in {} was not committed, and the last accepted one stays in force: {problems}",
        report.path.path().display()
    );
    // The guardian's log and the recovery state both need this: a runtime that exits
    // saying nothing leaves the session degraded with no reason in it.
    log::error!(area = "runtime", "{reason}");
    reporter.write(&report::config_rejected(0, &reason));
    RoleError::Invalid(reason)
}

/// The enabled autostart entries of `config`, in name order, each with its argv.
///
/// The schema requires a command on every entry, so an entry without one is refused here
/// rather than started with nothing.
pub(crate) fn enabled_autostart(config: &Config) -> Result<Vec<(&str, &[String])>, String> {
    let Some(autostart) = &config.autostart else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (name, entry) in &autostart.autostart {
        if !entry.enable.unwrap_or(true) {
            continue;
        }
        match entry.command.as_deref() {
            Some(command) if !command.is_empty() => entries.push((name.as_str(), command)),
            _ => return Err(format!("autostart \"{name}\" has no command")),
        }
    }
    Ok(entries)
}

/// Load the extensions the accepted configuration names, and bind the ones that pass.
///
/// A rejected document loads nothing: the commit that follows refuses it, and the last
/// accepted configuration stays in force. A refused extension is logged with its reason and
/// the rest still bind, so one broken extension never stops the runtime.
fn bind_configured_extensions<T: Transport + 'static>(
    host: &mut Host<T>,
    report: &RunReport,
    stock: &[Schema],
) -> Result<(), RoleError> {
    if report.config.is_err() {
        return Ok(());
    }
    let (registry, refusals) = configured_extensions(report, stock);
    for refusal in &refusals {
        log::warn!(area = "runtime", "{refusal}");
    }
    host.bind_extensions(&registry)
        .map_err(|error| RoleError::Config(error.to_string()))
}

/// The extensions the accepted configuration names, bound into a registry, and every reason
/// one was refused.
///
/// Shared by the runtime and `check`, so validation refuses an extension exactly when the
/// runtime would. A rejected document yields an empty registry and no refusals: its own
/// problems are reported by the commit that follows.
pub(crate) fn configured_extensions(report: &RunReport, stock: &[Schema]) -> (ExtensionRegistry, Vec<String>) {
    let mut registry = ExtensionRegistry::new();
    let mut refusals = Vec::new();
    if report.config.is_err() {
        return (registry, refusals);
    }
    let base = report.path.path().parent().unwrap_or_else(|| Path::new("."));
    let loaded = load_extensions(&report.document, base, stock);
    refusals.extend(loaded.refused.iter().map(ToString::to_string));
    for extension in loaded.accepted {
        let namespace = extension.namespace.clone();
        if let Err(error) = registry.register_extension(extension.schema) {
            refusals.push(format!("extension {namespace} refused at registration: {error:?}"));
        }
    }
    (registry, refusals)
}

/// Serve the handlers until the guardian asks the role to stop or the connection ends.
///
/// A reload the compositor asked for is acted on before the next pump, so no handler runs
/// against a document that is about to be replaced.
fn serve<T: Transport + 'static>(
    host: &mut Host<T>,
    stop: &impl Fn() -> bool,
    paths: &SearchPaths,
    stock: &[Schema],
    reporter: &mut Reporter,
) -> Result<(), RoleError> {
    let start = Instant::now();
    while !stop() {
        if host.take_reload_request() {
            reload(host, paths, stock, reporter)?;
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
    stock: &[Schema],
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
        ReloadOutcome::Accepted(loaded) => {
            bind_configured_extensions(host, &loaded, stock)?;
            let revision = commit(host, &loaded, reporter)?;
            log::info!(area = "runtime", path = %loaded.path.path().display(), revision, "configuration reloaded");
        }
        ReloadOutcome::Rejected(reason) => {
            log::error!(area = "runtime", "{reason}");
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
