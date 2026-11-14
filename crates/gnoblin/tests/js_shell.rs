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
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::{Compositor, InputTap, SessionConfig, StopHandle};
use gnoblin_js::{JsHealth, JsRole, JsRoleConfig};
use gnoblin_log::event::capture::logged;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, Version, read_frame, write_frame};
use gnoblin_types::{DeviceId, OutputId, Point};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

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

// ---- the scene, read as an observer ------------------------------------------------------------------------------

fn version() -> Version {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Version::new(major, minor, patch)
}

/// One window as the compositor's tree holds it.
#[derive(Clone, Debug, PartialEq)]
struct Window {
    id: u64,
    app_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Every node of the compositor's tree, read through a fresh observer connection.
fn observe(ipc: &Path) -> Vec<snapshot_node::Properties> {
    let transport = UnixTransport::connect(ipc).expect("the session is listening");
    let mut stream = Client::connect(transport, version())
        .expect("the handshake")
        .into_inner()
        .into_stream()
        .expect("the handshaken socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("a read deadline");
    write_frame(
        &mut stream,
        &Envelope::request_message(
            "tree",
            "attach",
            1,
            &AttachArgs {
                version: 1,
                mode: "observer".to_owned(),
            },
        ),
    )
    .expect("the attach");
    let _attached: AttachedPayload = read_frame(&mut stream)
        .expect("the attach reply frame")
        .expect("an attach reply")
        .payload_as()
        .expect("the attach payload decodes");
    let mut nodes: Vec<snapshot_node::Properties> = Vec::new();
    loop {
        let chunk: SnapshotPayload = read_frame(&mut stream)
            .expect("a snapshot frame")
            .expect("a snapshot chunk")
            .payload_as()
            .expect("the chunk decodes");
        nodes.extend(chunk.nodes);
        if chunk.last {
            break;
        }
    }
    nodes
}

/// Every window of the compositor's tree. A value at its default is not carried in a snapshot, so it reads as zero.
fn scene(ipc: &Path) -> Vec<Window> {
    observe(ipc)
        .into_iter()
        .filter(|node| node.class == "window")
        .map(|node| {
            let value = |key: &str| {
                node.props
                    .iter()
                    .find(|prop| prop.key == key)
                    .map_or(0.0, |prop| prop.value_double)
            };
            Window {
                id: node.id,
                app_id: node.app_id.clone(),
                x: value("x"),
                y: value("y"),
                width: value("width"),
                height: value("height"),
            }
        })
        .collect()
}

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
    observe(ipc)
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

/// The spots of the windows by app id.
fn spots(windows: &[Window]) -> BTreeMap<String, (f64, f64)> {
    windows
        .iter()
        .map(|window| (window.app_id.clone(), (window.x, window.y)))
        .collect()
}

fn ids(windows: &[Window]) -> BTreeMap<String, u64> {
    windows
        .iter()
        .map(|window| (window.app_id.clone(), window.id))
        .collect()
}

// ---- real Wayland windows ----------------------------------------------------------------------------------------

#[derive(Default)]
struct App;

/// A Wayland client that maps toplevels with a buffer, as an application does.
struct Mapper {
    _connection: Connection,
    queue: EventQueue<App>,
    app: App,
    qh: QueueHandle<App>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    /// The protocol objects of every window mapped, kept so they stay mapped.
    windows: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )>,
}

impl Mapper {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let shm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
        let wm_base = globals
            .bind(&qh, 1..=6, ())
            .expect("the session advertises xdg_wm_base");
        Self {
            _connection: connection,
            queue,
            app: App,
            qh,
            compositor,
            shm,
            wm_base,
            windows: Vec::new(),
        }
    }

    /// Processes every event the session has sent, which answers its pings.
    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.app).expect("a round trip");
    }

    /// Creates a toplevel and maps it with a buffer.
    fn map_window(&mut self, app_id: &str, title: &str) {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        let (width, height) = (200, 150);
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(u64::try_from(length).expect("a length"))
            .expect("a buffer that big");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
        self.windows.push((surface, xdg, toplevel));
    }
}

macro_rules! ignore_events {
    ($($proxy:ty),* $(,)?) => {$(
        impl Dispatch<$proxy, ()> for App {
            fn event(_: &mut Self, _: &$proxy, _: <$proxy as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    xdg_toplevel::XdgToplevel,
);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        _: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
        }
    }
}

// ---- the shell, in a thread of its own ---------------------------------------------------------------------------

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
                                .eval(
                                    "`${gnoblin.clients.length} clients, ${gnoblin.workspaces.length} workspaces: ` + \
                                     gnoblin.clients.map((c) => `${c.appId}@${c.x},${c.y}`).join(';')",
                                )
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

#[test]
fn a_shell_with_no_init_file_runs_bare_and_loads_the_file_when_it_appears() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");
    let config = tempfile::tempdir().expect("a config directory");
    let shell = Shell::start(&session, config.path(), |_| {});
    let ipc = session.ipc();
    // The first generation after the file appears is the first one with user code.
    wait_for("the shell is up", &session, &mut mapper, &shell, || {
        shell.status.lock().expect("the status lock").ended.is_none()
    });
    assert!(shell.diagnostics().is_empty(), "{:?}", shell.diagnostics());

    write_config(config.path(), r#"{ "org.example.first": [21, 22] }"#);
    wait_for("the new file places the window", &session, &mut mapper, &shell, || {
        spots(&scene(&ipc)).get("org.example.first") == Some(&(21.0, 22.0))
    });
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
