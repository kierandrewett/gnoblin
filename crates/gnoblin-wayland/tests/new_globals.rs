//! The standard globals this crate gained, driven by a real `wayland-client`.
//!
//! Each test starts a session, connects a real client, binds the new global and issues
//! the requests a client would, then reads the effect back through the session snapshot
//! or through the client's own events. The failure modes each test covers are listed at
//! the top of the module that implements the protocol:
//!
//! - `xdg_foreign_v2`: `a_toplevel_is_exported_and_imported_as_a_parent`,
//!   `a_handle_that_was_never_minted_imports_nothing`,
//!   `an_exported_surface_that_is_not_a_toplevel_is_refused`
//! - `xdg_dialog_v1`: `a_dialog_is_set_and_unset_modal`
//! - `xdg_toplevel_icon_v1`: `an_icon_is_set_by_name_on_a_toplevel`
//! - `xdg_toplevel_tag_v1`: `a_toplevel_tag_is_set`
//! - `wp_fifo_v1`: `a_fifo_barrier_is_released_on_a_frame`
//!
//! `wp_alpha_modifier_v1`: `a_surface_has_one_alpha_modifier`; the factor's effect on pixels is
//! `a_surface_alpha_factor_blends_with_the_background` in `render.rs`.
//!
//! Smithay posts a protocol error by closing the offending connection. The refusal test
//! reads the error code the client receives and checks a second client still connects.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use gnoblin_wm::{NewWindowFocus, WmOptions};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, QueueHandle};
use wayland_protocols::wp::alpha_modifier::v1::client::{wp_alpha_modifier_surface_v1, wp_alpha_modifier_v1};
use wayland_protocols::wp::fifo::v1::client::{wp_fifo_manager_v1, wp_fifo_v1};
use wayland_protocols::xdg::dialog::v1::client::{xdg_dialog_v1, xdg_wm_dialog_v1};
use wayland_protocols::xdg::foreign::zv2::client::{
    zxdg_exported_v2, zxdg_exporter_v2, zxdg_imported_v2, zxdg_importer_v2,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols::xdg::toplevel_icon::v1::client::{xdg_toplevel_icon_manager_v1, xdg_toplevel_icon_v1};
use wayland_protocols::xdg::toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1;

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The title of the toplevel a test makes when it does not name one.
const DEFAULT_TITLE: &str = "gnoblin test window";

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    fn socket(&self) -> &Path {
        &self.socket
    }

    /// Waits for the window state to satisfy `predicate`, or fails with the last one.
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.observation.lock().expect("the observation lock").clone();
            if predicate(&observation) {
                return observation;
            }
            assert!(
                Instant::now() < deadline,
                "the session never reached the expected state; last state: {observation:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Stops the session when the worker finishes, including when it panics.
struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A session that has been created but is not running yet.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the socket.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: Handle,
}

impl Session {
    fn start() -> Self {
        Self::start_with_wm(WmOptions::default())
    }

    /// Starts a session where a new window takes focus, as `focus_new_windows = allow` does.
    fn start_allowing_focus() -> Self {
        Self::start_with_wm(WmOptions {
            new_window_focus: NewWindowFocus::Allow,
            ..WmOptions::default()
        })
    }

    fn start_with_wm(wm: WmOptions) -> Self {
        let backend = Headless::new();
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            wm,
            ..Config::default()
        };
        let server = Server::new(Box::new(backend), config).expect("the session starts");
        let handle = Handle {
            socket,
            control: server.control(),
            observation: server.observation(),
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
        let handle = self.handle.clone();
        let worker = thread::spawn({
            let handle = handle.clone();
            move || {
                let _stop = StopOnDrop(handle.control.clone());
                body(&handle);
            }
        });
        let outcome: Result<(), LoopError> = self.server.run();
        if let Err(payload) = worker.join() {
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// What the test's client records. Only the events a test waits on are kept.
#[derive(Default)]
struct App {
    /// The `zxdg_exported_v2.handle` events, which name the exported surface.
    exported_handles: Vec<String>,
    /// How many `zxdg_imported_v2.destroyed` events arrived.
    imported_destroyed: u32,
    /// The tags of the frame callbacks that were answered, in the order they arrived.
    callbacks_done: Vec<u32>,
}

/// A real Wayland client for one connection.
struct Client {
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: wayland_client::globals::GlobalList,
}

impl Client {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
        }
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// Round-trips and returns the protocol error the server closed the connection with,
    /// as its object interface and code, or `None` when the connection is still open.
    fn roundtrip_protocol_error(&mut self) -> Option<(String, u32)> {
        match self.queue.roundtrip(&mut self.state) {
            Ok(_) => None,
            Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
                Some((error.object_interface.to_string(), error.code))
            }
            Err(other) => panic!("the round trip failed for another reason: {other}"),
        }
    }

    /// Round-trips until `done` holds for the client's events, or fails at the deadline.
    fn wait_until(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&self.state) {
            assert!(Instant::now() < deadline, "the client never saw the expected events");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn compositor(&self) -> wl_compositor::WlCompositor {
        self.globals
            .bind(&self.qh, 1..=6, ())
            .expect("the session advertises wl_compositor")
    }

    fn wm_base(&self) -> xdg_wm_base::XdgWmBase {
        self.globals
            .bind(&self.qh, 1..=6, ())
            .expect("the session advertises xdg_wm_base")
    }

    /// Creates a toplevel with `title` and commits it once, which asks for a configure.
    fn titled_toplevel(
        &mut self,
        title: &str,
    ) -> (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ) {
        let surface = self.compositor().create_surface(&self.qh, ());
        let xdg_surface = self.wm_base().get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        (surface, xdg_surface, toplevel)
    }

    fn toplevel(
        &mut self,
    ) -> (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ) {
        self.titled_toplevel(DEFAULT_TITLE)
    }

    fn shm(&self) -> wl_shm::WlShm {
        self.globals
            .bind(&self.qh, 1..=1, ())
            .expect("the session advertises wl_shm")
    }

    /// Maps a toplevel: acknowledges its first configure and commits a shared-memory buffer.
    fn mapped_toplevel(
        &mut self,
    ) -> (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ) {
        self.mapped_titled_toplevel(DEFAULT_TITLE)
    }

    /// Maps a toplevel with `title`: acknowledges its first configure and commits a shared-memory buffer.
    fn mapped_titled_toplevel(
        &mut self,
        title: &str,
    ) -> (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ) {
        let (surface, xdg_surface, toplevel) = self.titled_toplevel(title);
        let stride = 200 * 4;
        let length = stride * 150;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let pool = self.shm().create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, 200, 150, stride, wl_shm::Format::Argb8888, &self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 200, 150);
        surface.commit();
        self.roundtrip();
        (surface, xdg_surface, toplevel)
    }
}

/// Dispatch for globals the tests bind. A test only needs the requests it sends, so the
/// events of every object are accepted and ignored apart from the few a test reads.
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

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// A frame callback's tag is the one the test gave it when it asked for the callback.
impl Dispatch<wl_callback::WlCallback, u32> for App {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        tag: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.callbacks_done.push(*tag);
        }
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
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_exporter_v2::ZxdgExporterV2, ()> for App {
    fn event(
        _: &mut Self,
        _: &zxdg_exporter_v2::ZxdgExporterV2,
        _: zxdg_exporter_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_importer_v2::ZxdgImporterV2, ()> for App {
    fn event(
        _: &mut Self,
        _: &zxdg_importer_v2::ZxdgImporterV2,
        _: zxdg_importer_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_exported_v2::ZxdgExportedV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zxdg_exported_v2::ZxdgExportedV2,
        event: zxdg_exported_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_exported_v2::Event::Handle { handle } = event {
            state.exported_handles.push(handle);
        }
    }
}

impl Dispatch<zxdg_imported_v2::ZxdgImportedV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zxdg_imported_v2::ZxdgImportedV2,
        event: zxdg_imported_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_imported_v2::Event::Destroyed = event {
            state.imported_destroyed += 1;
        }
    }
}

impl Dispatch<xdg_wm_dialog_v1::XdgWmDialogV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_wm_dialog_v1::XdgWmDialogV1,
        _: xdg_wm_dialog_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_dialog_v1::XdgDialogV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_dialog_v1::XdgDialogV1,
        _: xdg_dialog_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1,
        _: xdg_toplevel_icon_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_toplevel_icon_v1::XdgToplevelIconV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel_icon_v1::XdgToplevelIconV1,
        _: xdg_toplevel_icon_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1,
        _: xdg_toplevel_tag_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_fifo_manager_v1::WpFifoManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_fifo_manager_v1::WpFifoManagerV1,
        _: wp_fifo_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_fifo_v1::WpFifoV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_fifo_v1::WpFifoV1,
        _: wp_fifo_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_alpha_modifier_v1::WpAlphaModifierV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_alpha_modifier_v1::WpAlphaModifierV1,
        _: wp_alpha_modifier_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1,
        _: wp_alpha_modifier_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for App {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[test]
fn a_modal_dialog_takes_focus_from_its_parent_and_is_reported_modal() {
    Session::start_allowing_focus().run(|session| {
        let mut client = Client::connect(session.socket());
        // The child maps first and the parent last, so the parent holds focus before the
        // child turns modal: the handoff below is then the window manager's doing.
        let (child_surface, _, child) = client.mapped_titled_toplevel("dialog child");
        let (parent_surface, _, parent) = client.mapped_titled_toplevel("dialog parent");
        session.wait_for(|snapshot| snapshot.focused().is_some_and(|window| window.title == "dialog parent"));
        // A dialog's modality is only honoured with a parent, so the child names one.
        child.set_parent(Some(&parent));
        child_surface.commit();
        client.roundtrip();
        let manager: xdg_wm_dialog_v1::XdgWmDialogV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises xdg_wm_dialog_v1");
        let dialog: xdg_dialog_v1::XdgDialogV1 = manager.get_xdg_dialog(&child, &client.qh, ());
        dialog.set_modal();
        child_surface.commit();
        client.roundtrip();

        let parent_id = session
            .wait_for(|snapshot| snapshot.titled("dialog parent").is_some())
            .titled("dialog parent")
            .expect("the parent window")
            .id;
        let modal = session.wait_for(|snapshot| snapshot.titled("dialog child").is_some_and(|window| window.modal));
        let child_window = modal.titled("dialog child").expect("the child window");
        assert_eq!(
            child_window.transient_for,
            Some(parent_id),
            "the child is a transient of the parent"
        );
        assert!(
            !modal.titled("dialog parent").expect("the parent window").modal,
            "the parent is not a dialog"
        );
        // The window manager moved focus from the parent to its open modal child.
        let focused =
            session.wait_for(|snapshot| snapshot.focused().is_some_and(|window| window.title == "dialog child"));
        assert!(
            !focused.titled("dialog parent").expect("the parent window").focused,
            "the parent has lost focus to its modal child"
        );

        dialog.unset_modal();
        child_surface.commit();
        client.roundtrip();
        session.wait_for(|snapshot| snapshot.titled("dialog child").is_some_and(|window| !window.modal));

        dialog.destroy();
        parent_surface.commit();
        client.roundtrip();
    });
}

#[test]
fn an_icon_is_set_by_name_on_a_toplevel() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, _, toplevel) = client.titled_toplevel("icon window");
        let manager: xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises xdg_toplevel_icon_manager_v1");
        let icon = manager.create_icon(&client.qh, ());
        icon.set_name("org.example.app".to_owned());
        manager.set_icon(&toplevel, Some(&icon));
        surface.commit();
        client.roundtrip();

        let set = session.wait_for(|snapshot| {
            snapshot
                .titled("icon window")
                .is_some_and(|window| window.icon_name.as_deref() == Some("org.example.app"))
        });
        let window = set.titled("icon window").expect("the window");
        assert_eq!(window.icon_buffer_count, 0, "a named icon carries no buffers");

        // Clearing the icon removes the name the shell reads.
        manager.set_icon(&toplevel, None);
        surface.commit();
        client.roundtrip();
        session.wait_for(|snapshot| {
            snapshot
                .titled("icon window")
                .is_some_and(|window| window.icon_name.is_none())
        });
    });
}

#[test]
fn a_toplevel_tag_is_set() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, _, toplevel) = client.titled_toplevel("tag window");
        let manager: xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises xdg_toplevel_tag_manager_v1");
        manager.set_toplevel_tag(&toplevel, "primary".to_owned());
        surface.commit();
        client.roundtrip();

        session.wait_for(|snapshot| {
            snapshot
                .titled("tag window")
                .is_some_and(|window| window.tag.as_deref() == Some("primary"))
        });
    });
}

#[test]
fn a_fifo_barrier_is_released_on_a_frame() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, _, _) = client.mapped_toplevel();
        let manager: wp_fifo_manager_v1::WpFifoManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises wp_fifo_manager_v1");
        let fifo = manager.get_fifo(&surface, &client.qh, ());

        // The first commit sets a barrier and asks for a frame callback.
        fifo.set_barrier();
        surface.frame(&client.qh, 1);
        surface.commit();
        // The second waits on that barrier, so it is held until the first is shown. Its
        // frame callback can only be answered after the barrier is released on a frame.
        fifo.wait_barrier();
        surface.frame(&client.qh, 2);
        surface.commit();

        client.wait_until(|app| app.callbacks_done.contains(&2));
        let first = client.state.callbacks_done.iter().position(|tag| *tag == 1);
        let second = client.state.callbacks_done.iter().position(|tag| *tag == 2);
        assert!(
            first.is_some() && first < second,
            "the first commit is shown before the held second one: {:?}",
            client.state.callbacks_done
        );
        fifo.destroy();
        client.roundtrip();
    });
}

#[test]
fn a_toplevel_is_exported_and_imported_as_a_parent() {
    Session::start().run(|session| {
        let mut exporter_client = Client::connect(session.socket());
        let (parent_surface, _, _parent) = exporter_client.titled_toplevel("exported parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = exporter_client
            .globals
            .bind(&exporter_client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let exported = exporter.export_toplevel(&parent_surface, &exporter_client.qh, ());
        exporter_client.roundtrip();
        let handle = exporter_client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");

        let mut importer_client = Client::connect(session.socket());
        let (child_surface, _, _child) = importer_client.titled_toplevel("imported child");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = importer_client
            .globals
            .bind(&importer_client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(handle, &importer_client.qh, ());
        imported.set_parent_of(&child_surface);
        child_surface.commit();
        importer_client.roundtrip();

        let both = session.wait_for(|snapshot| {
            snapshot.titled("exported parent").is_some() && snapshot.titled("imported child").is_some()
        });
        let parent_id = both.titled("exported parent").expect("the parent window").id;
        session.wait_for(|snapshot| {
            snapshot
                .titled("imported child")
                .is_some_and(|window| window.transient_for == Some(parent_id))
        });

        // Destroying the export revokes every relation made through its handle, so the
        // child is left without a parent.
        exported.destroy();
        exporter_client.roundtrip();
        session.wait_for(|snapshot| {
            snapshot
                .titled("imported child")
                .is_some_and(|window| window.transient_for.is_none())
        });
        imported.destroy();
        importer_client.roundtrip();
    });
}

#[test]
fn an_exported_handle_is_128_bits_of_lowercase_hex() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, _, _toplevel) = client.titled_toplevel("handle shape");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        // The handle is what an importer must present, so it is unguessable: 128 random bits
        // rendered as 32 lowercase hex digits, and nothing else.
        let _exported = exporter.export_toplevel(&surface, &client.qh, ());
        client.roundtrip();
        let handle = client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");
        assert_eq!(handle.len(), 32, "the handle is 32 hex digits: {handle}");
        assert!(
            handle
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "the handle is lowercase hex: {handle}"
        );
    });
}

#[test]
fn a_handle_that_was_never_minted_imports_nothing() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (_surface, _, _toplevel) = client.toplevel();
        let importer: zxdg_importer_v2::ZxdgImporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        // No exporter minted this handle, so Smithay tells the new import it is destroyed
        // and records no relation.
        let imported = importer.import_toplevel("never-minted".to_owned(), &client.qh, ());
        client.roundtrip();
        assert_eq!(
            client.state.imported_destroyed, 1,
            "an unknown handle is destroyed at once"
        );
        assert!(
            client.roundtrip_protocol_error().is_none(),
            "an unknown handle is not a client error"
        );
        imported.destroy();
        client.roundtrip();

        let mut fresh = Client::connect(session.socket());
        fresh.roundtrip();
    });
}

#[test]
fn an_exported_surface_that_is_not_a_toplevel_is_refused() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let surface = client.compositor().create_surface(&client.qh, ());
        surface.commit();
        client.roundtrip();
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        // A bare wl_surface is not a toplevel: the export is refused with
        // `zxdg_exporter_v2.invalid_surface`, which closes this connection.
        let _exported = exporter.export_toplevel(&surface, &client.qh, ());
        let (interface, code) = client
            .roundtrip_protocol_error()
            .expect("the server closes a connection that exports a non-toplevel");
        assert_eq!(interface, "zxdg_exporter_v2", "the error names the exporter");
        assert_eq!(
            code,
            zxdg_exporter_v2::Error::InvalidSurface as u32,
            "the error is invalid_surface"
        );
        assert!(
            client.state.exported_handles.is_empty(),
            "nothing was exported, so no handle was named"
        );

        // The refused client's connection is closed; a fresh client still gets the session.
        let mut fresh = Client::connect(session.socket());
        fresh.roundtrip();
    });
}

/// Every export names its toplevel with its own handle, across clients as well as within one.
#[test]
fn every_export_gets_a_distinct_handle() {
    Session::start().run(|session| {
        let mut exporter_client = Client::connect(session.socket());
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = exporter_client
            .globals
            .bind(&exporter_client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let (first_surface, _, _first) = exporter_client.titled_toplevel("first export");
        let (second_surface, _, _second) = exporter_client.titled_toplevel("second export");
        let _first_export = exporter.export_toplevel(&first_surface, &exporter_client.qh, ());
        let _second_export = exporter.export_toplevel(&second_surface, &exporter_client.qh, ());
        exporter_client.roundtrip();

        let mut other = Client::connect(session.socket());
        let (other_surface, _, _other) = other.titled_toplevel("other client export");
        let other_exporter: zxdg_exporter_v2::ZxdgExporterV2 = other
            .globals
            .bind(&other.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _other_export = other_exporter.export_toplevel(&other_surface, &other.qh, ());
        other.roundtrip();

        let mut handles = exporter_client.state.exported_handles.clone();
        handles.extend(other.state.exported_handles.iter().cloned());
        assert_eq!(handles.len(), 3, "each export names its toplevel once: {handles:?}");
        handles.sort();
        handles.dedup();
        assert_eq!(handles.len(), 3, "no two exports share a handle");
    });
}

/// Destroying the exported surface revokes its handle, so an importer is told `destroyed` and
/// the window it parented loses that parent.
#[test]
fn destroying_the_exported_surface_revokes_its_handle_and_tells_importers() {
    Session::start().run(|session| {
        let mut exporter_client = Client::connect(session.socket());
        let (parent_surface, parent_xdg, parent_toplevel) = exporter_client.titled_toplevel("destroyed parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = exporter_client
            .globals
            .bind(&exporter_client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _exported = exporter.export_toplevel(&parent_surface, &exporter_client.qh, ());
        exporter_client.roundtrip();
        let handle = exporter_client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");

        let mut importer_client = Client::connect(session.socket());
        let (child_surface, _, _child) = importer_client.titled_toplevel("destroyed child");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = importer_client
            .globals
            .bind(&importer_client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(handle, &importer_client.qh, ());
        imported.set_parent_of(&child_surface);
        child_surface.commit();
        importer_client.roundtrip();
        let parent_id = session
            .wait_for(|snapshot| snapshot.titled("destroyed parent").is_some())
            .titled("destroyed parent")
            .expect("the parent window")
            .id;
        session.wait_for(|snapshot| {
            snapshot
                .titled("destroyed child")
                .is_some_and(|window| window.transient_for == Some(parent_id))
        });

        // The toplevel role goes before its xdg_surface, and the xdg_surface before the wl_surface.
        parent_toplevel.destroy();
        parent_xdg.destroy();
        parent_surface.destroy();
        exporter_client.roundtrip();
        importer_client.wait_until(|app| app.imported_destroyed == 1);
        session.wait_for(|snapshot| {
            snapshot
                .titled("destroyed child")
                .is_some_and(|window| window.transient_for.is_none())
        });
        imported.destroy();
        importer_client.roundtrip();
    });
}

/// A handle whose export was destroyed is answered `destroyed` at once, and parenting through
/// that import is accepted without a protocol error.
#[test]
fn importing_a_destroyed_export_is_answered_destroyed() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, _, _toplevel) = client.titled_toplevel("revoked parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let exported = exporter.export_toplevel(&surface, &client.qh, ());
        client.roundtrip();
        let handle = client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");
        exported.destroy();
        client.roundtrip();

        let (child_surface, _, _child) = client.titled_toplevel("revoked child");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(handle, &client.qh, ());
        imported.set_parent_of(&child_surface);
        child_surface.commit();
        client.roundtrip();
        assert_eq!(
            client.state.imported_destroyed, 1,
            "a revoked handle is destroyed at once"
        );
        assert!(
            client.roundtrip_protocol_error().is_none(),
            "parenting through a destroyed import is not a client error"
        );
        imported.destroy();
        client.roundtrip();
    });
}

/// Setting the parent of a surface twice through one import keeps a single relation.
#[test]
fn set_parent_of_twice_keeps_one_parent() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (parent_surface, _, _parent) = client.titled_toplevel("twice parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _exported = exporter.export_toplevel(&parent_surface, &client.qh, ());
        client.roundtrip();
        let handle = client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");
        let (child_surface, _, _child) = client.titled_toplevel("twice child");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(handle, &client.qh, ());
        imported.set_parent_of(&child_surface);
        imported.set_parent_of(&child_surface);
        child_surface.commit();
        client.roundtrip();
        assert!(
            client.roundtrip_protocol_error().is_none(),
            "setting the same parent twice is not a client error"
        );

        let parent_id = session
            .wait_for(|snapshot| snapshot.titled("twice parent").is_some())
            .titled("twice parent")
            .expect("the parent window")
            .id;
        session.wait_for(|snapshot| {
            snapshot
                .titled("twice child")
                .is_some_and(|window| window.transient_for == Some(parent_id))
        });
        imported.destroy();
        client.roundtrip();
    });
}

/// A surface cannot be made its own parent: the import refuses it with `invalid_surface`.
#[test]
fn a_surface_cannot_be_its_own_parent() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, _, _toplevel) = client.titled_toplevel("self parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _exported = exporter.export_toplevel(&surface, &client.qh, ());
        client.roundtrip();
        let handle = client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(handle, &client.qh, ());
        imported.set_parent_of(&surface);
        surface.commit();
        let (interface, code) = client
            .roundtrip_protocol_error()
            .expect("the server closes a connection that parents a surface to itself");
        assert_eq!(interface, "zxdg_imported_v2", "the error names the importer");
        assert_eq!(
            code,
            zxdg_imported_v2::Error::InvalidSurface as u32,
            "the error is invalid_surface"
        );

        let mut fresh = Client::connect(session.socket());
        fresh.roundtrip();
    });
}

/// A surface cannot be parented to one of its own descendants: the second import would make
/// the chain a loop, so it is refused with `invalid_surface`.
#[test]
fn a_surface_cannot_be_parented_to_its_own_descendant() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (parent_surface, _, _parent) = client.titled_toplevel("cycle parent");
        let (child_surface, _, _child) = client.titled_toplevel("cycle child");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _child_export = exporter.export_toplevel(&child_surface, &client.qh, ());
        client.roundtrip();
        let child_handle = client.state.exported_handles[0].clone();
        let importer: zxdg_importer_v2::ZxdgImporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        // The parent becomes a child of the child: the child is now the parent's ancestor.
        let child_import = importer.import_toplevel(child_handle, &client.qh, ());
        child_import.set_parent_of(&parent_surface);
        parent_surface.commit();
        client.roundtrip();
        assert!(
            client.roundtrip_protocol_error().is_none(),
            "the first parenting is valid"
        );

        let _parent_export = exporter.export_toplevel(&parent_surface, &client.qh, ());
        client.roundtrip();
        let parent_handle = client.state.exported_handles[1].clone();
        let parent_import = importer.import_toplevel(parent_handle, &client.qh, ());
        // Making the child a child of the parent closes the loop: child -> parent -> child.
        parent_import.set_parent_of(&child_surface);
        child_surface.commit();
        let (interface, code) = client
            .roundtrip_protocol_error()
            .expect("the server closes a connection that makes a loop");
        assert_eq!(interface, "zxdg_imported_v2", "the error names the importer");
        assert_eq!(
            code,
            zxdg_imported_v2::Error::InvalidSurface as u32,
            "the error is invalid_surface"
        );

        let mut fresh = Client::connect(session.socket());
        fresh.roundtrip();
    });
}

/// A client that did not export a handle learns none, and a handle it guesses is refused: the
/// only way to a handle is the exporter's own `handle` event.
#[test]
fn a_client_cannot_learn_another_clients_handle() {
    Session::start().run(|session| {
        let mut exporter_client = Client::connect(session.socket());
        let (surface, _, _toplevel) = exporter_client.titled_toplevel("private export");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = exporter_client
            .globals
            .bind(&exporter_client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _exported = exporter.export_toplevel(&surface, &exporter_client.qh, ());
        exporter_client.roundtrip();
        let handle = exporter_client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");

        let mut stranger = Client::connect(session.socket());
        let (child_surface, _, _child) = stranger.titled_toplevel("stranger child");
        assert!(
            stranger.state.exported_handles.is_empty(),
            "a client that exported nothing receives no handle events"
        );
        let guess = "0".repeat(32);
        assert_ne!(guess, handle, "the guess is not the minted handle");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = stranger
            .globals
            .bind(&stranger.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(guess, &stranger.qh, ());
        imported.set_parent_of(&child_surface);
        stranger.roundtrip();
        assert_eq!(
            stranger.state.imported_destroyed, 1,
            "a guessed handle is destroyed at once"
        );
        assert!(
            stranger.roundtrip_protocol_error().is_none(),
            "a guessed handle is not a client error"
        );
        imported.destroy();
        stranger.roundtrip();
    });
}

/// The window manager floats an imported modal dialog above its parent: the parent link and the
/// modal kind both reach the window manager through `WindowRoleChanged`.
#[test]
fn an_imported_modal_dialog_floats_above_its_parent() {
    Session::start_allowing_focus().run(|session| {
        let mut client = Client::connect(session.socket());
        let (parent_surface, _, _parent) = client.mapped_titled_toplevel("floated parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _exported = exporter.export_toplevel(&parent_surface, &client.qh, ());
        client.roundtrip();
        let handle = client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");

        let (child_surface, _, child) = client.mapped_titled_toplevel("floated dialog");
        let importer: zxdg_importer_v2::ZxdgImporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_importer_v2");
        let imported = importer.import_toplevel(handle, &client.qh, ());
        imported.set_parent_of(&child_surface);
        let manager: xdg_wm_dialog_v1::XdgWmDialogV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises xdg_wm_dialog_v1");
        let dialog: xdg_dialog_v1::XdgDialogV1 = manager.get_xdg_dialog(&child, &client.qh, ());
        dialog.set_modal();
        child_surface.commit();
        client.roundtrip();

        let parent_id = session
            .wait_for(|snapshot| snapshot.titled("floated parent").is_some())
            .titled("floated parent")
            .expect("the parent window")
            .id;
        let floated = session.wait_for(|snapshot| snapshot.titled("floated dialog").is_some_and(|window| window.modal));
        let dialog_window = floated.titled("floated dialog").expect("the dialog window");
        assert_eq!(
            dialog_window.transient_for,
            Some(parent_id),
            "the imported dialog is a transient of its parent"
        );
        dialog.destroy();
        imported.destroy();
        client.roundtrip();
    });
}

#[test]
fn a_surface_has_one_alpha_modifier() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let surface = client.compositor().create_surface(&client.qh, ());
        surface.commit();
        client.roundtrip();
        let manager: wp_alpha_modifier_v1::WpAlphaModifierV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises wp_alpha_modifier_v1");
        // The first modifier is the surface's; a second one for the same surface is refused
        // with `wp_alpha_modifier_v1.already_constructed`, which closes this connection.
        let _first = manager.get_surface(&surface, &client.qh, ());
        client.roundtrip();
        let _second = manager.get_surface(&surface, &client.qh, ());
        let (interface, code) = client
            .roundtrip_protocol_error()
            .expect("the server closes a connection that builds a second modifier");
        assert_eq!(interface, "wp_alpha_modifier_v1", "the error names the manager");
        assert_eq!(
            code,
            wp_alpha_modifier_v1::Error::AlreadyConstructed as u32,
            "the error is already_constructed"
        );

        // The refused client's connection is closed; a fresh client still gets the session.
        let mut fresh = Client::connect(session.socket());
        fresh.roundtrip();
    });
}

/// The compositor's `handle -> window` lookup: an exported handle names the exporter's window
/// until its surface is destroyed, after which the handle resolves to nothing. An unknown
/// handle resolves to nothing too.
#[test]
fn an_exported_handle_resolves_to_its_window_until_revoked() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let (surface, xdg, toplevel) = client.titled_toplevel("looked up parent");
        let exporter: zxdg_exporter_v2::ZxdgExporterV2 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zxdg_exporter_v2");
        let _exported = exporter.export_toplevel(&surface, &client.qh, ());
        client.roundtrip();
        let handle = client
            .state
            .exported_handles
            .first()
            .cloned()
            .expect("the export names the toplevel with a handle");
        let window = session
            .wait_for(|snapshot| snapshot.titled("looked up parent").is_some())
            .titled("looked up parent")
            .expect("the parent window")
            .id;

        assert_eq!(
            session.control.exported_window(&handle),
            Some(Some(window)),
            "the exported handle names the exporter's window"
        );
        assert_eq!(
            session.control.exported_window("00000000000000000000000000000000"),
            Some(None),
            "a handle never minted names no window"
        );

        // Destroying the surface revokes the handle: the window is gone from the lookup.
        toplevel.destroy();
        xdg.destroy();
        surface.destroy();
        client.roundtrip();
        assert_eq!(
            session.control.exported_window(&handle),
            Some(None),
            "a revoked handle names no window"
        );
    });
}
