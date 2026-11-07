//! A real `wayland-client` speaking the vendored XML against a real display.
//!
//! The client half is generated from `protocols/mutter-x11-interop.xml`, the same file the
//! server half is generated from. The server half is this crate's own dispatch, with the
//! compositor replaced by [`Host`]: a real `wl_surface`, a real display, and the same
//! [`handle_request`] a session runs.
//!
//! The two failure modes the protocol has are reachable from a socket, and both are tested
//! here: an X11 window this session never managed is not a parent, and a relation is
//! recorded for a child the compositor knows.
//!
//! ## Why the compositor is a stand-in
//!
//! A full session needs a backend, and a compat crate may not link the engine
//! (`design/rust-compositor.md`, "Dependency rules"). What only a session can answer —
//! which windows exist on both sides of the X11 boundary — arrives through
//! [`gnoblin_wayland::X11InteropHost`], and the real implementation of that (the
//! compositor's window table and `gnoblin-xwayland`'s) is tested where it lives.

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use gnoblin_compat_core::{CallerContext, Gate, LockState, Mode};
use gnoblin_compat_wayland_mutter_x11_interop::global::{InteropData, handle_request};
use gnoblin_compat_wayland_mutter_x11_interop::{Admission, ParentRelations};
use gnoblin_types::WindowId;
use gnoblin_wayland::{Dispatch2, GlobalDispatch2, Surface, X11InteropHost, delegate_dispatch2};
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
        wayland_scanner::generate_interfaces!("protocols/mutter-x11-interop.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/mutter-x11-interop.xml");
}

/// The compositor, as far as this adapter can tell.
#[derive(Debug, Default)]
struct Host {
    /// Every window this host calls managed, by its `wl_surface` protocol id.
    windows: BTreeMap<u64, WindowId>,
    /// Every X11 window this host calls managed.
    x11: BTreeMap<u32, WindowId>,
}

impl X11InteropHost for Host {
    fn surface(&self, surface: &WlSurface) -> Surface {
        match self.windows.get(&u64::from(surface.id().protocol_id())) {
            Some(window) => Surface::Window(*window),
            None => Surface::Gone,
        }
    }

    fn window_for_x11(&self, xwindow: u32) -> Option<WindowId> {
        self.x11.get(&xwindow).copied()
    }
}

impl Dispatch2<server::MutterX11Interop, Host> for InteropData {
    fn request(
        &self,
        state: &mut Host,
        _client: &Client,
        _interop: &server::MutterX11Interop,
        request: server::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Host>,
    ) {
        handle_request(state, self, request);
    }
}

impl GlobalDispatch2<server::MutterX11Interop, Host> for InteropData {
    fn bind(
        &self,
        _state: &mut Host,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<server::MutterX11Interop>,
        data_init: &mut DataInit<'_, Host>,
    ) {
        let data = InteropData::with_relations(Arc::clone(self.admission()), self.relations());
        data_init.init(resource, data);
    }
}

/// The server-side protocol names, which is what a `Dispatch2` implementation names.
use gnoblin_compat_wayland_mutter_x11_interop::names::mutter_x11_interop as server;

/// The smallest compositor that can create a surface, so a real client can name a
/// `wl_surface` in `set_x11_parent`.
struct Compositor;

impl GlobalDispatch2<wl_compositor::WlCompositor, Host> for Compositor {
    fn bind(
        &self,
        _state: &mut Host,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<wl_compositor::WlCompositor>,
        data_init: &mut DataInit<'_, Host>,
    ) {
        data_init.init(resource, CompositorObject);
    }
}

/// One bound compositor object.
struct CompositorObject;

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
            let id = u64::from(surface.id().protocol_id());
            state.windows.insert(id, WindowId::new(100 + id));
        }
    }
}

impl Dispatch2<wl_surface::WlSurface, Host> for () {
    fn request(
        &self,
        _state: &mut Host,
        _client: &Client,
        _surface: &WlSurface,
        _request: wl_surface::Request,
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

/// A session with the redirect registered and a client connected to it.
fn session(mode: Mode, x11: BTreeMap<u32, WindowId>) -> (Display<Host>, Host, UnixStream, Arc<Mutex<ParentRelations>>) {
    let display: Display<Host> = Display::new().expect("a display");
    let mut handle = display.handle();
    let gate = Gate::new(mode, CallerContext::UNSANDBOXED, LockState::Unlocked);
    let admission = Admission::load(gate).expect("the checked-in manifest");
    let relations = Arc::new(Mutex::new(ParentRelations::new()));
    // The global's data is this test's own, so the table it records into is the one the
    // test reads.
    let data = InteropData::with_relations(Arc::new(admission), Arc::clone(&relations));
    handle.create_global::<Host, server::MutterX11Interop, InteropData>(1, data);
    handle.create_global::<Host, wl_compositor::WlCompositor, Compositor>(6, Compositor);

    let (server, client) = UnixStream::pair().expect("a socket pair");
    handle
        .insert_client(server, Arc::new(NoClientData))
        .expect("the client is inserted");
    let host = Host {
        windows: BTreeMap::new(),
        x11,
    };
    (display, host, client, relations)
}

/// The client's own dispatch state, which records nothing: this protocol has no events.
#[derive(Default)]
struct ClientState;

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

impl wayland_client::Dispatch<client::mutter_x11_interop::MutterX11Interop, ()> for ClientState {
    fn event(
        _state: &mut Self,
        _proxy: &client::mutter_x11_interop::MutterX11Interop,
        event: client::mutter_x11_interop::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()> for ClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_surface::WlSurface,
        _event: wayland_client::protocol::wl_surface::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_compositor::WlCompositor, ()> for ClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_compositor::WlCompositor,
        _event: wayland_client::protocol::wl_compositor::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

/// A managed X11 window is a parent, and the relation names the window the compositor
/// resolved it to.
#[test]
fn a_managed_x11_window_becomes_the_parent() {
    let mut x11 = BTreeMap::new();
    x11.insert(0x40_0003, WindowId::new(7));
    let (mut display, mut host, client, _relations) = session(Mode::Unmediated, x11);

    let worker = std::thread::spawn(move || {
        use wayland_client::globals::registry_queue_init;

        let connection = wayland_client::Connection::from_socket(client).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<ClientState>(&connection).expect("the registry");
        let qh = queue.handle();
        let mut state = ClientState;
        let compositor: wayland_client::protocol::wl_compositor::WlCompositor =
            globals.bind(&qh, 4..=6, ()).expect("a compositor");
        let surface = compositor.create_surface(&qh, ());
        let interop: client::mutter_x11_interop::MutterX11Interop =
            globals.bind(&qh, 1..=1, ()).expect("the redirect is advertised");
        interop.set_x11_parent(&surface, 0x40_0003);
        queue.roundtrip(&mut state).expect("a round trip");
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !worker.is_finished() {
        display.dispatch_clients(&mut host).expect("a dispatch");
        display.flush_clients().expect("a flush");
        assert!(std::time::Instant::now() < deadline, "the client never finished");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    worker.join().expect("the client finished");
    for _ in 0..10 {
        display.dispatch_clients(&mut host).expect("a dispatch");
        display.flush_clients().expect("a flush");
    }

    let relations = _relations.lock().expect("the relations");
    assert_eq!(relations.len(), 1, "the relation was recorded: {relations:?}");
    assert_eq!(
        relations.iter().next().map(|(_, parent)| parent),
        Some(WindowId::new(7)),
        "the parent is the window the X11 id resolved to"
    );
}

/// An X11 window this session never managed is not a parent, and the request records
/// nothing.
#[test]
fn an_unmanaged_x11_window_is_not_a_parent() {
    let (mut display, mut host, client, relations) = session(Mode::Unmediated, BTreeMap::new());

    let worker = std::thread::spawn(move || {
        use wayland_client::globals::registry_queue_init;

        let connection = wayland_client::Connection::from_socket(client).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<ClientState>(&connection).expect("the registry");
        let qh = queue.handle();
        let mut state = ClientState;
        let compositor: wayland_client::protocol::wl_compositor::WlCompositor =
            globals.bind(&qh, 4..=6, ()).expect("a compositor");
        let surface = compositor.create_surface(&qh, ());
        let interop: client::mutter_x11_interop::MutterX11Interop =
            globals.bind(&qh, 1..=1, ()).expect("the redirect is advertised");
        interop.set_x11_parent(&surface, 0xde_ad);
        queue.roundtrip(&mut state).expect("a round trip");
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !worker.is_finished() {
        display.dispatch_clients(&mut host).expect("a dispatch");
        display.flush_clients().expect("a flush");
        assert!(std::time::Instant::now() < deadline, "the client never finished");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    worker.join().expect("the client finished");
    for _ in 0..10 {
        display.dispatch_clients(&mut host).expect("a dispatch");
        display.flush_clients().expect("a flush");
    }
    assert!(
        relations.lock().expect("the relations").is_empty(),
        "an X11 window that is not managed is not a parent"
    );
}
