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
use gnoblin_compositor::{Compositor, InputTap, SessionConfig, StopHandle};
use gnoblin_js::{JsHealth, JsRole, JsRoleConfig};
use gnoblin_log::event::capture::logged;
use gnoblin_types::{DeviceId, OutputId, Point};

mod support;
use support::{Mapper, Watcher, ids, scene, spots};

/// How long a test waits for a state before it fails. A passing run never waits this long: it is the margin a busy
/// machine gets, so a failure is a state that never came and not a slow turn.
const PATIENCE: Duration = Duration::from_secs(60);

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
        std::thread::sleep(Duration::from_millis(10));
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
        gnoblin.clients.map((c) => `${c.appId}@${c.x},${c.y}`).join(";");
})()"#;

/// What the shell thread reports, refreshed after every frame.
#[derive(Default)]
struct Status {
    generation: u64,
    degraded: bool,
    diagnostics: Vec<String>,
    /// What the shell's script sees of the clients, as text, for a failure message.
    view: String,
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
    /// Starts the shell over `session`'s owner socket with `dir` as its config directory.
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
                    tune(&mut config);
                    let mut role = match JsRole::connect(&env, config) {
                        Ok(role) => role,
                        Err(error) => {
                            shared.lock().expect("the status lock").ended = Some(error.to_string());
                            return;
                        }
                    };
                    while !stopped.load(Ordering::Relaxed) {
                        let ticked = role.tick();
                        {
                            let mut status = shared.lock().expect("the status lock");
                            status.generation = role.generation_id();
                            status.degraded = matches!(role.health(), JsHealth::Degraded { .. });
                            status.diagnostics = role.diagnostics().to_vec();
                            status.view = role
                                .shell_mut()
                                .eval(VIEW_PROBE)
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
        Self {
            stop,
            status,
            thread: Mutex::new(Some(thread)),
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
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The config directory of the tests: `slots.ts` holds the places, `init.ts` a typed listener over them.
fn write_config(dir: &Path, places: &str) {
    fs::write(
        dir.join("slots.ts"),
        format!("export const places: Record<string, [number, number]> = {places};\n"),
    )
    .expect("slots.ts");
    fs::write(
        dir.join("init.ts"),
        r#"import { Layout, Workspace, gnoblin } from "gnoblin";
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
"#,
    )
    .expect("init.ts");
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
    let first_generation = shell.generation();

    // Slice criterion 3: an edit is a reload, and the windows keep their ids and take the new places.
    write_config(
        config.path(),
        r#"{ "org.example.first": [500, 400], "org.example.second": [600, 300], "*": [700, 200] }"#,
    );
    wait_for(
        "both windows at the rewritten places",
        &session,
        &mut mapper,
        &shell,
        || {
            let now = spots(&scene(&ipc));
            now.get("org.example.first") == Some(&(500.0, 400.0))
                && now.get("org.example.second") == Some(&(600.0, 300.0))
        },
    );
    assert_eq!(ids(&scene(&ipc)), before, "the reload kept every window id");
    let second_generation = shell.generation();
    assert!(
        second_generation > first_generation,
        "a reload runs a newer generation ({first_generation} -> {second_generation})"
    );
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());

    // Slice criterion 4: a syntax error keeps the running generation and is reported with its `.ts` position. The
    // old generation still places a window that maps afterwards.
    fs::write(
        config.path().join("init.ts"),
        "import { gnoblin } from \"gnoblin\";\n\nconst broken: number = = 5;\n",
    )
    .expect("a broken init.ts");
    wait_for("the syntax error is reported", &session, &mut mapper, &shell, || {
        shell.diagnostics().iter().any(|text| text.contains("init.ts:3:"))
    });
    // Nothing swaps in after the failed reload: the generation stays what it was when the error was reported.
    let at_the_error = shell.generation();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        shell.generation(),
        at_the_error,
        "the generation that ran before the error still runs"
    );
    mapper.map_window("org.example.third", "Third");
    wait_for(
        "the old generation places a new window",
        &session,
        &mut mapper,
        &shell,
        || spots(&scene(&ipc)).get("org.example.third") == Some(&(700.0, 200.0)),
    );
    assert_eq!(
        ids(&scene(&ipc)).get("org.example.first"),
        before.get("org.example.first"),
        "the failed reload kept the window ids"
    );

    // Fixing the file is seen even though the failed module was in no generation's graph.
    write_config(
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
    // A quarter of a second per listener call: long enough that a loaded machine does not trip an honest handler,
    // short enough that the test is quick.
    let shell = Shell::start(&session, config.path(), |config| {
        config.shell.entry_budget = Duration::from_millis(250);
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
        config.shell.entry_budget = Duration::from_millis(250);
        // The restaged generation replays the hanging window while it loads, so staging is what runs out.
        config.shell.js.stage_budget = Duration::from_millis(500);
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

    // No `init.ts`: the stock default config is a real runtime config. It declares one floating `desktop`, and
    // its keys are bound on the seat (through the real compositor, which took the claims or the shell would have
    // reported a refusal).
    wait_for("the stock config runs", &session, &mut mapper, &shell, || {
        section(&shell.view(), "workspaces [") == "desktop:FloatingLayout"
    });
    let keys = shell.view();
    let keys = bound_keys(&keys);
    assert_eq!(keys.first(), Some(&"Super+1"), "the stock keys: {keys:?}");
    assert!(keys.contains(&"XF86AudioMute"), "the stock keys: {keys:?}");
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());
    let stock = shell.generation();
    let window = ids(&scene(&ipc));

    // A user `defineConfig` replaces the workspaces whole and merges the keys by trigger: its `Super+Return` is
    // added, its `XF86AudioMute` replaces the stock one, `Super+1` (which showed the dropped `desktop`) goes, and
    // the other stock keys stay.
    fs::write(config.path().join("init.ts"), LAYERED_CONFIG).expect("init.ts");
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
        !keys.contains(&"Super+1"),
        "the stock key for the dropped workspace: {keys:?}"
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
    // process that was killed. The pause is for the compositor to take the close in before the next window maps.
    let log = shell.finish();
    assert!(shell.ended().is_none(), "the shell was stopped, not failed: {log:?}");
    std::thread::sleep(Duration::from_millis(300));

    // Criterion 6, the compositor's half: with no shell holding the tree it still answers, and the Rust fallback
    // policy is in charge again. It never knew where the shell put the two windows, so they return to the places it
    // gave them, and a window that maps now takes the next step of the cascade.
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
/// `expected` for `app`: the drag is live and has moved the window. The shell claims its chords in its first frames,
/// so a press that comes before a claim is live is the window's, and moves nothing: the gesture is repeated until a
/// press is the shell's. Super and the button stay down for the caller to release with [`end_drag`].
#[track_caller]
fn begin_drag(
    session: &Session,
    mapper: &mut Mapper,
    app: &str,
    code: u32,
    from: (i32, i32),
    first: (i32, i32),
    expected: (f64, f64, f64, f64),
) {
    let ipc = session.ipc();
    let deadline = Instant::now() + PATIENCE;
    loop {
        super_key(session, true);
        motion(session, from.0, from.1);
        std::thread::sleep(Duration::from_millis(40));
        button(session, code, true);
        motion(session, first.0, first.1);
        let attempt = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < attempt {
            mapper.roundtrip();
            if rect_of(&ipc, app) == Some(expected) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        button(session, code, false);
        super_key(session, false);
        assert!(
            Instant::now() < deadline,
            "no drag ever took the window: it is at {:?}, expected {expected:?}",
            rect_of(&ipc, app)
        );
        std::thread::sleep(Duration::from_millis(100));
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
        std::thread::sleep(Duration::from_millis(10));
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
