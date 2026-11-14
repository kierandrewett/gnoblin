//! `gnoblinctl config reload` reaching the JavaScript shell of a real session, as processes.
//!
//! The session is the shipped binary: a guardian that starts the compositor, the services role and the runtime
//! role, with `GNOBLIN_SHELL_VM=js` so the runtime role runs the JavaScript shell beside the Lua host. The
//! watcher is switched off (`GNOBLIN_JS_WATCH=0`), so a change to `init.ts` takes effect only when the request
//! comes through the compositor: `gnoblinctl config reload` is the real command line client, the compositor
//! raises `session.reload_requested`, the runtime role reloads the shell and answers. Windows are mapped by a
//! real Wayland client and the result is read back from the compositor's tree as an observer.

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
    /// Starts a session whose runtime role runs the JavaScript shell over `$XDG_CONFIG_HOME/gnoblin`, with the
    /// config watcher off. `init` is the `init.ts` the shell finds there at boot.
    fn start(init: &str) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let config_home = tempfile::tempdir().expect("a config home");
        std::fs::create_dir(config_home.path().join("gnoblin")).expect("the config directory");
        write_init(&config_home.path().join("gnoblin"), init);
        let log = directory.path().join("session.log");
        let file = std::fs::File::create(&log).expect("the log file");
        let bus = Bus::start();
        let guardian = Command::new(BIN)
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("XDG_CONFIG_HOME", config_home.path())
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .env("XDG_STATE_HOME", directory.path().join("state"))
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            .env("GNOBLIN_TEST_FAST", "1")
            .env("GNOBLIN_SHELL_VM", "js")
            .env("GNOBLIN_JS_WATCH", "0")
            // The compositor must run headless however the machine that runs the test is set up.
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("a duplicated log")))
            .stderr(Stdio::from(file))
            .spawn()
            .expect("the guardian starts");
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
