//! `gnoblin_session_host_v1` against a real client: the host's channel to one
//! hosted session.
//!
//! Every test starts a session on a socket of its own and connects a client whose
//! role the session promised first, which is what makes the global visible. The
//! tests are the failure-mode list in `src/protocols/gnoblin_session_host_v1.rs`, in order.

mod owned;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_lock::ProtectionState;
use gnoblin_wayland::{ClientRole, GnoblinSessionHostV1PowerAction, GnoblinSessionHostV1RequestKind};
use owned::protocols::gnoblin_session_host_v1::gnoblin_session_host_v1;
use owned::{Connected, Handle, Session, TIMEOUT};
use wayland_client::protocol::{wl_keyboard, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// What the client records from the events it receives.
#[derive(Default)]
struct App {
    host: Option<gnoblin_session_host_v1::GnoblinSessionHostV1>,
    protects: Vec<u32>,
    unprotects: usize,
    closed: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    enters: u32,
    leaves: u32,
    keys: u32,
}

impl App {
    /// Binds the session host global and the objects a window needs.
    fn bind(client: &mut Connected<Self>) -> gnoblin_session_host_v1::GnoblinSessionHostV1 {
        let host: gnoblin_session_host_v1::GnoblinSessionHostV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("a hosted session sees its own protocol");
        client.state.host = Some(host.clone());
        let _seat: wl_seat::WlSeat = client
            .globals
            .bind(&client.qh, 1..=7, ())
            .expect("every session advertises wl_seat");
        client.roundtrip();
        host
    }
}

impl Dispatch<gnoblin_session_host_v1::GnoblinSessionHostV1, ()> for App {
    fn event(
        state: &mut Self,
        _host: &gnoblin_session_host_v1::GnoblinSessionHostV1,
        event: gnoblin_session_host_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            gnoblin_session_host_v1::Event::Protect { serial } => state.protects.push(serial),
            gnoblin_session_host_v1::Event::Unprotect => state.unprotects += 1,
            gnoblin_session_host_v1::Event::Closed => state.closed = true,
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && state.keyboard.is_none()
        {
            state.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.enters += 1,
            wl_keyboard::Event::Leave { .. } => state.leaves += 1,
            wl_keyboard::Event::Key { .. } => state.keys += 1,
            _ => {}
        }
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
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            let _ = state;
        }
    }
}

wayland_client::delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_registry::WlRegistry);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_buffer::WlBuffer);

impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _registry: &wayland_client::protocol::wl_registry::WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// A session client that maps one window and can be protected.
struct SessionClient {
    client: Connected<App>,
    window: wl_surface::WlSurface,
}

impl SessionClient {
    /// Connects, binds the session host global and maps a window.
    fn start(handle: &Handle) -> Self {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        let _ = host;
        let wm_base: xdg_wm_base::XdgWmBase = client
            .globals
            .bind(&client.qh, 1..=5, ())
            .expect("every session advertises xdg_wm_base");
        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &client.qh, ());
        let toplevel = xdg.get_toplevel(&client.qh, ());
        toplevel.set_title("hosted".to_owned());
        toplevel.set_app_id("org.example.Session".to_owned());
        surface.commit();
        client.roundtrip();
        let buffer = client.buffer(200, 150);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 200, 150);
        surface.commit();
        client.roundtrip();
        client.wait_until("the window to be entered by the keyboard", |state| state.enters >= 1);
        Self {
            client,
            window: surface,
        }
    }

    /// The window the session maps.
    fn window(&self) -> &wl_surface::WlSurface {
        &self.window
    }
}

/// Waits until the hosted session's protection is in `state`.
fn wait_for_protection(
    handle: &Handle,
    state: ProtectionState,
) -> Option<gnoblin_wayland::GnoblinSessionHostV1HostedSession> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let session = handle
            .control
            .hosted_session(std::process::id())
            .expect("the loop answers");
        if session.as_ref().is_some_and(|session| session.protection == state) {
            return session;
        }
        assert!(
            Instant::now() < deadline,
            "the protection never became {state:?}; last session: {session:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits until the screen is a single colour, which is what a full cover looks
/// like in this slice.
fn wait_for_one_colour(handle: &Handle) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        // Every output the session presents on, so a hotplug is covered too.
        let uniform = (0..2).all(|raw| {
            handle
                .control
                .screenshot(gnoblin_types::OutputId::new(raw))
                .is_none_or(|image| owned::distinct_colours(&image) == 1)
        });
        if uniform {
            return;
        }
        assert!(Instant::now() < deadline, "the covered desktop never became one colour");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_ordinary_client_sees_no_session_host() {
    // No promise, so the connection is unprivileged and the global is hidden.
    let session = Session::start(None);
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        client.roundtrip();
        assert!(
            !client.has("gnoblin_session_host_v1"),
            "an application must not see the session channel"
        );
    });
}

#[test]
fn a_session_sees_only_the_session_host_protocol() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        client.roundtrip();
        let interfaces = client.interfaces();
        assert!(interfaces.iter().any(|name| name == "gnoblin_session_host_v1"));
        for hidden in [
            "zwlr_layer_shell_v1",
            "zwlr_foreign_toplevel_manager_v1",
            "ext_data_control_manager_v1",
            "zwlr_screencopy_manager_v1",
            "gnoblin_window_frame_manager_v1",
        ] {
            assert!(!client.has(hidden), "a hosted session must not see {hidden}");
        }
    });
}

#[test]
fn ready_is_recorded_and_ends_nothing() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut hosted = SessionClient::start(handle);
        let host = hosted.client.state.host.clone().expect("a bound host");
        host.ready();
        hosted.client.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let session = handle
                .control
                .hosted_session(std::process::id())
                .expect("the loop answers");
            if session.as_ref().is_some_and(|session| session.ready) {
                break;
            }
            assert!(Instant::now() < deadline, "the session was never ready");
            thread::sleep(Duration::from_millis(5));
        }
        // Readiness says nothing about being shown, and no request came of it.
        assert!(handle.control.host_requests().is_empty());
    });
}

#[test]
fn protection_waits_for_a_presented_cover_and_is_announced_once() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut hosted = SessionClient::start(handle);
        assert_eq!(
            handle.control.protect_session(std::process::id()),
            Some(true),
            "the session was found"
        );
        hosted
            .client
            .wait_until("the protect event", |state| !state.protects.is_empty());
        let serial = hosted.client.state.protects[0];
        let host = hosted.client.state.host.clone().expect("a bound host");
        // Frames are flowing, so the host's half of the proof arrives on its own; the
        // session's acknowledgement is the other half.
        hosted.client.roundtrip();
        thread::sleep(Duration::from_millis(20));
        host.protected(serial);
        hosted.client.roundtrip();
        wait_for_protection(handle, ProtectionState::Covered);
        // The covered screen is the cover and nothing else.
        wait_for_one_colour(handle);

        // Announced once: further frames change nothing and no second protect is
        // asked for.
        for _ in 0..3 {
            hosted.client.roundtrip();
            thread::sleep(Duration::from_millis(10));
        }
        let session = handle
            .control
            .hosted_session(std::process::id())
            .flatten()
            .expect("the session is connected");
        assert_eq!(session.protection, ProtectionState::Covered);
        assert_eq!(
            hosted.client.state.protects.len(),
            1,
            "a protection already in place is not asked for twice"
        );
    });
}

#[test]
fn an_acknowledgement_without_a_cover_frame_announces_nothing() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut hosted = SessionClient::start(handle);
        // A held present is neither described nor drawn, so no frame can prove the
        // cover while it is held.
        handle.backend.hold_presents(true);
        assert_eq!(handle.control.protect_session(std::process::id()), Some(true));
        hosted
            .client
            .wait_until("the protect event", |state| !state.protects.is_empty());
        let serial = hosted.client.state.protects[0];
        let host = hosted.client.state.host.clone().expect("a bound host");
        host.protected(serial);
        hosted.client.roundtrip();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            handle
                .control
                .hosted_session(std::process::id())
                .flatten()
                .map(|s| s.protection),
            Some(ProtectionState::Covering),
            "an acknowledgement is never taken for a frame the host does not have"
        );

        // Releasing the present re-damages the cover, so the next frame carries it
        // and the protection lands.
        handle.backend.hold_presents(false);
        wait_for_protection(handle, ProtectionState::Covered);
        wait_for_one_colour(handle);
    });
}

#[test]
fn a_hotplugged_output_is_covered_before_it_shows_anything() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut hosted = SessionClient::start(handle);
        assert_eq!(handle.control.protect_session(std::process::id()), Some(true));
        hosted
            .client
            .wait_until("the protect event", |state| !state.protects.is_empty());
        let serial = hosted.client.state.protects[0];
        let host = hosted.client.state.host.clone().expect("a bound host");
        host.protected(serial);
        hosted.client.roundtrip();
        wait_for_protection(handle, ProtectionState::Covered);

        // A second output appears: it never showed the session, and the protection
        // stays announced rather than being retracted for a new screen.
        handle
            .backend
            .connect_output(
                "HEADLESS-2",
                gnoblin_backend::headless::DEFAULT_MODE,
                gnoblin_types::Scale::integral(1),
            )
            .expect("a mode with pixels");
        wait_for_one_colour(handle);
        assert_eq!(
            handle
                .control
                .hosted_session(std::process::id())
                .flatten()
                .map(|s| s.protection),
            Some(ProtectionState::Covered),
            "an announcement is never taken back by a hotplug"
        );
    });
}

#[test]
fn a_session_that_crashes_while_protected_keeps_the_cover() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut hosted = SessionClient::start(handle);
        assert_eq!(handle.control.protect_session(std::process::id()), Some(true));
        hosted
            .client
            .wait_until("the protect event", |state| !state.protects.is_empty());
        let serial = hosted.client.state.protects[0];
        let host = hosted.client.state.host.clone().expect("a bound host");
        host.protected(serial);
        hosted.client.roundtrip();
        wait_for_protection(handle, ProtectionState::Covered);

        // The session's connection goes away without any request: nothing unlocks and
        // the screen keeps the cover.
        drop(hosted);
        let deadline = Instant::now() + TIMEOUT;
        while handle.control.hosted_session(std::process::id()).flatten().is_some() {
            assert!(Instant::now() < deadline, "the crashed session was never noticed");
            thread::sleep(Duration::from_millis(5));
        }
        wait_for_one_colour(handle);
        // A release for a session that no longer exists shows nothing and does not
        // panic.
        assert_eq!(handle.control.release_session(std::process::id()), Some(false));
        wait_for_one_colour(handle);
    });
}

#[test]
fn an_unknown_acknowledgement_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        // Nothing was ever protected, so this serial was never sent.
        host.protected(7);
        client.expect_protocol_error("gnoblin_session_host_v1", 1);
    });
}

#[test]
fn a_lock_request_reaches_the_ipc_as_a_request() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        host.lock();
        client.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let requests = handle.control.host_requests();
            if let Some(request) = requests.first() {
                assert_eq!(request.kind, GnoblinSessionHostV1RequestKind::Lock);
                break;
            }
            assert!(Instant::now() < deadline, "the lock request never reached the manager");
            thread::sleep(Duration::from_millis(5));
        }
        // The compositor itself did not cover anything: a session asks, it never
        // decides, so no protect event arrived and nothing was hidden.
        client.roundtrip();
        assert!(client.state.protects.is_empty(), "a lock request is not a protection");
    });
}

#[test]
fn a_logout_request_reaches_the_ipc_as_a_request() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        host.logout();
        client.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if handle
                .control
                .host_requests()
                .first()
                .is_some_and(|request| request.kind == GnoblinSessionHostV1RequestKind::Logout)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the logout request never reached the manager"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // The session keeps running: logging out is the manager's act, not this
        // request's.
        host.ready();
        client.roundtrip();
        assert!(!client.state.closed, "the session was not ended by its own request");
    });
}

#[test]
fn a_power_request_carries_its_action() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        host.power(gnoblin_session_host_v1::PowerAction::Suspend);
        client.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if handle.control.host_requests().first().is_some_and(|request| {
                request.kind == GnoblinSessionHostV1RequestKind::Power(GnoblinSessionHostV1PowerAction::Suspend)
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the suspend request never reached the manager"
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn an_invalid_output_hint_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        host.output_hint("HEAD-1".to_owned(), 0, 1080, 1.0);
        client.expect_protocol_error("gnoblin_session_host_v1", 2);
    });
}

#[test]
fn a_half_sent_hint_set_is_not_published() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let host = App::bind(&mut client);
        host.output_hint("HEAD-1".to_owned(), 1920, 1080, 1.0);
        client.roundtrip();
        // Without `output_hint_done` the set is incomplete, so nothing is published.
        let session = handle
            .control
            .hosted_session(std::process::id())
            .flatten()
            .expect("the session is connected");
        assert!(session.hints.is_empty(), "a half-sent set never reaches the manager");

        host.output_hint_done();
        client.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let session = handle
                .control
                .hosted_session(std::process::id())
                .flatten()
                .expect("the session is connected");
            if let Some(hint) = session.hints.first() {
                assert_eq!(hint.name, "HEAD-1");
                assert_eq!((hint.width, hint.height), (1920, 1080));
                assert!((hint.scale - 1.0).abs() < f64::EPSILON);
                break;
            }
            assert!(Instant::now() < deadline, "the hint set was never published");
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn a_protected_session_receives_no_input() {
    let session = Session::start(Some(ClientRole::SessionHost));
    session.run(|handle| {
        let mut hosted = SessionClient::start(handle);
        let serial = {
            assert_eq!(handle.control.protect_session(std::process::id()), Some(true));
            hosted
                .client
                .wait_until("the protect event", |state| !state.protects.is_empty());
            let host = hosted.client.state.host.clone().expect("a bound host");
            host.protected(hosted.client.state.protects[0]);
            hosted.client.roundtrip();
            wait_for_protection(handle, ProtectionState::Covered);
            hosted.client.state.protects[0]
        };
        let _ = serial;
        // The keyboard left with the pixels, and a key goes nowhere.
        hosted
            .client
            .wait_until("the keyboard to leave", |state| state.leaves >= 1);
        handle.control.inject_input(InputEvent::Key {
            keycode: 30,
            pressed: true,
        });
        handle.control.inject_input(InputEvent::Key {
            keycode: 30,
            pressed: false,
        });
        hosted.client.roundtrip();
        thread::sleep(Duration::from_millis(20));
        hosted.client.roundtrip();
        assert_eq!(
            hosted.client.state.keys, 0,
            "a protected session must not receive a key"
        );
        let _ = hosted.window();
    });
}
