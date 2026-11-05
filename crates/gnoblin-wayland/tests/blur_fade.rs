//! `gnoblin_blur_fade_v1` against a real client.
//!
//! The scenarios are the C suite's (`tests/blur-fade-protocol.c` and
//! `test-blur-fade-protocol.sh`) and the rules in `src/protocols/blur_fade.rs`:
//! the record layout, the limits, the `invalid_fades` error and the double
//! buffering with `wl_surface.commit`.

mod owned;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_types::WindowId;
use owned::protocols::blur_fade::gnoblin_blur_fade_manager_v1;
use owned::{Connected, Handle, Session, TIMEOUT};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// The 24.8 fixed point scale the protocol uses.
const FIXED: i32 = 256;

#[derive(Default)]
struct App {}

impl Dispatch<gnoblin_blur_fade_manager_v1::GnoblinBlurFadeManagerV1, ()> for App {
    fn event(
        _state: &mut Self,
        _manager: &gnoblin_blur_fade_manager_v1::GnoblinBlurFadeManagerV1,
        _event: gnoblin_blur_fade_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _state: &mut Self,
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
        _state: &mut Self,
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

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

/// One record as the wire carries it.
fn record_bytes(record: [i32; 5]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in record {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    bytes
}

/// One connection with a mapped window and the fade manager bound.
struct Fader {
    client: Connected<App>,
    manager: gnoblin_blur_fade_manager_v1::GnoblinBlurFadeManagerV1,
    surface: wl_surface::WlSurface,
}

impl Fader {
    fn start(handle: &Handle) -> Self {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager: gnoblin_blur_fade_manager_v1::GnoblinBlurFadeManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("every client may describe its own surface");
        let wm_base: xdg_wm_base::XdgWmBase = client.globals.bind(&client.qh, 1..=5, ()).expect("xdg_wm_base");
        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &client.qh, ());
        let toplevel = xdg.get_toplevel(&client.qh, ());
        toplevel.set_title("faded".to_owned());
        toplevel.set_app_id("org.example.Faded".to_owned());
        surface.commit();
        client.roundtrip();
        let buffer = client.buffer(200, 150);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 200, 150);
        surface.commit();
        client.roundtrip();
        Self {
            client,
            manager,
            surface,
        }
    }

    /// Sends records and commits them, as a client does for the frame they belong
    /// to.
    fn set(&mut self, bytes: &[u8]) {
        self.manager.set_fades(&self.surface.clone(), bytes.to_vec());
        self.surface.commit();
        self.client.roundtrip();
    }

    /// Sends an array the server is expected to refuse, without a round trip: a
    /// round trip panics on the error the test is about to assert on.
    fn send_invalid(&mut self, bytes: &[u8]) {
        self.manager.set_fades(&self.surface.clone(), bytes.to_vec());
        self.surface.commit();
        self.client.connection.flush().expect("the request reaches the server");
    }
}

/// The records a window's surface committed.
fn committed(handle: &Handle, window: WindowId) -> Option<Vec<gnoblin_wayland::Fade>> {
    handle.control.blur_fades(window).flatten()
}

/// Waits for the committed records to satisfy `predicate`.
fn wait_fades(
    handle: &Handle,
    window: WindowId,
    what: &str,
    predicate: impl Fn(&[gnoblin_wayland::Fade]) -> bool,
) -> Vec<gnoblin_wayland::Fade> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(fades) = committed(handle, window)
            && predicate(&fades)
        {
            return fades;
        }
        assert!(Instant::now() < deadline, "the client never saw {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// The window a test's client mapped; identities are handed out in creation order.
fn window() -> WindowId {
    WindowId::new(1)
}

#[test]
fn fades_apply_with_the_commit_that_carries_them() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        assert!(committed(handle, window()).is_none(), "nothing committed yet");

        fader.set(&record_bytes([0, 0, 100 * FIXED, 100 * FIXED, 128]));
        let fades = wait_fades(handle, window(), "one record", |fades| fades.len() == 1);
        assert!((fades[0].x - 0.0).abs() < f32::EPSILON);
        assert!((fades[0].width - 100.0).abs() < f32::EPSILON);
        assert!((fades[0].height - 100.0).abs() < f32::EPSILON);
        assert!((fades[0].opacity - 0.5).abs() < 0.01);

        // Two records describe two items, in the order they were sent.
        let mut two = record_bytes([0, 0, 10 * FIXED, 10 * FIXED, FIXED]);
        two.extend_from_slice(&record_bytes([5, 5, 20 * FIXED, 20 * FIXED, 0]));
        fader.set(&two);
        let fades = wait_fades(handle, window(), "two records", |fades| fades.len() == 2);
        assert!((fades[1].opacity - 0.0).abs() < f32::EPSILON);
    });
}

#[test]
fn an_empty_array_removes_the_metadata() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        fader.set(&record_bytes([0, 0, 100 * FIXED, 100 * FIXED, FIXED]));
        wait_fades(handle, window(), "one record", |fades| fades.len() == 1);

        fader.set(&[]);
        let fades = wait_fades(handle, window(), "an empty set", |fades| fades.is_empty());
        assert!(fades.is_empty());
    });
}

#[test]
fn each_surface_has_its_own_fades() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut first = Fader::start(handle);
        first.set(&record_bytes([0, 0, 100 * FIXED, 100 * FIXED, FIXED]));
        wait_fades(handle, window(), "the first surface's records", |fades| {
            fades.len() == 1
        });

        // A second surface's metadata never answers for the first one.
        let mut second = Connected::connect(handle.socket(), App::default());
        let manager: gnoblin_blur_fade_manager_v1::GnoblinBlurFadeManagerV1 =
            second.globals.bind(&second.qh, 1..=1, ()).expect("blur fade manager");
        let surface = second.compositor.create_surface(&second.qh, ());
        manager.set_fades(&surface, record_bytes([0, 0, 10 * FIXED, 10 * FIXED, 0]));
        surface.commit();
        second.roundtrip();
        let fades = committed(handle, window()).expect("the first surface's records");
        assert_eq!(fades.len(), 1, "the second surface changed nothing");
        assert!((fades[0].opacity - 1.0).abs() < f32::EPSILON);
    });
}

#[test]
fn a_short_record_is_refused() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        fader.send_invalid(&[0, 0, 0, 0]);
        fader.client.expect_protocol_error("gnoblin_blur_fade_manager_v1", 0);
    });
}

#[test]
fn a_negative_size_is_refused() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        fader.send_invalid(&record_bytes([0, 0, -1, 100 * FIXED, FIXED]));
        fader.client.expect_protocol_error("gnoblin_blur_fade_manager_v1", 0);
    });
}

#[test]
fn an_out_of_range_coordinate_is_refused() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        fader.send_invalid(&record_bytes([i32::MAX, 0, 100 * FIXED, 100 * FIXED, FIXED]));
        fader.client.expect_protocol_error("gnoblin_blur_fade_manager_v1", 0);
    });
}

#[test]
fn an_opacity_above_one_is_refused() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        fader.send_invalid(&record_bytes([0, 0, 100 * FIXED, 100 * FIXED, FIXED + 1]));
        fader.client.expect_protocol_error("gnoblin_blur_fade_manager_v1", 0);
    });
}

#[test]
fn more_than_thirty_two_records_is_refused() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut fader = Fader::start(handle);
        let mut bytes = Vec::new();
        for _ in 0..33 {
            bytes.extend_from_slice(&record_bytes([0, 0, 10 * FIXED, 10 * FIXED, FIXED]));
        }
        fader.send_invalid(&bytes);
        fader.client.expect_protocol_error("gnoblin_blur_fade_manager_v1", 0);
    });
}
