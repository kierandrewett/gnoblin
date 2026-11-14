//! What the tests that run the JavaScript shell against a real compositor share: a real Wayland client that maps
//! toplevels with a buffer, and the scene read back through a tree observer connection, so a test asserts what the
//! compositor holds and not what the shell believes.
//!
//! Each test crate uses its own part of this, so what one does not call is not dead.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, ChangesPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, Kind, Version, read_frame, write_frame};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

// ---- the scene, read as an observer ------------------------------------------------------------------------------

fn version() -> Version {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Version::new(major, minor, patch)
}

/// One window as the compositor's tree holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: u64,
    pub app_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Attaches a fresh observer connection and reads its snapshot. The stream goes on to carry the compositor's changes.
fn attach_observer(ipc: &Path) -> (UnixStream, Vec<snapshot_node::Properties>) {
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
    (stream, nodes)
}

/// Every node of the compositor's tree, read through a fresh observer connection.
pub fn snapshot(ipc: &Path) -> Vec<snapshot_node::Properties> {
    attach_observer(ipc).1
}

/// An observer that stays attached and follows the compositor's changes, so what it reports is what the compositor
/// has published up to now and not what a snapshot taken in one of its frames happened to hold. A window's `x`
/// and `y` that the compositor itself places (no shell holds the tree) reach an observer only as changes.
pub struct Watcher {
    windows: Arc<Mutex<BTreeMap<u64, Window>>>,
    history: Arc<Mutex<Vec<(String, f64, f64)>>>,
    stream: UnixStream,
}

impl Watcher {
    pub fn attach(ipc: &Path) -> Self {
        let (stream, nodes) = attach_observer(ipc);
        stream.set_read_timeout(None).expect("no read deadline");
        let windows: Arc<Mutex<BTreeMap<u64, Window>>> = Arc::new(Mutex::new(
            nodes
                .iter()
                .filter(|node| node.class == "window")
                .map(|node| (node.id, window_of(node)))
                .collect(),
        ));
        let history: Arc<Mutex<Vec<(String, f64, f64)>>> = Arc::default();
        let (mut reader, held, kept) = (
            stream.try_clone().expect("a second handle"),
            Arc::clone(&windows),
            Arc::clone(&history),
        );
        std::thread::Builder::new()
            .name("scene-watcher".to_owned())
            .spawn(move || {
                while let Ok(Some(envelope)) = read_frame(&mut reader) {
                    if !matches!(envelope.kind, Some(Kind::Event(_))) || envelope.member != "changes" {
                        continue;
                    }
                    let Some(batch) = envelope.payload_as::<ChangesPayload>() else {
                        continue;
                    };
                    let mut windows = held.lock().expect("the window table");
                    let mut touched: Vec<u64> = Vec::new();
                    for change in batch.changes {
                        match change.kind.as_str() {
                            "created" if change.class == "window" => {
                                windows.insert(
                                    change.node,
                                    Window {
                                        id: change.node,
                                        app_id: String::new(),
                                        x: 0.0,
                                        y: 0.0,
                                        width: 0.0,
                                        height: 0.0,
                                    },
                                );
                            }
                            "set" => {
                                if let Some(window) = windows.get_mut(&change.node) {
                                    match change.key.as_str() {
                                        "app_id" => window.app_id = change.value_text,
                                        "x" => {
                                            window.x = change.value_double;
                                            touched.push(change.node);
                                        }
                                        "y" => {
                                            window.y = change.value_double;
                                            touched.push(change.node);
                                        }
                                        "width" => window.width = change.value_double,
                                        "height" => window.height = change.value_double,
                                        _ => {}
                                    }
                                }
                            }
                            "destroyed" => {
                                windows.remove(&change.node);
                                for gone in change.removed {
                                    windows.remove(&gone);
                                }
                            }
                            _ => {}
                        }
                    }
                    // One entry for each window whose place the batch changed, after the whole batch: a window
                    // moved in `x` and `y` at once is one move.
                    let mut kept = kept.lock().expect("the placement history");
                    for id in touched {
                        let Some(window) = windows.get(&id) else {
                            continue;
                        };
                        let last = kept.iter().rev().find(|(app_id, ..)| *app_id == window.app_id);
                        if last.map(|(_, x, y)| (*x, *y)) != Some((window.x, window.y)) {
                            kept.push((window.app_id.clone(), window.x, window.y));
                        }
                    }
                }
            })
            .expect("the watcher thread");
        Self {
            windows,
            history,
            stream,
        }
    }

    /// The windows as published so far.
    pub fn scene(&self) -> Vec<Window> {
        self.windows
            .lock()
            .expect("the window table")
            .values()
            .cloned()
            .collect()
    }

    /// Every place the compositor has published for the window with `app_id`, in order, without repeats.
    pub fn places(&self, app_id: &str) -> Vec<(f64, f64)> {
        self.history
            .lock()
            .expect("the placement history")
            .iter()
            .filter(|(held, ..)| held == app_id)
            .map(|(_, x, y)| (*x, *y))
            .collect()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Ends the reader thread, which is blocked on the stream.
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// A window of a snapshot, with the values it carries.
fn window_of(node: &snapshot_node::Properties) -> Window {
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
}

/// How many screens the compositor's tree holds. A session that has none places no window by its own policy.
pub fn screens(ipc: &Path) -> usize {
    snapshot(ipc).iter().filter(|node| node.class == "screen").count()
}

/// Every window of the compositor's tree. A value at its default is not carried in a snapshot, so it reads as zero.
pub fn scene(ipc: &Path) -> Vec<Window> {
    snapshot(ipc)
        .iter()
        .filter(|node| node.class == "window")
        .map(window_of)
        .collect()
}

/// Whether the compositor's tree holds `key` true on the window of `app_id`. A value at its default is not carried in a
/// snapshot, so it reads as false.
pub fn flag_of(ipc: &Path, app_id: &str, key: &str) -> bool {
    snapshot(ipc)
        .iter()
        .filter(|node| node.class == "window" && node.app_id == app_id)
        .any(|node| node.props.iter().any(|prop| prop.key == key && prop.value_bool))
}

/// The spots of the windows by app id.
pub fn spots(windows: &[Window]) -> BTreeMap<String, (f64, f64)> {
    windows
        .iter()
        .map(|window| (window.app_id.clone(), (window.x, window.y)))
        .collect()
}

/// The node ids of the windows by app id.
pub fn ids(windows: &[Window]) -> BTreeMap<String, u64> {
    windows
        .iter()
        .map(|window| (window.app_id.clone(), window.id))
        .collect()
}

// ---- real Wayland windows ----------------------------------------------------------------------------------------

/// The `xdg_toplevel` state a configure carries when the window is fullscreen.
pub const STATE_FULLSCREEN: u32 = 2;

/// What the client has heard from the compositor.
#[derive(Default)]
pub struct App {
    /// The serial of the last pointer button press the client was sent, which is what earns an activation token.
    serial: u32,
    /// The seat's pointer, once the client asked for it.
    pointer: Option<wl_pointer::WlPointer>,
    /// The token an activation token object delivered.
    token: Option<String>,
    /// The size and the states of each toplevel's last configure, by the toplevel's protocol id.
    configures: BTreeMap<u32, (i32, i32, Vec<u32>)>,
}

/// A Wayland client that maps toplevels with a buffer, as an application does.
pub struct Mapper {
    _connection: Connection,
    queue: EventQueue<App>,
    app: App,
    qh: QueueHandle<App>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    globals: GlobalList,
    seat: Option<wl_seat::WlSeat>,
    /// The protocol objects of every window mapped, kept so they stay mapped.
    windows: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )>,
}

impl Mapper {
    pub fn connect(socket: &Path) -> Self {
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
            app: App::default(),
            qh,
            compositor,
            shm,
            wm_base,
            globals,
            seat: None,
            windows: Vec::new(),
        }
    }

    /// Processes every event the session has sent, which answers its pings.
    pub fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.app).expect("a round trip");
    }

    /// Creates a toplevel and maps it with a buffer.
    pub fn map_window(&mut self, app_id: &str, title: &str) {
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

    /// Asks the compositor to make window number `window` (in the order they were mapped) fullscreen.
    pub fn fullscreen(&mut self, window: usize) {
        self.windows[window].2.set_fullscreen(None);
        self.roundtrip();
    }

    /// Asks the compositor to take window number `window` out of fullscreen.
    pub fn unfullscreen(&mut self, window: usize) {
        self.windows[window].2.unset_fullscreen();
        self.roundtrip();
    }

    /// Asks the compositor to minimise window number `window`.
    pub fn minimize(&mut self, window: usize) {
        self.windows[window].2.set_minimized();
        self.roundtrip();
    }

    /// The states of window number `window`'s last configure (`STATE_FULLSCREEN` is one), or none before the first.
    pub fn states(&self, window: usize) -> Vec<u32> {
        let id = self.windows[window].2.id().protocol_id();
        self.app
            .configures
            .get(&id)
            .map_or_else(Vec::new, |(_, _, states)| states.clone())
    }

    /// The size of window number `window`'s last configure.
    pub fn configured(&self, window: usize) -> Option<(i32, i32)> {
        let id = self.windows[window].2.id().protocol_id();
        self.app.configures.get(&id).map(|(width, height, _)| (*width, *height))
    }

    /// Binds the seat's pointer, so the client is told the serial of a button press. A press the compositor sends
    /// this client afterwards is what `press_serial` reports.
    pub fn listen_pointer(&mut self) {
        let seat: wl_seat::WlSeat = self
            .globals
            .bind(&self.qh, 1..=5, ())
            .expect("the session advertises wl_seat");
        self.seat = Some(seat);
        self.roundtrip();
        self.roundtrip();
    }

    /// The serial of the last button press the compositor sent this client, if it has sent one.
    pub fn press_serial(&self) -> Option<u32> {
        (self.app.serial != 0).then_some(self.app.serial)
    }

    /// Asks for an activation token with `serial` (a button press the client was sent) and redeems it for window
    /// number `window`: the request a client makes to be brought forward. The compositor decides whether to allow it.
    pub fn activate(&mut self, window: usize, serial: u32) {
        let seat = self.seat.clone().expect("listen_pointer ran first");
        let activation: xdg_activation_v1::XdgActivationV1 = self
            .globals
            .bind(&self.qh, 1..=1, ())
            .expect("the session advertises xdg_activation_v1");
        self.app.token = None;
        let token = activation.get_activation_token(&self.qh, ());
        token.set_serial(serial, &seat);
        token.commit();
        for _ in 0..40 {
            self.roundtrip();
            if self.app.token.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let token = self
            .app
            .token
            .clone()
            .expect("the compositor minted a token for the click");
        activation.activate(token, &self.windows[window].0);
        self.roundtrip();
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
    xdg_activation_v1::XdgActivationV1,
);

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        app: &mut Self,
        toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, states } = event {
            let states = states
                .chunks_exact(4)
                .map(|state| u32::from_ne_bytes([state[0], state[1], state[2], state[3]]))
                .collect();
            app.configures
                .insert(toplevel.id().protocol_id(), (width, height, states));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        app: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && app.pointer.is_none()
        {
            app.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            app.serial = serial;
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            app.token = Some(token);
        }
    }
}

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

/// Every node of the tree as text, for a failure message.
pub fn dump(ipc: &Path) -> String {
    snapshot(ipc)
        .iter()
        .filter(|node| node.class == "window" || node.class == "screen")
        .map(|node| format!("{node:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}
