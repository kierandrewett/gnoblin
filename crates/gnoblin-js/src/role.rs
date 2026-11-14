//! The JavaScript shell as the runtime role runs it: the counterpart of `gnoblin-lua`'s `ShellRole`.
//!
//! [`JsRole`] connects a [`JsShell`] to the compositor's owner socket, loads the user's `init.ts` (or
//! `init.js`) from the config directory, and runs one [`JsShell::frame`] per turn of the role's loop. It owns
//! the three pieces of hot reload and puts them to work:
//!
//! - a [`ConfigWatcher`] on the config directory and on every module the running generation imported; a save
//!   is one reload, after the editor's burst of events has gone quiet;
//! - a [`JsReloader`], which stages a fresh generation from disk against the live tree. It evaluates the
//!   entry in a generation of its own, and only a generation that evaluated cleanly is handed to the shell
//!   ([`JsShell::swap_generation`]). A syntax error, a throw or a runaway loop in the new file leaves the
//!   running generation as it was, and the failure is logged once as `file:line:column: message` in the
//!   author's `.ts` coordinates;
//! - a [`JsSupervisor`], which decides what a budget trip means. A listener that runs past its wall-clock
//!   budget is interrupted (the compositor never waits for it: it is another process), counts a strike against
//!   the generation, and the role restages the generation from disk. Three strikes in 30 s stop the restaging
//!   and leave the last generation running until the file is edited or the user reloads.
//!
//! A link failure ends the shell: [`JsRole::tick`] returns it and the caller drops the role, as the Lua
//! shell's slot does, and the runtime keeps serving without a shell.
//!
//! Windows are nodes of the compositor's tree, so a reload re-creates nothing: the new generation wraps the
//! same node ids, and its listeners are told of the windows already there (replay on `addListener`) and then
//! that they are ready.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::{UnixTransport, socket_path};
use gnoblin_proto::Version;
use gnoblin_tree_client::slot_link::{ClientSlotLink, SlotLink};
use gnoblin_tree_client::tree_link::SocketTreeLink;
use gnoblin_tree_client::tree_mirror::TreeLink;

use crate::bindings::{JsSpawnError, JsSpawnRequest};
use crate::config::{JsClientConfigLink, JsConfigLink};
use crate::console::{JsConsole, JsConsoleSite, console_enabled, console_socket_path};
use crate::generation::JsGeneration;
use crate::host_session::{JsClientSessionLink, JsSessionLink};
use crate::host_idle::{JsClientIdleLink, JsIdleConnector, JsIdleLink};
use crate::reload::{JsReloader, ReloadReport};
use crate::shell::{JsShell, JsShellConfig, JsShellError};
use crate::supervisor::{JsFault, JsHealth, JsStrikePolicy, JsSupervisor, JsSystemClock};
use crate::watch::ConfigWatcher;

gnoblin_log::logger!(log = "js");

/// How long the config watcher waits for a quiet moment after a change before it asks for a reload.
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);

/// The config directory: `$XDG_CONFIG_HOME/gnoblin`, else `$HOME/.config/gnoblin`. `None` when neither
/// variable is set.
pub fn config_dir(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|value| !value.is_empty());
    let base = match set("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(&set("HOME")?).join(".config"),
    };
    Some(base.join("gnoblin"))
}

/// The user's entry module in `dir`: `init.ts`, else `init.js`. When neither is there it is `init.ts`, the
/// file whose creation the watcher then reloads.
pub fn entry_in(dir: &Path) -> PathBuf {
    let ts = dir.join("init.ts");
    if ts.is_file() {
        return ts;
    }
    let js = dir.join("init.js");
    if js.is_file() { js } else { ts }
}

/// What a [`JsRole`] is built from.
#[derive(Debug, Clone)]
pub struct JsRoleConfig {
    /// The shell's budgets, the config root (the directory the entry's relative imports resolve under) and the
    /// stock directory.
    pub shell: JsShellConfig,
    /// The entry module, an absolute path under the config root.
    pub entry: PathBuf,
    /// How many budget trips in what time stop the restaging of a generation.
    pub policy: JsStrikePolicy,
    /// The quiet time the watcher waits for after a change.
    pub debounce: Duration,
    /// Whether the config directory is watched and a save reloads. Off, a reload happens only when asked
    /// (`gnoblinctl config reload`): for a config directory on a file system that cannot be watched, and for a
    /// test that proves the request path and not the watcher.
    pub watch: bool,
    /// Where the console listens. See [`JsConsoleSite`].
    pub console: JsConsoleSite,
}

impl JsRoleConfig {
    /// The config for the user's directory `dir`, with the stock library compiled in unless `stock_dir` names one.
    pub fn new(dir: impl Into<PathBuf>, stock_dir: Option<PathBuf>) -> Self {
        let dir = dir.into();
        let entry = entry_in(&dir);
        let mut shell = JsShellConfig::new(dir);
        shell.stock_dir = stock_dir;
        Self {
            shell,
            entry,
            policy: JsStrikePolicy::default(),
            debounce: RELOAD_DEBOUNCE,
            watch: true,
            console: JsConsoleSite::default(),
        }
    }
}

/// The connections a role has besides the tree: each is optional, and the role runs without it.
#[derive(Default)]
pub struct JsRoleLinks {
    /// For slots: the observation of the compositor's own dialogs.
    pub slots: Option<Box<dyn SlotLink>>,
    /// For the configuration document: the settings a `defineConfig` sets.
    pub config: Option<Box<dyn JsConfigLink>>,
    /// For the session: `gnoblin.session`, `gnoblin.runtime.reloadConfig` and the schema's signals.
    pub session: Option<Box<dyn JsSessionLink>>,
    /// For the authentication slot (`gnoblin.authentication`): a connection of its own, so a provider's claim and
    /// answers never queue behind the dialog observation's.
    pub auth: Option<Box<dyn SlotLink>>,
    /// Opens the connection for the compositor's idle state when a script first wants it (`gnoblin.idle`).
    pub idle: Option<JsIdleConnector>,
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
    /// Every diagnostic reported since the role started, oldest first.
    diagnostics: Vec<String>,
    /// Why the shell ended during a reload, for the next [`tick`](Self::tick) to return.
    ended: Option<JsShellError>,
    /// The user's entry module, which may not exist.
    entry: PathBuf,
    /// The console socket, polled before each frame; `None` when it is off or could not be made.
    console: Option<JsConsole>,
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
        if config.console == JsConsoleSite::Environment {
            // The console follows the owner socket's directory, and `GNOBLIN_JS_CONSOLE=0` turns it off.
            config.console = match console_socket_path(env) {
                Ok(site) if console_enabled(env) => JsConsoleSite::At(site),
                _ => JsConsoleSite::Off,
            };
        }
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
        // Slots and the configuration document travel on the `gnoblin` namespace, which the tree socket does not
        // carry, so the shell holds two more owner connections. Without one the shell still runs: the compositor's
        // own dialogs serve every request and the document keeps what the Lua host committed, so a failure to
        // open either is reported and not fatal.
        let slots: Option<Box<dyn SlotLink>> = match ClientSlotLink::connect(&path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no slot connection ({error}); the built-in dialogs serve prompts"
                );
                None
            }
        };
        let document: Option<Box<dyn JsConfigLink>> = match JsClientConfigLink::connect(&path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no configuration connection ({error}); its document settings are not committed"
                );
                None
            }
        };
        let session: Option<Box<dyn JsSessionLink>> = match JsClientSessionLink::connect(&path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no session connection ({error}); gnoblin.session calls are refused"
                );
                None
            }
        };
        let auth: Option<Box<dyn SlotLink>> = match ClientSlotLink::connect(&path, version) {
            Ok(link) => Some(Box::new(link)),
            Err(error) => {
                log::warn!(
                    area = "shell-vm",
                    "the shell has no authentication connection ({error}); the built-in agent serves polkit requests"
                );
                None
            }
        };
        let idle_path = path.clone();
        let idle: JsIdleConnector = Box::new(move || {
            JsClientIdleLink::connect(&idle_path, version).map(|link| Box::new(link) as Box<dyn JsIdleLink>)
        });
        Self::over_link(
            Box::new(link),
            JsRoleLinks {
                slots,
                config: document,
                session,
                auth,
                idle: Some(idle),
            },
            config,
        )
    }

    /// Boots the shell over `link` and loads the user's entry. With no entry file, or an entry that fails to
    /// load, the stock default config runs instead (four floating workspaces, the stock keys): a shell is never
    /// left bare. The diagnostic says why a failing entry was not used.
    pub fn over_link(link: Box<dyn TreeLink>, links: JsRoleLinks, config: JsRoleConfig) -> Result<Self, JsShellError> {
        let mut shell = JsShell::new(link, config.shell.clone())?;
        shell.set_spawner(Box::new(spawn_detached));
        if let Some(slots) = links.slots {
            shell.set_slot_link(slots);
        }
        if let Some(document) = links.config {
            shell.set_config_link(document);
        }
        if let Some(session) = links.session {
            shell.set_session_link(session);
        }
        if let Some(auth) = links.auth {
            shell.set_auth_link(auth);
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
            JsSupervisor::new(config.policy, JsSystemClock),
        );
        reloader.set_prepare(Box::new(move |generation| staging.prepare(generation)));

        let root = &config.shell.js.root;
        let watcher = if config.watch {
            match ConfigWatcher::new(root, config.debounce) {
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
            diagnostics: Vec::new(),
            ended: None,
            entry: config.entry.clone(),
            console: None,
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
            self.staged(|reloader| reloader.reload_for(&request));
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
            let report = self.staged(|reloader| reloader.recover(JsFault::Budget));
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

    /// Puts the running generation's document settings back after another commit of the document: the Lua host's
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

    /// Every diagnostic the role has reported since it started, oldest first: a failed reload as
    /// `file:line:column: message`, a listener that threw, a refused transaction.
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// The id of the generation that runs. A restart or a successful reload raises it.
    pub fn generation_id(&self) -> u64 {
        self.shell.generation_id()
    }

    /// Whether the supervisor still allows restaging after a budget trip.
    pub fn health(&self) -> JsHealth {
        self.reloader.health()
    }

    /// The shell, for a caller that evaluates a probe in it.
    pub fn shell_mut(&mut self) -> &mut JsShell {
        &mut self.shell
    }
}

/// An autostart command: starts the program in a process group of its own with no terminal input or output, and
/// reaps it when it ends. The program and its arguments are run without a shell.
fn spawn_detached(request: &JsSpawnRequest) -> Result<(), JsSpawnError> {
    use std::os::unix::process::CommandExt as _;

    let JsSpawnRequest::Argv(argv) = request;
    let Some((program, arguments)) = argv.split_first() else {
        return Err(JsSpawnError("an empty argv names no program".to_owned()));
    };
    let mut command = Command::new(program);
    command.args(arguments);
    let name = program;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| JsSpawnError(format!("cannot start {name}: {error}")))?;
    // Nothing else waits for the child, and an unreaped one stays a zombie until the role exits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{config_dir, entry_in};

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn the_config_directory_is_xdg_config_home_then_home_dot_config() {
        let both = env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(config_dir(&both).as_deref(), Some(std::path::Path::new("/x/gnoblin")));
        let home = env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")]);
        assert_eq!(
            config_dir(&home).as_deref(),
            Some(std::path::Path::new("/h/.config/gnoblin"))
        );
        assert_eq!(config_dir(&env(&[])), None);
    }

    #[test]
    fn the_entry_is_init_ts_then_init_js_and_init_ts_when_neither_exists() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert_eq!(entry_in(dir.path()), dir.path().join("init.ts"));
        fs::write(dir.path().join("init.js"), "").expect("init.js");
        assert_eq!(entry_in(dir.path()), dir.path().join("init.js"));
        fs::write(dir.path().join("init.ts"), "").expect("init.ts");
        assert_eq!(entry_in(dir.path()), dir.path().join("init.ts"));
    }
}
