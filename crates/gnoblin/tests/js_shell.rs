//! The JavaScript shell against a real compositor session (design/js-runtime.md, "P6 role wiring").
//!
//! Nothing here is a fake. The session is [`Compositor::run`] on a headless backend with its sockets in a
//! temporary directory. The windows are mapped by a real `wayland-client` connection. The shell is a
//! [`JsRole`] that connects through the role's own path (`JsRole::connect` reads the socket from the
//! environment, handshakes as a client and attaches to the tree as the owner), and runs in a thread of its own
//! the way the runtime role is a process of its own, so a script that never returns stalls the shell and not the
//! compositor or the test. The scene is read back through a second connection that attaches as an observer, so
//! what the tests assert is what the compositor holds, not what the shell believes.
//!
//! The test lives in the binary's crate because that crate may name both the compositor and the shell; the
//! shell's crate is an IPC client and may link neither (`xtask deps`).
//!
//! The tests run in parallel and on a loaded machine. They wait for the state they need, with a deadline, and do not
//! sleep for the time they guess a state will take (design/js-runtime.md, "Real-compositor tests").

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_client::unix::UnixTransport;
use gnoblin_client::wire::{PortalRequest, VerifiedCaller};
use gnoblin_client::{Client, ClientError, PortalOutcome, PortalTicket};
use gnoblin_compositor::{Compositor, InputTap, OutputTap, SessionConfig, StopHandle};
use gnoblin_js::{JsHealth, JsRole, JsRoleConfig};
use gnoblin_log::event::capture::logged;
use gnoblin_model::{Fields, Filter, RequestCompletion, TypeKey, Value};
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::{Envelope, Kind, read_frame, write_frame};
use gnoblin_types::{DeviceId, OutputId, Point, Version};

mod support;
use support::{Mapper, Watcher, flag_of, ids, scene, screens, snapshot, spots};

/// How long a test waits for a state before it fails. A passing run never waits this long: it is the margin a busy
/// machine gets, so a failure is a state that never came and not a slow turn.
const PATIENCE: Duration = Duration::from_secs(60);

/// How often a wait looks at the state again. Each look at the compositor's tree is a connection of its own, so this is
/// also how hard a test leans on a compositor that a loaded machine is already slow to schedule.
const POLL: Duration = Duration::from_millis(25);

/// What the shell may spend on one listener call, one frame's calls and one staging in a test that is not about the
/// budgets. They are wall-clock budgets, so a loaded machine that does not schedule the shell's thread for a while
/// spends them on an honest script and the generation is stopped for it. A test that needs a script to run away sets
/// its own short ones, and no honest work of the shell must come near them.
const HONEST_BUDGET: Duration = Duration::from_secs(30);

// ---- the compositor session --------------------------------------------------------------------------------------

/// A headless compositor on sockets of its own.
struct Session {
    directory: tempfile::TempDir,
    stop: StopHandle,
    /// Where a test sends the input a device would: it takes the seat path a real pointer's events take.
    input: InputTap,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    fn start() -> Self {
        Self::start_with(|_| {})
    }

    /// A session whose configuration `tune` has changed (a liveness timeout, say).
    fn start_with(tune: impl FnOnce(&mut SessionConfig)) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        // The compositor refuses an IPC directory that is not private, as `$XDG_RUNTIME_DIR` is.
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let stop = StopHandle::new();
        let input = InputTap::new();
        let mut config = SessionConfig {
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            input_tap: Some(input.clone()),
            ..SessionConfig::default()
        };
        config.wayland.socket_path = Some(directory.path().join("wayland.sock"));
        // The compositor pings the client of every mapped window and marks one that has not answered within this
        // time as unresponsive, which the shell then paints an alert for. A test thread that a loaded machine does not
        // schedule for that long is not a hung application, so the pings are off unless the test is about them.
        config.wayland.check_alive_timeout = Duration::ZERO;
        tune(&mut config);
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || {
                Compositor::run(config).expect("the compositor runs until it is stopped");
            })
            .expect("the session thread");
        let session = Self {
            directory,
            stop,
            input,
            thread: Some(thread),
        };
        let deadline = Instant::now() + PATIENCE;
        while UnixStream::connect(session.ipc()).is_err() {
            assert!(Instant::now() < deadline, "the compositor never listened");
            std::thread::sleep(Duration::from_millis(5));
        }
        session
    }

    fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    fn wayland(&self) -> PathBuf {
        self.directory.path().join("wayland.sock")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.request();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---- canvases, read as an observer -------------------------------------------------------------------------------

/// One canvas as the compositor's tree holds it.
#[derive(Clone, Debug)]
struct Canvas {
    parent: u64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    /// The paint list the compositor accepted, decoded; `None` before the shell has painted.
    list: Option<gnoblin_paint::PaintList>,
}

impl Canvas {
    /// The canvas rasterised with the embedded font, so the pixels do not depend on this machine's fonts.
    fn picture(&self) -> gnoblin_paint::Picture {
        let list = self.list.as_ref().expect("a painted canvas");
        gnoblin_paint::CanvasPainter::new(Arc::new(gnoblin_paint::FontSet::embedded()))
            .paint(list, self.width as u32, self.height as u32)
            .expect("a canvas of a sane size")
            .picture
    }
}

/// Every canvas of the compositor's tree, read through a fresh observer connection.
fn canvases(ipc: &Path) -> Vec<Canvas> {
    support::snapshot(ipc)
        .into_iter()
        .filter(|node| node.class == "canvas")
        .map(|node| {
            let prop = |key: &str| node.props.iter().find(|prop| prop.key == key);
            Canvas {
                parent: node.parent,
                x: prop("x").map_or(0.0, |prop| prop.value_double),
                y: prop("y").map_or(0.0, |prop| prop.value_double),
                width: prop("width").map_or(0.0, |prop| prop.value_double),
                height: prop("height").map_or(0.0, |prop| prop.value_double),
                list: prop("content").map(|content| {
                    gnoblin_paint::decode(&content.value_bytes).expect("the compositor holds only paint lists")
                }),
            }
        })
        .collect()
}
// ---- the shell, in a thread of its own ---------------------------------------------------------------------------

/// Polls what the compositor has published of its windows, keeping the Wayland client answering, until `check` holds
/// on their spots. Returns the windows and their spots. The failure message carries the last scene.
#[track_caller]
fn wait_for_scene(
    what: &str,
    watch: &Watcher,
    mapper: &mut Mapper,
    mut check: impl FnMut(&BTreeMap<String, (f64, f64)>) -> bool,
) -> (Vec<support::Window>, BTreeMap<String, (f64, f64)>) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        mapper.roundtrip();
        let now = watch.scene();
        let held = spots(&now);
        if check(&held) {
            return (now, held);
        }
        assert!(
            Instant::now() < deadline,
            "never reached: {what}; the scene is {held:?}"
        );
        std::thread::sleep(POLL);
    }
}

/// What the shell thread reads out of the script after every frame, as text: the clients, the workspaces and the
/// keys bound on the first seat. A seat-less tree reads as `no seat` for the keys.
const VIEW_PROBE: &str = r#"(() => {
    const keys = (() => {
        try {
            return [...gnoblin.input.primary.bindings].map((b) => b.trigger).join(",");
        } catch (error) {
            return "no seat";
        }
    })();
    const spaces = gnoblin.workspaces.map((w) => `${w.id}:${w.layout.constructor.name}`).join(",");
    return `${gnoblin.clients.length} clients, ${gnoblin.workspaces.length} workspaces [${spaces}] keys[${keys}]: ` +
        gnoblin.clients.map((c) => `${c.appId}@${c.x},${c.y}`).join(";") +
        " | focus=" + (() => {
            try {
                return gnoblin.input.primary.focusedClient?.appId;
            } catch (error) {
                return "no seat";
            }
        })();
})()"#;

/// What a test's own script says of the shell, as text: whatever `globalThis.probe` returns once the script has set it,
/// for a state of the shell that the compositor's tree does not show.
const SCRIPT_PROBE: &str = r#"(typeof globalThis.probe === "function" ? String(globalThis.probe()) : "")"#;

/// What the shell thread reports, refreshed after every frame.
///
/// It is a snapshot the thread takes when a frame is over, and the frame's effects reach the compositor while it
/// runs. A test that has seen an effect in the compositor's tree therefore cannot read the shell's side of it here as
/// settled: the snapshot may be the one before the frame that made the effect. What a test wants to know of the shell
/// is waited for (a generation above the one it knew, a diagnostic) and read from the same snapshot it waited on.
#[derive(Default)]
struct Status {
    /// How many frames' snapshots the thread has published. Zero until the first frame is over.
    frames: u64,
    generation: u64,
    degraded: bool,
    diagnostics: Vec<String>,
    /// What the shell's script sees of the clients, as text, for a failure message.
    view: String,
    /// What the script's `probe` said at the end of the frame, if it set one.
    probe: String,
    /// Set when the shell ended: why.
    ended: Option<String>,
}

/// A [`JsRole`] running its frame loop in a thread, as the runtime role runs it in a process.
struct Shell {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    thread: Mutex<Option<JoinHandle<Vec<String>>>>,
}

impl Shell {
    /// Starts the shell over `session`'s owner socket with `dir` as its config directory, and returns once its first
    /// frame is over (or it has ended, which `ended` then says why), so that what `generation` and `diagnostics`
    /// read from here on is the shell's and not the zero of a thread that has not published yet.
    fn start(session: &Session, dir: &Path, tune: impl FnOnce(&mut JsRoleConfig) + Send + 'static) -> Self {
        let socket = session.ipc().to_str().expect("a UTF-8 socket path").to_owned();
        let dir = dir.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status::default()));
        let (stopped, shared) = (Arc::clone(&stop), Arc::clone(&status));
        let thread = std::thread::Builder::new()
            .name("js-shell".to_owned())
            .spawn(move || {
                // The log of the shell's thread, so a test reads what the role warned.
                let mut warnings = Vec::new();
                let events = logged(|| {
                    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
                    let mut config = JsRoleConfig::new(&dir, None);
                    config.shell.entry_budget = HONEST_BUDGET;
                    config.shell.frame_budget = HONEST_BUDGET;
                    config.shell.js.stage_budget = HONEST_BUDGET;
                    tune(&mut config);
                    let mut role = match JsRole::connect(&env, config) {
                        Ok(role) => role,
                        Err(error) => {
                            let mut status = shared.lock().expect("the status lock");
                            status.frames += 1;
                            status.ended = Some(error.to_string());
                            return;
                        }
                    };
                    // `connect` has loaded the entry, so the generation is known before the first frame: a test that reads it
                    // as "the generation that booted" must not read the zero the status starts at.
                    shared.lock().expect("the status lock").generation = role.generation_id();
                    while !stopped.load(Ordering::Relaxed) {
                        let ticked = role.tick();
                        {
                            let mut status = shared.lock().expect("the status lock");
                            status.frames += 1;
                            status.generation = role.generation_id();
                            status.degraded = matches!(role.health(), JsHealth::Degraded { .. });
                            status.diagnostics = role.diagnostics().to_vec();
                            status.view = role
                                .shell_mut()
                                .eval(VIEW_PROBE)
                                .unwrap_or_else(|error| Some(error.to_string()))
                                .unwrap_or_default();
                            status.probe = role
                                .shell_mut()
                                .eval(SCRIPT_PROBE)
                                .unwrap_or_else(|error| Some(error.to_string()))
                                .unwrap_or_default();
                            if let Err(error) = ticked {
                                status.ended = Some(error.to_string());
                                return;
                            }
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                });
                warnings.extend(events.into_iter().map(|event| event.message));
                warnings
            })
            .expect("the shell thread");
        let shell = Self {
            stop,
            status,
            thread: Mutex::new(Some(thread)),
        };
        shell.await_first_frame();
        shell
    }

    /// Waits until the thread has published a frame's status or ended, whichever comes first.
    #[track_caller]
    fn await_first_frame(&self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            {
                let status = self.status.lock().expect("the status lock");
                if status.frames > 0 {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "never reached: the shell's first frame, which it publishes once it has connected and booted"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn generation(&self) -> u64 {
        self.status.lock().expect("the status lock").generation
    }

    fn degraded(&self) -> bool {
        self.status.lock().expect("the status lock").degraded
    }

    fn diagnostics(&self) -> Vec<String> {
        self.status.lock().expect("the status lock").diagnostics.clone()
    }

    fn view(&self) -> String {
        self.status.lock().map_or_else(
            |poisoned| poisoned.into_inner().view.clone(),
            |status| status.view.clone(),
        )
    }

    fn probe(&self) -> String {
        self.status.lock().expect("the status lock").probe.clone()
    }

    fn ended(&self) -> Option<String> {
        self.status.lock().expect("the status lock").ended.clone()
    }

    /// Stops the shell and returns every message it logged.
    fn finish(&self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self.thread.lock().expect("the thread lock").take();
        thread.map_or_else(Vec::new, |thread| thread.join().expect("the shell thread ends"))
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Waits until `check` holds, keeping the Wayland client answering pings, or fails with `what` and the shell's
/// diagnostics.
#[track_caller]
fn wait_for(what: &str, session: &Session, mapper: &mut Mapper, shell: &Shell, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        mapper.roundtrip();
        if check() {
            return;
        }
        if shell.ended().is_some() {
            let log = shell.finish();
            panic!(
                "the shell ended while waiting for {what}: {:?}; the shell logged {:#?}",
                shell.ended(),
                &log[log.len().saturating_sub(60)..]
            );
        }
        if Instant::now() >= deadline {
            let log = shell.finish();
            panic!(
                "never reached: {what}; shell diagnostics {:?}; the shell sees {:?}; the compositor's scene {:?}; \
                 the shell logged {:#?}",
                shell.diagnostics(),
                shell.view(),
                scene(&session.ipc()),
                &log[log.len().saturating_sub(60)..]
            );
        }
        std::thread::sleep(POLL);
    }
}

/// Saves `text` as the file `path` the way an editor that saves atomically does: the text is written under a name the
/// watcher does not take for a module and renamed over the file, so a reload reads the old file or the whole new one and
/// never one that a write has emptied and not yet filled. `fs::write` truncates first, and a machine that leaves the
/// test thread unscheduled for the watcher's quiet time between the truncation and the data lets a reload run the empty
/// module.
fn save(path: &Path, text: &str) {
    let staged = path.with_extension("part");
    fs::write(&staged, text).expect("the staged file");
    fs::rename(&staged, path).expect("the file replaced");
}

/// The `init.ts` of the tests: a typed listener over the places `slots.ts` holds.
const PLACING_INIT: &str = r#"import { Layout, Workspace, gnoblin } from "gnoblin";
import { places } from "./slots.ts";

// A workspace whose layout places nothing, so the listener below is the only thing that moves a window: the
// stock floating layout would place each arrival itself, after the listeners have run.
gnoblin.workspaces.add(new Workspace({ id: "desk", layout: new Layout() }));

enum Mode {
    Tiled,
    Floating,
}

const mode: Mode = Mode.Floating;

gnoblin.clients.addListener({
    onAdded(client: { appId: string; x: number; y: number }): void {
        const at: [number, number] | undefined = places[client.appId] ?? places["*"];
        if (at !== undefined && mode === Mode.Floating) {
            client.x = at[0];
            client.y = at[1];
        }
    },
});
"#;

/// The `slots.ts` of the tests: the place of each app id, and `"*"` for the others.
fn write_places(dir: &Path, places: &str) {
    save(
        &dir.join("slots.ts"),
        &format!("export const places: Record<string, [number, number]> = {places};\n"),
    );
}

/// The config directory of the tests: `slots.ts` holds the places, `init.ts` a typed listener over them.
///
/// The places are saved first, since `init.ts` imports them. A save of two files is not one event for the watcher when
/// the test is not scheduled between them for longer than its quiet time, and a reload that comes in the middle then
/// reads the new places with the old `init.ts`, which is a working config, or none yet.
fn write_config(dir: &Path, places: &str) {
    write_places(dir, places);
    save(&dir.join("init.ts"), PLACING_INIT);
}

#[test]
fn init_ts_places_the_windows_reloads_on_save_with_the_same_ids_and_keeps_the_old_one_through_an_error() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");
    mapper.map_window("org.example.second", "Second");

    let config = tempfile::tempdir().expect("a config directory");
    write_config(
        config.path(),
        r#"{ "org.example.first": [111, 222], "org.example.second": [333, 444] }"#,
    );
    let shell = Shell::start(&session, config.path(), |_| {});

    // Slice criterion 1: `init.ts` with annotations and an enum, importing `./slots.ts`, evaluates against the real
    // tree, and its listener moves the two windows the compositor already holds.
    let ipc = session.ipc();
    wait_for(
        "both windows at the first file's places",
        &session,
        &mut mapper,
        &shell,
        || {
            let now = spots(&scene(&ipc));
            now.get("org.example.first") == Some(&(111.0, 222.0))
                && now.get("org.example.second") == Some(&(333.0, 444.0))
        },
    );
    let before = ids(&scene(&ipc));
    assert_eq!(before.len(), 2, "two windows: {before:?}");
    // `Shell::start` returned after the shell's first frame, so this is the generation that placed the windows.
    let first_generation = shell.generation();

    // Slice criterion 3: an edit is a reload, and the windows keep their ids and take the new places. The generation
    // is waited for with the places: the places reach the compositor while the frame that made them is still running,
    // and the generation is what the shell publishes once that frame is over.
    write_config(
        config.path(),
        r#"{ "org.example.first": [500, 400], "org.example.second": [600, 300], "*": [700, 200] }"#,
    );
    wait_for(
        "both windows at the rewritten places, under a newer generation",
        &session,
        &mut mapper,
        &shell,
        || {
            let now = spots(&scene(&ipc));
            shell.generation() > first_generation
                && now.get("org.example.first") == Some(&(500.0, 400.0))
                && now.get("org.example.second") == Some(&(600.0, 300.0))
        },
    );
    assert_eq!(ids(&scene(&ipc)), before, "the reload kept every window id");
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());

    // Slice criterion 4: a syntax error keeps the running generation and is reported with its `.ts` position. The
    // old generation still places a window that maps afterwards.
    save(
        &config.path().join("init.ts"),
        "import { gnoblin } from \"gnoblin\";\n\nconst broken: number = = 5;\n",
    );
    wait_for("the syntax error is reported", &session, &mut mapper, &shell, || {
        shell.diagnostics().iter().any(|text| text.contains("init.ts:3:"))
    });
    // Nothing swaps in after the failed reload: the generation the shell reports is still the one it reported when the
    // error was, and it is the one that places the window that maps now.
    let at_the_error = shell.generation();
    mapper.map_window("org.example.third", "Third");
    wait_for(
        "the old generation places a new window",
        &session,
        &mut mapper,
        &shell,
        || spots(&scene(&ipc)).get("org.example.third") == Some(&(700.0, 200.0)),
    );
    assert_eq!(
        shell.generation(),
        at_the_error,
        "the generation that ran before the error still runs"
    );
    assert_eq!(
        ids(&scene(&ipc)).get("org.example.first"),
        before.get("org.example.first"),
        "the failed reload kept the window ids"
    );

    // Fixing the file is seen even though the failed module was in no generation's graph. `init.ts` is saved before
    // the places it imports: a reload that comes between the two reads the fixed file with the old places, and not the
    // broken file again, which would be reported a second time.
    save(&config.path().join("init.ts"), PLACING_INIT);
    write_places(
        config.path(),
        r#"{ "org.example.first": [10, 20], "org.example.second": [30, 40], "*": [50, 60] }"#,
    );
    wait_for("the fixed file takes effect", &session, &mut mapper, &shell, || {
        spots(&scene(&ipc)).get("org.example.first") == Some(&(10.0, 20.0))
    });

    let warnings = shell.finish();
    assert!(
        warnings.iter().any(|message| message.contains("init.ts:3:")),
        "the syntax error was logged with its position: {warnings:?}"
    );
    assert_eq!(
        warnings.iter().filter(|message| message.contains("init.ts:3:")).count(),
        1,
        "and once: {warnings:?}"
    );
}

/// What a test that needs a script to run away gives one listener call, and what it gives a staging. A runaway is
/// interrupted only at the end of its budget, so these are also the longest the test waits for each interrupt, and they
/// are wall-clock: they have to stay far above what the shell's honest calls and stagings take on a machine that other
/// tests keep busy (a staging took up to two seconds on the Agent37 runner with nothing else running), or the shell
/// stops an honest script and the test has proved nothing about the runaway.
const RUNAWAY_CALL: Duration = Duration::from_secs(1);
const RUNAWAY_STAGE: Duration = Duration::from_secs(5);

/// An `init.ts` whose handler for the window `org.example.spin` never returns, unless the window has been marked.
///
/// A generation staged from this file runs `onAdded` for every window already there, so a handler that always
/// spun would spin again in the restarted generation. This one marks the window first (a write to `y` that the
/// shell has sent by the time the generation is restaged, and that the restaged generation reads back), so only
/// the first generation to see the window hangs.
const SPIN_ONCE: &str = r#"import { Layout, Workspace, gnoblin } from "gnoblin";

gnoblin.workspaces.add(new Workspace({ id: "desk", layout: new Layout() }));
gnoblin.clients.addListener({
    onAdded(client: { appId: string; x: number; y: number }): void {
        if (client.appId === "org.example.spin") {
            if (client.y !== 12345) {
                client.y = 12345;
                while (true) {
                    // Never returns: only the budget can stop it.
                }
            }
            return;
        }
        client.x = 700;
        client.y = 600;
    },
});
"#;

#[test]
fn a_handler_that_never_returns_is_interrupted_the_compositor_keeps_answering_and_the_generation_restarts() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");
    mapper.map_window("org.example.second", "Second");

    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), SPIN_ONCE).expect("init.ts");
    // The first generation's listener hangs once, and only that call is the runaway.
    let shell = Shell::start(&session, config.path(), |config| {
        config.shell.entry_budget = RUNAWAY_CALL;
    });
    let ipc = session.ipc();
    wait_for(
        "the two windows at the script's place",
        &session,
        &mut mapper,
        &shell,
        || {
            let now = spots(&scene(&ipc));
            now.get("org.example.first") == Some(&(700.0, 600.0))
                && now.get("org.example.second") == Some(&(700.0, 600.0))
        },
    );
    let booted = shell.generation();

    // Slice criterion 5: the spinning handler is interrupted by the budget and the generation is staged again from
    // disk. The compositor answers an observer the whole time: the shell's thread is the one that spins.
    mapper.map_window("org.example.spin", "Spin");
    let mut scenes_during_the_spin = 0_u32;
    wait_for("the generation restarts", &session, &mut mapper, &shell, || {
        let _ = scene(&ipc);
        scenes_during_the_spin += 1;
        shell.generation() > booted
    });
    assert!(scenes_during_the_spin > 0);
    assert!(!shell.degraded(), "one strike is within the supervisor's policy");
    assert!(
        shell
            .diagnostics()
            .iter()
            .any(|text| text.contains("listener call was stopped")),
        "the interrupted call was reported: {:?}",
        shell.diagnostics()
    );

    // The scene still updates: the restarted generation places the next window, and the two before it stayed put.
    mapper.map_window("org.example.fourth", "Fourth");
    wait_for(
        "a window mapped after the restart is placed",
        &session,
        &mut mapper,
        &shell,
        || spots(&scene(&ipc)).get("org.example.fourth") == Some(&(700.0, 600.0)),
    );
    let now = spots(&scene(&ipc));
    assert_eq!(now.get("org.example.first"), Some(&(700.0, 600.0)));
    assert_eq!(now.get("org.example.spin").map(|spot| spot.1), Some(12345.0));
}

#[test]
fn a_handler_that_always_hangs_exhausts_the_restarts_and_the_last_generation_keeps_running() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");

    let config = tempfile::tempdir().expect("a config directory");
    fs::write(
        config.path().join("init.ts"),
        r#"import { Layout, Workspace, gnoblin } from "gnoblin";

gnoblin.workspaces.add(new Workspace({ id: "desk", layout: new Layout() }));
gnoblin.clients.addListener({
    onAdded(client: { appId: string; x: number; y: number }): void {
        if (client.appId === "org.example.spin") {
            while (true) {
                // Never returns, in every generation: the replay of the window hangs the restaged one too.
            }
        }
        client.x = 700;
        client.y = 600;
    },
});
"#,
    )
    .expect("init.ts");
    let shell = Shell::start(&session, config.path(), |config| {
        config.shell.entry_budget = RUNAWAY_CALL;
        // The restaged generation replays the hanging window while it loads, so staging is what runs out.
        config.shell.js.stage_budget = RUNAWAY_STAGE;
    });
    let ipc = session.ipc();
    wait_for(
        "the window at the script's place",
        &session,
        &mut mapper,
        &shell,
        || spots(&scene(&ipc)).get("org.example.first") == Some(&(700.0, 600.0)),
    );
    let booted = shell.generation();

    // Three strikes in 30 s latch the supervisor: the first is the handler in the running generation, the next two
    // are the restages that hang while they load. The generation that was running stays.
    mapper.map_window("org.example.spin", "Spin");
    wait_for(
        "three strikes latch the supervisor",
        &session,
        &mut mapper,
        &shell,
        || {
            let _ = scene(&ipc);
            shell.degraded()
        },
    );
    assert_eq!(
        shell.generation(),
        booted,
        "no restage could take over: the last generation stays"
    );

    mapper.map_window("org.example.fourth", "Fourth");
    wait_for("the scene still updates", &session, &mut mapper, &shell, || {
        spots(&scene(&ipc)).get("org.example.fourth") == Some(&(700.0, 600.0))
    });
}

/// The workspaces the stock config declares, as the shell's view lists them: the Lua stock config's four.
const STOCK_WORKSPACES: &str = "1:FloatingLayout,2:FloatingLayout,3:FloatingLayout,4:FloatingLayout";

/// A section of the shell's view: what stands between `open` and the next `]`.
fn section<'a>(view: &'a str, open: &str) -> &'a str {
    view.split_once(open)
        .and_then(|(_, rest)| rest.split_once(']'))
        .map_or("", |(inside, _)| inside)
}

/// The triggers the shell's view says are bound, in order.
fn bound_keys(view: &str) -> Vec<&str> {
    section(view, "keys[")
        .split(',')
        .filter(|key| !key.is_empty())
        .collect()
}

const LAYERED_CONFIG: &str = r#"import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    workspaces: [
        { id: "a", layout: "master-stack" },
        { id: "b", layout: "grid" },
    ],
    keys: { "super+Return": spawn("true"), XF86AudioMute: spawn("true") },
});
"#;

#[test]
fn a_shell_with_no_init_file_runs_the_stock_config_and_a_defined_config_layers_over_it() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");
    let config = tempfile::tempdir().expect("a config directory");
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();

    // No `init.ts`: the stock default config is a real runtime config. It declares the four floating workspaces `1` to `4`, and
    // its keys are bound on the seat (through the real compositor, which took the claims or the shell would have
    // reported a refusal).
    wait_for("the stock config runs", &session, &mut mapper, &shell, || {
        section(&shell.view(), "workspaces [") == STOCK_WORKSPACES
    });
    let keys = shell.view();
    let keys = bound_keys(&keys);
    assert_eq!(keys.first(), Some(&"Super+1"), "the stock keys: {keys:?}");
    assert!(keys.contains(&"XF86AudioMute"), "the stock keys: {keys:?}");
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    let stock = shell.generation();
    let window = ids(&scene(&ipc));

    // A user `defineConfig` replaces the workspaces whole and merges the keys by trigger: its `Super+Return` is
    // added, its `XF86AudioMute` replaces the stock one, and the other stock keys stay, `Super+1` included: it shows
    // the workspace at position 1, so it still names something.
    save(&config.path().join("init.ts"), LAYERED_CONFIG);
    wait_for("the layered config runs", &session, &mut mapper, &shell, || {
        section(&shell.view(), "workspaces [") == "a:MasterStackLayout,b:GridLayout"
    });
    assert!(shell.generation() > stock, "the file is a reload");
    let view = shell.view();
    let keys = bound_keys(&view);
    assert!(keys.contains(&"super+Return"), "the user's key: {keys:?}");
    assert_eq!(
        keys.iter().filter(|key| **key == "XF86AudioMute").count(),
        1,
        "{keys:?}"
    );
    assert!(
        keys.contains(&"XF86AudioPlay"),
        "a stock key the user left alone: {keys:?}"
    );
    assert!(
        keys.contains(&"Super+1") && keys.contains(&"Swipe3"),
        "the stock keys that show by position and the slide gesture stay: {keys:?}"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    assert_eq!(ids(&scene(&ipc)), window, "the window kept its id through the reload");

    // And an imperative `init.ts` is still a reload that places the window.
    write_config(config.path(), r#"{ "org.example.first": [21, 22] }"#);
    wait_for(
        "the imperative file places the window",
        &session,
        &mut mapper,
        &shell,
        || spots(&scene(&ipc)).get("org.example.first") == Some(&(21.0, 22.0)),
    );
}

/// Places `y` from `onAdded` and `x` (plus one) from `onReady`, so the two ways a generation learns of a window are
/// told apart by where the window ends up. The workspace's layout places nothing.
fn write_guarded_config(dir: &Path, places: &str) {
    fs::write(
        dir.join("slots.ts"),
        format!("export const places: Record<string, [number, number]> = {places};\n"),
    )
    .expect("slots.ts");
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, Workspace, gnoblin } from "gnoblin";
import { places } from "./slots.ts";

gnoblin.workspaces.add(new Workspace({ id: "desk", layout: new Layout() }));

gnoblin.clients.addListener({
    onAdded(client: { appId: string; y: number }): void {
        const at: [number, number] | undefined = places[client.appId];
        if (at !== undefined) {
            client.y = at[1];
        }
    },
    onReady(client: { appId: string; x: number }): void {
        const at: [number, number] | undefined = places[client.appId];
        if (at !== undefined) {
            client.x = at[0] + 1;
        }
    },
});
"#,
    )
    .expect("init.ts");
}

#[test]
fn a_shell_that_dies_leaves_the_compositor_answering_and_a_restarted_one_reattaches() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    // An observer that stays attached from before the first window: a window the compositor places itself reaches
    // an observer as changes, and a snapshot of the tree does not carry them.
    let watch = Watcher::attach(&ipc);
    mapper.map_window("org.example.first", "First");
    mapper.map_window("org.example.second", "Second");
    // No shell has run yet, so the compositor's own policy placed these: a cascade from (32, 32) by 32.
    wait_for_scene("the fallback cascade", &watch, &mut mapper, |now| {
        now.get("org.example.first") == Some(&(32.0, 32.0)) && now.get("org.example.second") == Some(&(64.0, 64.0))
    });

    let config = tempfile::tempdir().expect("a config directory");
    write_guarded_config(
        config.path(),
        r#"{ "org.example.first": [111, 222], "org.example.second": [333, 444] }"#,
    );
    let shell = Shell::start(&session, config.path(), |_| {});
    wait_for(
        "both windows placed by the shell",
        &session,
        &mut mapper,
        &shell,
        || {
            let now = spots(&watch.scene());
            now.get("org.example.first") == Some(&(112.0, 222.0))
                && now.get("org.example.second") == Some(&(334.0, 444.0))
        },
    );
    let before = ids(&watch.scene());
    assert_eq!(before.len(), 2, "two windows: {before:?}");

    // The shell dies: its thread ends and its owner connection closes, which is all the compositor can see of a
    // process that was killed.
    let log = shell.finish();
    assert!(shell.ended().is_none(), "the shell was stopped, not failed: {log:?}");
    // The compositor has taken the close in once the two windows are back at the places its own policy gave them,
    // which it never knew the shell had changed. A window that mapped before that would be held for a shell that is no
    // longer there.
    wait_for_scene(
        "the fallback policy takes the two windows back",
        &watch,
        &mut mapper,
        |now| {
            now.get("org.example.first") == Some(&(32.0, 32.0)) && now.get("org.example.second") == Some(&(64.0, 64.0))
        },
    );

    // Criterion 6, the compositor's half: with no shell holding the tree it still answers, and the Rust fallback
    // policy is in charge again: a window that maps now takes the next step of the cascade.
    mapper.map_window("org.example.third", "Third");
    let (during, _) = wait_for_scene(
        "the fallback policy placing the three windows",
        &watch,
        &mut mapper,
        |now| {
            now.get("org.example.third") == Some(&(96.0, 96.0))
                && now.get("org.example.first") == Some(&(32.0, 32.0))
                && now.get("org.example.second") == Some(&(64.0, 64.0))
        },
    );
    assert_eq!(
        scene(&ipc).len(),
        3,
        "the compositor still answers a fresh connection while no shell holds the tree"
    );
    assert_eq!(
        ids(&during).get("org.example.first"),
        before.get("org.example.first"),
        "the window ids survive the shell"
    );

    // The shell's half: a restarted shell takes the tree again, is told every window is ready (`x` + 1) and learns
    // of each (`y`), places the window mapped while it was away, and none of them changes its id.
    write_guarded_config(
        config.path(),
        r#"{ "org.example.first": [500, 400], "org.example.second": [600, 300], "org.example.third": [900, 800] }"#,
    );
    let shell = Shell::start(&session, config.path(), |_| {});
    wait_for("the restarted shell reattaches", &session, &mut mapper, &shell, || {
        let now = spots(&watch.scene());
        now.get("org.example.first") == Some(&(501.0, 400.0))
            && now.get("org.example.second") == Some(&(601.0, 300.0))
            && now.get("org.example.third") == Some(&(901.0, 800.0))
    });
    let after = ids(&watch.scene());
    assert_eq!(after.get("org.example.first"), before.get("org.example.first"));
    assert_eq!(after.get("org.example.second"), before.get("org.example.second"));
    assert_eq!(
        after.get("org.example.third"),
        ids(&during).get("org.example.third"),
        "no window was re-created"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
}

/// The UI toolkit against the real compositor (design/js-runtime.md, "P7 UI toolkit status"): a script makes a `Box`
/// and asks for decorations, and the compositor's own tree, read through an observer, holds what the shell wrote: a
/// canvas of the box's size with a paint list the compositor decoded (a list it could not decode would have been
/// refused `bad-paint` and never stored), and one titlebar canvas under the mapped window with the window's title and
/// the close icon painted in its subtree. The box's list is then drawn with the compositor's painter and a pixel read.
#[test]
fn a_script_draws_a_box_and_a_titlebar_and_the_compositor_holds_what_it_painted() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.decorated", "Decorated");

    let config = tempfile::tempdir().expect("a config directory");
    fs::write(
        config.path().join("init.ts"),
        r##"import { Box, defineConfig, gnoblin } from "gnoblin";

defineConfig({ decorations: true });

// The compositor gives a window no size until the shell does, and a titlebar is as wide as its window.
gnoblin.clients.addListener({
    onAdded(client: { width: number; height: number }): void {
        client.width = 300;
        client.height = 200;
    },
});

// A canvas of its own: a node with no parent, as a script's first canvas is.
export const swatch = new Box({ width: 120, height: 40, fill: "#336699" });
"##,
    )
    .expect("init.ts");
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();

    let mut found: Vec<Canvas> = Vec::new();
    wait_for(
        "the swatch, the titlebar and its title painted",
        &session,
        &mut mapper,
        &shell,
        || {
            found = canvases(&ipc);
            let window = scene(&ipc)
                .into_iter()
                .find(|window| window.app_id == "org.example.decorated");
            let titled = found.iter().any(|canvas| {
                canvas.list.as_ref().is_some_and(|list| {
                    list.ops
                        .iter()
                        .any(|op| matches!(op, gnoblin_paint::PaintOp::Text { text, .. } if text == "Decorated"))
                })
            });
            window.is_some_and(|window| found.iter().any(|canvas| canvas.parent == window.id))
                && titled
                && found
                    .iter()
                    .any(|canvas| canvas.width == 120.0 && canvas.list.is_some())
        },
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());

    let swatch = found
        .iter()
        .find(|canvas| canvas.width == 120.0 && canvas.height == 40.0)
        .expect("the box is a canvas of its size");
    assert_eq!(swatch.parent, 0, "the box was never given a parent");
    assert_eq!(
        swatch.picture().pixel(10, 10),
        Some([0x33, 0x66, 0x99, 0xff]),
        "the compositor's painter draws the box's fill"
    );

    let window = scene(&ipc)
        .into_iter()
        .find(|window| window.app_id == "org.example.decorated")
        .expect("the mapped window");
    let bars: Vec<&Canvas> = found.iter().filter(|canvas| canvas.parent == window.id).collect();
    assert_eq!(bars.len(), 1, "exactly one titlebar under the window: {bars:?}");
    let fills_bar = |canvas: &Canvas| {
        canvas.list.as_ref().is_some_and(|list| {
            list.ops.iter().any(|op| {
                matches!(op, gnoblin_paint::PaintOp::Rect { fill, .. }
                    if [fill.r, fill.g, fill.b] == [0x41, 0x45, 0x59] || [fill.r, fill.g, fill.b] == [0x23, 0x26, 0x34])
            })
        })
    };
    assert!(
        fills_bar(bars[0]),
        "the bar is filled with a titlebar colour: {:?}",
        bars[0]
    );
    let icons = found
        .iter()
        .filter(|canvas| {
            canvas.list.as_ref().is_some_and(|list| {
                list.ops.iter().any(
                    |op| matches!(op, gnoblin_paint::PaintOp::Icon { name, .. } if name == "window-close-symbolic"),
                )
            })
        })
        .count();
    assert_eq!(icons, 1, "one close icon");
    assert_eq!(
        found.len(),
        7,
        "the box, and the bar with its drag area, title and three buttons: {found:?}"
    );
    let log = shell.finish();
    assert!(
        log.iter().all(|line| !line.contains("threw")),
        "the shell logged a throw: {log:#?}"
    );
}

// ---- pointer chord drags: move and resize a real window ----------------------------------------------------------

/// The evdev codes of the buttons and the key the stock bindings are made of: Super+Button1 moves, Super+Button3
/// resizes.
const BTN_LEFT: u32 = 272;
const BTN_RIGHT: u32 = 273;
const KEY_LEFTMETA: u32 = 125;

/// Sends `event` to the session as a device would: it takes the seat path of a real pointer's or keyboard's.
fn send(session: &Session, event: InputEvent) {
    assert!(session.input.inject(event), "the session is serving input");
}

fn motion(session: &Session, x: i32, y: i32) {
    send(
        session,
        InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output: OutputId::new(0),
            position: Point::new(x, y),
        },
    );
}

fn button(session: &Session, code: u32, pressed: bool) {
    send(
        session,
        InputEvent::PointerButton {
            device: DeviceId::VIRTUAL,
            button: code,
            pressed,
        },
    );
}

fn super_key(session: &Session, pressed: bool) {
    send(
        session,
        InputEvent::Key {
            device: DeviceId::VIRTUAL,
            keycode: KEY_LEFTMETA,
            pressed,
        },
    );
}

/// The rectangle the compositor holds for the window of `app`.
fn rect_of(ipc: &Path, app: &str) -> Option<(f64, f64, f64, f64)> {
    scene(ipc)
        .into_iter()
        .find(|window| window.app_id == app)
        .map(|window| (window.x, window.y, window.width, window.height))
}

/// Holds Super and presses `code` at `from`, moves the pointer to `first`, and returns once the compositor holds
/// `expected` for `app`: the drag is live and has moved the window.
///
/// The shell's claims of its chords reach the compositor a few frames after the shell starts. A press that comes before
/// that is nobody's claim: the compositor gives it to the window, and the window's client hears it. That tells the two
/// apart with no time guessed. A press the client heard moved nothing, is let go of, and the gesture is repeated; one the
/// client did not hear is the shell's, and the window follows it, however long the shell takes to say so. Super and the
/// button stay down for the caller to release with [`end_drag`].
#[track_caller]
#[allow(clippy::too_many_arguments)]
fn begin_drag(
    session: &Session,
    mapper: &mut Mapper,
    shell: &Shell,
    app: &str,
    code: u32,
    from: (i32, i32),
    first: (i32, i32),
    expected: (f64, f64, f64, f64),
) {
    let ipc = session.ipc();
    let pointer = mapper.pointer();
    let deadline = Instant::now() + PATIENCE;
    let mut attempts = 0_u32;
    loop {
        attempts += 1;
        super_key(session, true);
        motion(session, from.0, from.1);
        // The press is the window's, if nobody has claimed it, only once the compositor has the pointer over the window.
        wait_for("the pointer over the window", session, mapper, shell, || pointer.over());
        let heard = pointer.presses();
        button(session, code, true);
        motion(session, first.0, first.1);
        let mut unclaimed = false;
        wait_for(
            "the drag took the window, or the press reached the window's client",
            session,
            mapper,
            shell,
            || {
                unclaimed = pointer.presses() > heard;
                unclaimed || rect_of(&ipc, app) == Some(expected)
            },
        );
        if !unclaimed {
            return;
        }
        button(session, code, false);
        super_key(session, false);
        assert!(
            Instant::now() < deadline,
            "no drag ever took the window: {attempts} presses reached its client instead, and it is at {:?}, \
             expected {expected:?}",
            rect_of(&ipc, app)
        );
        std::thread::sleep(POLL);
    }
}

/// Moves the pointer on and waits for the window to follow it to `expected`.
#[track_caller]
fn follow(session: &Session, mapper: &mut Mapper, app: &str, to: (i32, i32), expected: (f64, f64, f64, f64)) {
    let ipc = session.ipc();
    motion(session, to.0, to.1);
    let deadline = Instant::now() + PATIENCE;
    loop {
        mapper.roundtrip();
        let now = rect_of(&ipc, app);
        if now == Some(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the window never followed the pointer to {to:?}: it is at {now:?}, expected {expected:?}"
        );
        std::thread::sleep(POLL);
    }
}

/// Lets go of the button and of Super, and checks the window stays where the drag left it.
#[track_caller]
fn end_drag(session: &Session, mapper: &mut Mapper, app: &str, code: u32, expected: (f64, f64, f64, f64)) {
    button(session, code, false);
    super_key(session, false);
    let ipc = session.ipc();
    let settled = Instant::now() + Duration::from_millis(500);
    while Instant::now() < settled {
        mapper.roundtrip();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        rect_of(&ipc, app),
        Some(expected),
        "the window stays where the drag left it"
    );
}

/// A config directory whose `init.ts` creates a floating workspace with the default `FloatingLayout` and puts every
/// window at (300, 200), 200 by 150.
fn write_drag_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { FloatingLayout, Workspace, gnoblin } from "gnoblin";

gnoblin.workspaces.add(new Workspace({ id: "desk", layout: new FloatingLayout() }));

// The window starts at a known place and size. The stock layout leaves a window that has floating bounds alone.
gnoblin.clients.addListener({
    onAdded(client: { x: number; y: number; width: number; height: number; floatingBounds?: object }): void {
        client.x = 300;
        client.y = 200;
        client.width = 200;
        client.height = 150;
        client.floatingBounds = { x: 300, y: 200, width: 200, height: 150 };
    },
});
"#,
    )
    .expect("init.ts");
}

#[test]
fn a_super_drag_takes_the_window_on_top_where_two_overlap() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_drag_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    // Both sit at the same place; the one mapped last is drawn above the other.
    mapper.map_window("org.example.under", "Under");
    mapper.map_window("org.example.over", "Over");
    let (under, over) = ("org.example.under", "org.example.over");
    let ipc = session.ipc();
    let start = Some((300.0, 200.0, 200.0, 150.0));
    wait_for("both windows at their start", &session, &mut mapper, &shell, || {
        rect_of(&ipc, under) == start && rect_of(&ipc, over) == start
    });
    // The press is over both: the window above takes it, and the one below stays.
    begin_drag(
        &session,
        &mut mapper,
        &shell,
        over,
        BTN_LEFT,
        (350, 250),
        (400, 290),
        (350.0, 240.0, 200.0, 150.0),
    );
    end_drag(&session, &mut mapper, over, BTN_LEFT, (350.0, 240.0, 200.0, 150.0));
    assert_eq!(rect_of(&ipc, under), start, "the window below stayed");
}

#[test]
fn super_drags_move_and_resize_a_real_window_through_the_pointer_chord_stream() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_drag_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.drag", "Drag");
    let app = "org.example.drag";
    let ipc = session.ipc();
    wait_for("the window at its start", &session, &mut mapper, &shell, || {
        rect_of(&ipc, app) == Some((300.0, 200.0, 200.0, 150.0))
    });

    // Super+Button1 moves the window by the pointer's delta. The press is at (350, 250), inside the window, so the
    // pointer's space is the window's space.
    begin_drag(
        &session,
        &mut mapper,
        &shell,
        app,
        BTN_LEFT,
        (350, 250),
        (400, 290),
        (350.0, 240.0, 200.0, 150.0),
    );
    follow(&session, &mut mapper, app, (410, 300), (360.0, 250.0, 200.0, 150.0));
    end_drag(&session, &mut mapper, app, BTN_LEFT, (360.0, 250.0, 200.0, 150.0));

    // Super+Button3 from the bottom-right quadrant moves that corner and keeps the top-left one.
    begin_drag(
        &session,
        &mut mapper,
        &shell,
        app,
        BTN_RIGHT,
        (550, 390),
        (590, 420),
        (360.0, 250.0, 240.0, 180.0),
    );
    follow(&session, &mut mapper, app, (610, 440), (360.0, 250.0, 260.0, 200.0));
    end_drag(&session, &mut mapper, app, BTN_RIGHT, (360.0, 250.0, 260.0, 200.0));

    // From the top-left quadrant that corner moves, and it is still the one that moves when the pointer goes past the
    // opposite corner: the bottom-right corner (620, 450) stays and the window is held at its minimum size, 100 by 80.
    begin_drag(
        &session,
        &mut mapper,
        &shell,
        app,
        BTN_RIGHT,
        (370, 260),
        (420, 290),
        (410.0, 280.0, 210.0, 170.0),
    );
    follow(&session, &mut mapper, app, (1200, 900), (520.0, 370.0, 100.0, 80.0));
    end_drag(&session, &mut mapper, app, BTN_RIGHT, (520.0, 370.0, 100.0, 80.0));

    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    let log = shell.finish();
    assert!(
        log.iter().all(|message| !message.contains("claim")),
        "the shell logged nothing about a claim: {log:?}"
    );
}

/// The size of the compositor's first screen, as the shell tiles in it.
fn screen_size(ipc: &Path) -> (f64, f64) {
    let nodes = snapshot(ipc);
    let screen = nodes
        .iter()
        .find(|node| node.class == "screen")
        .expect("the session has a screen");
    let value = |key: &str| {
        screen
            .props
            .iter()
            .find(|prop| prop.key == key)
            .map_or(0.0, |prop| prop.value_double)
    };
    (value("width"), value("height"))
}

/// An `init.ts` with one workspace whose layout is `layout`, a TypeScript expression.
fn write_layout_config(dir: &Path, layout: &str) {
    fs::write(
        dir.join("init.ts"),
        format!(
            "import {{ DwindleLayout, StripLayout, Workspace, gnoblin }} from \"gnoblin\";\n\
             gnoblin.workspaces.add(new Workspace({{ id: \"desk\", layout: {layout} }}));\n"
        ),
    )
    .expect("init.ts");
}

#[test]
fn a_dwindle_workspace_places_two_real_windows_side_by_side_at_the_halves_of_the_screen() {
    // Lua tiling_test.lua "Dwindle: 2 and 3 windows split the last cell by aspect", against the real compositor: two
    // real Wayland windows are mapped while the shell runs, and what the compositor's tree holds for them is the first
    // half and the second half of the screen, floored to whole pixels.
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_layout_config(config.path(), "new DwindleLayout()");
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    let (width, height) = screen_size(&ipc);
    assert!(
        width >= height && width >= 2.0,
        "a landscape screen splits side by side: {width}x{height}"
    );
    mapper.map_window("org.example.first", "First");
    mapper.map_window("org.example.second", "Second");
    let half = (width * 0.5).floor();
    let first = Some((0.0, 0.0, half, height.floor()));
    let second = Some((half, 0.0, (width - width * 0.5).floor(), height.floor()));
    wait_for("two windows tiled side by side", &session, &mut mapper, &shell, || {
        rect_of(&ipc, "org.example.first") == first && rect_of(&ipc, "org.example.second") == second
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
}

#[test]
fn a_strip_workspace_places_two_real_windows_in_columns_and_scrolls_to_the_focused_one() {
    // strip_test.lua "a strip lays out one column per window": a column is half the strip wide and the full height. With
    // two windows the strip is exactly as wide as the screen, so nothing scrolls whichever has focus.
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_layout_config(config.path(), "new StripLayout()");
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    let (width, height) = screen_size(&ipc);
    mapper.map_window("org.example.first", "First");
    mapper.map_window("org.example.second", "Second");
    let column = width * 0.5;
    let first = Some((0.0, 0.0, column, height));
    let second = Some((column, 0.0, column, height));
    wait_for("two windows in two columns", &session, &mut mapper, &shell, || {
        rect_of(&ipc, "org.example.first") == first && rect_of(&ipc, "org.example.second") == second
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
}

/// A config directory whose `init.ts` declares one workspace that places nothing and one window rule: a window of
/// `org.example.placed` goes to (321, 123) and is 222 by 111. No listener of the script touches a window.
fn write_rules_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, defineConfig } from "gnoblin";

defineConfig({
    workspaces: [{ id: "desk", layout: new Layout() }],
    rules: [
        { appId: "org.example.placed", position: { x: 321, y: 123 }, size: { width: 222, height: 111 } },
    ],
});
"#,
    )
    .expect("init.ts");
}

#[test]
fn a_window_rule_places_a_newly_mapped_window_of_its_app_and_leaves_another_alone() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_rules_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    // A window the shell found at boot is not the rules' to place, so the windows map once the config is running.
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 1 workspaces")
    });

    let (other, placed) = ("org.example.other", "org.example.placed");
    mapper.map_window(other, "Other");
    wait_for("the shell sees the other window", &session, &mut mapper, &shell, || {
        shell.view().contains(other) && rect_of(&ipc, other).is_some()
    });
    let untouched = rect_of(&ipc, other).expect("the other window is in the scene");
    mapper.map_window(placed, "Placed");
    wait_for("the rule places its window", &session, &mut mapper, &shell, || {
        rect_of(&ipc, placed) == Some((321.0, 123.0, 222.0, 111.0))
    });
    // The shell handled the other window before this one mapped and handles what the compositor tells it in order, so
    // when this window is where its rule puts it, the rule has been run against both.
    mapper.roundtrip();
    assert_eq!(
        rect_of(&ipc, other),
        Some(untouched),
        "the window of another app is where it was: {:?}",
        scene(&ipc)
    );
    assert_ne!(
        (untouched.0, untouched.1),
        (321.0, 123.0),
        "the other window did not happen to start at the rule's place"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

/// A config directory whose rules put `org.example.under` at (300, 200) and `org.example.over` at (400, 260), both
/// 200 by 150 so they overlap, and whose listener moves a window one pixel to the right whenever a press lands on it.
fn write_click_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, defineConfig, gnoblin } from "gnoblin";

defineConfig({
    workspaces: [{ id: "desk", layout: new Layout() }],
    rules: [
        { appId: "org.example.under", position: { x: 300, y: 200 }, size: { width: 200, height: 150 } },
        { appId: "org.example.over", position: { x: 400, y: 260 }, size: { width: 200, height: 150 } },
    ],
});

// The press is the proof of which window the compositor picked: a window that heard it moves one pixel.
gnoblin.clients.addListener({
    onPointerPress(client: { x: number }): void {
        client.x = client.x + 1;
    },
});
"#,
    )
    .expect("init.ts");
}

/// The app id of the window the seat's keyboard focus is on, as the shell reads the seat the compositor publishes. A
/// window's own `focused` is not kept current while a shell holds the tree, so the seat is where focus is read.
fn focused_app(shell: &Shell) -> Option<String> {
    let view = shell.view();
    let (_, focus) = view.split_once(" | focus=")?;
    (focus != "undefined").then(|| focus.to_owned())
}

/// Moves the pointer to `at` and waits until the compositor has it over a window of the client, which the client is told
/// when it is. A press that comes before is no window's, so it is not sent until the pointer is there.
#[track_caller]
fn point_at(session: &Session, mapper: &mut Mapper, shell: &Shell, at: (i32, i32)) {
    let pointer = mapper.pointer();
    motion(session, at.0, at.1);
    wait_for("the pointer over a window", session, mapper, shell, || pointer.over());
}

/// A press and release of the left button where the pointer is.
fn click(session: &Session) {
    button(session, BTN_LEFT, true);
    button(session, BTN_LEFT, false);
}

#[test]
fn a_click_focuses_and_raises_a_real_window_so_the_compositor_picks_it_over_the_one_above() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_click_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 1 workspaces")
    });
    let (under, over) = ("org.example.under", "org.example.over");
    mapper.map_window(under, "Under");
    mapper.map_window(over, "Over");
    wait_for("both windows at their places", &session, &mut mapper, &shell, || {
        rect_of(&ipc, under) == Some((300.0, 200.0, 200.0, 150.0))
            && rect_of(&ipc, over) == Some((400.0, 260.0, 200.0, 150.0))
    });
    // A window the shell found new takes the focus: the last one mapped has it.
    wait_for("the new window takes the focus", &session, &mut mapper, &shell, || {
        focused_app(&shell).as_deref() == Some(over)
    });

    // Where only the window below shows: the press is its, and the shell focuses and raises it. The click is sent once
    // the pointer is over the window, which is all that has to settle, so there is one click and the window has heard it
    // once.
    point_at(&session, &mut mapper, &shell, (320, 210));
    click(&session);
    wait_for(
        "the window below hears the press",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, under).map(|rect| rect.0) == Some(301.0),
    );
    wait_for("the window below has the focus", &session, &mut mapper, &shell, || {
        focused_app(&shell).as_deref() == Some(under)
    });

    // Where both show: the compositor gives the press to the window it draws on top. It is the one that was clicked
    // only if the raise reached the compositor's stacking.
    point_at(&session, &mut mapper, &shell, (450, 300));
    click(&session);
    wait_for("a press lands on one of the two", &session, &mut mapper, &shell, || {
        rect_of(&ipc, under).map(|rect| rect.0) == Some(302.0) || rect_of(&ipc, over).map(|rect| rect.0) == Some(401.0)
    });
    assert_eq!(
        rect_of(&ipc, under).map(|rect| rect.0),
        Some(302.0),
        "the raised window is the one the compositor picked: {:?}",
        scene(&ipc)
    );
    assert_eq!(rect_of(&ipc, over).map(|rect| rect.0), Some(400.0));
    assert_eq!(focused_app(&shell).as_deref(), Some(under));
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

// ---- a client's requests of its window: fullscreen, minimise, activate -------------------------------------------

/// A config directory whose floating workspace places nothing itself; rules put `first` at (100, 100), `second` at
/// (400, 300) and `third` at (250, 200), all 200 by 150, so `first` and `third` overlap. A press that lands on a window
/// moves it one pixel to the right, which is how a test reads which window the compositor picked.
fn write_requests_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, defineConfig, gnoblin } from "gnoblin";

defineConfig({
    workspaces: [{ id: "desk", layout: new Layout() }],
    decorations: false,
    rules: [
        { appId: "org.example.first", position: { x: 100, y: 100 }, size: { width: 200, height: 150 } },
        { appId: "org.example.second", position: { x: 400, y: 300 }, size: { width: 200, height: 150 } },
        { appId: "org.example.third", position: { x: 250, y: 200 }, size: { width: 200, height: 150 } },
    ],
});

gnoblin.clients.addListener({
    onPointerPress(client: { x: number }): void {
        client.x = client.x + 1;
    },
});
"#,
    )
    .expect("init.ts");
}

/// A session with the requests config running and a client connected, before any window is mapped.
fn requests_session() -> (Session, tempfile::TempDir, Shell, Mapper) {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_requests_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 1 workspaces")
    });
    (session, config, shell, mapper)
}

/// Round trips with the compositor until `check` holds of what the client has been told.
fn wait_client(what: &str, mapper: &mut Mapper, mut check: impl FnMut(&Mapper) -> bool) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        mapper.roundtrip();
        if check(mapper) {
            return;
        }
        assert!(Instant::now() < deadline, "the client was never told: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_client_fullscreen_request_is_granted_over_the_whole_output_and_unsetting_it_returns_the_window() {
    let (session, _config, shell, mut mapper) = requests_session();
    let ipc = session.ipc();
    let first = "org.example.first";
    mapper.map_window(first, "First");
    wait_for("the window at its place", &session, &mut mapper, &shell, || {
        rect_of(&ipc, first) == Some((100.0, 100.0, 200.0, 150.0))
    });
    let (width, height) = screen_size(&ipc);

    // `xdg_toplevel.set_fullscreen`: the compositor asks the shell, and the shell's answer is the whole output, not
    // the area the bars leave.
    mapper.fullscreen(0);
    wait_for("the compositor holds the output", &session, &mut mapper, &shell, || {
        rect_of(&ipc, first) == Some((0.0, 0.0, width, height)) && flag_of(&ipc, first, "fullscreen")
    });
    wait_client("the fullscreen configure", &mut mapper, |client| {
        client.states(0).contains(&support::STATE_FULLSCREEN)
            && client.configured(0) == Some((width as i32, height as i32))
    });

    // `unset_fullscreen`: the window returns to the rectangle it had, and the client is told.
    mapper.unfullscreen(0);
    wait_for("the window is back", &session, &mut mapper, &shell, || {
        rect_of(&ipc, first) == Some((100.0, 100.0, 200.0, 150.0)) && !flag_of(&ipc, first, "fullscreen")
    });
    wait_client("the configure that leaves fullscreen", &mut mapper, |client| {
        !client.states(0).contains(&support::STATE_FULLSCREEN)
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

#[test]
fn a_client_minimise_request_is_granted_and_focus_goes_to_the_next_window() {
    let (session, _config, shell, mut mapper) = requests_session();
    let ipc = session.ipc();
    let (first, second) = ("org.example.first", "org.example.second");
    mapper.map_window(first, "First");
    mapper.map_window(second, "Second");
    wait_for("both windows at their places", &session, &mut mapper, &shell, || {
        rect_of(&ipc, first) == Some((100.0, 100.0, 200.0, 150.0))
            && rect_of(&ipc, second) == Some((400.0, 300.0, 200.0, 150.0))
    });
    wait_for("the new window takes the focus", &session, &mut mapper, &shell, || {
        focused_app(&shell).as_deref() == Some(second)
    });

    // `xdg_toplevel.set_minimized` on the window that has the focus.
    mapper.minimize(1);
    wait_for("the window is minimised", &session, &mut mapper, &shell, || {
        flag_of(&ipc, second, "minimised")
    });
    wait_for(
        "the focus goes to the window that stays",
        &session,
        &mut mapper,
        &shell,
        || focused_app(&shell).as_deref() == Some(first),
    );
    assert!(!flag_of(&ipc, first, "minimised"));
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

#[test]
fn a_client_activate_request_raises_and_focuses_its_window() {
    let (session, _config, shell, mut mapper) = requests_session();
    let ipc = session.ipc();
    mapper.listen_pointer();
    let (first, third) = ("org.example.first", "org.example.third");
    mapper.map_window(first, "First");
    wait_for("the first window at its place", &session, &mut mapper, &shell, || {
        rect_of(&ipc, first) == Some((100.0, 100.0, 200.0, 150.0))
    });

    // A click on the first window earns it the serial an activation token needs. The click is sent once the pointer is
    // over the window, which is all that has to settle, so there is one click and the window has heard it once.
    let pointer = mapper.pointer();
    point_at(&session, &mut mapper, &shell, (120, 110));
    click(&session);
    wait_for(
        "the first window hears the click",
        &session,
        &mut mapper,
        &shell,
        || pointer.presses() > 0 && rect_of(&ipc, first).map(|rect| rect.0) == Some(101.0),
    );
    let serial = mapper.press_serial().expect("the click was sent to the client");

    // A third window appears over it and takes the focus.
    mapper.map_window(third, "Third");
    wait_for(
        "the third window takes the focus",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, third) == Some((250.0, 200.0, 200.0, 150.0)) && focused_app(&shell).as_deref() == Some(third),
    );

    // The first window redeems its token: the compositor asks the shell, which focuses and raises it.
    mapper.activate(0, serial);
    wait_for(
        "the activated window has the focus",
        &session,
        &mut mapper,
        &shell,
        || focused_app(&shell).as_deref() == Some(first),
    );
    // Where both show, the compositor gives a press to the window it draws on top: it is `first` only if the raise
    // reached the compositor's stacking.
    point_at(&session, &mut mapper, &shell, (280, 230));
    click(&session);
    wait_for("a press lands on one of the two", &session, &mut mapper, &shell, || {
        rect_of(&ipc, first).map(|rect| rect.0) == Some(102.0) || rect_of(&ipc, third).map(|rect| rect.0) == Some(251.0)
    });
    assert_eq!(
        rect_of(&ipc, first).map(|rect| rect.0),
        Some(102.0),
        "the activated window is the one the compositor picked: {:?}",
        scene(&ipc)
    );
    assert_eq!(rect_of(&ipc, third).map(|rect| rect.0), Some(250.0));
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

// ---- the configuration document and the dialog observation ------------------------------------------------------

/// A fresh owner connection to the session, for what a test reads or asks of the compositor itself.
fn owner_client(ipc: &Path) -> Client<UnixTransport> {
    let transport = UnixTransport::connect(ipc).expect("the session is listening");
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Client::connect(transport, Version::new(major, minor, patch)).expect("the handshake")
}

/// The configuration document the compositor holds, or `None` while it has accepted none.
fn document(ipc: &Path) -> Option<serde_json::Value> {
    let (_, bytes) = owner_client(ipc).configuration().expect("the document reads")?;
    Some(serde_json::from_slice(&bytes).expect("the document is JSON"))
}

fn radius(ipc: &Path) -> Option<i64> {
    document(ipc)?.pointer("/dialogs/style/radius")?.as_i64()
}

#[test]
fn defined_config_settings_reach_the_compositors_document_a_refused_value_is_reported_and_a_reload_takes_them_back() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let dir = tempfile::tempdir().expect("a config directory");
    let init = dir.path().join("init.ts");
    let write = |settings: &str| {
        save(
            &init,
            &format!("import {{ defineConfig }} from \"gnoblin\";\ndefineConfig({{ {settings} }});\n"),
        );
    };
    write("dialogs: { style: { radius: 12, buttonLayout: 'stretch' } }");
    let shell = Shell::start(&session, dir.path(), |_| {});

    wait_for("the document carries the radius", &session, &mut mapper, &shell, || {
        radius(&session.ipc()) == Some(12)
    });
    assert_eq!(
        document(&session.ipc()).and_then(|document| document.pointer("/dialogs/style/button-layout").cloned()),
        Some(serde_json::json!("stretch")),
        "a camelCase key is the document's kebab-case one"
    );

    // The compositor's own validation refuses 5000, and the reload is reported with the key path.
    wait_for("the first generation runs", &session, &mut mapper, &shell, || {
        shell.generation() > 0
    });
    let generation = shell.generation();
    write("dialogs: { style: { radius: 5000 } }");
    wait_for("the refusal is reported", &session, &mut mapper, &shell, || {
        shell
            .diagnostics()
            .iter()
            .any(|text| text.contains("dialogs.style.radius"))
    });
    assert_eq!(radius(&session.ipc()), Some(12), "the document in force is untouched");
    assert_eq!(shell.generation(), generation, "the running generation stays");

    // A reload that no longer sets the key puts it back to its default: it is gone from the document.
    write("");
    wait_for(
        "the setting is back at its default",
        &session,
        &mut mapper,
        &shell,
        || document(&session.ipc()).is_some_and(|document| document.pointer("/dialogs").is_none()),
    );
    wait_for(
        "the reload swapped the generation in",
        &session,
        &mut mapper,
        &shell,
        || shell.generation() > generation,
    );
}

/// The apps the policies of the tests below tell apart: one is refused, one is left alone, one never returns.
const STRICT: &str = "org.example.Strict";
const OTHER: &str = "org.example.Other";
const SLOW: &str = "org.example.Slow";

/// A screenshot request as a portal asks for it: the interface, and the app the portal verified as asking.
fn consent_request(app_id: &str) -> PortalRequest {
    PortalRequest {
        interface: "Screenshot".to_owned(),
        caller: VerifiedCaller::new(0, app_id),
        prompt: "Allow this?".to_owned(),
        timeout_ms: 30_000,
        options: Fields::default(),
        parent_window: String::new(),
    }
}

/// How the compositor settled `ticket` within `within`, or `None` while the request is still being asked.
/// `Client::portal_outcome` waits as long as the request lives, so this reads events and takes the outcome when it
/// has arrived.
fn settled(
    client: &mut Client<UnixTransport>,
    ticket: PortalTicket,
    within: Duration,
) -> Option<Result<PortalOutcome, ClientError>> {
    client
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("a read deadline");
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        match client.next_event() {
            Ok(_) | Err(ClientError::Idle) => {}
            Err(error) => panic!("the requester's connection failed: {error}"),
        }
        if let Some(outcome) = client.take_portal_outcome(ticket) {
            return Some(outcome);
        }
    }
    None
}

/// What the compositor did with a consent request.
#[derive(Debug, PartialEq, Eq)]
enum Consent {
    /// A dialog listener refused it at once, which reads as the user's own refusal.
    Refused,
    /// Nobody refused it, so the built-in dialog asked the user and the request stayed open until this withdrew it.
    Asked,
    /// The compositor was still taking an earlier request out of its dialog queue and did not look at this one.
    Busy,
}

/// Sends a consent request from `app_id` and says what became of it. A request nobody refuses is asked of the user by
/// the built-in dialog and stays open, so this withdraws it; the compositor answers the next request `busy` until it
/// has taken the withdrawn one out, which [`consent_answer`] waits out.
fn consent(session: &Session, app_id: &str) -> Consent {
    let mut requester = owner_client(&session.ipc());
    let ticket = requester
        .portal_ui(&consent_request(app_id))
        .expect("the request is sent");
    match settled(&mut requester, ticket, Duration::from_millis(1500)) {
        Some(Ok(PortalOutcome::Cancelled(who))) if who == "user" => Consent::Refused,
        Some(Err(ClientError::Refused { code, .. })) if code == "busy" => Consent::Busy,
        Some(other) => panic!("a consent request settled as neither a refusal nor a busy dialog: {other:?}"),
        None => {
            // The withdrawal's own acknowledgement carries the ticket's id and no outcome, so the outcome is not
            // read: the requester leaving ends the request as well.
            requester.portal_cancel(ticket).expect("the withdrawal is sent");
            Consent::Asked
        }
    }
}

/// Asks about `app_id` until the compositor looks at the request, and returns what it did. The shell's observation of
/// the dialogs is claimed in its first frames and a request is shown to it for a short window only, so a request that
/// the shell was too slow for is asked of the user and this returns [`Consent::Asked`] for it: a test that waits for a
/// refusal asks again.
#[track_caller]
fn consent_answer(session: &Session, mapper: &mut Mapper, shell: &Shell, app_id: &str) -> Consent {
    let mut answer = Consent::Busy;
    wait_for(
        "the compositor looks at a consent request",
        session,
        mapper,
        shell,
        || {
            answer = consent(session, app_id);
            answer != Consent::Busy
        },
    );
    answer
}

/// Asks about `app_id` until a dialog listener refuses it. The listener's refusal has to reach the compositor within
/// the observation window of the request, which a shell that a loaded machine is slow to schedule can miss, so the
/// request is repeated.
#[track_caller]
fn refused_once_observing(session: &Session, mapper: &mut Mapper, shell: &Shell, app_id: &str) {
    wait_for("a listener refuses the request", session, mapper, shell, || {
        consent(session, app_id) == Consent::Refused
    });
}

const POLICY: &str = r#"import { DialogListener, defineConfig } from "gnoblin";

class Policy extends DialogListener {
    onConsentRequest(request: { subject: string; appId: string; decline(reason?: string): void }): void {
        if (request.subject === "Screenshot" && request.appId === "org.example.Strict") {
            request.decline("screenshots are off");
        }
    }
}

defineConfig({
    setup(gnoblin: { dialogs: { addListener(listener: DialogListener): void } }): void {
        gnoblin.dialogs.addListener(new Policy());
    },
});
"#;

#[test]
fn a_dialog_listener_that_declines_a_consent_request_refuses_it_and_one_it_leaves_alone_reaches_the_built_in_dialog() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let dir = tempfile::tempdir().expect("a config directory");
    fs::write(dir.path().join("init.ts"), POLICY).expect("init.ts");
    let shell = Shell::start(&session, dir.path(), |_| {});

    refused_once_observing(&session, &mut mapper, &shell, STRICT);
    assert_eq!(
        consent_answer(&session, &mut mapper, &shell, OTHER),
        Consent::Asked,
        "a request the listener did not decline is asked of the user by the built-in dialog"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
}

#[test]
fn a_dialog_listener_that_runs_out_its_budget_is_skipped_and_the_built_in_dialog_opens() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let dir = tempfile::tempdir().expect("a config directory");
    fs::write(
        dir.path().join("init.ts"),
        r#"import { DialogListener, defineConfig } from "gnoblin";

class Policy extends DialogListener {
    onConsentRequest(request: { appId: string; decline(reason?: string): void }): void {
        if (request.appId === "org.example.Slow") {
            while (true) {}
        }
        request.decline("policy");
    }
}

defineConfig({
    setup(gnoblin: { dialogs: { addListener(listener: DialogListener): void } }): void {
        gnoblin.dialogs.addListener(new Policy());
    },
});
"#,
    )
    .expect("init.ts");
    let shell = Shell::start(&session, dir.path(), |config| {
        config.shell.entry_budget = RUNAWAY_CALL;
    });

    refused_once_observing(&session, &mut mapper, &shell, STRICT);
    let generation = shell.generation();
    assert_eq!(
        consent_answer(&session, &mut mapper, &shell, SLOW),
        Consent::Asked,
        "the runaway listener is skipped: nothing refuses the request and the built-in dialog opens"
    );
    wait_for("the runaway call is reported", &session, &mut mapper, &shell, || {
        shell
            .diagnostics()
            .iter()
            .any(|text| text.contains("a listener call was stopped"))
    });
    // The shell restages the generation after the strike and observes again.
    wait_for("the generation restarted", &session, &mut mapper, &shell, || {
        shell.generation() > generation
    });
    refused_once_observing(&session, &mut mapper, &shell, STRICT);
}

// ---- the stock UI: reserved titlebar room, the window menu and the not-responding alert ---------------------------

const KEY_ESC: u32 = 1;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;

/// Presses and releases a key as a keyboard would.
fn tap_key(session: &Session, keycode: u32) {
    for pressed in [true, false] {
        send(
            session,
            InputEvent::Key {
                device: DeviceId::VIRTUAL,
                keycode,
                pressed,
            },
        );
    }
}

/// Whether some canvas of the tree paints `text`.
fn shows(ipc: &Path, text: &str) -> bool {
    canvases(ipc).iter().any(|canvas| {
        canvas.list.as_ref().is_some_and(|list| {
            list.ops
                .iter()
                .any(|op| matches!(op, gnoblin_paint::PaintOp::Text { text: shown, .. } if shown == text))
        })
    })
}

fn write_ui_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        "import { defineConfig } from \"gnoblin\";\ndefineConfig({ decorations: true });\n",
    )
    .expect("init.ts");
}

/// A mapped window with decorations on has its content below the bar and the bar above it, and does not overlap it. A right
/// click on the bar, through the session's input path, opens the window menu, which the compositor holds as painted
/// canvases, and Escape closes it.
#[test]
fn a_decorated_windows_content_lies_below_its_bar_and_a_titlebar_right_click_opens_the_window_menu() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    write_ui_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();
    wait_for("the shell is running", &session, &mut mapper, &shell, || {
        shell.generation() >= 1
    });
    // The window arrives while the shell runs, so the floating layout places it as a frame: the bar, then the content.
    mapper.map_window("org.example.framed", "Framed");
    wait_for("the window placed below its bar", &session, &mut mapper, &shell, || {
        rect_of(&ipc, "org.example.framed").is_some_and(|(_, y, width, _)| y > 0.0 && width > 0.0)
            && shows(&ipc, "Framed")
    });
    let (x, y, width, height) = rect_of(&ipc, "org.example.framed").expect("the window");
    let window = scene(&ipc)
        .into_iter()
        .find(|window| window.app_id == "org.example.framed")
        .expect("the mapped window");
    let bars: Vec<Canvas> = canvases(&ipc)
        .into_iter()
        .filter(|canvas| canvas.parent == window.id)
        .collect();
    assert_eq!(bars.len(), 1, "one titlebar under the window: {bars:?}");
    let bar = &bars[0];
    assert_eq!(bar.x, 0.0, "the bar starts at the content's left edge");
    assert!(
        bar.height > 0.0 && width > 0.0 && height > 0.0,
        "{bar:?} {width}x{height}"
    );
    assert_eq!(
        bar.y, -bar.height,
        "the bar lies above the window's content, in the window's own coordinates"
    );
    assert!(
        y >= bar.height,
        "the bar (top at {}) is on screen, and the content (top at {y}) starts where the bar ends",
        y - bar.height
    );
    assert!(
        bar.y + bar.height <= 0.0,
        "no overlap: the bar's bottom edge is at or above the content's top edge"
    );

    // A right click on the bar opens the window menu. The bar is painted (its title is shown), so the press that lands
    // on it is the shell's, and one click is all there is to send.
    let (at_x, at_y) = ((x + 12.0) as i32, (y - bar.height / 2.0) as i32);
    motion(&session, at_x, at_y);
    button(&session, BTN_RIGHT, true);
    button(&session, BTN_RIGHT, false);
    wait_for("the window menu painted", &session, &mut mapper, &shell, || {
        shows(&ipc, "Maximise") && shows(&ipc, "Close")
    });
    tap_key(&session, KEY_ESC);
    wait_for("Escape closes the menu", &session, &mut mapper, &shell, || {
        if shows(&ipc, "Maximise") {
            tap_key(&session, KEY_ESC);
        }
        !shows(&ipc, "Maximise")
    });
    let log = shell.finish();
    assert!(
        log.iter().all(|line| !line.contains("threw")),
        "the shell logged a throw: {log:#?}"
    );
}

/// The UI config of the alert test: decorations, and a `probe` that says of the one window `waiting/armed`. `waiting` is
/// whether the alert still waits on a window its user chose Wait for, which it does until it has seen the window answer a
/// ping. `armed` is whether the Force quit of the window's open alert is armed, and `undefined` while no alert is open.
fn write_alert_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { defineConfig, gnoblin, notRespondingAlerts } from "gnoblin";

defineConfig({ decorations: true });

(globalThis as { probe?: () => string }).probe = () =>
    gnoblin.clients
        .map((client) => `${notRespondingAlerts.tracker.isWaiting(client)}/${notRespondingAlerts.cardOf(client)?.armed}`)
        .join(",");
"#,
    )
    .expect("init.ts");
}

/// A client that leaves the compositor's liveness ping unanswered gets the not-responding alert, which names its window, and
/// Escape (Wait) closes it. The Force quit button, armed after half a second, is answered by `Window:kill`, which the
/// compositor refuses for its own process (this test's), so the alert closes and the test survives.
#[test]
fn a_window_that_stops_answering_gets_the_alert_and_wait_and_force_quit_close_it() {
    let session = Session::start_with(|config| {
        config.wayland.check_alive_timeout = Duration::from_millis(800);
    });
    let mut mapper = Mapper::connect(&session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    write_alert_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();
    wait_for("the shell is running", &session, &mut mapper, &shell, || {
        shell.generation() >= 1
    });
    mapper.map_window("org.example.stuck", "Stuck");
    wait_for("the window is up", &session, &mut mapper, &shell, || {
        rect_of(&ipc, "org.example.stuck").is_some() && shows(&ipc, "Stuck")
    });

    // The client reads none of its events from here on, so it answers no ping.
    let wait_until = |what: &str, mut check: Box<dyn FnMut() -> bool>| {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(
                Instant::now() < deadline,
                "never reached: {what}; the shell logged {:?} and its probe says {:?}",
                shell.diagnostics(),
                shell.probe()
            );
            std::thread::sleep(POLL);
        }
    };
    let alert = "Stuck is not responding";
    wait_until("the alert opened", Box::new(|| shows(&ipc, alert)));
    // Escape is Wait. The alert takes the keyboard when it opens, and a key that comes before it has is the window's,
    // so Escape is sent again for as long as the alert is there.
    wait_until(
        "Escape (Wait) closed the alert",
        Box::new(|| {
            if shows(&ipc, alert) {
                tap_key(&session, KEY_ESC);
            }
            !shows(&ipc, alert)
        }),
    );

    // The client answers again, and goes on answering until the shell has seen it do so. A window that said Wait gets no
    // second alert before the shell has seen it answer and then stop, and the shell looks at its windows only a few times
    // a second: a client that stopped again at once would leave it to how soon a loaded machine schedules the shell.
    wait_for(
        "the shell sees the window answer",
        &session,
        &mut mapper,
        &shell,
        || shell.probe() == "false/undefined",
    );
    // Then it stops again: a second alert, and Force quit once the shell has armed it.
    wait_until("the alert came back", Box::new(|| shows(&ipc, alert)));
    wait_until("Force quit is armed", Box::new(|| shell.probe() == "false/true"));
    tap_key(&session, KEY_TAB);
    tap_key(&session, KEY_ENTER);
    wait_until("Force quit closed the alert", Box::new(|| !shows(&ipc, alert)));
    assert!(
        rect_of(&ipc, "org.example.stuck").is_some(),
        "the compositor refused to kill its own process: the window is still there"
    );
    mapper.roundtrip();
    let log = shell.finish();
    assert!(
        log.iter().all(|line| !line.contains("threw")),
        "the shell logged a throw: {log:#?}"
    );
}

// ---- the console ---------------------------------------------------------------------------------------------------

/// A client of the shell's console socket. The test thread keeps the Wayland client answering while it waits, as every
/// wait in this file does.
struct ConsoleClient {
    stream: UnixStream,
    pending: Vec<u8>,
    next_id: u64,
}

impl ConsoleClient {
    /// Connects once the shell is listening.
    fn connect(path: &Path, mapper: &mut Mapper) -> Self {
        let deadline = Instant::now() + PATIENCE;
        let stream = loop {
            mapper.roundtrip();
            match UnixStream::connect(path) {
                Ok(stream) => break stream,
                Err(error) => assert!(
                    Instant::now() < deadline,
                    "the shell never served a console at {}: {error}",
                    path.display()
                ),
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        stream.set_nonblocking(true).expect("a non-blocking socket");
        Self {
            stream,
            pending: Vec::new(),
            next_id: 1,
        }
    }

    /// Evaluates `expr` and returns the reply, whatever it is.
    #[track_caller]
    fn ask(&mut self, expr: &str, mapper: &mut Mapper) -> serde_json::Value {
        use std::io::{Read as _, Write as _};

        let id = self.next_id;
        self.next_id += 1;
        let request = serde_json::json!({"id": id, "expr": expr});
        let mut line = request.to_string();
        line.push('\n');
        self.stream.write_all(line.as_bytes()).expect("the request is written");
        let deadline = Instant::now() + PATIENCE;
        loop {
            mapper.roundtrip();
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => panic!("the console closed the connection while {expr} ran"),
                Ok(count) => self.pending.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("reading the console's reply: {error}"),
            }
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let reply: Vec<u8> = self.pending.drain(..=end).collect();
                let reply: serde_json::Value = serde_json::from_slice(&reply).expect("a reply is JSON");
                assert_eq!(
                    reply["id"],
                    serde_json::json!(id),
                    "the reply answers its request: {reply}"
                );
                return reply;
            }
            assert!(Instant::now() < deadline, "no reply to {expr}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The value of an expression that is expected to succeed.
    #[track_caller]
    fn value(&mut self, expr: &str, mapper: &mut Mapper) -> serde_json::Value {
        let reply = self.ask(expr, mapper);
        assert_eq!(reply["ok"], serde_json::json!(true), "{expr} failed: {reply}");
        reply["value"].clone()
    }
}

#[test]
fn the_console_reads_and_writes_the_live_shell_and_survives_errors_and_runaways() {
    let session = Session::start();
    // No init.ts: the stock config runs, with its seat claims and its keys.
    let config = tempfile::tempdir().expect("a config directory");
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    let ipc_dir = ipc.parent().expect("the socket's directory").to_path_buf();
    wait_for("the shell runs", &session, &mut mapper, &shell, || {
        shell.view().contains(", 4 workspaces")
    });
    let mut console = ConsoleClient::connect(&ipc_dir.join("js.sock"), &mut mapper);
    assert_eq!(
        fs::metadata(ipc_dir.join("js.sock"))
            .expect("the console socket")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "the socket is private to its owner"
    );

    let (first, second) = ("org.example.first", "org.example.second");
    mapper.map_window(first, "First");
    mapper.map_window(second, "Second");
    wait_for("the shell sees both windows", &session, &mut mapper, &shell, || {
        shell.view().starts_with("2 clients")
    });

    // Reads: the object model of init.ts, live.
    assert_eq!(
        console.value("gnoblin.clients.length", &mut mapper),
        serde_json::json!(2)
    );
    let mut apps: Vec<String> = serde_json::from_value(console.value("gnoblin.clients.map(c => c.appId)", &mut mapper))
        .expect("an array of strings");
    apps.sort();
    assert_eq!(apps, [first, second]);
    let counts = console.value("gnoblin.workspaces.map(ws => [ws.id, ws.clients.length])", &mut mapper);
    assert_eq!(counts, serde_json::json!([["1", 2], ["2", 0], ["3", 0], ["4", 0]]));
    let described = console.ask("gnoblin.clients.at(0)", &mut mapper);
    assert_eq!(described["type"], serde_json::json!("Client"), "{described}");
    assert!(
        described["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("Client#") && text.contains("appId: \"org.example.")),
        "{described}"
    );
    wait_for("the new window has the focus", &session, &mut mapper, &shell, || {
        focused_app(&shell).is_some()
    });
    let focus = console.value("gnoblin.clients.focused?.appId", &mut mapper);
    assert_eq!(
        focus.as_str(),
        focused_app(&shell).as_deref(),
        "`clients.focused` is the seat's focus"
    );

    // The inspector helpers see the stock bindings the shell claimed on the real seat.
    let bindings = console.value("inspect.bindings()", &mut mapper);
    let mute = bindings
        .as_array()
        .expect("an array")
        .iter()
        .find(|binding| binding["trigger"] == "XF86AudioMute")
        .unwrap_or_else(|| panic!("the stock mute key is bound: {bindings}"));
    assert_eq!(
        (&mute["owner"], &mute["rank"]),
        (&serde_json::json!("context"), &serde_json::json!(1))
    );
    let config_keys = console.value("Object.keys(inspect.config().keys).length", &mut mapper);
    assert!(config_keys.as_u64().is_some_and(|count| count > 5), "{config_keys}");

    // A syntax error is an error with a location, and the shell is as it was.
    let generation = shell.generation();
    let syntax = console.ask("gnoblin.clients.map(", &mut mapper);
    assert_eq!(syntax["ok"], serde_json::json!(false), "{syntax}");
    assert_eq!(
        syntax["error"]["location"]["file"],
        serde_json::json!("<console>"),
        "{syntax}"
    );
    assert_eq!(syntax["error"]["location"]["line"], serde_json::json!(1), "{syntax}");
    assert_eq!(
        console.value("gnoblin.clients.length", &mut mapper),
        serde_json::json!(2)
    );

    // A runaway is interrupted by the budget; the console and the shell still answer, and the generation is not
    // restarted over it.
    let runaway = console.ask("while (true) {}", &mut mapper);
    assert_eq!(runaway["error"]["kind"], serde_json::json!("budget"), "{runaway}");
    assert_eq!(console.value("1 + 1", &mut mapper), serde_json::json!(2));
    mapper.map_window("org.example.third", "Third");
    wait_for(
        "the shell sees a window mapped after the runaway",
        &session,
        &mut mapper,
        &shell,
        || shell.view().starts_with("3 clients"),
    );
    assert_eq!(
        console.value("gnoblin.clients.length", &mut mapper),
        serde_json::json!(3)
    );
    assert_eq!(shell.generation(), generation, "the generation was not restarted");

    // A write at the prompt reaches the compositor's scene.
    let app = console.value("gnoblin.clients.at(0).appId", &mut mapper);
    let app = app.as_str().expect("an app id").to_owned();
    assert_eq!(
        console.value("gnoblin.clients.at(0).x = 10", &mut mapper),
        serde_json::json!(10)
    );
    wait_for(
        "the compositor holds the window where the prompt put it",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, &app).map(|rect| rect.0) == Some(10.0),
    );
    assert_eq!(
        console.value("gnoblin.clients.at(0).x", &mut mapper),
        serde_json::json!(10)
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

#[test]
fn the_console_is_off_when_the_variable_says_so_and_its_socket_goes_with_the_shell() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    let ipc_dir = session.ipc().parent().expect("the socket's directory").to_path_buf();
    let socket = session.ipc().to_str().expect("a UTF-8 socket path").to_owned();

    let off = {
        let socket = socket.clone();
        move |name: &str| match name {
            "GNOBLIN_IPC_SOCKET" => Some(socket.clone()),
            "GNOBLIN_JS_CONSOLE" => Some("0".to_owned()),
            _ => None,
        }
    };
    let mut disabled =
        JsRole::connect(&off, JsRoleConfig::new(config.path(), None)).expect("the shell connects without a console");
    disabled.tick().expect("a frame");
    assert!(
        !ipc_dir.join("js.sock").exists(),
        "GNOBLIN_JS_CONSOLE=0 serves no console"
    );
    drop(disabled);

    let on = move |name: &str| (name == "GNOBLIN_IPC_SOCKET").then(|| socket.clone());
    let mut served = JsRole::connect(&on, JsRoleConfig::new(config.path(), None)).expect("the shell connects");
    served.tick().expect("a frame");
    assert!(ipc_dir.join("js.sock").exists(), "the console is on by default");
    drop(served);
    assert!(
        !ipc_dir.join("js.sock").exists(),
        "the socket goes with the shell that served it"
    );
}

// ---- the starter `gnoblinctl config init` writes -------------------------------------------------------------------

/// The starter with every example switched on. A comment of the form `// code` whose code ends the way a line of the
/// config does (`,` `{` `}` `;` `[` `]` `)`) is uncommented; prose ends in a word or a full stop and stays.
fn switched_on(starter: &str) -> String {
    let mut text = starter
        .lines()
        .map(|line| {
            let body = line.trim_start();
            match body.strip_prefix("// ") {
                Some(code) if code.ends_with([',', '{', '}', ';', '[', ']', ')']) => {
                    format!("{}{code}", &line[..line.len() - body.len()])
                }
                _ => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

/// Lets the Wayland client answer the compositor for `span`, so a negative check does not starve it.
fn idle(mapper: &mut Mapper, span: Duration) {
    let until = Instant::now() + span;
    while Instant::now() < until {
        mapper.roundtrip();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_starter_is_the_stock_config_as_written_and_its_examples_work_switched_on() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");
    let config = tempfile::tempdir().expect("a config directory");
    let init = config.path().join("init.ts");
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();

    // No `init.ts`: the zero-config behaviour, which is what the starter must reproduce.
    wait_for("the stock config runs", &session, &mut mapper, &shell, || {
        section(&shell.view(), "workspaces [") == STOCK_WORKSPACES
    });
    wait_for(
        "the stock config titles the window",
        &session,
        &mut mapper,
        &shell,
        || !canvases(&ipc).is_empty(),
    );
    let zero = shell.view();
    let zero_spaces = section(&zero, "workspaces [").to_owned();
    let zero_keys: Vec<String> = bound_keys(&zero).into_iter().map(str::to_owned).collect();
    let titlebars = canvases(&ipc).len();
    let zero_generation = shell.generation();
    let window = ids(&scene(&ipc));

    // The starter as written is a real config (a `defineConfig`, so a reload), and it binds and declares what the
    // stock one did.
    save(&init, gnoblin_js::STARTER_CONFIG);
    wait_for("the starter is loaded", &session, &mut mapper, &shell, || {
        shell.generation() > zero_generation
    });
    let written = shell.view();
    assert_eq!(
        section(&written, "workspaces ["),
        zero_spaces,
        "the workspaces of zero-config"
    );
    assert_eq!(bound_keys(&written), zero_keys, "the bindings of zero-config");
    wait_for("the titlebar is back", &session, &mut mapper, &shell, || {
        canvases(&ipc).len() == titlebars
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    assert_eq!(ids(&scene(&ipc)), window, "the window kept its id through the reload");

    // Switched on: the examples are the second workspace, three keys, a rule, an autostart command and `setup`. The
    // command appends a line to a file, and `notify-send` becomes `true` so the `setup` example starts nothing odd.
    let marker = config.path().join("autostarted");
    let on = switched_on(gnoblin_js::STARTER_CONFIG)
        .replace(
            "\"waybar\",",
            &format!("[\"/bin/sh\", \"-c\", \"echo started >> {}\"],", marker.display()),
        )
        .replace("\"notify-send\"", "\"true\"");
    assert_ne!(on, gnoblin_js::STARTER_CONFIG, "the examples were switched on");
    save(&init, &on);
    wait_for("the examples are on", &session, &mut mapper, &shell, || {
        section(&shell.view(), "workspaces [") == format!("{STOCK_WORKSPACES},work:MasterStackLayout")
    });
    let view = shell.view();
    let keys = bound_keys(&view);
    for key in ["Super+Return", "Super+b", "Super+q"] {
        assert!(keys.contains(&key), "the example key {key}: {keys:?}");
    }
    for key in &zero_keys {
        assert!(
            keys.contains(&key.as_str()),
            "the stock key {key} is still bound: {keys:?}"
        );
    }
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());

    // The autostart command ran once; a reload does not run it again.
    wait_for("the autostart command ran", &session, &mut mapper, &shell, || {
        fs::read_to_string(&marker).is_ok_and(|text| text.lines().count() == 1)
    });
    let generation = shell.generation();
    save(&init, &format!("{on}// touched\n"));
    wait_for("the reload", &session, &mut mapper, &shell, || {
        shell.generation() > generation
    });
    idle(&mut mapper, Duration::from_millis(500));
    assert_eq!(
        fs::read_to_string(&marker).expect("the marker").lines().count(),
        1,
        "a reload does not start an autostart command again"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
}

#[test]
fn protocols_and_touchpad_gestures_reach_the_compositors_document_and_a_reload_takes_them_back() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let dir = tempfile::tempdir().expect("a config directory");
    let init = dir.path().join("init.ts");
    let write = |settings: &str| {
        save(
            &init,
            &format!("import {{ defineConfig }} from \"gnoblin\";\ndefineConfig({{ {settings} }});\n"),
        );
    };
    let gesture = |fingers: u8| {
        format!(
            "touchpadGestures: [{{ name: 'launcher', gesture: 'pinch', fingers: {fingers}, direction: 'out', \
             command: ['true'] }}]"
        )
    };
    let held = |pointer: &str, ipc: &Path| document(ipc).and_then(|document| document.pointer(pointer).cloned());

    // Added: a camelCase protocol is the document's kebab-case one, and the gesture list is one value.
    write(&format!("protocols: {{ wlrScreencopy: false }}, {}", gesture(4)));
    let shell = Shell::start(&session, dir.path(), |_| {});
    // The generation the first config runs under, which `Shell::start` waited for the first frame of.
    let first = shell.generation();
    let ipc = session.ipc();
    wait_for("the document carries both", &session, &mut mapper, &shell, || {
        held("/protocols/wlr-screencopy", &ipc) == Some(serde_json::json!(false))
            && held("/touchpad-gestures/0/fingers", &ipc) == Some(serde_json::json!(4))
    });
    assert_eq!(
        held("/touchpad-gestures/0/name", &ipc),
        Some(serde_json::json!("launcher"))
    );

    // Changed.
    write(&format!("protocols: {{ wlrScreencopy: true }}, {}", gesture(3)));
    wait_for("the document carries the change", &session, &mut mapper, &shell, || {
        held("/protocols/wlr-screencopy", &ipc) == Some(serde_json::json!(true))
            && held("/touchpad-gestures/0/fingers", &ipc) == Some(serde_json::json!(3))
    });

    // Refused: nine fingers is not a gesture. The reload names the key path and the document is untouched. The
    // generation that took the change in is waited for: the document changes while the frame that swapped it in is
    // still running, and the generation is what the shell publishes when that frame is over.
    wait_for("the change was swapped in", &session, &mut mapper, &shell, || {
        shell.generation() > first
    });
    let generation = shell.generation();
    write(&gesture(9));
    wait_for("the refusal is reported", &session, &mut mapper, &shell, || {
        shell
            .diagnostics()
            .iter()
            .any(|text| text.contains("touchpad-gestures") && text.contains("fingers"))
    });
    assert_eq!(held("/touchpad-gestures/0/fingers", &ipc), Some(serde_json::json!(3)));
    assert_eq!(shell.generation(), generation, "the running generation stays");

    // Removed: both are gone from the document, which is their default.
    write("");
    wait_for("both are back at their defaults", &session, &mut mapper, &shell, || {
        held("/protocols", &ipc).is_none() && held("/touchpad-gestures", &ipc).is_none()
    });
}

/// One effect as the style tests compare it: `(kind, offset_y, blur_radius, colour)`.
type EffectRow = (String, f64, f64, String);

/// What the compositor holds on the window node of `app` for its style: the effects and the opacity (1 when the node
/// holds none, which is the schema's default).
fn style_of(ipc: &Path, app: &str) -> Option<(Vec<EffectRow>, f64)> {
    let node = support::snapshot(ipc)
        .into_iter()
        .find(|node| node.class == "window" && node.app_id == app)?;
    let effects = node
        .props
        .iter()
        .find(|prop| prop.key == "effects")
        .map(|prop| {
            prop.value_effects
                .iter()
                .map(|e| (e.kind.clone(), e.offset_y, e.blur_radius, e.colour.clone()))
                .collect()
        })
        .unwrap_or_default();
    let opacity = node
        .props
        .iter()
        .find(|prop| prop.key == "opacity")
        .map_or(1.0, |prop| prop.value_double);
    Some((effects, opacity))
}

#[test]
fn a_global_style_becomes_the_effect_list_and_opacity_of_a_mapped_windows_node_and_a_reload_clears_them() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(
        config.path().join("init.ts"),
        r##"import { Layout, defineConfig } from "gnoblin";

defineConfig({
    workspaces: [{ id: "desk", layout: new Layout() }],
    style: { opacity: 0.8, shadows: [{ x: 0, y: 8, blur: 24, color: "#00000066" }] },
    rules: [{ appId: "org.example.dim", style: { opacity: 0.5 } }],
    setup: (gnoblin) => {
        gnoblin.clients.where({ appId: "org.example.glow" }).style.shadows = [{ blur: 30, color: "#38bdf855" }];
    },
});
"##,
    )
    .expect("init.ts");
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 1 workspaces")
    });

    let (plain, dim, glow) = ("org.example.plain", "org.example.dim", "org.example.glow");
    for app in [plain, dim, glow] {
        mapper.map_window(app, app);
    }
    let shadow = |blur: f64, colour: &str| vec![("shadow".to_owned(), 0.0, blur, colour.to_owned())];
    let global = (vec![("shadow".to_owned(), 8.0, 24.0, "#00000066".to_owned())], 0.8);
    wait_for("the style reaches the nodes", &session, &mut mapper, &shell, || {
        style_of(&ipc, plain).as_ref() == Some(&global)
            && style_of(&ipc, dim) == Some((global.0.clone(), 0.5))
            && style_of(&ipc, glow) == Some((shadow(30.0, "#38bdf855"), 0.8))
    });

    // Removing the lines is a reload, and the new generation takes the effects and the opacity off the nodes the old
    // one styled.
    let generation = shell.generation();
    save(
        &config.path().join("init.ts"),
        r#"import { Layout, defineConfig } from "gnoblin";

defineConfig({ workspaces: [{ id: "desk", layout: new Layout() }] });
"#,
    );
    wait_for("the reload clears the style", &session, &mut mapper, &shell, || {
        shell.generation() > generation
            && [plain, dim, glow]
                .iter()
                .all(|app| style_of(&ipc, app) == Some((Vec::new(), 1.0)))
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

// ---- XR: the program's seat, and the room the stock shell builds for it ------------------------------------------

/// What the XR program does, as an observer of the owner socket: it makes a seat named `xr`, and a `controller` for a
/// hand with the values the program writes. The compositor destroys the seat and the controllers when this connection
/// closes, as it does when the real program's session ends. Nothing here is the program: it draws no layer and runs no
/// OpenXR; the seat and the hands are what the shell sees of it (docs/xr.md).
struct XrSeat {
    /// Held open: dropping it closes the connection and the seat goes.
    stream: UnixStream,
    /// The next id this connection makes, after the seat's.
    next: u64,
    /// The serial the compositor last said, which a transaction is built against.
    serial: u64,
    /// The next request id.
    request: u64,
}

/// A `set` of `key` on `node`, as the program writes it.
fn xr_set(node: u64, key: &str, value_kind: &str) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: value_kind.to_owned(),
        ..Default::default()
    }
}

fn xr_set_double(node: u64, key: &str, value: f64) -> op::Properties {
    op::Properties {
        value_double: value,
        ..xr_set(node, key, "double")
    }
}

impl XrSeat {
    fn attach(ipc: &Path) -> Self {
        let transport = UnixTransport::connect(ipc).expect("the session is listening");
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        let mut stream = Client::connect(transport, Version::new(major, minor, patch))
            .expect("the handshake")
            .into_inner()
            .into_stream()
            .expect("the handshaken socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("a read deadline");
        let attach = AttachArgs {
            version: 1,
            mode: "observer".to_owned(),
        };
        write_frame(&mut stream, &Envelope::request_message("tree", "attach", 1, &attach)).expect("the attach");
        let attached: AttachedPayload = read_frame(&mut stream)
            .expect("the attach reply frame")
            .expect("an attach reply")
            .payload_as()
            .expect("the attach payload decodes");
        loop {
            let chunk: SnapshotPayload = read_frame(&mut stream)
                .expect("a snapshot frame")
                .expect("a snapshot chunk")
                .payload_as()
                .expect("the chunk decodes");
            if chunk.last {
                break;
            }
        }
        let epoch = u64::from(attached.epoch as u32) << 32;
        let mut program = Self {
            stream,
            next: epoch | 2,
            serial: attached.serial,
            request: 1,
        };
        let seat = epoch | 1;
        let reply = program.transact(vec![
            op::Properties {
                kind: "new".to_owned(),
                class: "seat".to_owned(),
                id: seat,
                ..Default::default()
            },
            op::Properties {
                value_text: "xr".to_owned(),
                ..xr_set(seat, "name", "text")
            },
        ]);
        assert!(
            reply.applied,
            "the compositor lets an observer make its seat: {reply:?}"
        );
        program
    }

    /// Sends one transaction and returns the compositor's answer. The changes the compositor publishes to this
    /// observer in the meantime are read and dropped. A write built on a tree the shell has since moved is asked again
    /// on the serial the compositor named, as the real program does.
    fn transact(&mut self, ops: Vec<op::Properties>) -> ReplyPayload {
        for _ in 0..5 {
            self.request += 1;
            let transact = TransactArgs {
                base_serial: self.serial,
                serial: self.request,
                ops: ops.clone(),
            };
            write_frame(
                &mut self.stream,
                &Envelope::request_message("tree", "transact", self.request, &transact),
            )
            .expect("the transaction");
            let reply = loop {
                let envelope = read_frame(&mut self.stream)
                    .expect("a frame")
                    .expect("the compositor answers the transaction");
                if matches!(envelope.kind, Some(Kind::Reply(_))) {
                    break envelope.payload_as::<ReplyPayload>().expect("the reply decodes");
                }
            };
            self.serial = reply.serial;
            if reply.applied || reply.code != "stale-base" {
                return reply;
            }
        }
        panic!("the compositor kept calling the program's base stale");
    }

    /// Makes a tracked `controller` for `hand` at the origin, facing down negative z, as the program does for a hand
    /// the runtime reports, and returns its id.
    fn controller(&mut self, hand: &str) -> u64 {
        let id = self.next;
        self.next += 1;
        let reply = self.transact(vec![
            op::Properties {
                kind: "new".to_owned(),
                class: "controller".to_owned(),
                id,
                ..Default::default()
            },
            op::Properties {
                value_text: hand.to_owned(),
                ..xr_set(id, "hand", "text")
            },
            op::Properties {
                value_bool: true,
                ..xr_set(id, "tracked", "bool")
            },
            op::Properties {
                value_list: vec![0.0, 0.0, 0.0],
                ..xr_set(id, "rotation", "list_double")
            },
        ]);
        assert!(
            reply.applied,
            "the compositor lets an observer make a controller: {reply:?}"
        );
        id
    }

    /// Writes a controller's grip as the program does when the runtime reports a squeeze.
    fn squeeze(&mut self, controller: u64, grip: f64) {
        let reply = self.transact(vec![xr_set_double(controller, "grip", grip)]);
        assert!(reply.applied, "the program writes its controller's grip: {reply:?}");
    }
}

/// The snapshot nodes of `class`, as the compositor holds them.
fn xr_nodes(ipc: &Path, class: &str) -> Vec<gnoblin_proto::tree::snapshot_node::Properties> {
    snapshot(ipc).into_iter().filter(|node| node.class == class).collect()
}

fn prop_text(node: &gnoblin_proto::tree::snapshot_node::Properties, key: &str) -> String {
    node.props
        .iter()
        .find(|prop| prop.key == key)
        .map(|prop| prop.value_text.clone())
        .unwrap_or_default()
}

fn prop_number(node: &gnoblin_proto::tree::snapshot_node::Properties, key: &str) -> f64 {
    node.props
        .iter()
        .find(|prop| prop.key == key)
        .map_or(0.0, |prop| prop.value_double)
}

/// The room, if the tree has one: a node named `room`.
fn room_of(ipc: &Path) -> Option<gnoblin_proto::tree::snapshot_node::Properties> {
    xr_nodes(ipc, "node")
        .into_iter()
        .find(|node| prop_text(node, "name") == "room")
}

#[test]
fn the_xr_programs_seat_makes_the_stock_shell_build_a_room_with_a_view_per_window_and_its_going_takes_it_down() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(
        config.path().join("init.ts"),
        "import { defineConfig } from \"gnoblin\";\ndefineConfig({});\n",
    )
    .expect("init.ts");
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    // `defineConfig({})` declares no workspace, so the stock four are the ones the shell has.
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 4 workspaces")
    });
    let (one, two) = ("org.example.one", "org.example.two");
    mapper.map_window(one, "One");
    mapper.map_window(two, "Two");
    wait_for("the shell sees both windows", &session, &mut mapper, &shell, || {
        shell.view().contains(one) && shell.view().contains(two)
    });
    std::thread::sleep(Duration::from_millis(200));
    mapper.roundtrip();
    assert!(
        room_of(&ipc).is_none(),
        "no program, no room: nothing of XR is in the tree"
    );
    assert!(xr_nodes(&ipc, "view").is_empty() && xr_nodes(&ipc, "camera").is_empty());

    let program = XrSeat::attach(&ipc);
    wait_for(
        "the room has a View for each window",
        &session,
        &mut mapper,
        &shell,
        || xr_nodes(&ipc, "view").len() == 2,
    );
    let windows = ids(&scene(&ipc));
    let room = room_of(&ipc).expect("the room is in the compositor's tree");
    let root = snapshot(&ipc)
        .into_iter()
        .find(|node| node.parent == 0 && node.class == "node")
        .expect("the root")
        .id;
    assert_eq!(room.parent, root, "the room hangs from the root");
    assert!(
        (prop_number(&room, "y") - -1400.0).abs() < 1e-6,
        "the room's floor is an eye height below the viewer"
    );
    let heads = xr_nodes(&ipc, "camera");
    assert_eq!(heads.len(), 1, "one head camera: {heads:?}");
    assert_eq!(prop_text(&heads[0], "name"), "head");
    assert_eq!(prop_text(&heads[0], "projection"), "stereo");
    assert_eq!(heads[0].parent, room.id, "under the room");
    let mut shown = Vec::new();
    for view in xr_nodes(&ipc, "view") {
        assert_eq!(view.parent, room.id);
        let source = view
            .props
            .iter()
            .find(|prop| prop.key == "source")
            .map_or(0, |prop| prop.value_node);
        shown.push(source);
        assert!(prop_number(&view, "width") > 0.0, "the View has its window's size");
        assert!(
            (prop_number(&view, "y") - 1400.0).abs() < 1e-6,
            "the panels sit at eye height: {view:?}"
        );
    }
    shown.sort_unstable();
    let mut want = vec![windows[one], windows[two]];
    want.sort_unstable();
    assert_eq!(shown, want, "each View shows one of the windows");
    for id in want {
        let window = snapshot(&ipc)
            .into_iter()
            .find(|node| node.id == id)
            .expect("the window");
        assert_ne!(window.parent, room.id, "the window is not reparented under the room");
    }
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());

    drop(program);
    wait_for(
        "the room goes with the program's seat",
        &session,
        &mut mapper,
        &shell,
        || room_of(&ipc).is_none(),
    );
    assert!(xr_nodes(&ipc, "view").is_empty() && xr_nodes(&ipc, "camera").is_empty());
    assert_eq!(
        scene(&ipc).len(),
        2,
        "both windows are still in the tree: {:?}",
        scene(&ipc)
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

/// What the compositor holds as the claimed inputs of `controller`: the list the shell wrote, empty when it holds none.
fn claims_of(ipc: &Path, controller: u64) -> Vec<String> {
    snapshot(ipc)
        .into_iter()
        .find(|node| node.id == controller)
        .and_then(|node| {
            node.props
                .iter()
                .find(|prop| prop.key == "claimed")
                .map(|prop| prop.value_texts.clone())
        })
        .unwrap_or_default()
}

#[test]
fn the_stock_grab_claims_the_grip_on_the_real_controller_and_the_claim_goes_with_the_drag_and_with_the_shell() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(
        config.path().join("init.ts"),
        "import { defineConfig } from \"gnoblin\";\ndefineConfig({});\n",
    )
    .expect("init.ts");
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 4 workspaces")
    });
    let app = "org.example.one";
    mapper.map_window(app, "One");
    wait_for("the shell sees the window", &session, &mut mapper, &shell, || {
        shell.view().contains(app)
    });

    // The program: a seat named `xr` and a hand, made and written as an observer, as the real one does. The stock shell
    // builds the room, and the panel is straight ahead of a hand at the origin.
    let mut program = XrSeat::attach(&ipc);
    wait_for("the room has the window's View", &session, &mut mapper, &shell, || {
        xr_nodes(&ipc, "view").len() == 1
    });
    let hand = program.controller("right");
    assert!(claims_of(&ipc, hand).is_empty(), "nothing is claimed before a drag");

    // A squeeze starts the stock grab, which takes the grip from the window under the ray.
    program.squeeze(hand, 1.0);
    wait_for("the grab claims the grip", &session, &mut mapper, &shell, || {
        claims_of(&ipc, hand) == ["grip"]
    });
    // The compositor refuses the program the claims, which are the shell's to write.
    let refused = program.transact(vec![op::Properties {
        value_texts: Vec::new(),
        ..xr_set(hand, "claimed", "list_text")
    }]);
    assert!(!refused.applied, "an observer may not write `claimed`: {refused:?}");
    assert_eq!(refused.code, "not-holder");
    assert_eq!(claims_of(&ipc, hand), ["grip"], "the refused write changed nothing");

    // Letting go ends the drag and gives the grip back.
    program.squeeze(hand, 0.0);
    wait_for("the claim goes with the drag", &session, &mut mapper, &shell, || {
        claims_of(&ipc, hand).is_empty()
    });

    // A new squeeze claims again, and then the shell dies with it held: the compositor gives the claim back, because
    // the program would otherwise withhold the grip from its windows for ever.
    program.squeeze(hand, 1.0);
    wait_for("the second grab claims the grip", &session, &mut mapper, &shell, || {
        claims_of(&ipc, hand) == ["grip"]
    });
    let diagnostics = shell.diagnostics();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    shell.finish();
    assert!(shell.ended().is_none(), "the shell was stopped, not failed");
    let deadline = Instant::now() + PATIENCE;
    while !claims_of(&ipc, hand).is_empty() {
        assert!(
            Instant::now() < deadline,
            "never reached: the compositor gives a dead shell's claim back; it holds {:?}",
            claims_of(&ipc, hand)
        );
        mapper.roundtrip();
        std::thread::sleep(POLL);
    }
    assert_eq!(
        xr_nodes(&ipc, "controller").len(),
        1,
        "the hand is the program's and stays"
    );
}

// ---- outputs -----------------------------------------------------------------------------------------------------

/// An `init.ts` whose two workspaces fill the output that shows them, and whose rule sends the windows of
/// `org.example.second` to the second workspace.
fn write_outputs_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, defineConfig } from "gnoblin";

class Fill extends Layout {
    compute(context: any) {
        return context.children.map((node: any) => ({ node, tiled: true, bounds: { ...context.area } }));
    }
}

defineConfig({
    workspaces: [{ id: "1", layout: new Fill() }, { id: "2", layout: new Fill() }],
    decorations: false,
    rules: [{ appId: "org.example.second", workspace: "2" }],
});
"#,
    )
    .expect("init.ts");
}

/// The screens of the compositor's tree by place: each one's output number (from its `output-N` name) and rectangle.
fn screens_by_place(ipc: &Path) -> Vec<(u64, (f64, f64, f64, f64))> {
    let mut found: Vec<(u64, (f64, f64, f64, f64))> = snapshot(ipc)
        .iter()
        .filter(|node| node.class == "screen")
        .map(|node| {
            let value = |key: &str| {
                node.props
                    .iter()
                    .find(|prop| prop.key == key)
                    .map_or(0.0, |prop| prop.value_double)
            };
            let name = node
                .props
                .iter()
                .find(|prop| prop.key == "name")
                .map(|prop| prop.value_text.clone())
                .unwrap_or_default();
            let number = name
                .strip_prefix("output-")
                .and_then(|n| n.parse().ok())
                .expect("a screen is named output-N");
            (number, (value("x"), value("y"), value("width"), value("height")))
        })
        .collect();
    found.sort_by(|a, b| a.1.0.total_cmp(&b.1.0));
    found
}

/// Whether the shell hid the window of `app`: its node holds `visible` false (true is the default, and not carried).
fn hidden(ipc: &Path, app: &str) -> bool {
    snapshot(ipc)
        .iter()
        .filter(|node| node.class == "window" && node.app_id == app)
        .any(|node| node.props.iter().any(|prop| prop.key == "visible" && !prop.value_bool))
}

#[test]
fn a_window_on_a_second_virtual_output_follows_to_the_first_when_it_is_unplugged_and_parked_windows_return_with_the_next_output()
 {
    // The headless backend takes outputs at runtime through the session's `OutputTap`. Two workspaces fill the output
    // that shows them; the rule sends `org.example.second`'s window to the second workspace, which the second output
    // shows. Unplugging the second output moves that window to the workspace the first shows; unplugging the first too
    // parks both windows hidden; the next output adopts them.
    let tap = OutputTap::new();
    let session = Session::start_with({
        let tap = tap.clone();
        move |config| config.output_tap = Some(tap)
    });
    let config = tempfile::tempdir().expect("a config directory");
    write_outputs_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains(", 2 workspaces")
    });
    assert!(tap.connect("HEADLESS-2", 800, 600), "the session takes a second output");
    wait_for("two screens", &session, &mut mapper, &shell, || screens(&ipc) == 2);
    let places = screens_by_place(&ipc);
    let (first_output, first_rect) = places[0];
    let (second_output, second_rect) = places[1];
    assert_eq!(
        second_rect.2, 800.0,
        "the second output is the 800 by 600 one: {places:?}"
    );

    let (first, second) = ("org.example.first", "org.example.second");
    mapper.map_window(first, "First");
    wait_for(
        "the first window fills the first output",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, first) == Some(first_rect),
    );
    mapper.map_window(second, "Second");
    wait_for(
        "the second window fills the second output",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, second) == Some(second_rect),
    );

    // The second output is unplugged: its window follows to the first output.
    assert!(tap.disconnect(OutputId::new(second_output)));
    wait_for("one screen", &session, &mut mapper, &shell, || screens(&ipc) == 1);
    wait_for(
        "the second window is on the first output",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, second) == Some(first_rect) && rect_of(&ipc, first) == Some(first_rect),
    );

    // The last output is unplugged: both windows wait hidden.
    assert!(tap.disconnect(OutputId::new(first_output)));
    wait_for("no screen", &session, &mut mapper, &shell, || screens(&ipc) == 0);
    wait_for("both windows are hidden", &session, &mut mapper, &shell, || {
        hidden(&ipc, first) && hidden(&ipc, second)
    });

    // The next output adopts them.
    assert!(tap.connect("HEADLESS-3", 640, 480));
    wait_for("a screen again", &session, &mut mapper, &shell, || screens(&ipc) == 1);
    let back = screens_by_place(&ipc)[0].1;
    assert_eq!((back.2, back.3), (640.0, 480.0));
    wait_for(
        "the parked windows come back on the new output",
        &session,
        &mut mapper,
        &shell,
        || rect_of(&ipc, first) == Some(back) && rect_of(&ipc, second) == Some(back) && !hidden(&ipc, first),
    );
}

// ---- the host API: session, process, timers, events ---------------------------------------------------------------

/// A config that records what the host API did, for the probe: a timer that ran, a program that ended, the session's
/// status as the compositor published it, and a reload it asked for.
const HOST_API_INIT: &str = r#"import { ProcessListener, gnoblin } from "gnoblin";

const state: Record<string, unknown> = {
    timers: 0,
    exits: [],
    status: gnoblin.session.status(),
    signals: [],
    reload: null,
};
(globalThis as { probe?: () => string }).probe = () => JSON.stringify(state);

gnoblin.timers.after(20, () => {
    state.timers = (state.timers as number) + 1;
});

class Exits extends ProcessListener {
    onExit(process: { argv: readonly string[]; exitCode: number | null }, code: number | null): void {
        (state.exits as unknown[]).push([process.argv.join(" "), code]);
    }
}
gnoblin.process.addListener(new Exits());
gnoblin.process.spawn(["sh", "-c", "exit 7"]);
gnoblin.process.spawn(["sh", "-c", "printf hello"], { stdout: "capture" }).wait().onComplete((exit) => {
    state.captured = exit?.stdout;
});

// The compositor raises `reload_requested` on its session object when a privileged caller asks for a reload, and
// the script's own request is one: the signal comes back to the script that asked.
gnoblin.on("gnoblin.session.reload-requested", (event) => {
    (state.signals as unknown[]).push(event.generation);
});
gnoblin.timers.after(50, () => {
    gnoblin.runtime.reloadConfig().onComplete((value, error) => {
        state.reload = error === undefined ? ["ok", value] : [error.code, error.message];
    });
});
"#;

#[test]
fn a_script_reads_the_sessions_status_runs_a_timer_and_a_process_and_is_told_of_a_reload_by_the_real_compositor() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    save(&config.path().join("init.ts"), HOST_API_INIT);
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();
    let probe = |shell: &Shell| -> serde_json::Value {
        serde_json::from_str(&shell.probe()).unwrap_or(serde_json::Value::Null)
    };

    // The session status is the compositor's: a shell with no session connection says `unavailable`.
    wait_for("the script's probe answers", &session, &mut mapper, &shell, || {
        probe(&shell)["status"]["state"] == "running"
    });
    let status = probe(&shell)["status"].clone();
    assert_eq!(status["lockAvailable"], true, "{status}");
    assert_eq!(
        status["lockState"], "unlocked",
        "the compositor's own published lock state: {status}"
    );

    // A timer ran once and each process ended, with its exit code and its captured output, on the shell's frame loop.
    wait_for(
        "the timer ran and both programs ended",
        &session,
        &mut mapper,
        &shell,
        || {
            let state = probe(&shell);
            state["timers"] == 1 && state["exits"].as_array().is_some_and(|exits| exits.len() == 2)
        },
    );
    let state = probe(&shell);
    let mut exits: Vec<String> = state["exits"]
        .as_array()
        .expect("the exits")
        .iter()
        .map(|exit| exit.to_string())
        .collect();
    exits.sort();
    assert_eq!(exits, [r#"["sh -c exit 7",7]"#, r#"["sh -c printf hello",0]"#]);
    wait_for("the captured output arrived", &session, &mut mapper, &shell, || {
        probe(&shell)["captured"] == "hello"
    });

    // The script asked for a reload. The compositor raised `reload_requested` and waits for a runtime to answer; this
    // test is the runtime, and refuses, so the operation fails with the typed error and the compositor's reason, and
    // the running generation, which asked, is still there to hear it.
    wait_for(
        "the compositor raised reload_requested",
        &session,
        &mut mapper,
        &shell,
        || {
            probe(&shell)["signals"]
                .as_array()
                .is_some_and(|signals| !signals.is_empty())
        },
    );
    let generation = shell.generation();
    owner_client(&ipc)
        .reject_config("init.ts:3:1: the test refuses this document")
        .expect("the rejection is accepted");
    wait_for("the reload failed", &session, &mut mapper, &shell, || {
        probe(&shell)["reload"].is_array()
    });
    let reload = probe(&shell)["reload"].clone();
    assert_eq!(reload[0], "config_rejected", "{reload}");
    assert!(
        reload[1]
            .as_str()
            .is_some_and(|message| message.contains("the test refuses this document")),
        "{reload}"
    );
    assert_eq!(
        shell.generation(),
        generation,
        "a refused reload leaves the generation that asked"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

// ---- idle watches, the authentication slot and launch feedback ----------------------------------------------------

/// What the compositor's `idle` object publishes: how long the session has been idle in milliseconds, and how many
/// holds keep it awake.
fn idle_object(ipc: &Path) -> (u64, u64) {
    let mut client = owner_client(ipc);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    let view = snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        .expect("the compositor publishes an idle object");
    let uint = |name: &str| match view.property(name) {
        Some(Value::Uint(value)) => *value,
        other => panic!("the idle object's {name} is {other:?}"),
    };
    (uint("idle_ms"), uint("inhibitors"))
}

/// An `init.ts` with an idle watch of 400 ms that records what it heard, and whatever else `extra` adds.
fn write_idle_config(dir: &Path, extra: &str) {
    save(
        &dir.join("init.ts"),
        &format!(
            r#"import {{ gnoblin }} from "gnoblin";

const seen: string[] = [];
{extra}
gnoblin.idle.watch(400, {{
    onIdle: () => seen.push("idle"),
    onResume: () => seen.push("resume"),
}});
(globalThis as {{ probe?: () => string }}).probe = () => seen.join(",");
"#
        ),
    );
}

/// A script's idle watch runs `onIdle` when the compositor's own idle clock passes its time and `onResume` on the first
/// input after, and counts again: the whole path from the `idle` object over the shell's owner connection.
#[test]
fn an_idle_watch_hears_the_compositors_idle_clock_pass_its_time_and_the_input_that_ends_it() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    write_idle_config(config.path(), "");
    let shell = Shell::start(&session, config.path(), |_| {});
    wait_for(
        "the watch heard the session go idle",
        &session,
        &mut mapper,
        &shell,
        || shell.probe() == "idle",
    );
    let mut step = 0;
    wait_for("the watch heard input", &session, &mut mapper, &shell, || {
        // Input keeps the session active until the shell has seen it, and stops once it has.
        step += 1;
        if shell.probe() == "idle" {
            motion(&session, 40 + (step % 2) * 8, 40);
        }
        shell.probe().starts_with("idle,resume")
    });
    wait_for(
        "the watch counted again and went idle again",
        &session,
        &mut mapper,
        &shell,
        || shell.probe() == "idle,resume,idle",
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

/// A hold is the compositor's: the `idle` object counts it in `inhibitors` and the watch is not idle while one stands,
/// however long the session has been. A reload retires the generation that held it, and that gives the hold back.
#[test]
fn an_idle_hold_delays_the_watch_and_a_reload_that_retires_its_generation_gives_it_back() {
    let session = Session::start();
    let ipc = session.ipc();
    let mut mapper = Mapper::connect(&session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    write_idle_config(config.path(), r#"gnoblin.idle.inhibit("js-shell-test");"#);
    let shell = Shell::start(&session, config.path(), |_| {});
    wait_for("the compositor counts the hold", &session, &mut mapper, &shell, || {
        idle_object(&ipc).1 == 1
    });
    // The session has been idle for far longer than the watch's 400 ms, and the watch has heard nothing: nothing else
    // can be waited for, because a hold that works is an event that does not happen.
    wait_for(
        "the session is idle well past the watch's time",
        &session,
        &mut mapper,
        &shell,
        || idle_object(&ipc).0 >= 1_500,
    );
    assert_eq!(shell.probe(), "", "a held session is not idle");

    let before = shell.generation();
    write_idle_config(config.path(), "");
    wait_for(
        "the reload gave the hold back and the new generation's watch went idle",
        &session,
        &mut mapper,
        &shell,
        || shell.generation() > before && idle_object(&ipc).1 == 0 && shell.probe() == "idle",
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

/// The shell's launch registry over real windows: a window of the application starts the launch, the deadline times
/// another out, and the application ID is matched the way the page says.
#[test]
fn a_launch_is_started_by_its_applications_window_and_timed_out_by_its_deadline() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    save(
        &config.path().join("init.ts"),
        r#"import { gnoblin } from "gnoblin";

const launches = [
    gnoblin.launches.begin("opens", "org.example.Opens.desktop", { timeoutMs: 10000 }),
    gnoblin.launches.begin("never", "org.example.Never", { timeoutMs: 300 }),
];
(globalThis as { probe?: () => string }).probe = () => launches.map((launch) => launch.state).join(",");
"#,
    );
    let shell = Shell::start(&session, config.path(), |_| {});
    wait_for("the second launch timed out", &session, &mut mapper, &shell, || {
        shell.probe() == "pending,timed_out"
    });
    mapper.map_window("org.example.opens", "Opened");
    wait_for("the window started its launch", &session, &mut mapper, &shell, || {
        shell.probe() == "started,timed_out"
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}

/// A session whose guardian handed it one privileged connection, for the `services` role: the end that role holds, which
/// is the one that may offer a request.
struct Services {
    // Dropped before the session: the compositor's control thread reads the guardian's end until it closes.
    _guardian: std::os::fd::OwnedFd,
    role: Option<UnixStream>,
    session: Session,
}

impl Services {
    fn start() -> Self {
        let (guardian, compositor_control) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("a control socket pair");
        let (compositor_end, role_end) = UnixStream::pair().expect("a socket pair");
        gnoblin_proto::privilege::send_handover(&guardian, "services", &compositor_end).expect("the handover");
        let mut privileged = gnoblin_compositor::Privileged::declared();
        privileged.retain(|entry| entry.role == "services");
        let session = Session::start_with(move |config| {
            config.privileged = privileged;
            config.guardian = Some(UnixStream::from(compositor_control));
        });
        Self {
            _guardian: guardian,
            role: Some(role_end),
            session,
        }
    }

    /// The services role's own connection: the one an agent offers requests on.
    fn role_client(&mut self) -> Client<UnixTransport> {
        let stream = self.role.take().expect("the role's end is taken once");
        let transport = UnixTransport::from_stream(stream).expect("a transport");
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        let mut client = Client::connect(transport, Version::new(major, minor, patch)).expect("the handshake");
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("a read deadline");
        client
    }

    /// Who provides the `auth.polkit` slot, as the session object publishes it.
    fn provider(&self) -> String {
        let mut client = owner_client(&self.session.ipc());
        let snapshot = client.subscribe(Filter::All).expect("the snapshot");
        snapshot
            .objects()
            .iter()
            .find(|view| view.ty().to_string() == "gnoblin.session")
            .and_then(|view| match view.property("slot_auth_polkit") {
                Some(Value::Text(provider)) => Some(provider.clone()),
                _ => None,
            })
            .expect("the session object names the slot's provider")
    }
}

const AUTHENTICATION: &str = r#"import { AuthenticationProvider, gnoblin } from "gnoblin";

const seen: string[] = [];
class Prompt extends AuthenticationProvider {
    onRequest(request) {
        seen.push(`${request.subject}:${request.title}`);
        if (request.subject === "org.example.allow") {
            request.allow("unix-user:test");
        } else if (request.subject === "org.example.deny") {
            request.decline("denied");
        }
        // Any other request is left open, as a prompt the user has not answered yet.
    }
}
gnoblin.authentication.setProvider(new Prompt());
(globalThis as { probe?: () => string }).probe = () => seen.join(",");
"#;

/// The next event the services connection is sent that `pick` takes, within the patience of a test.
#[track_caller]
fn services_event<T>(
    services: &mut Client<UnixTransport>,
    what: &str,
    mut pick: impl FnMut(gnoblin_client::Event) -> Option<T>,
) -> T {
    let deadline = Instant::now() + PATIENCE;
    loop {
        match services.next_event() {
            Ok(event) => {
                if let Some(found) = pick(event) {
                    return found;
                }
            }
            Err(ClientError::Idle) => {}
            Err(error) => panic!("the services connection failed while waiting for {what}: {error}"),
        }
        assert!(Instant::now() < deadline, "never reached: {what}");
    }
}

/// The shell holds the `auth.polkit` slot while a provider is set, so the services role's offer reaches the script, and
/// what the script settles goes back: an identity is an answer only the offering host reads, a decline is a refusal.
/// A reload that drops the provider gives the slot back to the services role.
#[test]
fn an_authentication_provider_answers_polkit_requests_and_gives_the_slot_back_on_reload() {
    let mut services = Services::start();
    let mut offerer = services.role_client();
    let mut mapper = Mapper::connect(&services.session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    save(&config.path().join("init.ts"), AUTHENTICATION);
    let shell = Shell::start(&services.session, config.path(), |_| {});
    wait_for(
        "the shell holds the slot",
        &services.session,
        &mut mapper,
        &shell,
        || services.provider() == "shell",
    );

    let allowed = offerer
        .offer(
            "auth.polkit",
            "Authentication required",
            "Authenticate to mount",
            b"org.example.allow",
            Duration::from_secs(60),
        )
        .expect("the slot has a provider");
    let answer = services_event(&mut offerer, "the provider's answer", |event| match event {
        gnoblin_client::Event::RequestAnswered { request, answer } if request == allowed => Some(answer),
        _ => None,
    });
    assert_eq!(
        answer.as_slice(),
        b"unix-user:test",
        "the answer is the identity and only it"
    );

    let denied = offerer
        .offer(
            "auth.polkit",
            "Authentication required",
            "Authenticate to mount",
            b"org.example.deny",
            Duration::from_secs(60),
        )
        .expect("the slot has a provider");
    let completion = services_event(&mut offerer, "the refusal", |event| match event {
        gnoblin_client::Event::RequestFinished { request, completion } if request == denied => Some(completion),
        _ => None,
    });
    assert!(matches!(completion, RequestCompletion::Refused(_)), "{completion:?}");

    // A request the script leaves open ends when the shell does: the compositor knows the shell holds it.
    let open = offerer
        .offer(
            "auth.polkit",
            "Authentication required",
            "Authenticate to mount",
            b"org.example.later",
            Duration::from_secs(60),
        )
        .expect("the slot has a provider");
    wait_for(
        "the script saw all three requests",
        &services.session,
        &mut mapper,
        &shell,
        || shell.probe().matches("Authentication required").count() == 3,
    );

    let before = shell.generation();
    save(&config.path().join("init.ts"), "// no provider\n");
    wait_for(
        "the reload released the slot",
        &services.session,
        &mut mapper,
        &shell,
        || shell.generation() > before && services.provider() == "services",
    );
    let completion = services_event(
        &mut offerer,
        "the open request ended with the generation",
        |event| match event {
            gnoblin_client::Event::RequestFinished { request, completion } if request == open => Some(completion),
            _ => None,
        },
    );
    assert!(
        matches!(completion, RequestCompletion::Refused(_) | RequestCompletion::Ended),
        "a request a retired generation held is ended, not left to its deadline: {completion:?}"
    );
    shell.finish();
}

/// A shell that goes away without a word, with a request open, is the case a crash is: the compositor holds the claim
/// and ends the request when the connection closes, so polkit's caller is not left to the request's deadline, and the
/// slot is the services role's again.
#[test]
fn an_open_authentication_request_ends_at_once_when_the_shell_goes_away() {
    let mut services = Services::start();
    let mut offerer = services.role_client();
    let mut mapper = Mapper::connect(&services.session.wayland());
    let config = tempfile::tempdir().expect("a config directory");
    save(&config.path().join("init.ts"), AUTHENTICATION);
    let shell = Shell::start(&services.session, config.path(), |_| {});
    wait_for(
        "the shell holds the slot",
        &services.session,
        &mut mapper,
        &shell,
        || services.provider() == "shell",
    );
    let open = offerer
        .offer(
            "auth.polkit",
            "Authentication required",
            "Authenticate to mount",
            b"org.example.later",
            Duration::from_secs(600),
        )
        .expect("the slot has a provider");
    wait_for(
        "the script saw the request",
        &services.session,
        &mut mapper,
        &shell,
        || shell.probe().contains("org.example.later"),
    );
    shell.finish();
    let completion = services_event(&mut offerer, "the request ended with the shell", |event| match event {
        gnoblin_client::Event::RequestFinished { request, completion } if request == open => Some(completion),
        _ => None,
    });
    assert!(
        !matches!(completion, RequestCompletion::TimedOut | RequestCompletion::Answered),
        "a request the shell held ends with it, not at its deadline: {completion:?}"
    );
    let deadline = Instant::now() + PATIENCE;
    while services.provider() != "services" {
        assert!(Instant::now() < deadline, "the slot went back to the services role");
        std::thread::sleep(POLL);
    }
}

// ---- the keyboard move and resize of a window -------------------------------------------------------------------

const KEY_UP: u32 = 103;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;
const KEY_F11: u32 = 87;
const KEY_F12: u32 = 88;

/// The hint the keyboard mode draws while it runs.
const MOVE_HINT: &str = "Move: arrow keys, Return to keep it, Escape to cancel";
const RESIZE_HINT: &str = "Resize: arrow keys, Return to keep it, Escape to cancel";

fn write_reposition_config(dir: &Path) {
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, defineConfig, gnoblin } from "gnoblin";

defineConfig({
    workspaces: [{ id: "desk", layout: new Layout() }],
    rules: [{ appId: "org.example.keys", position: { x: 300, y: 200 }, size: { width: 200, height: 150 } }],
    keys: {
        F11: () => {
            gnoblin.clients.at(0)?.moveWithKeys();
        },
        F12: () => {
            gnoblin.clients.at(0)?.resizeWithKeys();
        },
    },
});
"#,
    )
    .expect("init.ts");
}

/// Presses `key` until the compositor shows `hint`: the shell's claim of the key reaches the compositor a few frames
/// after the shell starts, and a press that comes before is the window's and starts nothing.
#[track_caller]
fn start_mode(session: &Session, mapper: &mut Mapper, shell: &Shell, key: u32, hint: &str) {
    let ipc = session.ipc();
    wait_for("the keyboard mode runs", session, mapper, shell, || {
        if !shows(&ipc, hint) {
            tap_key(session, key);
        }
        shows(&ipc, hint)
    });
}

/// Presses `key` and waits until the window of `app` has `expected`. A key that comes before the mode has the keyboard is the
/// window's and moves nothing, so the key is pressed again once a second for as long as the rectangle has not arrived.
#[track_caller]
fn tap_until(what: &str, ctx: (&Session, &mut Mapper, &Shell), key: u32, app: &str, expected: (f64, f64, f64, f64)) {
    let (session, mapper, shell) = ctx;
    let ipc = session.ipc();
    tap_key(session, key);
    let mut last = Instant::now();
    wait_for(what, session, mapper, shell, || {
        if rect_of(&ipc, app) == Some(expected) {
            return true;
        }
        if last.elapsed() > Duration::from_secs(1) {
            tap_key(session, key);
            last = Instant::now();
        }
        false
    });
}

/// The arrow keys, Return and Escape of a real keyboard move, resize and cancel a real window, and the keyboard goes
/// back to it. The mode holds the keyboard, so the keys reach it through the compositor's seat and not through a
/// binding: this is the path the fake link cannot show.
#[test]
fn the_keyboard_moves_and_resizes_a_real_window_keeps_it_on_return_and_puts_it_back_on_escape() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    write_reposition_config(config.path());
    let shell = Shell::start(&session, config.path(), |_| {});
    let mut mapper = Mapper::connect(&session.wayland());
    let ipc = session.ipc();
    let app = "org.example.keys";
    wait_for("the shell runs the config", &session, &mut mapper, &shell, || {
        shell.generation() > 0 && shell.view().contains("F11")
    });
    mapper.map_window(app, "Keys");
    wait_for("the window at its place", &session, &mut mapper, &shell, || {
        rect_of(&ipc, app) == Some((300.0, 200.0, 200.0, 150.0))
    });

    start_mode(&session, &mut mapper, &shell, KEY_F11, MOVE_HINT);
    tap_until(
        "one step right",
        (&session, &mut mapper, &shell),
        KEY_RIGHT,
        app,
        (316.0, 200.0, 200.0, 150.0),
    );
    tap_until(
        "one step down",
        (&session, &mut mapper, &shell),
        KEY_DOWN,
        app,
        (316.0, 216.0, 200.0, 150.0),
    );
    tap_key(&session, KEY_ENTER);
    wait_for(
        "Return keeps the place and ends the mode",
        &session,
        &mut mapper,
        &shell,
        || !shows(&ipc, MOVE_HINT),
    );
    assert_eq!(rect_of(&ipc, app), Some((316.0, 216.0, 200.0, 150.0)));
    wait_for(
        "the keyboard is the window's again",
        &session,
        &mut mapper,
        &shell,
        || focused_app(&shell).as_deref() == Some(app),
    );

    // Resize: Down grows, Up shrinks, and Escape puts the size back.
    start_mode(&session, &mut mapper, &shell, KEY_F12, RESIZE_HINT);
    tap_until(
        "one step taller",
        (&session, &mut mapper, &shell),
        KEY_DOWN,
        app,
        (316.0, 216.0, 200.0, 166.0),
    );
    tap_until(
        "one step narrower",
        (&session, &mut mapper, &shell),
        KEY_LEFT,
        app,
        (316.0, 216.0, 184.0, 166.0),
    );
    tap_until(
        "one step shorter",
        (&session, &mut mapper, &shell),
        KEY_UP,
        app,
        (316.0, 216.0, 184.0, 150.0),
    );
    tap_key(&session, KEY_ESC);
    wait_for("Escape restores the rectangle", &session, &mut mapper, &shell, || {
        !shows(&ipc, RESIZE_HINT) && rect_of(&ipc, app) == Some((316.0, 216.0, 200.0, 150.0))
    });
    wait_for(
        "the keyboard is the window's again",
        &session,
        &mut mapper,
        &shell,
        || focused_app(&shell).as_deref() == Some(app),
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    shell.finish();
}
