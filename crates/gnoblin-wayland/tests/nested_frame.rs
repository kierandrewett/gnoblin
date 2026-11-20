//! The window a nested session opens on its host, and who draws its frame.
//!
//! A nested session asks its host to draw the window's frame (`xdg-decoration`, server-side) and follows the answer.
//! Every test here runs the real [`Nested`] backend against a real `gnoblin-wayland` host, so what the host sends back
//! is what a compositor sends, and what the test reads is what a person would see: the host's own window rectangle,
//! its screenshot, and the pointer positions the session is handed.
//!
//! | The host answers                                  | Test                                                                          |
//! | ------------------------------------------------- | ----------------------------------------------------------------------------- |
//! | server-side (a shell declares decorations)        | `a_host_that_answers_server_side_gets_no_frame_and_was_asked_for_one`           |
//! | client-side (the default of a host with no shell) | the frame tests: size, pointer offset, close, move, maximise, resize, menu     |
//! | nothing: the host has no `xdg-decoration`         | `a_host_without_xdg_decoration_is_client_side_for_auto_and_bare_for_off`        |
//!
//! The host here is Gnoblin's own compositor, which is the host the frame is for in a hosted session and which answers
//! like Mutter does to a window that asks for what it cannot give. A real GNOME shell cannot be driven headlessly, so
//! nothing here is a picture of one; `docs/backends.md` says which hosts were seen to do what.

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::nested::{
    DecorationAnswer, FRAME_BORDER, FRAME_BUTTON_WIDTH, FRAME_TITLE_HEIGHT, Nested, NestedConfig,
};
use gnoblin_backend::{Backend, BackendEvent, InputEvent};
use gnoblin_config::NestedDecorations;
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{DeviceId, OutputId, Point, Size};

use gnoblin_wayland_testkit::stock;
mod nested_host;
use nested_host::*;

use gnoblin_wayland_testkit::session;
use session::shell::Shell;

const BORDER: i32 = FRAME_BORDER as i32;
const TITLE: i32 = FRAME_TITLE_HEIGHT as i32;
const BUTTON: i32 = FRAME_BUTTON_WIDTH as i32;

/// The colour of the bar and the borders of a window that has the keyboard, and of one that has not.
const FRAME_PIXELS: [[u8; 4]; 2] = [[0x2d, 0x2d, 0x30, 0xff], [0x24, 0x24, 0x26, 0xff]];

/// A session that draws a frame of its own when its host does not.
fn framed(host: &Handle) -> NestedConfig {
    NestedConfig {
        decorations: NestedDecorations::Auto,
        ..NestedConfig::new(host.socket())
    }
}

/// The size a window of an output of `content` has with the session's frame around it.
fn with_frame(content: Size) -> Size {
    Size::new(
        content.width + 2 * FRAME_BORDER,
        content.height + FRAME_TITLE_HEIGHT + FRAME_BORDER,
    )
}

fn text(value: &str) -> TreeValue {
    TreeValue::Text(value.to_owned())
}

/// Pumps the session for `duration`, so what the host sent is read and what the session answers is sent. What the
/// session reported meanwhile is returned.
fn pump_for(nested: &mut Nested, probe: &mut Probe, duration: Duration) -> Vec<BackendEvent> {
    let end = Instant::now() + duration;
    let mut events = Vec::new();
    while Instant::now() < end {
        nested.roundtrip().expect("the host answers a round trip");
        events.extend(nested.dispatch(probe));
        thread::sleep(Duration::from_millis(5));
    }
    events
}

/// Puts the host's pointer at `at` and lets the session hear of it.
fn point_at(host: &Handle, nested: &mut Nested, probe: &mut Probe, at: Point) -> Vec<BackendEvent> {
    host.control.inject_input(InputEvent::PointerMotion {
        device: DeviceId::VIRTUAL,
        output: window_owner(host),
        position: at,
    });
    pump_for(nested, probe, Duration::from_millis(80))
}

/// Presses or releases the host's primary button and lets the session hear of it.
fn click(host: &Handle, nested: &mut Nested, probe: &mut Probe, button: u32, pressed: bool) -> Vec<BackendEvent> {
    host.control.inject_input(InputEvent::PointerButton {
        device: DeviceId::VIRTUAL,
        button,
        pressed,
    });
    pump_for(nested, probe, Duration::from_millis(80))
}

/// Waits for the host's screenshot to show one of `colours` at `point`.
fn wait_for_one_of(host: &Handle, output: OutputId, point: Point, colours: &[[u8; 4]]) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let seen = host.control.screenshot(output).and_then(|image| image.pixel(point));
        if seen.is_some_and(|pixel| colours.contains(&pixel)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the host never drew one of {colours:?} at {point:?}; last pixel: {seen:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// A point of the window `window`, given from its top left corner.
fn from_corner(window: &gnoblin_wayland::ObservedWindow, dx: i32, dy: i32) -> Point {
    Point::new(window.geometry.x + dx, window.geometry.y + dy)
}

/// A framed session on a host with no shell, drawn once and mapped.
fn mapped_session(host: &Handle) -> (Nested, Probe, gnoblin_wayland::ObservedWindow) {
    let mut nested = Nested::connect(&framed(host)).expect("the session connects");
    let mut probe = Probe::drawing(RED);
    draw(&mut nested, &mut probe);
    let window = host
        .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
        .only()
        .expect("one mapped window")
        .clone();
    (nested, probe, window)
}

#[test]
fn a_host_that_answers_client_side_gets_a_frame_around_an_output_the_size_of_its_content() {
    let session = Session::start();
    session.run(|host| {
        let (nested, _probe, window) = mapped_session(host);
        let output = only_output(&nested);
        assert_eq!(
            nested.decoration_answer(output.id),
            Some(DecorationAnswer::Client),
            "a host with no shell holding the tree cannot promise a frame"
        );
        assert!(nested.frame_drawn(output.id), "so the session draws one");
        assert_eq!(
            extent(&window),
            with_frame(output.logical_size()),
            "the host's window is the output and the frame together: the frame takes nothing from the output"
        );

        // What the host composes: the session's red is the output and the frame is around it, at the title bar's height
        // above and the border's width beside and below. The host's screen is smaller than the window, so an edge that
        // lies past it is not looked at.
        let host_output = window_owner(host);
        let width = i32::try_from(window.geometry.width).expect("a width");
        let height = i32::try_from(window.geometry.height).expect("a height");
        let screen = Point::new(
            i32::try_from(HEADLESS_SCREEN.width).expect("a width"),
            i32::try_from(HEADLESS_SCREEN.height).expect("a height"),
        );
        let visible = |at: &Point| at.x < screen.x && at.y < screen.y;
        let session_pixels = [
            from_corner(&window, BORDER, TITLE),
            from_corner(&window, width - BORDER - 1, TITLE + 50),
            from_corner(&window, BORDER, height - BORDER - 1),
        ];
        let frame_pixels = [
            from_corner(&window, BORDER + 5, TITLE - 1),
            from_corner(&window, BORDER - 1, TITLE + 50),
            from_corner(&window, width - 1, TITLE + 50),
            from_corner(&window, width / 2, height - 1),
            from_corner(&window, 40, TITLE / 2),
        ];
        assert!(
            visible(&session_pixels[0]),
            "the window's top left is on the host's screen"
        );
        for at in session_pixels.into_iter().filter(visible) {
            host.wait_for_colour(host_output, at, RED_PIXEL);
        }
        for at in frame_pixels.into_iter().filter(visible) {
            wait_for_one_of(host, host_output, at, &FRAME_PIXELS);
        }
    });
}

#[test]
fn the_pointer_over_the_session_is_where_the_session_is_and_over_the_frame_it_is_nowhere() {
    let session = Session::start();
    session.run(|host| {
        let (mut nested, mut probe, window) = mapped_session(host);
        let output = only_output(&nested);

        // A point 10 and 20 pixels into the session's own pixels is 10 and 20, whatever the frame's size.
        point_at(
            host,
            &mut nested,
            &mut probe,
            from_corner(&window, BORDER + 10, TITLE + 20),
        );
        pump_until(&mut nested, &mut probe, |probe| {
            probe
                .motions()
                .iter()
                .any(|(on, at)| *on == output.id && *at == Point::new(10, 20))
        });

        // Over the title bar and the borders the pointer is the frame's: the session is handed nothing, and above all
        // nothing that is the window's own coordinates, which would be the session's shifted by the frame.
        let before = probe.motions().len();
        for over in [
            from_corner(&window, 100, 10),
            from_corner(&window, 2, TITLE + 100),
            from_corner(&window, 300, 5),
        ] {
            point_at(host, &mut nested, &mut probe, over);
        }
        point_at(
            host,
            &mut nested,
            &mut probe,
            from_corner(&window, BORDER + 12, TITLE + 22),
        );
        pump_until(&mut nested, &mut probe, |probe| {
            probe.motions().iter().any(|(_, at)| *at == Point::new(12, 22))
        });
        let seen: Vec<Point> = probe.motions().iter().skip(before).map(|(_, at)| *at).collect();
        assert_eq!(
            seen,
            [Point::new(12, 22)],
            "of the four points the pointer went through, only the one over the session's own pixels was the session's"
        );
    });
}

#[test]
fn the_close_button_closes_the_window_and_ends_the_session() {
    let session = Session::start();
    session.run(|host| {
        let (mut nested, mut probe, window) = mapped_session(host);
        let width = i32::try_from(window.geometry.width).expect("a width");
        let output = only_output(&nested).id;

        // The pointer over the button, and a press that is let go elsewhere, close nothing.
        point_at(
            host,
            &mut nested,
            &mut probe,
            from_corner(&window, width - BUTTON / 2, TITLE / 2),
        );
        click(host, &mut nested, &mut probe, BTN_LEFT, true);
        point_at(
            host,
            &mut nested,
            &mut probe,
            from_corner(&window, width - BUTTON - BUTTON / 2, TITLE / 2),
        );
        click(host, &mut nested, &mut probe, BTN_LEFT, false);
        assert!(
            host.windows().only().is_some(),
            "a press that slid off the button is not a click on it"
        );

        let mut events = point_at(
            host,
            &mut nested,
            &mut probe,
            from_corner(&window, width - BUTTON / 2, TITLE / 2),
        );
        events.extend(click(host, &mut nested, &mut probe, BTN_LEFT, true));
        events.extend(click(host, &mut nested, &mut probe, BTN_LEFT, false));
        if !events.contains(&BackendEvent::SessionEnded) {
            events.extend(collect_until(&mut nested, &mut probe, |events| {
                events.contains(&BackendEvent::SessionEnded)
            }));
        }
        assert!(
            events.contains(&BackendEvent::OutputDisconnected(output)),
            "the window closed, as when the host closes it: {events:?}"
        );
        host.wait_for(|observation| observation.only().is_none());
    });
}

#[test]
fn dragging_the_title_bar_moves_the_window() {
    let session = Session::start();
    session.run(|host| {
        let (mut nested, mut probe, window) = mapped_session(host);
        let grab = from_corner(&window, 40, TITLE / 2);

        point_at(host, &mut nested, &mut probe, grab);
        click(host, &mut nested, &mut probe, BTN_LEFT, true);
        point_at(host, &mut nested, &mut probe, Point::new(grab.x + 60, grab.y + 40));
        click(host, &mut nested, &mut probe, BTN_LEFT, false);
        let moved = host
            .wait_for(|observation| {
                observation.only().is_some_and(|moved| {
                    moved.geometry.x == window.geometry.x + 60 && moved.geometry.y == window.geometry.y + 40
                })
            })
            .only()
            .expect("the window")
            .clone();
        assert_eq!(extent(&moved), extent(&window), "moving a window does not resize it");
    });
}

#[test]
fn a_double_click_on_the_title_bar_maximises_the_window_and_another_restores_it() {
    let session = Session::start();
    session.run(|host| {
        let (mut nested, mut probe, window) = mapped_session(host);
        let content = only_output(&nested).logical_size();
        let grab = from_corner(&window, 40, TITLE / 2);

        point_at(host, &mut nested, &mut probe, grab);
        for _ in 0..2 {
            click(host, &mut nested, &mut probe, BTN_LEFT, true);
            click(host, &mut nested, &mut probe, BTN_LEFT, false);
        }
        let maximised = host
            .wait_for(|observation| observation.only().is_some_and(|now| now.geometry != window.geometry))
            .only()
            .expect("the window")
            .clone();
        pump_until(&mut nested, &mut probe, |_| true);
        let output = only_output(&nested);
        let window_size = extent(&maximised);
        assert_eq!(
            output.logical_size(),
            Size::new(window_size.width, window_size.height - FRAME_TITLE_HEIGHT),
            "a maximised window keeps its title bar and loses its borders, and the output is what is left"
        );
        assert!(nested.frame_drawn(output.id));

        // The title bar is where it was, so the same two clicks restore the window.
        let grab = Point::new(maximised.geometry.x + 40, maximised.geometry.y + TITLE / 2);
        point_at(host, &mut nested, &mut probe, grab);
        for _ in 0..2 {
            click(host, &mut nested, &mut probe, BTN_LEFT, true);
            click(host, &mut nested, &mut probe, BTN_LEFT, false);
        }
        host.wait_for(|observation| observation.only().is_some_and(|now| extent(now) == extent(&window)));
        pump_for(&mut nested, &mut probe, Duration::from_millis(200));
        assert_eq!(
            only_output(&nested).logical_size(),
            content,
            "restored to the size it had"
        );
    });
}

#[test]
fn dragging_a_corner_resizes_the_window_and_the_output_follows_by_the_content() {
    let session = Session::start();
    session.run(|host| {
        let (mut nested, mut probe, window) = mapped_session(host);
        let width = i32::try_from(window.geometry.width).expect("a width");
        let height = i32::try_from(window.geometry.height).expect("a height");
        let content = only_output(&nested).logical_size();
        let corner = from_corner(&window, width - 2, height - 2);

        point_at(host, &mut nested, &mut probe, corner);
        click(host, &mut nested, &mut probe, BTN_LEFT, true);
        point_at(host, &mut nested, &mut probe, Point::new(corner.x - 40, corner.y - 30));
        click(host, &mut nested, &mut probe, BTN_LEFT, false);
        let resized = host
            .wait_for(|observation| {
                observation.only().is_some_and(|now| {
                    now.geometry.width == window.geometry.width - 40
                        && now.geometry.height == window.geometry.height - 30
                })
            })
            .only()
            .expect("the window")
            .clone();
        pump_until(&mut nested, &mut probe, |_| true);
        let output = only_output(&nested);
        assert_eq!(
            output.logical_size(),
            Size::new(content.width - 40, content.height - 30),
            "the output follows the window by what the frame leaves"
        );
        assert_eq!(extent(&resized), with_frame(output.logical_size()));
    });
}

#[test]
fn a_host_that_answers_server_side_gets_no_frame_and_was_asked_for_one() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |host| {
        shell.attach();
        shell.declare_decorations(true);
        shell.wait_decorations_declared(true);

        let mut nested = Nested::connect(&framed(host)).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        draw(&mut nested, &mut probe);

        let node = shell.wait_window_node();
        shell.wait_published(node, "decoration_wanted", &text("server"));
        shell.wait_published(node, "decoration", &text("server"));
        assert_eq!(
            nested.decoration_answer(output.id),
            Some(DecorationAnswer::Server),
            "the host told the window it draws the frame"
        );
        assert!(
            !nested.frame_drawn(output.id),
            "so the session draws none: the host, or the shell it runs, owns the frame"
        );
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one mapped window")
            .clone();
        assert_eq!(
            extent(&window),
            output.logical_size(),
            "the window is the output exactly, with no room taken for a frame"
        );
    });
}

#[test]
fn the_session_asks_for_a_server_side_frame_whatever_it_draws_when_the_host_refuses() {
    for setting in [NestedDecorations::Auto, NestedDecorations::Off] {
        let (shell, hooks) = Shell::new();
        let session = Session::start_shelled(hooks);
        session.run(move |host| {
            // A shell that holds the tree and draws no decorations: the host answers client-side.
            shell.attach();
            let mut nested = Nested::connect(&NestedConfig {
                decorations: setting,
                ..NestedConfig::new(host.socket())
            })
            .expect("the session connects");
            let mut probe = Probe::drawing(RED);
            let output = only_output(&nested);
            draw(&mut nested, &mut probe);

            let node = shell.wait_window_node();
            shell.wait_published(node, "decoration_wanted", &text("server"));
            assert_ne!(
                shell.published(node, "decoration"),
                Some(text("server")),
                "the host promised no frame: {setting:?}"
            );
            assert_eq!(
                nested.decoration_answer(output.id),
                Some(DecorationAnswer::Client),
                "{setting:?}"
            );
            assert_eq!(
                nested.frame_drawn(output.id),
                setting == NestedDecorations::Auto,
                "the request is the same and only `auto` draws a frame of its own when it is refused: {setting:?}"
            );
        });
    }
}

#[test]
fn a_host_without_xdg_decoration_is_client_side_for_auto_and_bare_for_off() {
    for setting in [NestedDecorations::Auto, NestedDecorations::Off] {
        let session = Session::start_without_decoration();
        session.run(move |host| {
            let mut nested = Nested::connect(&NestedConfig {
                decorations: setting,
                ..NestedConfig::new(host.socket())
            })
            .expect("the session connects");
            let mut probe = Probe::drawing(RED);
            let output = only_output(&nested);
            draw(&mut nested, &mut probe);
            assert_eq!(
                nested.decoration_answer(output.id),
                None,
                "a host with no protocol answers nothing"
            );
            let window = host
                .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
                .only()
                .expect("one mapped window")
                .clone();
            match setting {
                NestedDecorations::Auto => {
                    assert!(nested.frame_drawn(output.id));
                    assert_eq!(extent(&window), with_frame(output.logical_size()));
                }
                NestedDecorations::Off => {
                    assert!(!nested.frame_drawn(output.id));
                    assert_eq!(
                        extent(&window),
                        output.logical_size(),
                        "bare, as a host without the protocol always was"
                    );
                }
            }
        });
    }
}

#[test]
fn off_draws_no_frame_for_a_host_that_answers_client_side() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig {
            decorations: NestedDecorations::Off,
            ..NestedConfig::new(host.socket())
        })
        .expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        draw(&mut nested, &mut probe);
        assert_eq!(nested.decoration_answer(output.id), Some(DecorationAnswer::Client));
        assert!(!nested.frame_drawn(output.id));
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one mapped window")
            .clone();
        assert_eq!(extent(&window), output.logical_size());
    });
}

#[test]
fn a_right_click_on_the_title_bar_asks_the_shell_for_the_window_menu_where_it_was_made() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |host| {
        // A shell that holds the tree and draws no decorations: the host answers client-side, so the session draws its
        // frame, and the host's requests of a window are the shell's to answer.
        shell.attach();
        let mut nested = Nested::connect(&framed(host)).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        let node = shell.wait_window_node();
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one mapped window")
            .clone();
        shell.write(
            node,
            &[
                ("x", TreeValue::Double(100.0)),
                ("y", TreeValue::Double(100.0)),
                ("width", TreeValue::Double(f64::from(window.geometry.width))),
                ("height", TreeValue::Double(f64::from(window.geometry.height))),
            ],
        );
        let at = Point::new(100 + 40, 100 + TITLE / 2);
        // The pointer is moved until the session has heard of the window being there: a press needs its enter.
        let deadline = Instant::now() + TIMEOUT;
        while shell.menu_anchors(node).is_empty() {
            assert!(Instant::now() < deadline, "the shell was never asked for the menu");
            point_at(host, &mut nested, &mut probe, at);
            click(host, &mut nested, &mut probe, BTN_RIGHT, true);
            click(host, &mut nested, &mut probe, BTN_RIGHT, false);
        }
        assert_eq!(
            shell.menu_anchors(node)[0],
            (at.x, at.y),
            "the menu opens where the pointer was, whatever the frame's size"
        );
    });
}
