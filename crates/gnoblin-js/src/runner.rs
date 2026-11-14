//! The runtime role when the JavaScript shell is selected: everything `gnoblin --role runtime` does, with no Lua host.
//!
//! [`JsRoleRunner`] is what the Lua host's `RoleRunner` is to a Lua session. The guardian starts it with one
//! inherited descriptor (the privileged connection) and a report pipe, and it:
//!
//! 1. adopts the privileged connection, subscribes to the object graph, and watches the `session` object for
//!    `reload_requested`, the signal the compositor raises when `gnoblinctl config reload` (or a script's
//!    `gnoblin.runtime.reloadConfig()`) asks for a reload;
//! 2. locates the user's config ([`gnoblin_config::locate`]: `init.ts`, `init.js`, then `init.lua`), warns about
//!    the files that lost to it, and boots a [`JsRole`] that evaluates it. The role owns the whole document
//!    ([`JsRoleConfig::owns_document`]): the settings `defineConfig` sets, plus its `autostart` commands as the
//!    document's `autostart` section, are validated and committed to the compositor as one revision;
//! 3. tells the guardian each revision the compositor accepted: one `autostart` line for every enabled entry, then
//!    `config-accepted`, and after the first commit `ready` with `GNOBLIN_CONFIG_REVISION`;
//! 4. serves: one shell frame per turn, a reload when the compositor asks for one or the watcher saw a save, until
//!    the guardian stops it or the compositor goes away.
//!
//! # Failure modes first
//!
//! 1. **The guardian did not pass a connection.** The role refuses at once with the variable named, so the
//!    guardian's log says what is wrong instead of the child hanging
//!    (`tests::the_role_refuses_without_a_privileged_connection`).
//! 2. **The first document was not committed.** The compositor refused it (or the autostart check did), so there
//!    is no revision to report: the role says why, reports `config-rejected`, and exits non-zero. The guardian's
//!    restart ladder and its circuit breaker bound the attempts, as they do for the Lua host. A config that does
//!    not evaluate is not this case: the stock config runs in its place and its document is committed.
//! 3. **A reload fails.** The compositor is holding the caller of `config reload`. The role refuses the request
//!    with the reason (`config.reject`), keeps the running generation and the last accepted document, and serves on.
//! 4. **The shell's link fails.** The tree connection is the shell, so a role without it has nothing to serve:
//!    the role ends non-zero and the guardian starts it again, which attaches a fresh shell to the windows already
//!    there.
//!
//! # The stock library
//!
//! `GNOBLIN_JS_STOCK_DIR` names a directory to read the stock library from instead of the copy compiled in, for a
//! person working on the stock shell. There is one runtime role and nothing selects it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{
    Client, ClientError, Environment, Event, Inherited, Reporter, inherited_stream, privilege, report,
};
use gnoblin_config::{Config, ConfigPath, Language, SearchPaths, enabled_autostart, locate};
use gnoblin_model::{Filter, ObjectId, ObjectView, Observation};
use gnoblin_proto::Version;
use thiserror::Error;

use crate::config::{JsCommitLog, JsCommitRecord};
use crate::role::{JsRole, JsRoleConfig};
use crate::shell::JsShellError;

gnoblin_log::logger!(log = "js");

/// The variable that names a directory to read the stock library from, instead of the copy compiled in.
pub const STOCK_DIR_VARIABLE: &str = "GNOBLIN_JS_STOCK_DIR";

/// The variable that turns the config watcher off when it is `0`, so a config is reloaded only when something asks
/// (`gnoblinctl config reload`).
pub const WATCH_VARIABLE: &str = "GNOBLIN_JS_WATCH";

/// A test-only hook: the runtime exits at once, on every start.
///
/// The guardian's circuit breaker exists for a child that crashes as fast as it starts, and a real-process test needs
/// such a child without building one. It is the variable the Lua role reads, and both roles honour it.
pub const TEST_CRASH_VARIABLE: &str = "GNOBLIN_TEST_RUNTIME_CRASH";

/// How long one turn waits for the compositor to say something before it runs the shell's frame.
///
/// A frame applies the tree's changes and runs the listeners, so this is the latency a script sees. It is one display
/// frame at 60 Hz; an idle role wakes that often and does one cheap frame.
pub const FRAME_WAIT: Duration = Duration::from_millis(16);

/// How many events of the object graph one turn takes before it runs a frame, so a compositor that never goes quiet
/// cannot starve the shell.
const EVENT_BATCH: usize = 256;

/// The signal the compositor raises on the `session` object when a reload is asked for.
const RELOAD_REQUESTED: &str = "reload_requested";

/// The page that tells a Lua user how to move to the JavaScript shell. The warning about an `init.lua` names it.
const MIGRATION_DOC: &str = "docs/migrating-from-lua.md";

/// What the role tells a user whose only configuration is a Lua file, or `None` when the located file is not one.
///
/// Nothing reads Lua, so the stock configuration runs in its place. The session still starts, because a login that
/// fails over a file the user has not touched yet would lock them out of the machine that holds it; the warning says
/// what happened and the one command that fixes it.
fn lua_config_notice(located: &ConfigPath) -> Option<String> {
    (located.exists() && located.language() == Language::Lua).then(|| {
        format!(
            "{} is a Lua config and Gnoblin no longer runs Lua, so the stock config runs instead; run `gnoblinctl \
             config migrate` to convert it to init.ts (see {MIGRATION_DOC})",
            located.path().display()
        )
    })
}

/// Why the JavaScript runtime role could not run.
#[derive(Debug, Error)]
pub enum JsRoleError {
    /// The privileged connection the guardian passes was missing or unusable.
    #[error("the runtime needs its privileged connection: {0}")]
    Connection(Inherited),
    /// The handshake with the compositor failed.
    #[error("{0}")]
    Connect(String),
    /// The configuration could not be located, or the shell could not start over it.
    #[error("{0}")]
    Config(String),
    /// The first document was not committed.
    #[error("{0}")]
    Commit(String),
    /// The connection failed while the role was serving.
    #[error("{0}")]
    Serving(String),
}

/// How the role runs, for a caller that is not the process itself.
///
/// `env` and `paths` are passed in so a test drives the shipped role over a socket pair and a configuration
/// directory of its own, without a process-global environment.
pub struct JsRoleRunner {
    /// The environment to read the inherited descriptors, the socket and the switches from.
    pub env: Environment,
    /// Where the configuration lives.
    pub paths: SearchPaths,
}

impl JsRoleRunner {
    /// The role as the process finds it: the real environment and the real search path.
    pub fn from_environment() -> Self {
        Self {
            env: Box::new(|name| std::env::var(name).ok()),
            paths: SearchPaths::from_env(),
        }
    }

    /// Runs the role until the session ends.
    ///
    /// `stop` is asked between turns: the guardian's `SIGTERM` sets a flag rather than interrupting a read, so the
    /// role leaves its connection in a state the compositor can drop cleanly.
    ///
    /// # Errors
    ///
    /// [`JsRoleError`] for each documented failure mode.
    pub fn run(self, stop: impl Fn() -> bool) -> Result<(), JsRoleError> {
        if (self.env)(TEST_CRASH_VARIABLE).is_some() {
            return Err(JsRoleError::Serving(format!(
                "{TEST_CRASH_VARIABLE} is set, so the runtime exits at once"
            )));
        }
        let mut reporter = Reporter::new(&self.env);
        let stream = inherited_stream(privilege::FD_VARIABLE, &self.env)
            .map_err(JsRoleError::Connection)?
            .ok_or(JsRoleError::Connection(Inherited::Absent))?;
        let transport = UnixTransport::from_stream(stream).map_err(|error| {
            JsRoleError::Connection(Inherited::NotASocket {
                number: -1,
                reason: error.to_string(),
            })
        })?;
        let version = {
            let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
            Version::new(major, minor, patch)
        };
        let mut client =
            Client::connect(transport, version).map_err(|error| JsRoleError::Connect(error.to_string()))?;
        let snapshot = client
            .subscribe(Filter::All)
            .map_err(|error| JsRoleError::Connect(format!("cannot subscribe to the session: {error}")))?;
        // From here every read that times out is "nothing yet", and the timeout is the clock the turns below run on.
        // It is set after the handshake and the snapshot, which are waits for a reply a loaded compositor may be slow to
        // send.
        client
            .set_read_timeout(Some(FRAME_WAIT))
            .map_err(|error| JsRoleError::Connect(format!("cannot set the read deadline: {error}")))?;
        let mut sessions: BTreeSet<ObjectId> = snapshot
            .objects()
            .iter()
            .filter(|view| is_session(view))
            .map(ObjectView::id)
            .collect();

        let commits = JsCommitLog::default();
        let config = self.role_config(&commits)?;
        let entry = config.entry.clone();
        let mut role = JsRole::connect(&self.env, config).map_err(|error| JsRoleError::Config(error.to_string()))?;

        // The shell committed the document while it started. Readiness is reported after the commit, because a
        // running role with no configuration is not usable: the guardian waits for this line before it starts
        // anything that reads the document.
        let Some(revision) = publish(&commits, &mut reporter) else {
            let reason = format!(
                "the configuration in {} was not committed: {}",
                entry.display(),
                last_diagnostics(&role, 0)
            );
            log::error!(area = "runtime", "{reason}");
            reporter.write(&report::config_rejected(0, &reason));
            return Err(JsRoleError::Commit(reason));
        };
        log::info!(area = "runtime", path = %entry.display(), revision, "configuration committed");
        reporter.ready(&[("GNOBLIN_CONFIG_REVISION".to_owned(), revision.to_string())]);

        let mut reload_asked = false;
        while !stop() {
            // The wait for the compositor is the turn's clock: it ends at once when something arrives and after
            // `FRAME_WAIT` when nothing does.
            for _ in 0..EVENT_BATCH {
                match client.next_event() {
                    Ok(Event::Model(Observation::Opened { view, .. })) => {
                        if is_session(&view) {
                            sessions.insert(view.id());
                        }
                    }
                    Ok(Event::Model(Observation::Signalled { object, signal, .. })) => {
                        reload_asked |= signal == RELOAD_REQUESTED && sessions.contains(&object);
                    }
                    Ok(_) => {}
                    Err(ClientError::Idle) => break,
                    Err(ClientError::Disconnected) => return Ok(()),
                    Err(error) => return Err(JsRoleError::Serving(error.to_string())),
                }
            }
            // A reload the compositor asked for is acted on before the next frame, so no listener runs against a
            // config that is about to be replaced.
            if std::mem::take(&mut reload_asked) {
                reload(&mut role, &commits, &mut client, &mut reporter, &entry)?;
            }
            role.tick()
                .map_err(|error: JsShellError| JsRoleError::Serving(format!("the shell ended: {error}")))?;
            // A save the watcher saw reloads inside the frame and commits like a requested reload does.
            publish(&commits, &mut reporter);
        }
        Ok(())
    }

    /// The shell's config: the directory and entry the located file names, over the session's owner socket.
    fn role_config(&self, commits: &JsCommitLog) -> Result<JsRoleConfig, JsRoleError> {
        let env = &self.env;
        let located = locate(&self.paths).map_err(|error| JsRoleError::Config(error.to_string()))?;
        for ignored in located.ignored() {
            log::warn!(
                area = "config",
                "{} is not read: {} takes precedence in the same directory",
                ignored.display(),
                located.path().display()
            );
        }
        let dir = located
            .path()
            .parent()
            .ok_or_else(|| JsRoleError::Config(format!("{} has no directory", located.path().display())))?
            .to_path_buf();
        let stock = env(STOCK_DIR_VARIABLE).filter(|dir| !dir.is_empty()).map(PathBuf::from);
        // The loader confines every import to the config directory, so the directory has to exist. A first login has
        // none: it is made here, which is also what lets the watcher see `init.ts` the day it is written.
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir).map_err(|error| {
                JsRoleError::Config(format!("cannot create the config directory {}: {error}", dir.display()))
            })?;
            log::info!(area = "config", "created the config directory {}", dir.display());
        }
        let mut config = JsRoleConfig::new(dir, stock);
        if let Some(notice) = lua_config_notice(&located) {
            log::warn!(area = "config", "{notice}");
        } else if located.exists() {
            config.entry = located.path().to_path_buf();
        }
        config.watch = env(WATCH_VARIABLE).as_deref() != Some("0");
        config.shell.xr_declared = env("GNOBLIN_XR").as_deref() == Some("1");
        config.owns_document = true;
        config.commits = Some(commits.clone());
        Ok(config)
    }
}

/// Whether `view` is the `session` object, the one whose `reload_requested` the role acts on.
fn is_session(view: &ObjectView) -> bool {
    let ty = view.ty();
    ty.namespace() == "gnoblin" && ty.name() == "session"
}

/// Tells the guardian the revision the shell committed last since the previous call, and returns it. `None` when the
/// shell committed nothing.
///
/// The enabled autostart entries go out as their own lines before `config-accepted`, so the guardian has them by the
/// time it learns the revision was accepted. Only the last commit is reported when there were several: the guardian
/// reconciles what it runs with the final document, and an intermediate one would start programs the next stops.
fn publish(commits: &JsCommitLog, reporter: &mut Reporter) -> Option<u64> {
    let records: Vec<JsCommitRecord> = std::mem::take(&mut *commits.borrow_mut());
    let last = records.last()?;
    for notice in &last.notices {
        log::warn!(area = "config", "not applied: {notice}");
    }
    for (name, command) in autostart_of(&last.document) {
        reporter.write(&report::autostart(last.revision, &name, &command));
    }
    reporter.write(&report::config_accepted(last.revision));
    Some(last.revision)
}

/// The enabled autostart entries of a committed document, in name order. The shell validated the document before it
/// committed it, so a document that does not read back is a bug and starts nothing.
fn autostart_of(document: &[u8]) -> Vec<(String, Vec<String>)> {
    let config = match Config::from_json_bytes(document) {
        Ok(config) => config,
        Err(problems) => {
            log::error!(
                area = "runtime",
                "the committed document does not read back, so it starts nothing: {problems}"
            );
            return Vec::new();
        }
    };
    match enabled_autostart(&config) {
        Ok(entries) => entries
            .into_iter()
            .map(|(name, command)| (name.to_owned(), command.to_vec()))
            .collect(),
        Err(problem) => {
            log::error!(area = "runtime", "{problem}");
            Vec::new()
        }
    }
}

/// Reloads the config from disk and commits it, or refuses the request and keeps the running generation.
///
/// The compositor holds the caller of `config reload` until a commit or a rejection answers it. A reload that
/// committed has answered it; one that did not is refused here with why, which the caller reads.
fn reload(
    role: &mut JsRole,
    commits: &JsCommitLog,
    client: &mut Client<UnixTransport>,
    reporter: &mut Reporter,
    entry: &std::path::Path,
) -> Result<(), JsRoleError> {
    let seen = role.diagnostics().len();
    let report = role.reload_now();
    if let Some(revision) = publish(commits, reporter) {
        log::info!(area = "runtime", path = %entry.display(), revision, "configuration reloaded");
        return Ok(());
    }
    let mut parts: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
    parts.extend(report.warnings.iter().cloned());
    let later = last_diagnostics(role, seen);
    if !later.is_empty() {
        parts.push(later);
    }
    if parts.is_empty() {
        parts.push("nothing was committed".to_owned());
    }
    let reason = format!(
        "the reload of {} was refused, and the last accepted configuration stays in force: {}",
        entry.display(),
        parts.join("; ")
    );
    log::error!(area = "runtime", "{reason}");
    reporter.write(&report::config_rejected(0, &reason));
    client
        .reject_config(&reason)
        .map_err(|error| JsRoleError::Commit(error.to_string()))
}

/// The diagnostics the role reported after the first `seen`, joined on one line.
fn last_diagnostics(role: &JsRole, seen: usize) -> String {
    role.diagnostics()[seen..].join("; ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// Search paths whose config directory holds `files`, and the directory that owns them.
    fn config_with(files: &[&str]) -> (tempfile::TempDir, SearchPaths) {
        let root = tempfile::tempdir().expect("a temp dir");
        let directory = root.path().join("config").join("gnoblin");
        std::fs::create_dir_all(&directory).expect("the config directory");
        for name in files {
            std::fs::write(directory.join(name), "// a config\n").expect("the config file");
        }
        let paths = SearchPaths {
            config_home: Some(root.path().join("config")),
            ..SearchPaths::default()
        };
        (root, paths)
    }

    #[test]
    fn the_role_refuses_without_a_privileged_connection() {
        // The guardian's log has to say what is wrong, so the error names the variable.
        let runner = JsRoleRunner {
            env: Box::new(|_| None),
            paths: SearchPaths::default(),
        };
        let error = runner.run(|| false).expect_err("no descriptor, no role");
        assert!(matches!(error, JsRoleError::Connection(Inherited::Absent)), "{error:?}");
        assert!(error.to_string().contains("GNOBLIN_IPC_FD"), "{error}");
    }

    #[test]
    fn the_crash_hook_exits_at_once() {
        let runner = JsRoleRunner {
            env: Box::new(env_of(&[(TEST_CRASH_VARIABLE, "1")])),
            paths: SearchPaths::default(),
        };
        let error = runner.run(|| false).expect_err("the hook ends the role");
        assert!(error.to_string().contains(TEST_CRASH_VARIABLE), "{error}");
    }

    #[test]
    fn a_lua_config_alone_gets_one_warning_that_names_the_migration() {
        let (root, paths) = config_with(&["init.lua"]);
        let located = locate(&paths).expect("a path");
        let notice = lua_config_notice(&located).expect("a Lua file with nothing to replace it is reported");
        let init_lua = root.path().join("config").join("gnoblin").join("init.lua");
        assert!(notice.contains(&init_lua.display().to_string()), "{notice}");
        assert!(notice.contains("gnoblinctl config migrate"), "{notice}");
        assert!(notice.contains(MIGRATION_DOC), "{notice}");
    }

    #[test]
    fn a_lua_config_alone_runs_the_stock_config_and_keeps_the_session_alive() {
        let (root, paths) = config_with(&["init.lua"]);
        let runner = JsRoleRunner {
            env: Box::new(env_of(&[])),
            paths,
        };
        let config = runner.role_config(&JsCommitLog::default()).expect("a config");
        let directory = root.path().join("config").join("gnoblin");
        assert_eq!(
            config.entry,
            directory.join("init.ts"),
            "the Lua file is not the entry: {:?}",
            config.entry
        );
    }

    #[test]
    fn a_typescript_or_javascript_config_is_not_reported_as_lua() {
        for files in [&["init.ts"][..], &["init.js"], &["init.lua", "init.ts"], &[]] {
            let (_root, paths) = config_with(files);
            let located = locate(&paths).expect("a path");
            assert_eq!(lua_config_notice(&located), None, "{files:?}");
        }
    }

    #[test]
    fn the_stock_library_comes_from_the_named_directory_when_there_is_one() {
        let (_root, paths) = config_with(&[]);
        let runner = JsRoleRunner {
            env: Box::new(env_of(&[(STOCK_DIR_VARIABLE, "/opt/stock")])),
            paths: paths.clone(),
        };
        let config = runner.role_config(&JsCommitLog::default()).expect("a config");
        assert_eq!(config.shell.stock_dir.as_deref(), Some(Path::new("/opt/stock")));
        for env in [env_of(&[]), env_of(&[(STOCK_DIR_VARIABLE, "")])] {
            let runner = JsRoleRunner {
                env: Box::new(env),
                paths: paths.clone(),
            };
            let config = runner.role_config(&JsCommitLog::default()).expect("a config");
            assert_eq!(config.shell.stock_dir, None);
        }
    }

    #[test]
    fn a_first_login_has_no_config_directory_and_the_role_makes_it_for_the_loader_and_the_watcher() {
        let root = tempfile::tempdir().expect("a temp dir");
        let runner = JsRoleRunner {
            env: Box::new(env_of(&[])),
            paths: SearchPaths {
                config_home: Some(root.path().join("config")),
                ..SearchPaths::default()
            },
        };
        let config = runner
            .role_config(&JsCommitLog::default())
            .expect("a config for a missing directory");
        let directory = root.path().join("config").join("gnoblin");
        assert!(directory.is_dir(), "the directory was made");
        assert_eq!(
            config.entry,
            directory.join("init.ts"),
            "the entry is the file a new user creates"
        );
        assert!(config.owns_document && config.commits.is_some());
    }

    #[test]
    fn the_config_names_the_located_file_and_turns_the_watcher_off_on_request() {
        let (_root, paths) = config_with(&["init.js"]);
        let runner = JsRoleRunner {
            env: Box::new(env_of(&[(WATCH_VARIABLE, "0"), ("GNOBLIN_XR", "1")])),
            paths,
        };
        let config = runner.role_config(&JsCommitLog::default()).expect("a config");
        assert!(config.entry.ends_with("init.js"), "{:?}", config.entry);
        assert!(!config.watch);
        assert!(config.shell.xr_declared);
    }
}
