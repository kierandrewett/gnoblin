//! `gnoblinctl config reload` and the configuration document of a JavaScript shell in a real session, as processes.
//!
//! The session is the shipped binary: a guardian that resolves the shell it finds (`shell.ts` in the config directory), starts the
//! compositor with it in its process, and the services role. Unless a test is about the watcher, the watcher is switched off
//! (`GNOBLIN_JS_WATCH=0`), so a change to `shell.ts` takes effect only when the request comes through the compositor:
//! `gnoblinctl config reload` is the real command line client, the compositor asks the shell to reload and answers with what it
//! committed. Windows are mapped by a real Wayland client and the result is read back from the compositor's tree as an observer.

use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use gnoblinctl::SocketConnection;
use rustix::process::{Pid, Signal};
use tempfile::TempDir;

mod support;
use support::native_shell::{BIN, Prefix, RunCopy as _, script_environment};
use support::{Mapper, ids, scene, spots};

/// How long a test waits for a state before it fails.
const PATIENCE: Duration = Duration::from_secs(60);

/// A private message bus for one session, killed on drop (the portal child owns a name on the session bus).
struct Bus {
    child: Child,
    address: String,
}

impl Bus {
    fn start() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is required by the guardian tests");
        let stdout = child.stdout.take().expect("stdout is piped");
        let address = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .find(|line| line.starts_with("unix:"))
            .expect("dbus-daemon prints the address of its bus");
        Self { child, address }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A running session: the guardian process, its log, and the directory it owns.
struct Session {
    directory: TempDir,
    config_home: TempDir,
    guardian: Option<Child>,
    log: PathBuf,
    _bus: Bus,
}

impl Session {
    /// Starts a session over `$XDG_CONFIG_HOME/gnoblin` with the config watcher off. `init` is the `shell.ts` the shell finds there at boot.
    fn start(init: &str) -> Self {
        Self::start_with(&[("shell.ts", init)], &[("GNOBLIN_JS_WATCH", "0")])
    }

    /// Starts a session whose config directory holds `files`, with `env` added to the guardian's environment.
    fn start_with(files: &[(&str, &str)], env: &[(&str, &str)]) -> Self {
        Self::start_binary(Path::new(BIN), files, env)
    }

    /// [`Session::start_with`] with the launcher at `binary` (an installed prefix's, to see its packaged default).
    fn start_binary(binary: &Path, files: &[(&str, &str)], env: &[(&str, &str)]) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let config_home = tempfile::tempdir().expect("a config home");
        std::fs::create_dir(config_home.path().join("gnoblin")).expect("the config directory");
        for (name, source) in files {
            std::fs::write(config_home.path().join("gnoblin").join(name), source).expect("a config file");
        }
        let log = directory.path().join("session.log");
        let file = std::fs::File::create(&log).expect("the log file");
        let bus = Bus::start();
        let mut command = Command::new(binary);
        command
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("XDG_CONFIG_HOME", config_home.path())
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .env("XDG_STATE_HOME", directory.path().join("state"))
            // The keyring's store lives under the data home, so the test must not write the machine's.
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            .env("GNOBLIN_TEST_FAST", "1")
            // The session has to run headless however the machine that runs the test is set up: `auto` would nest in a
            // desktop that sets WAYLAND_DISPLAY, or take the seat of a login session.
            .env("GNOBLIN_BACKEND", "headless")
            .env("GNOBLIN_LOG", "info")
            .env("GNOBLIN_LOG_FORMAT", "plain")
            .env_remove("GNOBLIN_JS_WATCH")
            .env_remove("GNOBLIN_SHELL")
            // The compositor must run headless however the machine that runs the test is set up.
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("a duplicated log")))
            .stderr(Stdio::from(file));
        // Where the runtimes, the stock library and the Python package are: the source tree stands for an installed prefix.
        for (name, value) in script_environment() {
            command.env(name, value);
        }
        for (name, value) in env {
            command.env(name, value);
        }
        let guardian = command.spawn_copy().expect("the guardian starts");
        Self {
            directory,
            config_home,
            guardian: Some(guardian),
            log,
            _bus: bus,
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Waits until the log contains `text`, and returns the whole log.
    #[track_caller]
    fn wait_for_log(&self, text: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let log = self.log();
            if log.contains(text) {
                return log;
            }
            assert!(
                Instant::now() < deadline,
                "the session never logged `{text}`; it said:\n{log}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    /// Where the compositor serves Wayland clients: the default name in the runtime directory.
    fn wayland(&self) -> PathBuf {
        self.directory.path().join("gnoblin-wayland.sock")
    }

    fn config_dir(&self) -> PathBuf {
        self.config_home.path().join("gnoblin")
    }

    /// Runs the real `gnoblinctl` entry point against the session and returns its exit code, standard output and
    /// standard error.
    fn ctl(&self, args: &[&str]) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = gnoblinctl::run(
            args.iter().map(OsString::from),
            SocketConnection::at(self.ipc()),
            &mut out,
            &mut err,
        );
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    /// A Wayland client on the session, once the compositor serves it.
    fn mapper(&self) -> Mapper {
        let deadline = Instant::now() + PATIENCE;
        while UnixStream::connect(self.wayland()).is_err() {
            assert!(
                Instant::now() < deadline,
                "the compositor never served Wayland; it said:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Mapper::connect(&self.wayland())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let pids: Vec<i32> = self
            .log()
            .lines()
            .filter_map(|line| line.split("pid ").nth(1)?.split(')').next()?.trim().parse().ok())
            .collect();
        if let Some(mut guardian) = self.guardian.take() {
            let _ = guardian.kill();
            let _ = guardian.wait();
        }
        for pid in pids {
            if let Some(id) = Pid::from_raw(pid) {
                let _ = rustix::process::kill_process(id, Signal::KILL);
            }
        }
    }
}

/// Waits until `check` holds, keeping the Wayland client answering pings, or fails with `what` and the session's log.
#[track_caller]
fn wait_for(what: &str, session: &Session, mapper: &mut Mapper, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        mapper.roundtrip();
        if check() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never reached: {what}; the scene {:?}; the session said:\n{}",
            scene(&session.ipc()),
            session.log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A `shell.ts` that puts every window at (`x`, `y`) when the shell learns of it.
fn init_at(x: i32, y: i32) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin-core\"; import {{ Layout, Workspace }} from \"gnoblin-std\";\n\
         \n\
         // A layout that places nothing, so the window stays where the listener put it.\n\
         class Keep extends Layout {{\n\
         \x20   compute(): [] {{\n\
         \x20       return [];\n\
         \x20   }}\n\
         }}\n\
         gnoblin.workspaces.add(new Workspace(\"desk\", {{ layout: new Keep() }}));\n\
         gnoblin.clients.addListener({{\n\
         \x20   onAdded(client: {{ x: number; y: number }}): void {{\n\
         \x20       client.x = {x};\n\
         \x20       client.y = {y};\n\
         \x20   }},\n\
         }});\n"
    )
}

fn write_init(dir: &Path, source: &str) {
    std::fs::write(dir.join("shell.ts"), source).expect("shell.ts");
}

#[test]
fn config_reload_reloads_the_javascript_shell_in_the_runtime_role_and_reports_a_broken_file_with_its_position() {
    let session = Session::start(&init_at(111, 222));
    session.wait_for_log("configuration revision 1 accepted");
    let ipc = session.ipc();
    let mut mapper = session.mapper();
    mapper.map_window("org.example.first", "First");
    wait_for("the first file places the window", &session, &mut mapper, || {
        spots(&scene(&ipc)).get("org.example.first") == Some(&(111.0, 222.0))
    });
    let id = ids(&scene(&ipc));

    // The watcher is off: an edit alone changes nothing, so what follows is the request's doing.
    write_init(&session.config_dir(), &init_at(333, 444));
    std::thread::sleep(Duration::from_millis(700));
    mapper.roundtrip();
    assert_eq!(
        spots(&scene(&ipc)).get("org.example.first"),
        Some(&(111.0, 222.0)),
        "nothing reloads the file on its own"
    );

    // `gnoblinctl config reload` is the request: it exits 0 and the shell takes the new file, with the same window.
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    assert!(out.starts_with("reloaded: revision "), "{out}");
    wait_for("the reload places the window again", &session, &mut mapper, || {
        spots(&scene(&ipc)).get("org.example.first") == Some(&(333.0, 444.0))
    });
    assert_eq!(ids(&scene(&ipc)), id, "the reload kept the window id");

    // A file that fails: the request is refused with the file, line and column, the exit code is 1, and the running
    // generation keeps placing windows.
    write_init(
        &session.config_dir(),
        "import { gnoblin } from \"gnoblin-core\";\n\nconst broken: number = = 5;\n",
    );
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!(code, 1, "a refused reload exits 1; stdout {out}, stderr {err}");
    assert!(err.contains("shell.ts:3:"), "the report names the position: {err}");
    mapper.map_window("org.example.second", "Second");
    wait_for("the old generation places a new window", &session, &mut mapper, || {
        spots(&scene(&ipc)).get("org.example.second") == Some(&(333.0, 444.0))
    });

    // Fixing it and asking again recovers.
    write_init(&session.config_dir(), &init_at(555, 666));
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    wait_for("the fixed file takes effect", &session, &mut mapper, || {
        let now = spots(&scene(&ipc));
        now.get("org.example.first") == Some(&(555.0, 666.0)) && now.get("org.example.second") == Some(&(555.0, 666.0))
    });
}

// ---- the JavaScript shell owns the configuration -------------------------------------------------------------------

/// The `document` of the configuration the compositor holds, and its revision, read through `gnoblinctl`.
fn committed(session: &Session) -> (u64, serde_json::Value) {
    let (code, out, err) = session.ctl(&["--json", "config", "get"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    let answer: serde_json::Value = serde_json::from_str(&out).expect("`config get --json` prints JSON");
    (
        answer["revision"].as_u64().expect("a revision"),
        answer["document"].clone(),
    )
}

/// Waits until the guardian has logged `revision` as accepted, and returns the document the compositor holds.
#[track_caller]
fn wait_for_revision(session: &Session, revision: u64) -> serde_json::Value {
    session.wait_for_log(&format!("configuration revision {revision} accepted"));
    committed(session).1
}

/// A `shell.ts` that runs `setting`, a write to the compositor's own settings, which the document carries in its `compositor`
/// section. The session is headless and has no input device, and a script cannot make one (a device is reported by the seat), so
/// the setting is one that belongs to no device.
fn init_with(setting: &str) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin-core\";\n\
         \n\
         {setting}\n"
    )
}

/// A `shell.ts` whose document is `compositor.user-shaders = enabled`.
fn init_with_shaders(enabled: bool) -> String {
    init_with(&format!("gnoblin.compositor.userShaders = {enabled};"))
}

/// The `compositor.user-shaders` of a committed document.
fn shaders_of(document: &serde_json::Value) -> &serde_json::Value {
    &document["compositor"]["user-shaders"]
}

#[test]
fn the_default_session_runs_shell_ts_in_the_compositor_and_commits_its_document() {
    // The shell read `shell.ts`, validated the document and committed it.
    let session = Session::start_with(&[("shell.ts", &init_with_shaders(false))], &[("GNOBLIN_JS_WATCH", "0")]);
    let document = wait_for_revision(&session, 1);
    assert_eq!(shaders_of(&document), &serde_json::json!(false), "{document}");
    let log = session.log();
    assert!(log.contains("serves in process"), "{log}");
}

#[test]
fn a_session_with_no_shell_of_the_users_runs_the_packaged_starter_and_commits_its_document() {
    // First login: no `shell.ts` yet. The packaged default is the starter, which the launcher finds in the prefix it is installed
    // in; its document is committed, so the guardian has a revision and the session comes up.
    let prefix = Prefix::new();
    std::fs::create_dir_all(prefix.default_shell_dir()).expect("the default shell's directory");
    std::fs::copy(
        support::native_shell::repo_root().join("shells/default/shell.ts"),
        prefix.default_shell_dir().join("shell.ts"),
    )
    .expect("the starter, installed");
    let session = Session::start_binary(&prefix.binary(), &[], &[("GNOBLIN_JS_WATCH", "0")]);
    let (revision, _) = {
        session.wait_for_log("configuration revision 1 accepted");
        committed(&session)
    };
    assert_eq!(revision, 1);
    session.wait_for_log("[guardian] session ready");
}

#[test]
fn config_reload_commits_the_new_document_and_a_refused_file_leaves_the_accepted_one() {
    let session = Session::start(&init_with_shaders(false));
    let document = wait_for_revision(&session, 1);
    assert_eq!(shaders_of(&document), &serde_json::json!(false));

    write_init(&session.config_dir(), &init_with_shaders(true));
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    assert!(out.starts_with("reloaded: revision 2"), "{out}");
    assert_eq!(shaders_of(&committed(&session).1), &serde_json::json!(true));

    // A value the setter refuses fails the reload with the property named as the shell wrote it (`gnoblin.compositor.userShaders`), not by
    // its schema name (`compositor_settings.user_shaders`), and revision 2 stays in force.
    write_init(
        &session.config_dir(),
        &init_with("gnoblin.compositor.userShaders = \"huge\";"),
    );
    let (code, _, err) = session.ctl(&["config", "reload"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("CompositorSettings.userShaders"),
        "the refusal names the property: {err}"
    );
    let (revision, document) = committed(&session);
    assert_eq!(revision, 2, "nothing new was committed");
    assert_eq!(shaders_of(&document), &serde_json::json!(true));

    // The role kept serving: the fixed file commits as revision 3 from the same process.
    write_init(&session.config_dir(), &init_with_shaders(false));
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    assert!(out.starts_with("reloaded: revision 3"), "{out}");
    assert_eq!(shaders_of(&committed(&session).1), &serde_json::json!(false));
}

#[test]
fn saving_init_ts_reloads_it_with_the_watcher_on_and_commits_a_new_revision() {
    // The watcher is on (the default): nothing asks for the reload, a save is the request.
    let session = Session::start_with(&[("shell.ts", &init_with_shaders(false))], &[]);
    wait_for_revision(&session, 1);
    write_init(&session.config_dir(), &init_with_shaders(true));
    let document = wait_for_revision(&session, 2);
    assert_eq!(shaders_of(&document), &serde_json::json!(true), "{document}");
}
