//! A session in a session in a session: a nested session is an ordinary client of its host and an ordinary session on
//! its own, at any depth.
//!
//! Three compositors run in one process, each on a thread of its own, as three processes would. The **host** is this
//! repository's own compositor on the headless backend. The **first session** is a second compositor whose backend is a
//! [`Nested`] connected to the host, and the **second session** is a third whose backend is a [`Nested`] connected to the
//! first. Nothing in the host knows about either: there is no hosting protocol, no role and no registry entry for a hosted
//! session (`design/rust-compositor.md`, "Nested sessions without special cases").
//!
//! A real `wayland-client` connection sits on each level. The tests read three things the design promises:
//!
//! - every level offers its clients the same globals, and none of them is a Gnoblin-owned one;
//! - the host's windows are the first session's window and nothing else, however deep the clients are;
//! - a client of the innermost session draws into the host's screenshot, and a click on the host reaches it.

mod nested_host;
use gnoblin_wayland_testkit::session;
use gnoblin_wayland_testkit::stock;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_types::{DeviceId, OutputId, Point};
use gnoblin_wayland::{Config, ControlHandle, NewWindowFocus, ObservedWindow, Server, SessionSnapshot, WmOptions};

use nested_host::{BLACK_PIXEL, BTN_LEFT, HEADLESS_SCREEN, RED_PIXEL, Session as HostSession, window_owner};
use session::{Client, TIMEOUT};

/// The `app_id` a nested backend gives its windows, which is how a host tells a window of a session from any other.
const SESSION_APP_ID: &str = "org.gnoblin.Session";

/// The B, G, R, A bytes of red for an `Argb8888` buffer on a little-endian machine: a screenshot reads it as [`RED_PIXEL`].
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];

/// One session of the chain: a compositor on a nested backend, running on its own thread.
struct Level {
    /// The socket its own clients connect to.
    socket: PathBuf,
    /// What a test can read of its windows.
    observation: Arc<Mutex<SessionSnapshot>>,
    /// Stops its loop when the level is dropped.
    control: ControlHandle,
}

impl Level {
    /// Starts a session whose window is a window of the compositor at `parent`, and returns once it is up.
    fn start(parent: &Path) -> Self {
        let parent = parent.to_path_buf();
        let (up, ready) = mpsc::channel();
        thread::spawn(move || {
            let directory = tempfile::tempdir().expect("a temporary directory");
            let socket = directory.path().join("session-0.sock");
            let nested = Nested::connect(&NestedConfig::new(&parent)).expect("the session connects to its host");
            let config = Config {
                socket_path: Some(socket.clone()),
                wm: WmOptions {
                    new_window_focus: NewWindowFocus::Allow,
                    ..WmOptions::default()
                },
                ..crate::stock::config()
            };
            let server = Server::new(Box::new(nested), config).expect("the session starts");
            up.send(Self {
                socket,
                observation: server.observation(),
                control: server.control(),
            })
            .expect("the test is waiting");
            let _ = server.run();
            drop(directory);
        });
        ready.recv_timeout(TIMEOUT).expect("the session came up")
    }

    /// The windows this level shows now.
    fn windows(&self) -> Vec<ObservedWindow> {
        self.observation
            .lock()
            .expect("the observation lock")
            .windows()
            .cloned()
            .collect()
    }

    /// Waits until `predicate` holds for this level's windows, or fails with what it had.
    fn wait_for(&self, what: &str, predicate: impl Fn(&[ObservedWindow]) -> bool) -> Vec<ObservedWindow> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let windows = self.windows();
            if predicate(&windows) {
                return windows;
            }
            assert!(
                Instant::now() < deadline,
                "the session never showed {what}; it shows {windows:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Level {
    fn drop(&mut self) {
        self.control.stop();
    }
}

/// The windows of a level that are a nested session's.
fn session_windows(windows: &[ObservedWindow]) -> Vec<&ObservedWindow> {
    windows
        .iter()
        .filter(|window| window.app_id == SESSION_APP_ID)
        .collect()
}

/// The smallest rectangle of `screen` that holds `colour`, as its first and last pixel, or `None` when it shows none.
fn extent_of(screen: &gnoblin_render_frame::image::Image, colour: [u8; 4]) -> Option<(Point, Point)> {
    let (mut first, mut last): (Option<Point>, Option<Point>) = (None, None);
    for y in 0..HEADLESS_SCREEN.height as i32 {
        for x in 0..HEADLESS_SCREEN.width as i32 {
            let at = Point::new(x, y);
            if screen.pixel(at) == Some(colour) {
                first = Some(first.map_or(at, |f| Point::new(f.x.min(x), f.y.min(y))));
                last = Some(last.map_or(at, |l| Point::new(l.x.max(x), l.y.max(y))));
            }
        }
    }
    first.zip(last)
}

#[test]
fn a_session_nested_two_deep_behaves_like_one() {
    HostSession::start().run(|host| {
        let first = Level::start(host.socket());
        host.wait_for(|observation| {
            observation
                .windows()
                .any(|window| window.app_id == SESSION_APP_ID && window.mapped)
        });
        let second = Level::start(&first.socket);
        first.wait_for("the second session's window", |windows| {
            session_windows(windows).iter().any(|window| window.mapped)
        });

        // Every level is an ordinary compositor to its clients: the same globals at depth one and depth two, and none
        // that Gnoblin owns.
        let mut on_host = Client::connect(host.socket());
        let mut on_first = Client::connect(&first.socket);
        let mut on_second = Client::connect(&second.socket);
        let (host_globals, first_globals, second_globals) = (
            on_host.state.interfaces(),
            on_first.state.interfaces(),
            on_second.state.interfaces(),
        );
        assert_eq!(
            first_globals, second_globals,
            "a session offers its clients the same globals at depth one and at depth two"
        );
        for (level, globals) in [
            ("host", &host_globals),
            ("first", &first_globals),
            ("second", &second_globals),
        ] {
            assert!(
                !globals.is_empty() && globals.iter().all(|name| !name.starts_with("gnoblin_")),
                "the {level} level advertises a Gnoblin-owned global: {globals:?}"
            );
        }

        // The host sees the first session's window and nothing of what is inside it.
        let host_windows: Vec<ObservedWindow> = host.windows().windows().cloned().collect();
        assert_eq!(
            host_windows.len(),
            1,
            "the host's only window is the first session's, however deep the clients are: {host_windows:#?}"
        );
        assert_eq!(host_windows[0].app_id, SESSION_APP_ID);
        assert_eq!(
            session_windows(&first.windows()).len(),
            1,
            "the first session's only window is the second session's"
        );
        assert!(
            second.windows().is_empty(),
            "the second session has no client window yet: {:#?}",
            second.windows()
        );

        // An ordinary client of the innermost session draws, and its pixels reach the host's screen through both
        // sessions' windows.
        let _deep = on_second.map_solid_window("org.example.Deep", "Deep", (200, 150), RED_BGRA);
        second.wait_for("the deep client's window", |windows| {
            windows.iter().any(|window| window.title == "Deep" && window.mapped)
        });
        let output: OutputId = window_owner(host);
        let deadline = Instant::now() + TIMEOUT;
        let (top_left, bottom_right) = loop {
            let screen = host.control.screenshot(output).expect("a host screenshot");
            if let Some(extent) = extent_of(&screen, RED_PIXEL) {
                assert_eq!(
                    screen.pixel(Point::new(
                        HEADLESS_SCREEN.width as i32 - 1,
                        HEADLESS_SCREEN.height as i32 - 1
                    )),
                    Some(BLACK_PIXEL),
                    "the host's own background is still black where no session draws"
                );
                break extent;
            }
            assert!(
                Instant::now() < deadline,
                "the deep client's red never reached the host's screen"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(
            bottom_right.x - top_left.x >= 150 && bottom_right.y - top_left.y >= 100,
            "the deep client's window is drawn whole on the host: {top_left:?} to {bottom_right:?}"
        );

        // A click on the host over that picture is the host's pointer. It reaches the first session as the nested
        // backend's pointer, the second session as its own, and the deep client as an ordinary click.
        let centre = Point::new((top_left.x + bottom_right.x) / 2, (top_left.y + bottom_right.y) / 2);
        let _ = on_second.seat_devices();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            host.control.inject_input(InputEvent::PointerMotion {
                device: DeviceId::VIRTUAL,
                output,
                position: centre,
            });
            on_second.roundtrip();
            if on_second.state.pointer_enters > 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the pointer never entered the deep client's window"
            );
            thread::sleep(Duration::from_millis(10));
        }
        for pressed in [true, false] {
            host.control.inject_input(InputEvent::PointerButton {
                device: DeviceId::VIRTUAL,
                button: BTN_LEFT,
                pressed,
            });
        }
        on_second.wait_until("the click", |app| {
            app.pointer_buttons.contains(&(BTN_LEFT, true)) && app.pointer_buttons.contains(&(BTN_LEFT, false))
        });

        // The ordinary clients of the other levels are untouched by it.
        on_first.roundtrip();
        on_host.roundtrip();
        assert_eq!(
            on_first.state.pointer_enters, 0,
            "a client of the first session never saw the deep pointer"
        );
        assert_eq!(
            on_host.state.pointer_enters, 0,
            "a client of the host never saw the deep pointer"
        );
    });
}
