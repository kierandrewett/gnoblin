//! The harness against a real Wayland client: the descriptor tells the truth about the registry, a window is
//! placed where wlcs asks, and injected input reaches the client before the call returns.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd};
use std::time::{Duration, Instant};

use gnoblin_backend::TouchPhase;
use gnoblin_types::Rect;
use gnoblin_wlcs::{ConformanceSession, extensions};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const TIMEOUT: Duration = Duration::from_secs(10);
const BTN_LEFT: u32 = 0x110;

/// What the client saw: pointer enters and button presses, with surface-local coordinates, and touch downs.
#[derive(Default)]
struct App {
    pointer_enters: Vec<(f64, f64)>,
    buttons: Vec<u32>,
    touch_downs: Vec<(f64, f64)>,
    frames_done: u32,
}

impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.frames_done += 1;
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
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
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

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x, surface_y, ..
            } => state.pointer_enters.push((surface_x, surface_y)),
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => state.buttons.push(button),
            _ => {}
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_touch::Event::Down { x, y, .. } = event {
            state.touch_downs.push((x, y));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

delegate_noop!(App: wl_compositor::WlCompositor);
delegate_noop!(App: ignore wl_shm::WlShm);
delegate_noop!(App: wl_shm_pool::WlShmPool);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
delegate_noop!(App: ignore wl_surface::WlSurface);
delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);

/// A client of a harness with one mapped toplevel.
struct Mapped {
    queue: EventQueue<App>,
    app: App,
    surface: wl_surface::WlSurface,
    connection: Connection,
    fd: i32,
    // Keep the protocol objects alive for the window's lifetime.
    _keep: Vec<Box<dyn std::any::Any>>,
}

impl Mapped {
    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.app).expect("a round trip");
    }
}

/// Connects a client, maps a `width` x `height` toplevel and waits for the harness to report it mapped.
fn map_toplevel(harness: &ConformanceSession, width: i32, height: i32) -> Mapped {
    let stream = harness.client_stream().expect("a client connection");
    stream.set_read_timeout(Some(TIMEOUT)).expect("a read timeout");
    let fd = stream.as_raw_fd();
    let connection = Connection::from_socket(stream).expect("a client connection");
    let (globals, mut queue) = registry_queue_init::<App>(&connection).expect("the registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=5, ()).expect("xdg_wm_base");
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("wl_seat");
    let pointer = seat.get_pointer(&qh, ());
    let touch = seat.get_touch(&qh, ());

    let surface = compositor.create_surface(&qh, ());
    let xdg = base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    surface.commit();
    let mut app = App::default();
    queue.roundtrip(&mut app).expect("the first configure");

    let stride = width * 4;
    let length = stride * height;
    let mut file = tempfile::tempfile().expect("a shared memory file");
    file.write_all(&vec![0xff; usize::try_from(length).expect("a size")])
        .expect("the pixels");
    let pool = shm.create_pool(file.as_fd(), length, &qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &qh, ());
    surface.attach(Some(&buffer), 0, 0);
    surface.frame(&qh, ());
    surface.commit();
    queue.roundtrip(&mut app).expect("the mapping commit");

    let deadline = Instant::now() + TIMEOUT;
    while app.frames_done == 0 {
        assert!(
            Instant::now() < deadline,
            "the frame callback of the mapping commit never came"
        );
        queue.roundtrip(&mut app).expect("a round trip");
        std::thread::sleep(Duration::from_millis(5));
    }
    while !harness.snapshot().windows().any(|window| window.mapped) {
        assert!(Instant::now() < deadline, "the window never mapped");
        std::thread::sleep(Duration::from_millis(5));
    }
    Mapped {
        queue,
        app,
        surface,
        connection,
        fd,
        _keep: vec![
            Box::new(xdg),
            Box::new(toplevel),
            Box::new(pool),
            Box::new(buffer),
            Box::new(pointer),
            Box::new(touch),
            Box::new(seat),
            Box::new(base),
            Box::new(shm),
            Box::new(compositor),
            Box::new(file),
        ],
    }
}

fn geometry_of_only_window(harness: &ConformanceSession) -> Rect {
    harness.snapshot().windows().next().expect("one window").geometry
}

#[test]
fn the_descriptor_lists_what_a_client_sees_in_the_registry() {
    let harness = ConformanceSession::start().expect("the session starts");
    let stream = harness.client_stream().expect("a client connection");
    let connection = Connection::from_socket(stream).expect("a client connection");
    let (globals, _queue) = registry_queue_init::<App>(&connection).expect("the registry");
    let mut advertised = BTreeMap::new();
    globals.contents().with_list(|list| {
        for global in list {
            advertised.insert(global.interface.clone(), global.version);
        }
    });
    let described: BTreeMap<String, u32> = extensions()
        .into_iter()
        .map(|(name, version)| (name.to_owned(), version))
        .collect();
    assert_eq!(
        described, advertised,
        "the wlcs descriptor must list exactly the globals the session advertises"
    );
}

#[test]
fn a_window_is_placed_where_wlcs_asks() {
    let harness = ConformanceSession::start().expect("the session starts");
    let mapped = map_toplevel(&harness, 200, 100);
    let id = mapped.surface.id().protocol_id();
    let before = geometry_of_only_window(&harness);
    assert!(
        harness.position_window(mapped.fd, id, 300, 700),
        "an exact placement is accepted, even where the work area would clamp it; the window was {before:?}, now {:?}",
        geometry_of_only_window(&harness)
    );
    let placed = geometry_of_only_window(&harness);
    assert_eq!((placed.x, placed.y), (300, 700));
    assert_eq!(
        (placed.width, placed.height),
        (before.width, before.height),
        "placing keeps the size"
    );
    drop(mapped.connection);
}

#[test]
fn an_unknown_surface_is_not_placed() {
    let harness = ConformanceSession::start().expect("the session starts");
    let mapped = map_toplevel(&harness, 200, 100);
    let before = geometry_of_only_window(&harness);
    assert!(!harness.position_window(mapped.fd, 9999, 10, 10));
    assert!(!harness.position_window(-1, mapped.surface.id().protocol_id(), 10, 10));
    assert_eq!(geometry_of_only_window(&harness), before);
}

#[test]
fn injected_pointer_input_reaches_the_client_before_the_call_returns() {
    let harness = ConformanceSession::start().expect("the session starts");
    let mut mapped = map_toplevel(&harness, 200, 100);
    let id = mapped.surface.id().protocol_id();
    assert!(harness.position_window(mapped.fd, id, 100, 100));
    let injector = harness.injector();
    injector.pointer_move_to(150, 120);
    mapped.roundtrip();
    assert_eq!(mapped.app.pointer_enters.last(), Some(&(50.0, 20.0)));
    injector.pointer_button(BTN_LEFT, true);
    injector.pointer_button(BTN_LEFT, false);
    mapped.roundtrip();
    assert_eq!(mapped.app.buttons, vec![BTN_LEFT]);
}

#[test]
fn injected_touch_input_reaches_the_client_in_surface_coordinates() {
    let harness = ConformanceSession::start().expect("the session starts");
    let mut mapped = map_toplevel(&harness, 200, 100);
    let id = mapped.surface.id().protocol_id();
    assert!(harness.position_window(mapped.fd, id, 100, 100));
    let injector = harness.injector();
    let slot = harness.next_touch_slot();
    injector.touch(slot, 130, 110, TouchPhase::Down);
    mapped.roundtrip();
    injector.touch(slot, 130, 110, TouchPhase::Up);
    assert_eq!(mapped.app.touch_downs, vec![(30.0, 10.0)]);
}

#[test]
fn two_touch_devices_get_different_slots() {
    let harness = ConformanceSession::start().expect("the session starts");
    assert_ne!(harness.next_touch_slot(), harness.next_touch_slot());
}
