//! A soak of the real session's memory: the compositor, the stock shell and a client that churns
//! (tests/gpu-memory-soak/DESIGN.md).
//!
//! The session is [`Compositor::run`] on a headless backend with the renderer the soak is told to use, the stock
//! JavaScript shell runs over its owner socket as it does in a real session (a [`JsRole`] over a core with the std library
//! loaded, as `js_shell.rs` runs it), and a Wayland client opens, resizes and closes windows, takes screenshots, toggles
//! supersampling and hotplugs a second output, for as long as it is told to. The shell is checked alive every cycle: a
//! shell that ended, stalled or degraded fails the soak at that time, a panic of its thread or of the compositor's is the
//! soak's failure with the thread's own message, and at the end the shell's own script must have counted the stock entry's
//! workspaces and the windows the churn opened. Every sample interval one JSON line records what the process holds
//! (resident set, threads, descriptors) and what the compositor says it holds for each output (the `gpu_memory_*`
//! properties of its screen nodes), so growth shows as a series and the allocator that grows is named by the column that
//! rises.
//!
//! It is `#[ignore]`: a soak is minutes, not a unit test. Run it on purpose:
//!
//! ```text
//! GNOBLIN_SOAK_RENDERER=gles GNOBLIN_SOAK_SECONDS=1200 GNOBLIN_SOAK_OUT=soak-gles.jsonl \
//!     cargo test -p gnoblin --test gpu_memory_soak -- --ignored --nocapture
//! ```
//!
//! What it does not prove: a real GPU's memory (this crate counts what it asked the driver for, and a headless
//! software driver does not report video memory at all) and the DRM scan-out path.
//!
//! Two things are held in the churn on purpose. A capture session (`capture.rs`) stays open on the first output for
//! the whole run and is fed a frame whenever the last one ended. The session lock is taken and released every
//! `GNOBLIN_SOAK_LOCK_EVERY` cycles (25 by default; 0 for never) through the same tree call and the same keystrokes
//! as `js_lock.rs`, so the cover the lock draws and the compositor's password card come and go too (the stock starter
//! makes no `LockScreen`, so the card is what takes the password). A lock ends the capture session, and the one frame the
//! recorder wants of it as it stops is refused (`capture_frames_failed`, at most one per lock); the soak asserts that is
//! the only refusal there is.
//! `GNOBLIN_SOAK_RSS_LIMIT_MIB` (0 by default: none) ends the run, and fails it, when the process grows past it, so
//! a run that cannot stay bounded leaves its series instead of an OOM kill.
//!
//! `GNOBLIN_SOAK_BUDGET_MIB` (unset by default: the session's default budget) sets the compositor's memory budget to a
//! small value, and turns on a client that leans on it: every fifth cycle it asks for one buffer (`pressure.rs`), in
//! turn one that fits, one a little larger than what is left (which fits only if a cache is given back) and one as large
//! as the whole budget, is admitted or ended with `wl_display.error.no_memory`, and lets go at once. The series then
//! carries what the budget did (`refused_total`, `evicted_total`, `clients_bytes`) beside what the process held, which
//! is how a refusal that leaks or a session that does not survive one shows.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::{Compositor, InputTap, OutputTap, SessionConfig, StopHandle};
use gnoblin_config::RendererChoice;
use gnoblin_js::{JsRole, JsRoleConfig};
use gnoblin_proto::Version;
use gnoblin_shell_host::supervisor::Health;
use serde_json::json;

mod support;
use support::{Mapper, scene, snapshot};

#[path = "gpu_memory_soak/capture.rs"]
mod capture;
use capture::{Capture, Refusals};

#[path = "gpu_memory_soak/pressure.rs"]
mod pressure;

/// The only user the scripted password check knows, and the password it accepts.
const USER: &str = "kieran";
const PASSWORD_KEYS: [u32; 2] = [KEY_H, KEY_I];
const KEY_H: u32 = 35;
const KEY_I: u32 = 23;
const KEY_ENTER: u32 = 28;

/// The keyboard `support::plug_keyboard` announces.
const KEYBOARD: gnoblin_types::DeviceId = gnoblin_types::DeviceId::new(7);

/// How long the session may take to start listening.
const PATIENCE: Duration = Duration::from_secs(60);

/// What the stock shell may spend on one listener call, one frame and one staging. They are wall-clock budgets, and a soak keeps the
/// machine busy for minutes with a compositor, its renderer and a churning client, so the product's short ones would stop the
/// generation of an honest script for a stretch the machine did not schedule its thread through. The soak is not about the budgets.
const HONEST_BUDGET: Duration = Duration::from_secs(30);

/// How long the stock shell may run no frame before the soak calls it dead. A frame takes milliseconds and the longest the role lets
/// one run is [`HONEST_BUDGET`], so a shell that has gone this long without one is not slow.
const SHELL_STALL: Duration = Duration::from_secs(60);

/// The sizes the churn client opens and resizes windows to, in pixels: a small dialog up to a 1080p window.
const SIZES: [(i32, i32); 6] = [
    (320, 240),
    (800, 600),
    (1280, 720),
    (1920, 1080),
    (640, 480),
    (1024, 768),
];

/// The supersample factors the churn cycles an output through.
const FACTORS: [&str; 3] = ["2", "4", "1"];

/// How many windows the long-lived client keeps before it disconnects and starts over.
const MAX_WINDOWS: usize = 6;

/// The environment's value, or `default`.
fn setting(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

/// The renderer the soak runs on: `software`, `gles` or `vulkan`.
fn renderer() -> RendererChoice {
    match setting("GNOBLIN_SOAK_RENDERER", "software").as_str() {
        "software" => RendererChoice::Software,
        "gles" => RendererChoice::Gles,
        "vulkan" => RendererChoice::Vulkan,
        other => panic!("GNOBLIN_SOAK_RENDERER is software, gles or vulkan, not {other:?}"),
    }
}

/// A headless compositor on sockets of its own.
struct Session {
    directory: tempfile::TempDir,
    stop: StopHandle,
    outputs: OutputTap,
    input: InputTap,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    fn start(renderer: RendererChoice, limits: gnoblin_wayland::MemoryLimits) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let stop = StopHandle::new();
        let outputs = OutputTap::default();
        let input = InputTap::new();
        let mut config = SessionConfig {
            providers: support::stock_providers(),
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            output_tap: Some(outputs.clone()),
            input_tap: Some(input.clone()),
            renderer: Some(renderer),
            ..SessionConfig::default()
        };
        config.wayland.gpu_limits = limits;
        config.wayland.socket_path = Some(directory.path().join("wayland.sock"));
        config.wayland.privileged_socket_path = Some(directory.path().join("wayland-privileged.sock"));
        // The password check is scripted, as in `js_lock.rs`: PAM is not what a memory soak is about.
        config.wayland.lock_check = Some(gnoblin_wayland::LockCheck {
            authenticator: Arc::new(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, "hi")),
            user: Some(USER.to_owned()),
        });
        // A pause of the loop under load is not a hung application, so the liveness pings stay off.
        config.wayland.check_alive_timeout = Duration::ZERO;
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || {
                Compositor::run(config).expect("the compositor runs until it is stopped");
            })
            .expect("the session thread");
        let session = Self {
            directory,
            stop,
            outputs,
            input,
            thread: Some(thread),
        };
        let deadline = Instant::now() + PATIENCE;
        while UnixStream::connect(session.ipc()).is_err() {
            assert!(Instant::now() < deadline, "the compositor never listened");
            std::thread::sleep(Duration::from_millis(5));
        }
        support::plug_keyboard(&session.input, &session.ipc());
        session
    }

    fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    fn wayland(&self) -> PathBuf {
        self.directory.path().join("wayland.sock")
    }

    fn privileged(&self) -> PathBuf {
        self.directory.path().join("wayland-privileged.sock")
    }

    /// One key, pressed and released, on the keyboard the seat lists.
    fn key(&self, code: u32) {
        for pressed in [true, false] {
            assert!(self.input.inject(InputEvent::Key {
                device: KEYBOARD,
                keycode: code,
                pressed,
            }));
        }
    }

    /// Stops the compositor and waits for its thread. A panic of the thread (`Compositor::run` ended in an error) fails the test with the
    /// thread's own message. A second call does nothing.
    fn finish(&mut self) {
        self.stop.request();
        if let Some(thread) = self.thread.take() {
            support::join_worker(thread);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The stock shell: a [`JsRole`] over a core with the std library loaded, in a thread of its own as the runtime role is a process of its own.
///
/// The classes the stock entry makes are written over the core's object protocol (`__gnoblinObjects`) and std's natives, so the role is
/// built over a [`support::core_rig::CoreRig`] as the shells of `js_shell.rs` and `support/lock_session.rs` are. A role built without it
/// cannot load the entry ("the object classes need the JavaScript runtime library"), which is how this soak once ran with no shell at all
/// while the thread's panic was thrown away. Liveness is checked as the soak runs ([`Shell::assert_alive`]) and again when it ends
/// ([`Shell::finish`]), so a shell that is gone, stuck or degraded fails the soak.
struct Shell {
    stop: Arc<AtomicBool>,
    /// The frames the thread has run, and the most clients and workspaces the shell's own script has counted at once: the stock entry adds
    /// workspaces when it runs, and the clients are the windows the soak opens.
    frames: Arc<AtomicU64>,
    most_clients: Arc<AtomicU64>,
    most_workspaces: Arc<AtomicU64>,
    /// Whether the role reports its generation degraded (a budget trip that was not restaged): the thread still ticks, the script no longer runs.
    degraded: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<String>>>,
    /// The place of this shell's core in the process's queue, until the shell ends.
    turn: Option<support::core_rig::Turn>,
    /// The frame count the last liveness check saw, and when the count last rose.
    progress: (u64, Instant),
}

impl Shell {
    /// Starts the shell with an empty configuration directory, so the stock configuration runs, and returns once it has run a frame: it has
    /// attached to the session and loaded the stock entry, or it has ended and this fails the test with why.
    fn start(ipc: &Path, config: &Path) -> Self {
        let turn = support::core_rig::Turn::wait();
        let socket = ipc.to_str().expect("a UTF-8 socket path").to_owned();
        let dir = config.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let most_clients = Arc::new(AtomicU64::new(0));
        let most_workspaces = Arc::new(AtomicU64::new(0));
        let degraded = Arc::new(AtomicBool::new(false));
        let (stopped, ran, counted, spaces, health) = (
            Arc::clone(&stop),
            Arc::clone(&frames),
            Arc::clone(&most_clients),
            Arc::clone(&most_workspaces),
            Arc::clone(&degraded),
        );
        let thread = std::thread::Builder::new()
            .name("js-shell".to_owned())
            .spawn(move || {
                let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
                let mut config = JsRoleConfig::new(&dir, Some(support::stock_dir()));
                config.shell.entry_budget = HONEST_BUDGET;
                config.shell.frame_budget = HONEST_BUDGET;
                config.shell.js.stage_budget = HONEST_BUDGET;
                // The shell runs over a core with the std library loaded, as in the compositor: the library computes the stock layouts, and
                // the core is what the object classes are written over.
                let rig = support::core_rig::CoreRig::new(Arc::clone(&config.shell.clock))
                    .expect("a core with the std library");
                config.shell.js.natives.push(rig.std_native());
                config.shell.js.natives.push(rig.object_native());
                // The verbs of the `gnoblin` namespace and the dialog slot reach the session over the owner socket, as in a session.
                if let Some(path) = env(gnoblin_client::unix::SOCKET_VARIABLE) {
                    rig.reach(Path::new(&path));
                }
                let mut role =
                    JsRole::connect_through(&env, config, |link| rig.take_over(link)).expect("the shell attaches");
                let mut problems = Vec::new();
                let mut taken = 0;
                while !stopped.load(Ordering::Relaxed) {
                    let ticked = role.tick();
                    rig.dispatch(role.shell_mut());
                    if let Err(error) = ticked {
                        problems.push(error.to_string());
                        break;
                    }
                    problems.extend(role.diagnostics_since(taken).iter().cloned());
                    taken = role.diagnostics_reported();
                    health.store(matches!(role.health(), Health::Degraded { .. }), Ordering::Relaxed);
                    let round = ran.fetch_add(1, Ordering::Relaxed) + 1;
                    // What the shell's own script sees, every few frames: the workspaces the stock entry made and the clients the soak opens.
                    if round.is_multiple_of(20)
                        && let Ok(Some(text)) = role
                            .shell_mut()
                            .eval("gnoblin.clients.length + ',' + (gnoblin.workspaces?.list ?? []).length")
                        && let Some((clients, workspaces)) = text.split_once(',')
                        && let (Ok(clients), Ok(workspaces)) = (clients.parse::<u64>(), workspaces.parse::<u64>())
                    {
                        counted.fetch_max(clients, Ordering::Relaxed);
                        spaces.fetch_max(workspaces, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                drop(role);
                problems
            })
            .expect("the shell thread");
        let mut shell = Self {
            stop,
            frames,
            most_clients,
            most_workspaces,
            degraded,
            thread: Some(thread),
            turn: Some(turn),
            progress: (0, Instant::now()),
        };
        while shell.frames.load(Ordering::Relaxed) == 0 {
            shell.assert_alive();
            std::thread::sleep(Duration::from_millis(5));
        }
        shell
    }

    /// Fails the soak unless the shell is running: its thread is still there, its generation is not degraded, and it has run a frame in the
    /// last [`SHELL_STALL`]. Called every cycle of the churn, so a shell that dies in the middle of a run is the run's failure at that
    /// time and not a discovery at the end.
    #[track_caller]
    fn assert_alive(&mut self) {
        if self.thread.as_ref().is_none_or(JoinHandle::is_finished) {
            // The thread ends only when told to or when it fails; `finish` raises its own panic when that is how it ended.
            let problems = self.finish();
            panic!("the stock shell ended while the soak ran; it reported {problems:?}");
        }
        assert!(
            !self.degraded.load(Ordering::Relaxed),
            "the stock shell's generation is degraded: its script no longer runs"
        );
        let frames = self.frames.load(Ordering::Relaxed);
        if frames > self.progress.0 {
            self.progress = (frames, Instant::now());
        }
        assert!(
            self.progress.1.elapsed() < SHELL_STALL,
            "the stock shell ran no frame for {:?}; it is stuck at {frames} frames",
            self.progress.1.elapsed()
        );
    }

    /// Stops the shell and returns the problems it reported. A panic of the shell's thread (it could not attach) fails the test with the
    /// thread's own message. A second call returns nothing.
    fn finish(&mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        let problems = self.thread.take().and_then(support::join_worker).unwrap_or_default();
        // The thread's core is gone with it: the next shell of the process may have its turn.
        self.turn.take();
        problems
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.finish();
    }
}

/// A fresh owner connection, for the screenshots.
fn owner(ipc: &Path) -> Client<UnixTransport> {
    let transport = UnixTransport::connect(ipc).expect("the session is listening");
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Client::connect(transport, Version::new(major, minor, patch)).expect("the handshake")
}

/// One `gnoblinctl` run against the session.
fn ctl(ipc: &Path, arguments: &[&str]) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = gnoblinctl::run(
        arguments.iter().map(std::ffi::OsString::from),
        gnoblinctl::SocketConnection::at(ipc),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// The object id of the first output `output list` prints.
fn first_output(ipc: &Path) -> Option<u64> {
    let (_, out, _) = ctl(ipc, &["output", "list"]);
    out.lines().next()?.split('\t').next()?.parse().ok()
}

/// Applies a supersample factor to an output; every other setting is left as it is.
fn supersample(ipc: &Path, output: u64, factor: &str) -> bool {
    let id = output.to_string();
    let setting = format!("supersample={factor}");
    let (code, out, error) = ctl(ipc, &["output", "call", &id, "apply", &setting]);
    if code != 0 {
        eprintln!("supersample {factor} was refused (exit {code}): out {out:?} err {error:?}");
    }
    code == 0
}

/// The lock's state as the tree names it: `locked`, `not-locked` and the states between.
fn lock_state(ipc: &Path) -> String {
    snapshot(ipc)
        .into_iter()
        .find(|node| node.class == "lock")
        .and_then(|node| {
            node.props
                .iter()
                .find(|prop| prop.key == "state")
                .map(|prop| prop.value_text.clone())
        })
        .unwrap_or_else(|| "not-locked".to_owned())
}

/// How many capture frames the recorder has had refused so far.
fn refused(capture: &Option<Capture>) -> u64 {
    capture.as_ref().map_or(0, |capture| capture.counts().1)
}

/// Locks the session the way the screensaver service does and unlocks it with the password typed at the keyboard,
/// asserting both happen: a soak that carries on with a stuck lock measures nothing. The shell is checked alive throughout, so one
/// that dies while the session is locked fails the lock and not the end of the run.
///
/// Returns how many capture frames the recorder had refused between the lock being taken and the session being unlocked.
fn lock_and_unlock(session: &Session, shell: &mut Shell, capture: &mut Option<Capture>) -> u64 {
    let ipc = session.ipc();
    if let Some(capture) = capture {
        capture.step();
    }
    let refused_before = refused(capture);
    support::take_lock(&ipc);
    let deadline = Instant::now() + PATIENCE;
    while lock_state(&ipc) != "locked" {
        shell.assert_alive();
        assert!(Instant::now() < deadline, "the session never locked");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Held long enough for the cover and the password card to be drawn a few frames, with the recorder still asking.
    for _ in 0..10 {
        shell.assert_alive();
        if let Some(capture) = capture {
            capture.step();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    loop {
        shell.assert_alive();
        for code in PASSWORD_KEYS.into_iter().chain([KEY_ENTER]) {
            session.key(code);
        }
        std::thread::sleep(Duration::from_millis(300));
        let state = lock_state(&ipc);
        if state == "not-locked" {
            break;
        }
        assert!(Instant::now() < deadline, "the session never unlocked: {state}");
    }
    if let Some(capture) = capture {
        capture.step();
    }
    refused(capture) - refused_before
}

/// A numeric field of `/proc/self/status`: kilobytes for the `Vm*` ones, a count for `Threads`.
fn status_field(name: &str) -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix(name)?
                .strip_prefix(':')?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// How many entries a directory of `/proc/self` has.
fn entries(path: &str) -> usize {
    fs::read_dir(path).map_or(0, Iterator::count)
}

/// Every screen node's `gpu_memory_*` properties, as one JSON object per screen.
fn screens(ipc: &Path) -> serde_json::Value {
    let mut rows = Vec::new();
    for node in snapshot(ipc).into_iter().filter(|node| node.class == "screen") {
        let mut row = serde_json::Map::new();
        row.insert("node".to_owned(), json!(node.id));
        for prop in &node.props {
            if prop.key.starts_with("gpu_memory_") || prop.key == "supersample" || prop.key == "mode" {
                let value = if prop.key == "mode" {
                    json!(prop.value_text)
                } else {
                    json!(prop.value_double)
                };
                row.insert(prop.key.clone(), value);
            }
        }
        rows.push(serde_json::Value::Object(row));
    }
    serde_json::Value::Array(rows)
}

/// The largest `key` over the screens a sample holds: the session-wide numbers are the same on every screen, and a sum
/// would count them once for each.
fn greatest(screens: &serde_json::Value, key: &str) -> f64 {
    screens.as_array().map_or(0.0, |rows| {
        rows.iter().filter_map(|row| row[key].as_f64()).fold(0.0, f64::max)
    })
}

/// The sum of `key` over the screens a sample holds.
fn sum(screens: &serde_json::Value, key: &str) -> f64 {
    screens
        .as_array()
        .map_or(0.0, |rows| rows.iter().filter_map(|row| row[key].as_f64()).sum())
}

#[test]
#[ignore = "a soak runs for minutes: GNOBLIN_SOAK_SECONDS, see this file's header"]
fn the_session_keeps_its_memory_bounded_under_churn() {
    let seconds: u64 = setting("GNOBLIN_SOAK_SECONDS", "60")
        .parse()
        .expect("GNOBLIN_SOAK_SECONDS is a number");
    let sample_every: u64 = setting("GNOBLIN_SOAK_SAMPLE_SECONDS", "10")
        .parse()
        .expect("GNOBLIN_SOAK_SAMPLE_SECONDS is a number");
    let out_path = setting("GNOBLIN_SOAK_OUT", "gpu-memory-soak.jsonl");
    let mut out = fs::File::create(&out_path).expect("the series file");
    let renderer_name = setting("GNOBLIN_SOAK_RENDERER", "software");
    // A run that grows without bound would end in the kernel's OOM killer, which leaves no series. Past this many
    // MiB resident (0: no limit) the churn stops, the last sample is written and the test fails, naming the time.
    let rss_limit_mib: u64 = setting("GNOBLIN_SOAK_RSS_LIMIT_MIB", "0")
        .parse()
        .expect("GNOBLIN_SOAK_RSS_LIMIT_MIB is a number");
    let mut over_limit_at: Option<f64> = None;
    let lock_every: u64 = setting("GNOBLIN_SOAK_LOCK_EVERY", "25")
        .parse()
        .expect("GNOBLIN_SOAK_LOCK_EVERY is a number");

    // A budget the churn leans on: the session's own default when the variable is unset.
    let budget_mib: u64 = setting("GNOBLIN_SOAK_BUDGET_MIB", "0")
        .parse()
        .expect("GNOBLIN_SOAK_BUDGET_MIB is a number");
    let mut limits = gnoblin_wayland::MemoryLimits::default();
    if budget_mib > 0 {
        limits.budget_bytes = budget_mib << 20;
    }
    let mut session = Session::start(renderer(), limits);
    let config = tempfile::tempdir().expect("a config directory");
    let mut shell = Shell::start(&session.ipc(), config.path());
    let ipc = session.ipc();
    let output = {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(output) = first_output(&ipc) {
                break output;
            }
            assert!(Instant::now() < deadline, "the session never listed an output");
            std::thread::sleep(Duration::from_millis(25));
        }
    };

    let started = Instant::now();
    let mut next_sample = started;
    let mut cycle: u64 = 0;
    let mut mapper = Mapper::connect(&session.wayland());
    // The recorder is opened once, before the churn, and kept: the run is about what an open session costs over time.
    let mut capture = Capture::open(&session.privileged());
    assert!(capture.is_some(), "the privileged socket offered no capture session");
    let mut locks = 0u64;
    let mut windows = 0usize;
    let mut extra: Option<u64> = None;
    let mut screenshots = 0u64;
    let mut refused_screenshots = 0u64;
    let mut factor = "1";
    // What the client that leans on the budget was told: buffers admitted and buffers refused.
    let (admitted, refused_buffers) = (std::cell::Cell::new(0u64), std::cell::Cell::new(0u64));
    // What the shell's script counted of the churn, and the capture frames refused while the session was locked.
    let (shell_frames, shell_clients, shell_workspaces) = (
        Arc::clone(&shell.frames),
        Arc::clone(&shell.most_clients),
        Arc::clone(&shell.most_workspaces),
    );
    let refused_in_locks = std::cell::Cell::new(0u64);
    let mut sample = |cycle: u64,
                      windows: usize,
                      factor: &str,
                      screenshots: u64,
                      refused: u64,
                      locks: u64,
                      capture: &Option<Capture>| {
        let screens = screens(&ipc);
        let (frames, frames_failed, capture_sessions) = capture.as_ref().map_or((0, 0, 0), Capture::counts);
        let why = capture.as_ref().map_or_else(Refusals::default, Capture::refusals);
        let line = json!({
            "t": started.elapsed().as_secs_f64(),
            "renderer": renderer_name,
            "cycle": cycle,
            "windows_mapped": windows,
            "windows_in_scene": scene(&ipc).len(),
            "supersample": factor,
            "screenshots": screenshots,
            "screenshots_refused": refused,
            "lock_cycles": locks,
            "capture_frames": frames,
            "capture_frames_failed": frames_failed,
            "capture_sessions": capture_sessions,
            "capture_refused_stopped": why.stopped,
            "capture_refused_constraints": why.constraints,
            "capture_refused_unknown": why.unknown,
            "capture_refused_in_locks": refused_in_locks.get(),
            "capture_sessions_ended": why.sessions_ended,
            "shell_frames": shell_frames.load(Ordering::Relaxed),
            "shell_clients_most": shell_clients.load(Ordering::Relaxed),
            "shell_workspaces_most": shell_workspaces.load(Ordering::Relaxed),
            "rss": status_field("VmRSS") * 1024,
            "threads": status_field("Threads"),
            "fds": entries("/proc/self/fd"),
            "gpu_memory_bytes": sum(&screens, "gpu_memory_bytes"),
            "gpu_memory_capture_bytes": sum(&screens, "gpu_memory_capture_bytes"),
            "gpu_memory_staging_bytes": sum(&screens, "gpu_memory_staging_bytes"),
            "budget_bytes": greatest(&screens, "gpu_memory_budget_bytes"),
            "clients_bytes": greatest(&screens, "gpu_memory_clients_bytes"),
            "refused_total": greatest(&screens, "gpu_memory_refused_total"),
            "evicted_total": greatest(&screens, "gpu_memory_evicted_total"),
            "pressure_admitted": admitted.get(),
            "pressure_refused": refused_buffers.get(),
            "screens": screens,
        });
        writeln!(out, "{line}").expect("a series line");
        out.flush().expect("a flushed series");
        println!("{line}");
    };
    sample(
        cycle,
        windows,
        factor,
        screenshots,
        refused_screenshots,
        locks,
        &capture,
    );

    while started.elapsed() < Duration::from_secs(seconds) {
        if rss_limit_mib > 0 && status_field("VmRSS") / 1024 > rss_limit_mib {
            over_limit_at = Some(started.elapsed().as_secs_f64());
            break;
        }
        shell.assert_alive();
        cycle += 1;
        let (width, height) = SIZES[(cycle as usize) % SIZES.len()];
        // Windows come and go: the long-lived client opens one per cycle, resizes an older one, and disconnects whole
        // when it has too many, so both a window's close and a client's teardown are exercised.
        if windows >= MAX_WINDOWS {
            mapper = Mapper::connect(&session.wayland());
            windows = 0;
        }
        mapper.map_window_sized(&format!("org.example.soak{cycle}"), "soak", width, height);
        windows += 1;
        if windows > 1 {
            let (width, height) = SIZES[(cycle as usize + 2) % SIZES.len()];
            mapper.commit_geometry(cycle as usize % (windows - 1), (0, 0, width, height));
        }
        // A client that opens and closes at once: a connection per cycle.
        {
            let mut short = Mapper::connect(&session.wayland());
            short.map_window_sized("org.example.short", "short", 400, 300);
        }
        if let Some(capture) = &mut capture {
            capture.step();
        }
        // The client that leans on a small budget, every fifth cycle, and in turn: a buffer that fits (16 MiB), one a
        // little larger than what is left, which fits only if the compositor gives back a cache, and one as large as
        // the whole budget, which nothing can make room for. It lets go at once, before the soak's own clients ask for
        // anything, so that a refusal of theirs would be the budget's doing and not this client's.
        if budget_mib > 0 && cycle.is_multiple_of(5) {
            let now = screens(&ipc);
            let left = greatest(&now, "gpu_memory_budget_bytes")
                - greatest(&now, "gpu_memory_clients_bytes")
                - sum(&now, "gpu_memory_bytes");
            let bytes = match (cycle / 5) % 3 {
                0 => 16 << 20,
                1 => left.max(0.0) as u64 + (2 << 20),
                _ => budget_mib << 20,
            };
            let (answer, connection) = pressure::lean(&session.wayland(), bytes);
            match answer {
                pressure::Answer::Accepted => admitted.set(admitted.get() + 1),
                pressure::Answer::Refused => refused_buffers.set(refused_buffers.get() + 1),
            }
            std::thread::sleep(Duration::from_millis(100));
            drop(connection);
            std::thread::sleep(Duration::from_millis(100));
        }
        // Screenshots through the control socket: both the composed frame and the one the display shows.
        let mut control = owner(&ipc);
        let shot = if cycle.is_multiple_of(2) {
            control.capture_screen()
        } else {
            control.capture_screen_display()
        };
        match shot {
            Ok(_) => screenshots += 1,
            Err(_) => refused_screenshots += 1,
        }
        // A factor every fifth cycle: 2, 4, then back to 1.
        if cycle.is_multiple_of(5) {
            let next = FACTORS[(cycle / 5 % 3) as usize];
            if supersample(&ipc, output, next) {
                factor = next;
            }
        }
        // The lock, with the windows still mapped and the recorder still open.
        if lock_every > 0 && cycle.is_multiple_of(lock_every) {
            refused_in_locks.set(refused_in_locks.get() + lock_and_unlock(&session, &mut shell, &mut capture));
            locks += 1;
        }
        // A second output plugged for three cycles of every seven.
        match (cycle % 7, extra) {
            (1, None) => {
                session.outputs.connect("Virtual-9", 1024, 768);
                extra = Some(cycle);
            }
            (4, Some(_)) => {
                if let Some(second) = plugged(&ipc, "Virtual-9") {
                    session.outputs.disconnect(gnoblin_types::OutputId::new(second));
                }
                extra = None;
            }
            _ => {}
        }
        if Instant::now() >= next_sample {
            sample(
                cycle,
                windows,
                factor,
                screenshots,
                refused_screenshots,
                locks,
                &capture,
            );
            next_sample += Duration::from_secs(sample_every);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    // The end: every client gone, the factor back to one, the second output unplugged, then a settled reading.
    drop(mapper);
    let _ = supersample(&ipc, output, "1");
    std::thread::sleep(Duration::from_secs(3));
    sample(cycle, 0, "1", screenshots, refused_screenshots, locks, &capture);
    // What the recorder was refused over the whole run, read before the recorder is closed.
    let refusals = capture.as_ref().map_or_else(Refusals::default, Capture::refusals);
    let refused_frames = refused(&capture);
    println!(
        "capture refusals: {refusals:?} of {refused_frames} refused frames, {} of them while locked",
        refused_in_locks.get()
    );
    if budget_mib > 0 {
        // With every client of the churn gone and the recorder closed, nothing may stay counted: a refusal that left a
        // buffer or a surface in the ledger would show here as bytes no client holds. The budget must also have been
        // leaned on, or the run proved nothing about it.
        drop(capture.take());
        let deadline = Instant::now() + PATIENCE;
        let mut last;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            last = screens(&ipc);
            if greatest(&last, "gpu_memory_clients_bytes") == 0.0 || Instant::now() >= deadline {
                break;
            }
        }
        sample(cycle, 0, "1", screenshots, refused_screenshots, locks, &capture);
        assert_eq!(
            greatest(&last, "gpu_memory_clients_bytes"),
            0.0,
            "buffers stayed counted after every client was gone"
        );
        assert!(
            greatest(&last, "gpu_memory_refused_total") > 0.0 && refused_buffers.get() > 0,
            "the budget was never refused anything, so the run proved nothing about refusal: {last}"
        );
    }
    // The shell and the session end here, on the normal path: a panic of either thread is the soak's failure with its own message, and
    // the shell, which was checked every cycle, must still be running when the churn is over.
    shell.assert_alive();
    let (ran, counted, spaces) = (
        shell.frames.load(Ordering::Relaxed),
        shell.most_clients.load(Ordering::Relaxed),
        shell.most_workspaces.load(Ordering::Relaxed),
    );
    let problems = shell.finish();
    session.finish();
    println!("the shell ran {ran} frames; its script counted up to {counted} clients and {spaces} workspaces");
    assert!(
        over_limit_at.is_none(),
        "the process passed {rss_limit_mib} MiB resident after {:.0} s: the series in {out_path} shows what grew",
        over_limit_at.unwrap_or_default()
    );
    assert!(
        problems.is_empty(),
        "the stock shell reported problems during the soak: {problems:#?}"
    );
    assert!(
        spaces > 0 && counted > 0,
        "the stock shell's own script counted {spaces} workspaces and {counted} clients in {ran} frames: the stock entry did not run, or it \
         was not looking at the session"
    );
    // The recorder's refused frames are expected, and only at a lock. The lock ends the capture session (`Stop::Embargoed`), and the
    // recorder always has a frame wanted, so one reaches the session as it stops. The compositor refuses a frame of a stopped session
    // (a session that stopped has no constraints: failure mode 4 of `ext-image-copy-capture-v1`) and the client reads that and the
    // `stopped` event together; the recorder then asks for nothing more of that session. So every refused frame is read while a lock is
    // held, there is at most one per session the server ended, and none has the reason `unknown`. A frame refused anywhere else (at a
    // supersample change, an output hotplug, a window closing) is a bug.
    assert_eq!(
        refusals.unknown, 0,
        "a capture frame was refused with a reason an ended session does not give: {refusals:?}"
    );
    assert_eq!(
        refused_frames,
        refused_in_locks.get(),
        "capture frames were refused outside the session locks ({locks} locks): {refusals:?}"
    );
    assert!(
        refused_frames <= u64::from(refusals.sessions_ended),
        "more capture frames were refused than sessions the server ended: {refusals:?}"
    );
}

/// The backend id of the output on `connector`: the screen node is named `output-N` after it.
fn plugged(ipc: &Path, connector: &str) -> Option<u64> {
    snapshot(ipc)
        .into_iter()
        .filter(|node| node.class == "screen")
        .find_map(|node| {
            let prop = |key: &str| {
                node.props
                    .iter()
                    .find(|prop| prop.key == key)
                    .map(|prop| prop.value_text.clone())
            };
            (prop("connector")? == connector)
                .then(|| prop("name")?.strip_prefix("output-")?.parse().ok())
                .flatten()
        })
}
