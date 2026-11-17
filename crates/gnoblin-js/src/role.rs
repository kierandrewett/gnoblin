//! The JavaScript shell as the runtime role runs it: the counterpart of `the retired retired host`'s `ShellRole`.
//!
//! [`JsRole`] connects a [`JsShell`] to the compositor's owner socket, loads the user's `shell.ts` (or
//! `shell.js`) from the config directory, and runs one [`JsShell::frame`] per turn of the role's loop. It owns
//! the three pieces of hot reload and puts them to work:
//!
//! - a [`ConfigWatcher`] on the config directory and on every module the running generation imported; a save
//!   is one reload, after the editor's burst of events has gone quiet;
//! - a [`JsReloader`], which stages a fresh generation from disk against the live tree. It evaluates the
//!   entry in a generation of its own, and only a generation that evaluated cleanly is handed to the shell
//!   ([`JsShell::swap_generation`]). A syntax error, a throw or a runaway loop in the new file leaves the
//!   running generation as it was, and the failure is logged once as `file:line:column: message` in the
//!   author's `.ts` coordinates;
//! - a [`StrikeSupervisor`], which decides what a budget trip means. A listener that runs past its wall-clock
//!   budget is interrupted (the compositor never waits for it: it is another process), counts a strike against
//!   the generation, and the role restages the generation from disk. Three strikes in 30 s stop the restaging
//!   and leave the last generation running until the file is edited or the user reloads.
//!
//! A link failure ends the shell: [`JsRole::tick`] returns it and the caller drops the role, as the the retired shell
//! shell's slot does, and the runtime keeps serving without a shell.
//!
//! Windows are nodes of the compositor's tree, so a reload re-creates nothing: the new generation wraps the
//! same node ids, and its listeners are told of the windows already there (replay on `addListener`) and then
//! that they are ready.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::{UnixTransport, socket_path};
use gnoblin_proto::Version;
use gnoblin_tree_client::slot_link::{ClientSlotLink, SlotLink};
use gnoblin_tree_client::tree_link::SocketTreeLink;
use gnoblin_tree_client::tree_mirror::TreeLink;

use crate::config::{JsClientConfigLink, JsCommitLog, JsConfigLink, JsRecordingLink};
use crate::console::{JsConsole, JsConsoleSite, console_enabled, console_socket_path};
use crate::diagnostics::{DIAGNOSTICS_KEPT, Diagnostics};
use crate::generation::JsGeneration;
use crate::host_idle::{JsClientIdleLink, JsIdleConnector, JsIdleLink};
use crate::host_session::{JsClientSessionLink, JsSessionLink};
use crate::reload::{JsReloader, ReloadReport, SCRIPT_EXTENSIONS};
use crate::shell::{JsShell, JsShellConfig, JsShellError};
use gnoblin_shell_host::supervisor::{GenerationFault, Health, StrikePolicy, StrikeSupervisor, SystemClock};
use gnoblin_shell_host::watch::{ConfigWatcher, with_extensions};

gnoblin_log::logger!(log = "js");

/// How long the config watcher waits for a quiet moment after a change before it asks for a reload.
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);

/// What a [`JsRole`] is built from.
#[derive(Debug, Clone)]
pub struct JsRoleConfig {
    /// The shell's budgets, the config root (the directory the entry's relative imports resolve under) and the
    /// stock directory.
    pub shell: JsShellConfig,
    /// The entry module, an absolute path under the config root.
    pub entry: PathBuf,
    /// How many budget trips in what time stop the restaging of a generation.
    pub policy: StrikePolicy,
    /// The quiet time the watcher waits for after a change.
    pub debounce: Duration,
    /// Whether the config directory is watched and a save reloads. Off, a reload happens only when asked
    /// (`gnoblinctl config reload`): for a config directory on a file system that cannot be watched, and for a
    /// test that proves the request path and not the watcher.
    pub watch: bool,
    /// Where the console listens. See [`JsConsoleSite`].
    pub console: JsConsoleSite,
    /// Whether the shell owns the whole configuration document, as in the JavaScript runtime role
    /// ([`JsShell::own_document`]). Off, the shell lays its settings over a document another host committed.
    pub owns_document: bool,
    /// Where the shell records each document commit it makes, for the runtime role that reports them to the
    /// guardian. `None` records nothing.
    pub commits: Option<JsCommitLog>,
    /// How many diagnostics the role keeps (see [`JsRole::diagnostics`]). A test that floods the role sets it low.
    pub diagnostics_kept: usize,
    /// Whether a change the watcher sees is left to the host. On, the role does not reload itself: it notes that a reload is
    /// wanted ([`JsRole::take_reload_wanted`]) and the host, which owns the shell's lifecycle, asks for one and calls
    /// [`JsRole::reload_now`] when it takes it. The runtime library does this so that a script reloads through the loader
    /// that every language shares. Off, the role reloads inside its own turn, as the runtime role always has.
    pub external_reload: bool,
}

impl JsRoleConfig {
    /// The config for the user's directory `dir`, with the stock library compiled in unless `stock_dir` names one.
    pub fn new(dir: impl Into<PathBuf>, stock_dir: Option<PathBuf>) -> Self {
        let dir = dir.into();
        // The file a new user creates, and the one the watcher then reloads. The runner replaces it with the file
        // discovery selected when there is one.
        let entry = dir.join("shell.ts");
        let mut shell = JsShellConfig::new(dir);
        shell.stock_dir = stock_dir;
        Self {
            shell,
            entry,
            policy: StrikePolicy::default(),
            debounce: RELOAD_DEBOUNCE,
            watch: true,
            console: JsConsoleSite::default(),
            owns_document: false,
            commits: None,
            diagnostics_kept: DIAGNOSTICS_KEPT,
            external_reload: false,
        }
    }

    /// Resolves [`JsConsoleSite::Environment`] against `env`: the console follows the owner socket's directory, and
    /// `GNOBLIN_JS_CONSOLE=0` turns it off. Any other site is left as it is.
    pub fn resolve_console(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        if self.console == JsConsoleSite::Environment {
            self.console = match console_socket_path(env) {
                Ok(site) if console_enabled(env) => JsConsoleSite::At(site),
                _ => JsConsoleSite::Off,
            };
        }
    }
}

/// The connections a role has besides the tree: each is optional, and the role runs without it.
#[derive(Default)]
pub struct JsRoleLinks {
    /// For slots: the observation of the compositor's own dialogs.
    pub slots: Option<Box<dyn SlotLink>>,
    /// For the configuration document: the device settings the spec objects write.
    pub config: Option<Box<dyn JsConfigLink>>,
    /// For the session: `gnoblin.session`, `gnoblin.runtime.reloadConfig` and the schema's signals.
    pub session: Option<Box<dyn JsSessionLink>>,
    /// Opens the connection for the compositor's idle state when a script first wants it (`gnoblin.idle`).
    pub idle: Option<JsIdleConnector>,
}

impl JsRoleLinks {
    /// The role's three connections beside the tree, opened on the owner socket at `path`: slots, the configuration document and
    /// the session (the idle connection is opened when a script first wants it). Slots and the document travel on the `gnoblin`
    /// namespace, which the tree link does not carry, so each is a connection of its own. Without one the shell still runs: the
    /// compositor's own dialogs serve every request and the document keeps what the retired host committed, so a failure to open
    /// either is reported and not fatal.
    pub fn over_owner_socket(path: &std::path::Path, version: Version) -> Self {
        let slots: Option<Box<dyn SlotLink>> = match ClientSlotLink::connect(path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no slot connection ({error}); the built-in dialogs serve prompts"
                );
                None
            }
        };
        let config: Option<Box<dyn JsConfigLink>> = match JsClientConfigLink::connect(path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no configuration connection ({error}); its document settings are not committed"
                );
                None
            }
        };
        let session: Option<Box<dyn JsSessionLink>> = match JsClientSessionLink::connect(path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no session connection ({error}); gnoblin.session calls are refused"
                );
                None
            }
        };
        let idle_path = path.to_path_buf();
        let idle: JsIdleConnector = Box::new(move || {
            JsClientIdleLink::connect(&idle_path, version).map(|link| Box::new(link) as Box<dyn JsIdleLink>)
        });
        Self {
            slots,
            config,
            session,
            idle: Some(idle),
        }
    }

    /// The links over the owner socket `env` names (`GNOBLIN_IPC_SOCKET`, else the runtime directory's default socket), or none
    /// at all when there is no such socket to name, as in a process with no compositor socket. The role is the owner by the
    /// credentials of its process, so a shell that runs inside the compositor is one too.
    pub fn from_environment(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let version = {
            let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
            Version::new(major, minor, patch)
        };
        match socket_path(env) {
            Ok(path) => Self::over_owner_socket(&path, version),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no owner socket ({error}); it runs without slots, a configuration document or session calls"
                );
                Self::default()
            }
        }
    }
}

/// The reloader's commit: it hands a generation that evaluated cleanly to the role, which swaps it into the
/// shell once the reloader has finished.
type Commit = Box<dyn FnMut(&JsGeneration) -> Result<(), String>>;

/// A running JavaScript shell, driven one frame per turn of the runtime loop.
pub struct JsRole {
    shell: JsShell,
    reloader: JsReloader<Commit>,
    watcher: Option<ConfigWatcher>,
    /// The generation the reloader last committed, until the role has swapped it into the shell.
    handoff: Rc<RefCell<Option<JsGeneration>>>,
    /// How many of the shell's strikes the role has already acted on.
    handled_strikes: u32,
    /// The newest diagnostics reported since the role started, oldest first, and a count of the ones let go.
    diagnostics: Diagnostics,
    /// Why the shell ended during a reload, for the next [`tick`](Self::tick) to return.
    ended: Option<JsShellError>,
    /// The user's entry module, which may not exist.
    entry: PathBuf,
    /// The console socket, polled before each frame; `None` when it is off or could not be made.
    console: Option<JsConsole>,
    /// Whether the host reloads for the watcher ([`JsRoleConfig::external_reload`]).
    external_reload: bool,
    /// A change the watcher saw that the host has not been told of.
    reload_wanted: bool,
}

impl JsRole {
    /// Connects to the owner socket `env` names and boots the shell over it, then loads the user's entry.
    ///
    /// The path is the one every client reads (`GNOBLIN_IPC_SOCKET`, else the runtime directory's default
    /// socket). The handshake is the role's own: the version this build serves, sent as a client. The
    /// compositor grants the owner connection by the peer's credentials, so the session owner's second
    /// connection is the one that can attach the tree.
    pub fn connect(env: &dyn Fn(&str) -> Option<String>, mut config: JsRoleConfig) -> Result<Self, JsShellError> {
        let path = socket_path(env).map_err(JsShellError::Link)?;
        config.resolve_console(env);
        let transport = UnixTransport::connect(&path)
            .map_err(|error| JsShellError::Link(format!("cannot connect to {}: {error}", path.display())))?;
        let version = {
            let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
            Version::new(major, minor, patch)
        };
        let client = Client::connect(transport, version).map_err(|error| JsShellError::Link(error.to_string()))?;
        let stream = client
            .into_inner()
            .into_stream()
            .map_err(|error| JsShellError::Link(error.to_string()))?;
        let link = SocketTreeLink::new(stream).map_err(|error| JsShellError::Link(error.to_string()))?;
        Self::over_link(Box::new(link), JsRoleLinks::over_owner_socket(&path, version), config)
    }

    /// Boots the shell over `link` and loads the user's entry. With no entry file, or an entry that fails to
    /// load, the stock default config runs instead (four floating workspaces, the stock keys): a shell is never
    /// left bare. The diagnostic says why a failing entry was not used.
    pub fn over_link(link: Box<dyn TreeLink>, links: JsRoleLinks, config: JsRoleConfig) -> Result<Self, JsShellError> {
        let mut shell = JsShell::new(link, config.shell.clone())?;
        if let Some(slots) = links.slots {
            shell.set_slot_link(slots);
        }
        if config.owns_document {
            shell.own_document();
        }
        if let Some(document) = links.config {
            match &config.commits {
                Some(log) => shell.set_config_link(Box::new(JsRecordingLink::new(document, Rc::clone(log)))),
                None => shell.set_config_link(document),
            }
        }
        if let Some(session) = links.session {
            shell.set_session_link(session);
        }
        if let Some(idle) = links.idle {
            shell.set_idle_connector(idle);
        }
        let staging = shell.staging();

        let handoff: Rc<RefCell<Option<JsGeneration>>> = Rc::default();
        let sink = Rc::clone(&handoff);
        let commit: Commit = Box::new(move |generation| {
            *sink.borrow_mut() = Some(generation.share());
            Ok(())
        });
        let mut reloader = JsReloader::new(
            staging.config().clone(),
            config.entry.clone(),
            commit,
            StrikeSupervisor::new(config.policy, SystemClock),
        );
        reloader.set_prepare(Box::new(move |generation| staging.prepare(generation)));

        let root = &config.shell.js.root;
        let watcher = if config.watch {
            match ConfigWatcher::new(root, config.debounce, with_extensions(SCRIPT_EXTENSIONS)) {
                Ok(watcher) => Some(watcher),
                Err(error) => {
                    log::warn!(area = "shell-vm", "the config directory is not watched: {error}");
                    None
                }
            }
        } else {
            None
        };
        let mut role = Self {
            shell,
            reloader,
            watcher,
            handoff,
            handled_strikes: 0,
            diagnostics: Diagnostics::new(config.diagnostics_kept),
            ended: None,
            entry: config.entry.clone(),
            console: None,
            external_reload: config.external_reload,
            reload_wanted: false,
        };
        if let JsConsoleSite::At(site) = &config.console {
            // A shell without a console is still a shell: the reason is logged and the loop goes on.
            match JsConsole::bind(site) {
                Ok(console) => role.console = Some(console),
                Err(error) => log::warn!(area = "console", "the console is not served: {error}"),
            }
        }
        if config.entry.is_file() {
            if !role.reload_now().ok {
                // The entry is broken at startup: the shell still has to work, so the stock config runs until the
                // file is fixed (the watcher, or a reload, then swaps the user's generation in).
                role.run_default_config();
            }
        } else {
            log::info!(
                area = "shell-vm",
                "no {} to load; the stock config runs until it exists",
                config.entry.display()
            );
            role.run_default_config();
            role.rearm_watcher();
        }
        Ok(role)
    }

    /// How long until the shell needs a frame although nothing arrived (see [`JsShell::next_wake`]): the role waits no
    /// longer than this for the compositor.
    pub fn next_wake(&self) -> Option<Duration> {
        self.shell.next_wake()
    }

    /// Sleeps until a connection of the shell (or `own`, the caller's connection to the compositor) has something to
    /// read or `timeout` passes (see [`JsShell::wait_for_input`]). The runtime role's turn waits here.
    ///
    /// # Errors
    ///
    /// The kernel refused the wait: a connection's descriptor is no longer open. The shell has ended.
    pub fn wait_for_input(
        &self,
        own: Option<std::os::fd::BorrowedFd<'_>>,
        timeout: Duration,
    ) -> Result<bool, JsShellError> {
        self.shell.wait_for_input(own, timeout)
    }

    /// Runs one frame: acts on a reload the watcher asked for, applies the compositor's batches and runs the
    /// listeners, sends what they wrote, and restages the generation when a listener ran past its budget.
    ///
    /// # Errors
    ///
    /// The link failed or the compositor sent something the shell cannot take. The shell has ended; the caller
    /// drops the role.
    pub fn tick(&mut self) -> Result<(), JsShellError> {
        if let Some(error) = self.ended.take() {
            return Err(error);
        }
        if let Some(request) = self.watcher.as_ref().and_then(|w| w.next_request(Duration::ZERO)) {
            if self.external_reload {
                self.reload_wanted = true;
            } else {
                self.staged(|reloader| reloader.reload_for(&request));
            }
        }
        // Console expressions run here, between frames: never beside one, so what they read and write is the state
        // the frame after them sees.
        if let Some(console) = self.console.as_mut() {
            console.serve(&self.shell);
        }
        let frame = self.shell.frame()?;
        for diagnostic in frame.diagnostics {
            log::warn!(area = "shell-vm", "{diagnostic}");
            self.diagnostics.push(diagnostic);
        }
        let strikes = self.shell.strikes();
        if strikes > self.handled_strikes {
            // The interrupted script skipped its cleanup, so the generation is suspect: the supervisor decides
            // whether a fresh one is staged from the files on disk.
            self.handled_strikes = strikes;
            let report = self.staged(|reloader| reloader.recover(GenerationFault::Budget));
            if report.ok {
                log::warn!(
                    area = "shell-vm",
                    "a listener ran past its budget; generation {} restarted from disk",
                    self.shell.generation_id()
                );
            }
        }
        Ok(())
    }

    /// Whether the watcher saw a change since the last call, once. Only a role built with [`JsRoleConfig::external_reload`]
    /// sets it; the host answers it by reloading through its own lifecycle and calling [`reload_now`](Self::reload_now).
    pub fn take_reload_wanted(&mut self) -> bool {
        std::mem::take(&mut self.reload_wanted)
    }

    /// Reloads from disk because the user asked (`gnoblinctl config reload`) or at startup. A deliberate retry
    /// clears the supervisor's strikes. With no entry file there is nothing to reload: the stock config keeps
    /// running and the report says so.
    pub fn reload_now(&mut self) -> ReloadReport {
        if !self.entry.is_file() {
            return ReloadReport {
                ok: true,
                generation: Some(self.shell.generation_id()),
                errors: Vec::new(),
                warnings: vec![format!(
                    "{} does not exist; the running config stays",
                    self.entry.display()
                )],
                changed: Vec::new(),
                health: self.reloader.health(),
            };
        }
        self.staged(JsReloader::reload_now)
    }

    /// Puts the running generation's document settings back after another commit of the document: the retired host's
    /// own reload commits a document that has none of them. Called by whoever commits (the runtime role's loop).
    pub fn config_committed(&mut self) {
        match self.shell.config_committed() {
            Ok(diagnostics) => {
                for diagnostic in diagnostics {
                    log::warn!(area = "shell-vm", "{diagnostic}");
                    self.diagnostics.push(diagnostic);
                }
            }
            Err(error) => self.ended = Some(error),
        }
    }

    /// Runs the stock default config in the running generation, which is the bare one. A failure is a
    /// diagnostic; a failed link ends the shell, as everywhere else.
    fn run_default_config(&mut self) {
        match self.shell.load_default_config() {
            Ok(diagnostics) => {
                for diagnostic in diagnostics {
                    log::warn!(area = "shell-vm", "{diagnostic}");
                    self.diagnostics.push(diagnostic);
                }
            }
            Err(JsShellError::Js(error)) => {
                log::warn!(area = "shell-vm", "the stock config did not load: {error}");
                self.diagnostics.push(error.to_string());
            }
            Err(error) => self.ended = Some(error),
        }
    }

    /// Stages with `run` and swaps the staged generation into the shell when it was accepted. A failed stage puts
    /// the shell's pending writes back, and its diagnostics are logged and kept.
    fn staged(&mut self, run: impl FnOnce(&mut JsReloader<Commit>) -> ReloadReport) -> ReloadReport {
        self.shell.begin_stage();
        let mut report = run(&mut self.reloader);
        let handed = self.handoff.borrow_mut().take();
        match handed {
            Some(generation) => {
                match self.shell.swap_generation(generation) {
                    Ok(diagnostics) => {
                        for diagnostic in diagnostics {
                            log::warn!(area = "shell-vm", "{diagnostic}");
                            self.diagnostics.push(diagnostic);
                        }
                    }
                    // The link failed under the swap: the shell has ended, and the next tick says so.
                    Err(error) => self.ended = Some(error),
                }
                self.handled_strikes = self.shell.strikes();
            }
            None => self.shell.abandon_stage(),
        }
        if let Some(watcher) = self.watcher.as_mut()
            && let Err(error) = watcher.watch_modules(&self.reloader.watch_set())
        {
            report.warnings.push(error.to_string());
        }
        for error in &report.errors {
            let text = format!("the running generation stays: {error}");
            log::warn!(area = "shell-vm", "{text}");
            self.diagnostics.push(error.to_string());
        }
        for warning in &report.warnings {
            log::warn!(area = "shell-vm", "{warning}");
        }
        report
    }

    /// Watches the config directory alone, for a role that has no module graph yet.
    fn rearm_watcher(&mut self) {
        if let Some(watcher) = self.watcher.as_mut()
            && let Err(error) = watcher.watch_modules(&self.reloader.watch_set())
        {
            log::warn!(area = "shell-vm", "{error}");
        }
    }

    /// The newest diagnostics the role has reported, oldest first: a failed reload as `file:line:column: message`, a
    /// listener that threw, a refused transaction. The list is bounded ([`crate::DIAGNOSTICS_KEPT`] lines, each cut at
    /// [`crate::DIAGNOSTIC_LINE_BYTES`]): once the role has let any go, the first line says how many, and
    /// [`Self::diagnostics_reported`] still counts all of them.
    pub fn diagnostics(&self) -> &[String] {
        self.diagnostics.lines()
    }

    /// How many diagnostics the role has reported since it started, those it let go of included.
    pub fn diagnostics_reported(&self) -> u64 {
        self.diagnostics.reported()
    }

    /// How many diagnostics the role let go to keep its list bounded.
    pub fn diagnostics_dropped(&self) -> u64 {
        self.diagnostics.dropped()
    }

    /// The kept diagnostics reported after the first `reported`, where `reported` is what
    /// [`Self::diagnostics_reported`] answered when the caller last looked. It does not move when the list drops its
    /// oldest, which an index into [`Self::diagnostics`] does.
    pub fn diagnostics_since(&self, reported: u64) -> &[String] {
        self.diagnostics.since(reported)
    }

    /// The id of the generation that runs. A restart or a successful reload raises it.
    pub fn generation_id(&self) -> u64 {
        self.shell.generation_id()
    }

    /// Whether the supervisor still allows restaging after a budget trip.
    pub fn health(&self) -> Health {
        self.reloader.health()
    }

    /// Runs `f` in the running generation as a listener would run: see [`JsShell::call_script`].
    ///
    /// # Errors
    ///
    /// The script threw, or ran past its budget.
    pub fn call_script<R>(
        &self,
        f: impl for<'js> FnOnce(&rquickjs::Ctx<'js>) -> rquickjs::Result<R>,
    ) -> Result<R, crate::JsError> {
        self.shell.call_script(f)
    }

    /// The shell, for a caller that evaluates a probe in it.
    pub fn shell_mut(&mut self) -> &mut JsShell {
        &mut self.shell
    }
}
