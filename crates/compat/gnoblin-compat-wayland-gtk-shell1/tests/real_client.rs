//! A real `wayland-client` speaking the vendored XML against a real display.
//!
//! The client half is generated from `protocols/gtk-shell.xml` — the same file the
//! server half is generated from — so a client and the adapter can only agree if they
//! agree about the protocol itself. The server half is this crate's own dispatch,
//! driven through [`handle_shell_request`], with the compositor replaced by
//! [`Host`]: a real `wl_surface`, a real seat-less display and the real code path a
//! GTK application takes.
//!
//! ## What is not here, and why
//!
//! A full session needs a backend and a seat, and a compat crate may not link the
//! engine (`design/rust-compositor.md`, "Dependency rules": a compat crate depends on
//! its own transport and not on the crates that make a session run). So the compositor
//! side is the smallest display that can hold the global: a hand-written `wl_compositor`
//! that creates surfaces, and a `Host` that answers the two host traits. Everything the
//! compositor would decide — activation, the window manager, the bell — is unit-tested
//! where that decision lives (`gnoblin-wayland`'s `protocols::extension` tests).
//!
//! ## The tests
//!
//! One per failure mode in the crate documentation that a socket can reach:
//!
//! - `a_real_client_binds_the_redirect_and_rings_the_bell` (mode 3, and the bind path);
//! - `a_real_client_sets_an_application_id` (the surface half end to end);
//! - `present_from_a_real_client_reaches_the_host` (mode 1's dispatch half);
//! - `an_off_adapter_answers_nothing` (mode 7).

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use gnoblin_compat_core::Mode;
use gnoblin_compat_wayland_gtk_shell1::global::{
    ShellData, ShellUserData, SurfaceData, handle_shell_request, handle_surface_request,
};
use gnoblin_compat_wayland_gtk_shell1::names::{gtk_shell1, gtk_surface1};
use gnoblin_compat_wayland_gtk_shell1::{Admission, Redirect, SHELL_VERSION};
use gnoblin_types::WindowId;
use gnoblin_wayland::{
    Activation, Dispatch2, GlobalDispatch2, Shell1Host, Surface, Titlebar, TitlebarActions, delegate_dispatch2,
};
use wayland_server::protocol::wl_compositor;
use wayland_server::protocol::wl_surface::{self, WlSurface};
use wayland_server::{Client, DataInit, Display, DisplayHandle, New, Resource};

/// The client half of the vendored protocol, generated from the same XML.
mod client {
    // The generated code names `wayland_client` from this module, so the crate is
    // imported here rather than used.
    #[allow(clippy::single_component_path_imports)]
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/gtk-shell.xml");
}

/// What the compositor side of this test saw.
#[derive(Clone, Debug, Default)]
struct Recorded {
    bells: Vec<Option<WindowId>>,
    launches: Vec<String>,
    startup_ids: Vec<String>,
    app_ids: Vec<String>,
    presents: Vec<WindowId>,
}

/// The compositor, as far as the adapter can tell.
#[derive(Debug, Default)]
struct Host {
    recorded: Arc<Mutex<Recorded>>,
    actions: TitlebarActions,
    /// Every window this host calls managed.
    windows: BTreeMap<u64, WindowId>,
}

impl Host {
    fn record(&self) -> std::sync::MutexGuard<'_, Recorded> {
        self.recorded.lock().expect("the record lock")
    }
}

impl Shell1Host for Host {
    fn surface(&self, surface: &WlSurface) -> Surface {
        match self.windows.get(&u64::from(surface.id().protocol_id())) {
            Some(window) => Surface::Window(*window),
            None => Surface::Gone,
        }
    }

    fn titlebar_actions(&self) -> TitlebarActions {
        self.actions
    }

    fn set_app_id(&mut self, surface: &WlSurface, app_id: &str) -> Surface {
        let Surface::Window(window) = self.surface(surface) else {
            return Surface::Gone;
        };
        self.record().app_ids.push(app_id.to_owned());
        Surface::Window(window)
    }

    fn titlebar(&mut self, _window: WindowId, _action: gnoblin_config::TitlebarAction) -> Titlebar {
        Titlebar::Performed
    }

    fn present(&mut self, window: WindowId, _time: u32) -> Activation {
        self.record().presents.push(window);
        Activation::Focused
    }

    fn request_focus(&mut self, _window: WindowId, startup_id: Option<&str>) -> Activation {
        if let Some(startup_id) = startup_id {
            self.record().startup_ids.push(startup_id.to_owned());
        }
        Activation::Focused
    }

    fn bell(&mut self, window: Option<WindowId>) {
        self.record().bells.push(window);
    }

    fn launch(&mut self, startup_id: &str) {
        self.record().launches.push(startup_id.to_owned());
    }
}

/// The dispatch the adapter's production impl uses, for this test's state.
impl Dispatch2<gtk_shell1::GtkShell1, Host> for ShellUserData {
    fn request(
        &self,
        state: &mut Host,
        _client: &Client,
        _shell: &gtk_shell1::GtkShell1,
        request: gtk_shell1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Host>,
    ) {
        handle_shell_request(state, self.admission(), request, data_init);
    }
}

impl Dispatch2<gtk_surface1::GtkSurface1, Host> for SurfaceData {
    fn request(
        &self,
        state: &mut Host,
        _client: &Client,
        resource: &gtk_surface1::GtkSurface1,
        request: gtk_surface1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Host>,
    ) {
        handle_surface_request(state, self, resource, request);
    }
}

impl GlobalDispatch2<gtk_shell1::GtkShell1, Host> for ShellData {
    fn bind(
        &self,
        state: &mut Host,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<gtk_shell1::GtkShell1>,
        data_init: &mut DataInit<'_, Host>,
    ) {
        let admission = Arc::clone(self.admission());
        let shell = data_init.init(resource, ShellUserData::new(admission));
        shell.capabilities(0);
        // The adapter's real bind is the session's; this one records the surface the
        // client creates so `surface()` can answer about it.
        let _ = state;
    }
}

/// The smallest compositor that can create a surface, so a real client can name a
/// `wl_surface` in `get_gtk_surface`.
struct Compositor(u32);

/// One bound compositor object, so a surface can be created through it.
///
/// The number is the base of the window identities this compositor hands out, so two
/// surfaces of one test are two windows.
struct CompositorObject(u32);

impl GlobalDispatch2<wl_compositor::WlCompositor, Host> for Compositor {
    fn bind(
        &self,
        _state: &mut Host,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<wl_compositor::WlCompositor>,
        data_init: &mut DataInit<'_, Host>,
    ) {
        data_init.init(resource, CompositorObject(self.0));
    }
}

impl Dispatch2<wl_compositor::WlCompositor, Host> for CompositorObject {
    fn request(
        &self,
        state: &mut Host,
        _client: &Client,
        _compositor: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Host>,
    ) {
        if let wl_compositor::Request::CreateSurface { id } = request {
            let surface = data_init.init(id, ());
            let surface_id = u64::from(surface.id().protocol_id());
            state
                .windows
                .insert(surface_id, WindowId::new(u64::from(self.0) + surface_id));
        }
    }
}

impl Dispatch2<wl_surface::WlSurface, Host> for () {
    fn request(
        &self,
        _state: &mut Host,
        _client: &Client,
        _surface: &WlSurface,
        _request: wayland_server::protocol::wl_surface::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Host>,
    ) {
    }
}

delegate_dispatch2!(Host);

/// No per-client state: this display serves one test client at a time.
#[derive(Debug)]
struct NoClientData;

impl wayland_server::backend::ClientData for NoClientData {}

/// A display with the redirect registered, and a client connected to it.
fn session(mode: Mode) -> (Display<Host>, Host, UnixStream, Arc<Mutex<Recorded>>) {
    let display: Display<Host> = Display::new().expect("a display");
    let mut handle = display.handle();
    let admission = Admission::load(mode).expect("the checked-in manifest");
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let host = Host {
        recorded: Arc::clone(&recorded),
        actions: TitlebarActions::default(),
        windows: BTreeMap::new(),
    };

    // The redirect's own registration path, for this test's state type: a change to it
    // is a test failure here, and the session's own `Extension::register` calls the same
    // method with `gnoblin_wayland::State`.
    Redirect::new(admission)
        .install::<Host>(&handle)
        .expect("the redirect registers");
    handle.create_global::<Host, wl_compositor::WlCompositor, Compositor>(6, Compositor(1_000));

    let (server, client) = UnixStream::pair().expect("a socket pair");
    handle
        .insert_client(server, Arc::new(NoClientData))
        .expect("the client is inserted");
    (display, host, client, recorded)
}

/// Dispatch until `done` says so, flushing every turn.
fn serve(display: &mut Display<Host>, state: &mut Host, done: impl Fn(&Host, &Recorded) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        display.dispatch_clients(state).expect("a dispatch");
        display.flush_clients().expect("a flush");
        let recorded = state.recorded.lock().expect("the record lock").clone();
        if done(state, &recorded) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the session never saw the request"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The client's own dispatch state: it records the capabilities event.
#[derive(Default)]
struct ClientState {
    capabilities: Option<u32>,
}

impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for ClientState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_registry::WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &wayland_client::globals::GlobalListContents,
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<client::gtk_shell1::GtkShell1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _proxy: &client::gtk_shell1::GtkShell1,
        event: client::gtk_shell1::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let client::gtk_shell1::Event::Capabilities { capabilities } = event;
        state.capabilities = Some(capabilities);
    }
}

impl wayland_client::Dispatch<client::gtk_surface1::GtkSurface1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _proxy: &client::gtk_surface1::GtkSurface1,
        _event: client::gtk_surface1::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let _ = state;
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()> for ClientState {
    fn event(
        state: &mut Self,
        _proxy: &wayland_client::protocol::wl_surface::WlSurface,
        _event: wayland_client::protocol::wl_surface::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let _ = state;
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_compositor::WlCompositor, ()> for ClientState {
    fn event(
        state: &mut Self,
        _proxy: &wayland_client::protocol::wl_compositor::WlCompositor,
        _event: wayland_client::protocol::wl_compositor::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let _ = state;
    }
}

/// The bind path and the bell: a real client binds the global, hears the capabilities
/// event, names a launch and rings the bell.
#[test]
fn a_real_client_binds_the_redirect_and_rings_the_bell() {
    let (mut display, mut host, client, recorded) = session(Mode::Portal);
    let worker = std::thread::spawn(move || {
        use wayland_client::globals::registry_queue_init;

        let connection = wayland_client::Connection::from_socket(client).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<ClientState>(&connection).expect("the registry");
        let qh = queue.handle();
        let mut state = ClientState::default();
        let shell: client::gtk_shell1::GtkShell1 = globals
            .bind(&qh, 3..=SHELL_VERSION, ())
            .expect("the redirect is advertised");
        shell.set_startup_id(None);
        shell.notify_launch("gnoblin-launch".to_owned());
        shell.system_bell(None);
        queue.roundtrip(&mut state).expect("a round trip");
        assert_eq!(state.capabilities, Some(0), "no global app menu is claimed");
    });

    serve(&mut display, &mut host, |_, recorded| recorded.bells.len() == 1);
    worker.join().expect("the client finished");
    let recorded = recorded.lock().expect("the record lock");
    assert_eq!(
        recorded.bells,
        vec![None],
        "a bell with no surface names no window, and is not invented"
    );
    assert_eq!(recorded.launches, vec!["gnoblin-launch".to_owned()]);
}

/// The surface half end to end: a real client creates a `wl_surface`, asks for a
/// `gtk_surface1`, sets its application id and presents it.
#[test]
fn a_real_client_sets_an_application_id_and_presents() {
    let (mut display, mut host, client, recorded) = session(Mode::Portal);
    let worker = std::thread::spawn(move || {
        use wayland_client::globals::registry_queue_init;

        let connection = wayland_client::Connection::from_socket(client).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<ClientState>(&connection).expect("the registry");
        let qh = queue.handle();
        let mut state = ClientState::default();
        let compositor: wayland_client::protocol::wl_compositor::WlCompositor =
            globals.bind(&qh, 4..=6, ()).expect("a compositor");
        let surface = compositor.create_surface(&qh, ());
        let shell: client::gtk_shell1::GtkShell1 = globals
            .bind(&qh, 3..=SHELL_VERSION, ())
            .expect("the redirect is advertised");
        let gtk_surface = shell.get_gtk_surface(&surface, &qh, ());
        gtk_surface.set_dbus_properties(Some("org.example.Editor".to_owned()), None, None, None, None, None);
        gtk_surface.present(0);
        queue.roundtrip(&mut state).expect("a round trip");
    });

    serve(&mut display, &mut host, |_, recorded| {
        !recorded.app_ids.is_empty() && !recorded.presents.is_empty()
    });
    worker.join().expect("the client finished");
    let recorded = recorded.lock().expect("the record lock");
    assert_eq!(recorded.app_ids, vec!["org.example.Editor".to_owned()]);
    assert_eq!(recorded.presents.len(), 1, "the present reached the host exactly once");
}

/// An adapter the configuration turned off answers nothing, on a real socket.
#[test]
fn an_off_adapter_answers_nothing() {
    let (mut display, mut host, client, recorded) = session(Mode::Off);
    let worker = std::thread::spawn(move || {
        use wayland_client::globals::registry_queue_init;

        let connection = wayland_client::Connection::from_socket(client).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<ClientState>(&connection).expect("the registry");
        let qh = queue.handle();
        let mut state = ClientState::default();
        let compositor: wayland_client::protocol::wl_compositor::WlCompositor =
            globals.bind(&qh, 4..=6, ()).expect("a compositor");
        let surface = compositor.create_surface(&qh, ());
        let shell: client::gtk_shell1::GtkShell1 = globals
            .bind(&qh, 3..=SHELL_VERSION, ())
            .expect("the global exists even when the adapter is off");
        let gtk_surface = shell.get_gtk_surface(&surface, &qh, ());
        gtk_surface.set_dbus_properties(Some("org.example.Editor".to_owned()), None, None, None, None, None);
        gtk_surface.present(0);
        shell.system_bell(None);
        queue.roundtrip(&mut state).expect("a round trip");
    });
    // The client's own registry setup round-trips, so the server has to be dispatching
    // while it runs rather than after it: an off adapter still answers the transport,
    // and only the host calls are refused.
    while !worker.is_finished() {
        display.dispatch_clients(&mut host).expect("a dispatch");
        display.flush_clients().expect("a flush");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    worker.join().expect("the client finished");
    for _ in 0..10 {
        display.dispatch_clients(&mut host).expect("a dispatch");
        display.flush_clients().expect("a flush");
    }
    let recorded = recorded.lock().expect("the record lock");
    assert!(
        recorded.bells.is_empty() && recorded.app_ids.is_empty() && recorded.presents.is_empty(),
        "an adapter that is off performs no host call at all: {recorded:?}"
    );
}
