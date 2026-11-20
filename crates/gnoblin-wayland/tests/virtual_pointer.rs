//! `zwlr_virtual_pointer_manager_v1` and `zwlr_virtual_pointer_v1` against real clients.
//!
//! The scenarios are the failure modes listed at the top of `src/protocols/zwlr_virtual_pointer_v1.rs`. A virtual
//! pointer's whole point is what the client *under* it hears, so each test runs two real clients on a real socket
//! each: the injector on the privileged socket, and an application with a window on the ordinary one. Who may bind
//! the manager is `tests/access.rs`: the manager is in its `PRIVILEGED` list, so an application and a sandbox are
//! refused it and the privileged socket is not.

mod session;
mod stock;

use gnoblin_render_frame::frame::Overlay;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{OutputId, Rect, Size};
use gnoblin_wayland::{Modal, ModalEvent};
use session::{BTN_LEFT, Client, Handle, Session};
use wayland_client::protocol::wl_pointer;
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

wayland_client::delegate_noop!(session::App: ignore ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(session::App: ignore ZwlrVirtualPointerV1);

/// Where a click inside the test window lands, as the window's surface sees it.
const INSIDE: (f64, f64) = (100.0, 75.0);

/// How far a position the client reports may be from the one the test asked for: the layout fraction is rounded to
/// a logical pixel.
const PIXEL: f64 = 1.5;

/// A virtual pointer made by a client on the privileged socket.
struct Injector {
    client: Client,
    pointer: ZwlrVirtualPointerV1,
    /// The output extents the absolute motion is stated against.
    extent: (u32, u32),
}

impl Injector {
    /// Connects on the privileged socket and makes a virtual pointer on the session's seat.
    fn connect(session: &Handle) -> Self {
        let client = Client::connect(session.privileged_socket());
        let manager: ZwlrVirtualPointerManagerV1 = client
            .globals
            .bind(&client.qh, 1..=2, ())
            .expect("a privileged client sees the virtual pointer manager");
        let pointer = manager.create_virtual_pointer(Some(&client.seat), &client.qh, ());
        let (width, height) = session.screen();
        Self {
            client,
            pointer,
            // The last pixel of an output is `width - 1`, so the fraction `x / (width - 1)` lands on `x` exactly.
            extent: (width as u32 - 1, height as u32 - 1),
        }
    }

    /// Moves the pointer to a logical position on the output, in whole pixels.
    fn move_to(&mut self, x: f64, y: f64) {
        self.pointer
            .motion_absolute(0, x as u32, y as u32, self.extent.0, self.extent.1);
        self.client.roundtrip();
    }

    /// Presses or releases the left button.
    fn button(&mut self, pressed: bool) {
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        self.pointer.button(0, BTN_LEFT, state);
        self.client.roundtrip();
    }
}

/// An application with a mapped window and the window's logical origin on the output.
fn application_with_window(session: &Handle) -> (Client, (f64, f64)) {
    let mut app = Client::connect(session.socket());
    // The window lives as long as the connection: dropping the handle destroys nothing.
    let _window = app.map_window("org.gnoblin.Pointee", "Pointee");
    let geometry = session
        .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
        .only()
        .expect("the window is managed")
        .geometry;
    (app, (f64::from(geometry.x), f64::from(geometry.y)))
}

/// Asserts the client's pointer is at `(x, y)` in its surface, within a pixel's rounding.
#[track_caller]
fn assert_pointer_at(app: &Client, (x, y): (f64, f64)) {
    let (seen_x, seen_y) = app.state.pointer_at.expect("the pointer is over the window");
    assert!(
        (seen_x - x).abs() <= PIXEL && (seen_y - y).abs() <= PIXEL,
        "the pointer should be at ({x}, {y}) and the client saw ({seen_x}, {seen_y})"
    );
}

#[test]
fn an_unprivileged_client_does_not_see_the_virtual_pointer_manager() {
    Session::start().run(|session| {
        let application = Client::connect(session.socket());
        assert!(!application.has_global("zwlr_virtual_pointer_manager_v1"));
        let privileged = Client::connect(session.privileged_socket());
        assert_eq!(
            privileged.state.globals.get("zwlr_virtual_pointer_manager_v1"),
            Some(&2),
            "the privileged socket serves version 2, which carries the output"
        );
    });
}

#[test]
fn a_virtual_pointer_moves_and_clicks_a_window() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);

        injector.move_to(origin.0 + INSIDE.0, origin.1 + INSIDE.1);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);
        assert_pointer_at(&app, INSIDE);

        // A relative move is whole pixels; the fraction waits for the next one.
        injector.pointer.motion(0, 10.0, 5.0);
        injector.client.roundtrip();
        app.wait_until("the relative move", |app| {
            app.pointer_at
                .is_some_and(|(x, y)| (x - 110.0).abs() <= PIXEL && (y - 80.0).abs() <= PIXEL)
        });

        injector.button(true);
        injector.button(false);
        app.wait_until("the click", |app| {
            app.pointer_buttons == [(BTN_LEFT, true), (BTN_LEFT, false)]
        });
    });
}

#[test]
fn a_relative_move_keeps_its_fraction() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);
        injector.move_to(origin.0 + INSIDE.0, origin.1 + INSIDE.1);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);

        // Three nudges of 0.4 are one pixel in all: none moves the pointer alone, the third carries the fraction.
        for _ in 0..3 {
            injector.pointer.motion(0, 0.4, 0.0);
        }
        injector.client.roundtrip();
        app.wait_until("the carried pixel", |app| {
            app.pointer_at.is_some_and(|(x, _)| (x - 101.0).abs() <= 0.5)
        });
    });
}

#[test]
fn an_absolute_motion_lands_at_its_fraction_of_the_layout() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);
        // The middle of the window: half of its logical size is the fraction of the output's.
        injector.move_to(origin.0 + 50.0, origin.1 + 40.0);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);
        assert_pointer_at(&app, (50.0, 40.0));

        // A pointer made for an output states the fraction against that output, which is the whole layout here.
        let manager: ZwlrVirtualPointerManagerV1 = injector
            .client
            .globals
            .bind(&injector.client.qh, 2..=2, ())
            .expect("version 2 is served");
        let named = manager.create_virtual_pointer_with_output(
            Some(&injector.client.seat),
            Some(&injector.client.output),
            &injector.client.qh,
            (),
        );
        let (width, height) = injector.extent;
        named.motion_absolute(0, (origin.0 as u32) + 120, (origin.1 as u32) + 90, width, height);
        injector.client.roundtrip();
        app.wait_until("the named output's position", |app| {
            app.pointer_at
                .is_some_and(|(x, y)| (x - 120.0).abs() <= PIXEL && (y - 90.0).abs() <= PIXEL)
        });
    });
}

#[test]
fn a_wheel_frame_reaches_the_client_with_its_steps() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);
        injector.move_to(origin.0 + INSIDE.0, origin.1 + INSIDE.1);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);
        app.state.pointer_axes.clear();

        injector.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        injector
            .pointer
            .axis_discrete(0, wl_pointer::Axis::VerticalScroll, 15.0, 1);
        injector.pointer.frame();
        injector.client.roundtrip();
        app.wait_until("the scroll group", |app| {
            app.pointer_axes.last().is_some_and(|last| last == "frame")
        });

        let seen = app.state.pointer_axes.join(", ");
        assert!(
            seen.contains("source Value(Wheel)"),
            "a wheel scroll names its source: {seen}"
        );
        assert!(
            seen.contains("axis Value(VerticalScroll) 15.00"),
            "and its distance: {seen}"
        );
        assert!(
            seen.contains("discrete Value(VerticalScroll) 1") || seen.contains("value120 Value(VerticalScroll) 120"),
            "and the step it is worth: {seen}"
        );
    });
}

#[test]
fn a_finger_frame_reaches_the_client_and_stops() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);
        injector.move_to(origin.0 + INSIDE.0, origin.1 + INSIDE.1);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);
        app.state.pointer_axes.clear();

        injector.pointer.axis_source(wl_pointer::AxisSource::Finger);
        injector.pointer.axis(0, wl_pointer::Axis::VerticalScroll, 3.5);
        injector.pointer.frame();
        injector.pointer.axis_source(wl_pointer::AxisSource::Finger);
        injector.pointer.axis_stop(0, wl_pointer::Axis::VerticalScroll);
        injector.pointer.frame();
        injector.client.roundtrip();
        app.wait_until("both scroll groups", |app| {
            app.pointer_axes.iter().filter(|line| *line == "frame").count() >= 2
        });

        let seen = app.state.pointer_axes.join(", ");
        assert!(
            seen.contains("source Value(Finger)"),
            "a finger scroll names its source: {seen}"
        );
        assert!(
            seen.contains("axis Value(VerticalScroll) 3.50"),
            "and its distance in pixels: {seen}"
        );
        assert!(
            !seen.contains("discrete") && !seen.contains("value120"),
            "a finger counts no steps: {seen}"
        );
        assert!(
            seen.contains("stop Value(VerticalScroll)"),
            "and its stop ends the kinetic scroll: {seen}"
        );
    });
}

#[test]
fn a_destroyed_pointer_releases_its_held_button() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);
        injector.move_to(origin.0 + INSIDE.0, origin.1 + INSIDE.1);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);

        injector.button(true);
        app.wait_until("the press", |app| app.pointer_buttons == [(BTN_LEFT, true)]);
        injector.pointer.destroy();
        injector.client.roundtrip();
        app.wait_until("the release the destroyed pointer sends", |app| {
            app.pointer_buttons == [(BTN_LEFT, true), (BTN_LEFT, false)]
        });
    });
}

/// The output the headless backend starts with.
fn default_output() -> OutputId {
    OutputId::new(0)
}

/// An opaque white modal over the default output, the shape a consent prompt takes.
fn modal() -> Modal {
    let size = Size { width: 2, height: 2 };
    let image = Image::new(size, 8, vec![255; 16]).expect("a 2x2 image");
    let overlay = Overlay::new(Rect::from_size(size), &image).expect("a non-empty overlay");
    Modal::new(default_output(), overlay)
}

#[test]
fn a_virtual_click_under_a_modal_goes_to_the_modal() {
    Session::start().run(|session| {
        let (mut app, origin) = application_with_window(session);
        let mut injector = Injector::connect(session);
        injector.move_to(origin.0 + INSIDE.0, origin.1 + INSIDE.1);
        app.wait_until("the pointer to enter the window", |app| app.pointer_enters == 1);

        session.control.set_modal(Some(modal()));
        injector.client.roundtrip();
        injector.button(true);
        injector.button(false);

        let events = session.control.modal_events().expect("a modal is up");
        assert!(
            matches!(
                events.as_slice(),
                [
                    ModalEvent::PointerButton { button: BTN_LEFT, .. },
                    ModalEvent::PointerButton { button: BTN_LEFT, .. }
                ]
            ),
            "the modal takes both edges: {events:?}"
        );
        app.roundtrip();
        assert!(
            app.state.pointer_buttons.is_empty(),
            "and the window under it hears none: {:?}",
            app.state.pointer_buttons
        );
    });
}
