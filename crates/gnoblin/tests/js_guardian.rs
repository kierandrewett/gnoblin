//! `gnoblinctl config reload` reaching the JavaScript shell of a real session, as processes.
//!
//! The session is the shipped binary: a guardian that starts the compositor, the services role and the runtime
//! role. The runtime role is the JavaScript one (`gnoblin_js::JsRoleRunner`) and nothing selects it. Unless a test is
//! about the watcher, it is switched off
//! (`GNOBLIN_JS_WATCH=0`), so a change to `init.ts` takes effect only when the request comes through the
//! compositor: `gnoblinctl config reload` is the real command line client, the compositor raises
//! `session.reload_requested`, the runtime role reloads the shell and answers. Windows are mapped by a real
//! Wayland client and the result is read back from the compositor's tree as an observer.

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
use support::{Mapper, Watcher, ids, scene, spots};

/// The binary under test, built by cargo for this test.
const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

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
    /// Starts a session over `$XDG_CONFIG_HOME/gnoblin` whose runtime role is the default one, with the config
    /// watcher off. `init` is the `init.ts` the shell finds there at boot.
    fn start(init: &str) -> Self {
        Self::start_with(&[("init.ts", init)], &[("GNOBLIN_JS_WATCH", "0")])
    }

    /// Starts a session whose config directory holds `files`, with `env` added to the guardian's environment. The
    /// runtime role is the JavaScript one whatever `env` says.
    fn start_with(files: &[(&str, &str)], env: &[(&str, &str)]) -> Self {
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
        let mut command = Command::new(BIN);
        command
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("XDG_CONFIG_HOME", config_home.path())
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .env("XDG_STATE_HOME", directory.path().join("state"))
            // The keyring's store lives under the data home, so the test must not write the machine's.
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            .env("GNOBLIN_TEST_FAST", "1")
            // The session must not inherit the machine's own choice of stock library.
            .env_remove("GNOBLIN_JS_STOCK_DIR")
            .env_remove("GNOBLIN_JS_WATCH")
            .env_remove("GNOBLIN_CONFIG")
            // The compositor must run headless however the machine that runs the test is set up.
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("a duplicated log")))
            .stderr(Stdio::from(file));
        for (name, value) in env {
            command.env(name, value);
        }
        let guardian = command.spawn().expect("the guardian starts");
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

/// An `init.ts` that puts every window at (`x`, `y`) when the shell learns of it.
fn init_at(x: i32, y: i32) -> String {
    format!(
        "import {{ Layout, Workspace, gnoblin }} from \"gnoblin\";\n\
         \n\
         gnoblin.workspaces.add(new Workspace({{ id: \"desk\", layout: new Layout() }}));\n\
         gnoblin.clients.addListener({{\n\
         \x20   onAdded(client: {{ x: number; y: number }}): void {{\n\
         \x20       client.x = {x};\n\
         \x20       client.y = {y};\n\
         \x20   }},\n\
         }});\n"
    )
}

fn write_init(dir: &Path, source: &str) {
    std::fs::write(dir.join("init.ts"), source).expect("init.ts");
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
        "import { gnoblin } from \"gnoblin\";\n\nconst broken: number = = 5;\n",
    );
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!(code, 1, "a refused reload exits 1; stdout {out}, stderr {err}");
    assert!(err.contains("init.ts:3:"), "the report names the position: {err}");
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

/// The pid on the latest `started runtime (generation N, pid P)` line of the guardian's log.
fn runtime_pid(log: &str) -> i32 {
    log.lines()
        .filter(|line| line.contains("started runtime ("))
        .filter_map(|line| line.split("pid ").nth(1)?.split(')').next()?.trim().parse().ok())
        .next_back()
        .unwrap_or_else(|| panic!("no runtime pid in:\n{log}"))
}

#[test]
fn killing_the_runtime_process_hands_the_windows_to_the_fallback_policy_until_the_restarted_shell_places_them_again() {
    let session = Session::start(&init_at(111, 222));
    session.wait_for_log("configuration revision 1 accepted");
    let ipc = session.ipc();
    // Attached before any window maps, so every place the compositor publishes is heard in order.
    let watch = Watcher::attach(&ipc);
    let mut mapper = session.mapper();
    mapper.map_window("org.example.first", "First");
    mapper.map_window("org.example.second", "Second");
    let placed = |now: &std::collections::BTreeMap<String, (f64, f64)>| {
        now.get("org.example.first") == Some(&(111.0, 222.0)) && now.get("org.example.second") == Some(&(111.0, 222.0))
    };
    wait_for("the shell places both windows", &session, &mut mapper, || {
        placed(&spots(&watch.scene()))
    });
    let id = ids(&watch.scene());
    let heard = (
        watch.places("org.example.first").len(),
        watch.places("org.example.second").len(),
    );

    // The shell's process dies, as a crash does. The compositor keeps serving, so the scene still answers, and the
    // fallback policy has the windows back: each goes to the place it was given before the shell moved it. Then the
    // guardian starts the runtime again, and its shell attaches, learns of both windows and places them again.
    let pid = runtime_pid(&session.log());
    rustix::process::kill_process(Pid::from_raw(pid).expect("a pid"), Signal::KILL).expect("the runtime is killed");
    session.wait_for_log("started runtime (generation 2");
    wait_for(
        "the restarted shell places both windows again",
        &session,
        &mut mapper,
        || {
            let first = watch.places("org.example.first");
            let second = watch.places("org.example.second");
            first[heard.0..] == [(32.0, 32.0), (111.0, 222.0)] && second[heard.1..] == [(64.0, 64.0), (111.0, 222.0)]
        },
    );
    assert_eq!(ids(&watch.scene()), id, "no window was re-created");
    assert_eq!(
        scene(&session.ipc()).len(),
        2,
        "the compositor answers a fresh connection"
    );
}

// ---- the JavaScript role owns the configuration -------------------------------------------------------------------

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

const CURSOR_32: &str =
    "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ cursor: { size: 32 } });\n";

#[test]
fn the_default_session_runs_the_javascript_role_and_commits_init_ts() {
    // The JavaScript role read `init.ts`, validated the document and committed it.
    let session = Session::start_with(&[("init.ts", CURSOR_32)], &[("GNOBLIN_JS_WATCH", "0")]);
    let document = wait_for_revision(&session, 1);
    assert_eq!(document["cursor"]["size"], serde_json::json!(32), "{document}");
    let log = session.log();
    assert!(log.contains("runtime role: JavaScript"), "{log}");
    assert!(!log.contains("gnoblinctl config migrate"), "{log}");
}

#[test]
fn a_session_with_no_config_commits_the_stock_config_and_reports_ready() {
    // First login: no `init.ts` yet. The role runs the stock config and commits its (empty) document, so the guardian
    // has a revision and the session comes up.
    let session = Session::start_with(&[], &[("GNOBLIN_JS_WATCH", "0")]);
    let document = wait_for_revision(&session, 1);
    assert_eq!(document, serde_json::json!({}), "the stock config sets no document key");
    session.wait_for_log("[guardian] session ready");
}

#[test]
fn config_reload_commits_the_new_document_and_a_refused_file_leaves_the_accepted_one() {
    let session = Session::start(CURSOR_32);
    let document = wait_for_revision(&session, 1);
    assert_eq!(document["cursor"]["size"], serde_json::json!(32));

    write_init(
        &session.config_dir(),
        "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ cursor: { size: 48 } });\n",
    );
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    assert!(out.starts_with("reloaded: revision 2"), "{out}");
    assert_eq!(committed(&session).1["cursor"]["size"], serde_json::json!(48));

    // A value the validator refuses fails the reload with the key path, and revision 2 stays in force.
    write_init(
        &session.config_dir(),
        "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ cursor: { size: \"huge\" } });\n",
    );
    let (code, _, err) = session.ctl(&["config", "reload"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("cursor"), "the refusal names the key: {err}");
    let (revision, document) = committed(&session);
    assert_eq!(revision, 2, "nothing new was committed");
    assert_eq!(document["cursor"]["size"], serde_json::json!(48));

    // The role kept serving: the fixed file commits as revision 3 from the same process.
    write_init(&session.config_dir(), CURSOR_32);
    let (code, out, err) = session.ctl(&["config", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}");
    assert!(out.starts_with("reloaded: revision 3"), "{out}");
    assert_eq!(committed(&session).1["cursor"]["size"], serde_json::json!(32));
}

#[test]
fn saving_init_ts_reloads_it_with_the_watcher_on_and_commits_a_new_revision() {
    // The watcher is on (the default): nothing asks for the reload, a save is the request.
    let session = Session::start_with(&[("init.ts", CURSOR_32)], &[]);
    wait_for_revision(&session, 1);
    write_init(
        &session.config_dir(),
        "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ cursor: { size: 40 } });\n",
    );
    let document = wait_for_revision(&session, 2);
    assert_eq!(document["cursor"]["size"], serde_json::json!(40), "{document}");
}

#[test]
fn a_restarted_runtime_commits_what_the_file_says_and_not_what_its_predecessor_left() {
    // The compositor keeps the last accepted document across a runtime restart. A role that laid its settings over
    // it would carry a key the user has since deleted from `init.ts` into the new session.
    let session = Session::start(CURSOR_32);
    let document = wait_for_revision(&session, 1);
    assert_eq!(document["cursor"]["size"], serde_json::json!(32));
    write_init(
        &session.config_dir(),
        "import { defineConfig } from \"gnoblin\";\nexport default defineConfig({ keyring: { enabled: true } });\n",
    );
    let pid = runtime_pid(&session.log());
    rustix::process::kill_process(Pid::from_raw(pid).expect("a pid"), Signal::KILL).expect("the runtime is killed");
    let document = wait_for_revision(&session, 2);
    assert!(
        document.get("cursor").is_none(),
        "the deleted key came back: {document}"
    );
    assert_eq!(document["keyring"]["enabled"], serde_json::json!(true));
}

#[test]
fn autostart_commands_are_handed_to_the_guardian_which_starts_them() {
    let marker = tempfile::tempdir().expect("a marker directory");
    let file = marker.path().join("started");
    let init = format!(
        "import {{ defineConfig }} from \"gnoblin\";\nexport default defineConfig({{ autostart: [[\"sh\", \"-c\", \"echo started > {}\"]] }});\n",
        file.display()
    );
    let session = Session::start(&init);
    let document = wait_for_revision(&session, 1);
    assert_eq!(
        document["autostart"]["sh"]["command"][0],
        serde_json::json!("sh"),
        "the commands are the document's autostart section: {document}"
    );
    let deadline = Instant::now() + PATIENCE;
    while !file.exists() {
        assert!(
            Instant::now() < deadline,
            "the guardian never started the command; it said:\n{}",
            session.log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let log = session.log();
    assert!(log.contains("autostart sh"), "the guardian reports the entry:\n{log}");
}

#[test]
fn an_init_lua_with_no_init_ts_runs_the_stock_config_and_says_once_how_to_migrate() {
    // Nothing reads Lua, so the stock config runs; one warning names the file, the command and the page.
    let session = Session::start_with(
        &[("init.lua", "gnoblin.configure({ cursor = { size = 36 } })\n")],
        &[("GNOBLIN_JS_WATCH", "0")],
    );
    let document = wait_for_revision(&session, 1);
    assert_eq!(document, serde_json::json!({}), "the Lua file is not read: {document}");
    let log = session.log();
    let starts = log.matches("runtime role: JavaScript").count();
    assert!(starts >= 1, "{log}");
    // Once for each runtime process: the guardian restarts one whose stock config ran past its wall-clock budget on a
    // machine under load, and the new process says it again. What must not happen is two warnings from one process.
    assert_eq!(
        log.matches("gnoblinctl config migrate").count(),
        starts,
        "one warning for each runtime start:\n{log}"
    );
    assert!(
        log.contains("docs/migrating-from-lua.md"),
        "the warning names the page:\n{log}"
    );
}

#[test]
fn an_init_ts_beside_an_init_lua_wins_and_the_lua_file_is_named_as_ignored() {
    let session = Session::start_with(
        &[
            ("init.ts", CURSOR_32),
            ("init.lua", "gnoblin.configure({ cursor = { size = 36 } })\n"),
        ],
        &[("GNOBLIN_JS_WATCH", "0")],
    );
    let document = wait_for_revision(&session, 1);
    assert_eq!(
        document["cursor"]["size"],
        serde_json::json!(32),
        "init.ts won: {document}"
    );
    let log = session.log();
    assert!(log.contains("runtime role: JavaScript"), "{log}");
    assert!(
        log.contains("init.lua is not read"),
        "the ignored file is named:\n{log}"
    );
    assert!(!log.contains("gnoblinctl config migrate"), "{log}");
}

#[test]
fn gnoblin_shell_vm_selects_nothing_any_more() {
    // The variable that used to pick the Lua role is not read: a session that still sets it runs the JavaScript role.
    let session = Session::start_with(
        &[("init.ts", CURSOR_32)],
        &[("GNOBLIN_SHELL_VM", "lua"), ("GNOBLIN_JS_WATCH", "0")],
    );
    let document = wait_for_revision(&session, 1);
    assert_eq!(document["cursor"]["size"], serde_json::json!(32), "{document}");
    let log = session.log();
    assert!(log.contains("runtime role: JavaScript"), "{log}");
}
