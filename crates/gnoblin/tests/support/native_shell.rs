//! A real `gnoblin` process that runs a native shell in its compositor (`design/modular-framework.md`, M3.3 and M3.5).
//!
//! Nothing here is linked into the test. The session is the shipped binary, which links `libgnoblin-core.so.1` and serves the
//! compositor role from it, started on a headless backend with its sockets in a temporary directory; the shell is a `.so`
//! built from the workspace's own crates (the two examples and the fault fixture) and loaded by the compositor on its shell
//! thread. The test reads what the session holds through observer connections to the tree, and what the process says through
//! its log, which each line of is stamped with the time the test read it, so a duration is measured from outside the shell
//! thread, as design section 19 asks.
//!
//! The binary finds the core through its run path (`$ORIGIN`), so the core must be staged beside it: `cargo xtask stage`, or a
//! workspace build, which stages it (`crates/gnoblin-core-impl/build.rs`).
#![allow(dead_code)]

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// The binary under test, built by cargo for this test.
pub const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

/// How long a test waits for a state before it fails. A passing run never waits this long.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// How often a wait looks at the state again.
pub const POLL: Duration = Duration::from_millis(25);

// ---- the shells -----------------------------------------------------------------------------------------------------

/// The native shells the tests run, built once per test binary.
pub struct Shells {
    /// Example 1: places windows in a cascade, starts a terminal on `Super+Return`, closes the focused window on `Super+q`.
    pub minimal: PathBuf,
    /// Example 8: tiles the windows master-and-stack; `Super+Return` starts a terminal.
    pub tiler: PathBuf,
    /// The fault fixture: misbehaves when its directory says so.
    pub fault: PathBuf,
}

/// Builds the shells into a target directory of their own, so this does not wait for the lock of the one the test run holds.
/// A shell is the SDK and its own code, so this compiles little.
pub fn shells() -> &'static Shells {
    static SHELLS: LazyLock<Result<Shells, String>> = LazyLock::new(build_shells);
    match &*SHELLS {
        Ok(shells) => shells,
        Err(message) => panic!("cannot build the native shells: {message}"),
    }
}

fn build_shells() -> Result<Shells, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native-shells");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--locked", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args([
            "-p",
            "gnoblin-example-minimal-core-shell",
            "-p",
            "gnoblin-example-no-workspaces",
            "-p",
            "gnoblin-fixture-shell-fault",
        ])
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build of the shells failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let debug = target.join("debug");
    Ok(Shells {
        minimal: debug.join("libminimal_core_shell.so"),
        tiler: debug.join("libno_workspaces.so"),
        fault: debug.join("libfixture_shell_fault.so"),
    })
}

// ---- the process -----------------------------------------------------------------------------------------------------

/// One line the process wrote, and when this test read it.
#[derive(Clone, Debug)]
pub struct Line {
    pub at: Instant,
    pub text: String,
}

/// What a session is started as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// `gnoblin --role compositor PATH`: the compositor alone, with the shell in its process.
    Compositor,
    /// `gnoblin PATH`: the guardian, which starts the compositor with the path and the other children without a runtime.
    Guardian,
}

/// A private message bus for a guardian session, killed on drop (as `tests/guardian.rs` does).
struct Bus {
    child: Child,
    address: String,
}

impl Bus {
    fn start() -> Bus {
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
        Bus { child, address }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A running `gnoblin`, its log and the directories it owns. Killed on drop.
pub struct Gnoblin {
    child: Option<Child>,
    directory: tempfile::TempDir,
    fixture: PathBuf,
    lines: Arc<Mutex<Vec<Line>>>,
    _bus: Option<Bus>,
}

impl Gnoblin {
    /// Starts `gnoblin` with `shell` as `start` says. `extra` is added to the environment, and `prepared` names files (and what
    /// is in them) that are in the fixture directory before the shell loads, for a command it must find at its start.
    pub fn start(start: Start, shell: &Path, extra: &[(&str, &str)], prepared: &[(&str, &str)]) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let fixture = directory.path().join("fixture");
        fs::create_dir(&fixture).expect("the fixture directory");
        for (name, text) in prepared {
            fs::write(fixture.join(name), text).expect("a prepared command");
        }
        // A stand-in terminal: the shells start `foot`, and the test finds out that they did.
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).expect("the bin directory");
        let foot = bin.join("foot");
        fs::write(&foot, "#!/bin/sh\n: > \"$GNOBLIN_FIXTURE_DIR/terminal\"\n").expect("the stand-in terminal");
        fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).expect("the stand-in's mode");
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())
        );
        let mut command = Command::new(BIN);
        match start {
            Start::Compositor => {
                command.args(["--role", "compositor"]).arg(shell);
            }
            Start::Guardian => {
                command.arg(shell);
            }
        }
        let bus = (start == Start::Guardian).then(Bus::start);
        command
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("XDG_STATE_HOME", directory.path().join("state"))
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            // The session has to run headless however the machine that runs the test is set up.
            .env("GNOBLIN_BACKEND", "headless")
            .env("GNOBLIN_LOG", "info")
            .env("GNOBLIN_LOG_FORMAT", "plain")
            .env("GNOBLIN_FIXTURE_DIR", &fixture)
            .env("PATH", path)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("GNOBLIN_SHELL")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(bus) = &bus {
            command
                .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
                .env("GNOBLIN_TEST_FAST", "1");
        }
        for (key, value) in extra {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("gnoblin starts");
        let lines: Arc<Mutex<Vec<Line>>> = Arc::default();
        for stream in [
            child
                .stdout
                .take()
                .map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let lines = Arc::clone(&lines);
            std::thread::Builder::new()
                .name("gnoblin-log".to_owned())
                .spawn(move || {
                    for text in BufReader::new(stream).lines().map_while(Result::ok) {
                        lines
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(Line {
                                at: Instant::now(),
                                text,
                            });
                    }
                })
                .expect("the log reader");
        }
        Self {
            child: Some(child),
            directory,
            fixture,
            lines,
            _bus: bus,
        }
    }

    /// The process id of the process `gnoblin` was started as.
    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("the process is held").id()
    }

    /// The tree's socket.
    pub fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    /// The Wayland socket clients connect to.
    pub fn wayland(&self) -> PathBuf {
        self.directory.path().join(gnoblin_wayland::DEFAULT_SOCKET_NAME)
    }

    /// The directory the test and the fixture shell command each other through.
    pub fn fixture(&self) -> &Path {
        &self.fixture
    }

    /// Everything the process has written so far.
    pub fn log(&self) -> Vec<Line> {
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The log as text, for a failure message.
    pub fn log_text(&self) -> String {
        self.log()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The first line that satisfies `matches`, and when it was read.
    pub fn find(&self, matches: impl Fn(&str) -> bool) -> Option<Line> {
        self.log().into_iter().find(|line| matches(&line.text))
    }

    /// Whether the process has ended, and how.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.as_mut().and_then(|child| child.try_wait().ok().flatten())
    }

    /// Waits for the process to end, within `patience`.
    pub fn wait_exit(&mut self, patience: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + patience;
        loop {
            if let Some(status) = self.exited() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Waits until `check` holds, failing with the log if the process ends or the patience runs out.
    #[track_caller]
    pub fn wait_for(&mut self, what: &str, mut check: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if check(self) {
                return;
            }
            if let Some(status) = self.exited() {
                panic!("gnoblin ended ({status}) before: {what}\n{}", self.log_text());
            }
            assert!(Instant::now() < deadline, "never reached: {what}\n{}", self.log_text());
            std::thread::sleep(POLL);
        }
    }

    /// Waits until a line of the log satisfies `matches`, and returns it.
    #[track_caller]
    pub fn wait_log(&mut self, what: &str, matches: impl Fn(&str) -> bool) -> Line {
        self.wait_for(what, |session| session.find(&matches).is_some());
        self.find(matches).expect("the line was there")
    }

    /// Waits until the tree's socket accepts a connection, which is when the compositor serves.
    #[track_caller]
    pub fn wait_serving(&mut self) {
        let ipc = self.ipc();
        self.wait_for("the compositor serves its tree", |_| UnixStream::connect(&ipc).is_ok());
        let wayland = self.wayland();
        self.wait_for("the compositor serves Wayland", |_| wayland.exists());
    }

    /// What the fixture shell wrote to `name`, trimmed, if it has.
    pub fn noted(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.fixture.join(name))
            .ok()
            .map(|text| text.trim().to_owned())
    }

    /// Waits for the fixture shell to write `name`, and returns what it wrote.
    #[track_caller]
    pub fn wait_noted(&mut self, name: &str) -> String {
        self.wait_for(&format!("the fixture shell wrote `{name}`"), |session| {
            session.noted(name).is_some()
        });
        self.noted(name).expect("the note was there")
    }

    /// Tells the fixture shell to do `command` (`hang`, `panic`, `exit`, `fail-init`), with `text` in the file.
    pub fn command(&self, command: &str, text: &str) {
        fs::write(self.fixture.join(command), text).expect("the command file");
    }

    /// The maps of the process, as `/proc/<pid>/maps` has them: the paths of the files mapped.
    pub fn mapped_files(&self) -> Vec<String> {
        let maps = fs::read_to_string(format!("/proc/{}/maps", self.pid())).unwrap_or_default();
        let mut files: Vec<String> = maps
            .lines()
            .filter_map(|line| line.split_whitespace().nth(5))
            .filter(|path| path.starts_with('/'))
            .map(str::to_owned)
            .collect();
        files.sort();
        files.dedup();
        files
    }
}

impl Drop for Gnoblin {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // A guardian's children are in their own process groups; ending the guardian ends them in its stop order.
            let _ = rustix::process::kill_process(
                rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX)).expect("a pid"),
                rustix::process::Signal::TERM,
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(POLL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---- reading the scene ---------------------------------------------------------------------------------------------------

/// The size of the first screen of the tree.
pub fn screen_size(ipc: &Path) -> (f64, f64) {
    let nodes = super::snapshot(ipc);
    let screen = nodes
        .iter()
        .find(|node| node.class == "screen")
        .expect("the tree has a screen");
    let value = |key: &str| {
        screen
            .props
            .iter()
            .find(|prop| prop.key == key)
            .map_or(0.0, |prop| prop.value_double)
    };
    (value("width"), value("height"))
}

/// Waits (up to [`PATIENCE`]) until the window of `app_id` is placed by `places`: a predicate over its rectangle. A failure
/// says what the tree held, which is what the shell had done by then.
#[track_caller]
pub fn wait_for_window(
    gnoblin: &mut Gnoblin,
    what: &str,
    app_id: &str,
    places: impl Fn(&super::Window) -> bool,
) -> super::Window {
    let ipc = gnoblin.ipc();
    let deadline = Instant::now() + PATIENCE;
    loop {
        let windows = super::scene(&ipc);
        if let Some(window) = windows.iter().find(|window| window.app_id == app_id && places(window)) {
            return window.clone();
        }
        if let Some(status) = gnoblin.exited() {
            panic!("gnoblin ended ({status}) before: {what}\n{}", gnoblin.log_text());
        }
        assert!(
            Instant::now() < deadline,
            "never reached: {what}\nthe tree held {windows:?} on a screen of {:?}\nnodes (id, class, parent): {:?}\n{}",
            screen_size(&ipc),
            super::snapshot(&ipc)
                .iter()
                .map(|node| (node.id, node.class.clone(), node.parent))
                .collect::<Vec<_>>(),
            gnoblin.log_text()
        );
        std::thread::sleep(POLL);
    }
}

// ---- typing --------------------------------------------------------------------------------------------------------------

/// A keymap that compiles against the system's xkb data.
const KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
    xkb_geometry { include \"pc(pc105)\" };\n\
};\n";

/// evdev codes of the keys the tests press.
pub const KEY_LEFTMETA: u32 = 125;
pub const KEY_ENTER: u32 = 28;
pub const KEY_Q: u32 = 16;
pub const KEY_M: u32 = 50;

/// The `Mod4` bit of an xkb modifier mask, which `pc+us` gives the Super keys.
const MOD_SUPER: u32 = 1 << 6;

struct Typing;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Typing {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Typing: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Typing: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(Typing: ignore ZwpVirtualKeyboardV1);

/// A virtual keyboard on the session's own socket: the way an on-screen keyboard types, and the one door into a session
/// that runs as a process, which has no input tap.
pub struct Typist {
    _connection: Connection,
    queue: EventQueue<Typing>,
    keyboard: ZwpVirtualKeyboardV1,
    clock: u32,
}

impl Typist {
    pub fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<Typing>(&connection).expect("the global list");
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises zwp_virtual_keyboard_manager_v1");
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let mut keymap = tempfile::tempfile().expect("a file for the keymap");
        std::io::Write::write_all(&mut keymap, KEYMAP.as_bytes()).expect("the keymap is written");
        keyboard.keymap(1, keymap.as_fd(), u32::try_from(KEYMAP.len()).expect("a small keymap"));
        queue.roundtrip(&mut Typing).expect("the keymap is delivered");
        Self {
            _connection: connection,
            queue,
            keyboard,
            clock: 1000,
        }
    }

    fn key(&mut self, code: u32, pressed: bool) {
        self.clock += 10;
        self.keyboard.key(self.clock, code, u32::from(pressed));
    }

    /// Presses `Super` and `key` together and lets both go.
    pub fn super_plus(&mut self, key: u32) {
        self.key(KEY_LEFTMETA, true);
        self.keyboard.modifiers(MOD_SUPER, 0, 0, 0);
        self.key(key, true);
        self.key(key, false);
        self.keyboard.modifiers(0, 0, 0, 0);
        self.key(KEY_LEFTMETA, false);
        self.queue.roundtrip(&mut Typing).expect("the keys are delivered");
    }
}
