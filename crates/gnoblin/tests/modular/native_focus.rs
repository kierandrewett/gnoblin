//! Keyboard focus of the example shells, run for real: the shipped binary on a headless backend serving an example, a real Wayland
//! client that maps a window, a virtual pointer that clicks it and a virtual keyboard that types.
//!
//! A shell that uses `libgnoblin-std` gets the desktop's stock focus with no line of its own: a new window takes the keyboard, a click
//! gives it to the window that was pressed, and a key then reaches that window's client. What the client hears is what is asserted, not what
//! the shell believes: `wl_keyboard.enter`, the `activated` state of its `xdg_toplevel` configure, and the key.
//!
//! A core-only shell (example 1) holds no tree, so the window manager the std library registers for the stock session decides focus, and the
//! same assertions hold for it.

use crate::support::native_shell::{
    Gnoblin, KEY_ENTER, Pointer, Start, Typist, shells, start_script, std_shells, wait_for_window,
};
use crate::support::{KeyLog, Mapper, STATE_ACTIVATED, Window, scene};
use std::time::Duration;

use crate::{copy_example, screen_of, serving};

use super::native_std::start_std;

/// The evdev code of the left button.
const BTN_LEFT: u32 = 272;

/// A client with one window of `app`, and what its keyboard hears.
struct Client {
    mapper: Mapper,
    keys: KeyLog,
}

impl Client {
    /// Connects a client and maps its window, which is number 0 of the client.
    fn map(session: &Gnoblin, app: &str) -> Client {
        let mut mapper = Mapper::connect(&session.wayland());
        let keys = mapper.keyboard();
        mapper.map_window(app, app);
        Client { mapper, keys }
    }

    /// Whether the compositor has told the client's window it is `activated`.
    fn activated(&self) -> bool {
        self.mapper.states(0).contains(&STATE_ACTIVATED)
    }
}

/// The window of `app` once the shell has placed it for good: a layout that a shell writes lands a turn after the window opens, so the
/// rectangle is read until it stops changing.
#[track_caller]
fn placed(session: &mut Gnoblin, app: &str) -> Window {
    wait_for_window(session, "the window is placed", app, |window| window.width > 0.0);
    let mut window = None;
    session.wait_for("the window stays where it is placed", |session| {
        let now = scene(&session.ipc()).into_iter().find(|window| window.app_id == app);
        std::thread::sleep(Duration::from_millis(300));
        let later = scene(&session.ipc()).into_iter().find(|window| window.app_id == app);
        let settled = now.is_some() && now == later;
        window = later;
        settled
    });
    window.expect("the window settled")
}

/// Waits, dispatching `clients`, until `held` is true.
#[track_caller]
fn wait_clients(session: &mut Gnoblin, clients: &mut [&mut Client], what: &str, held: impl Fn(&[&mut Client]) -> bool) {
    session.wait_for(what, |_| {
        for client in clients.iter_mut() {
            client.mapper.roundtrip();
        }
        held(clients)
    });
}

/// A new window takes the keyboard with no click: the client is entered, its configure says `activated` and a key reaches it.
#[track_caller]
fn a_new_window_takes_the_keyboard(session: &mut Gnoblin) {
    let mut client = Client::map(session, "plain");
    placed(session, "plain");
    wait_clients(
        session,
        &mut [&mut client],
        "the new window has the keyboard",
        |clients| clients[0].keys.holding() && clients[0].activated(),
    );
    let mut typist = Typist::connect(&session.wayland());
    typist.tap(KEY_ENTER);
    wait_clients(session, &mut [&mut client], "a key reaches the new window", |clients| {
        clients[0].keys.pressed().contains(&KEY_ENTER)
    });
}

/// A click on a window takes the keyboard from the window that had it: with two windows open, the second has it, and a click on the
/// first moves it there. The clients are entered and left, the configures say `activated` for the first only, and a key reaches the first.
#[track_caller]
fn a_click_moves_the_keyboard_to_the_window(session: &mut Gnoblin) {
    let mut first = Client::map(session, "first");
    placed(session, "first");
    let mut second = Client::map(session, "second");
    placed(session, "second");
    // The first window may have moved when the second opened.
    let window = placed(session, "first");
    let held = first.mapper.pointer();
    let (width, height) = screen_of(session);
    let mut pointer = Pointer::connect(&session.wayland_privileged(), (width as u32, height as u32));
    // A layout may give the window more room than the client drew (its buffer is 200 by 150), so the pointer goes near the corner.
    pointer.move_to((window.x + 20.0).round() as u32, (window.y + 20.0).round() as u32);
    wait_clients(
        session,
        &mut [&mut first, &mut second],
        "the pointer is over the first window",
        |_| held.over(),
    );
    let before = held.presses();
    pointer.click(BTN_LEFT);
    wait_clients(
        session,
        &mut [&mut first, &mut second],
        "the click reaches the client",
        |_| held.presses() > before,
    );
    wait_clients(
        session,
        &mut [&mut first, &mut second],
        "the clicked window has the keyboard",
        |clients| {
            clients[0].keys.holding() && clients[0].activated() && !clients[1].keys.holding() && !clients[1].activated()
        },
    );
    let mut typist = Typist::connect(&session.wayland());
    typist.tap(KEY_ENTER);
    wait_clients(
        session,
        &mut [&mut first, &mut second],
        "a key reaches the clicked window",
        |clients| clients[0].keys.pressed().contains(&KEY_ENTER),
    );
    assert!(
        second.keys.pressed().is_empty(),
        "the other window heard a key: {}",
        session.log_text()
    );
}

#[test]
fn example_02_a_new_window_takes_the_keyboard() {
    a_new_window_takes_the_keyboard(&mut start_std(&std_shells().prelude));
}

#[test]
fn example_02_a_click_moves_the_keyboard_to_the_window() {
    a_click_moves_the_keyboard_to_the_window(&mut start_std(&std_shells().prelude));
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_03_a_typescript_shell_gives_a_new_window_the_keyboard() {
    let (_dir, mut session) = script_session("03-typescript-std", "shell.ts");
    a_new_window_takes_the_keyboard(&mut session);
}

#[test]
fn example_03_a_typescript_shell_moves_the_keyboard_on_a_click() {
    let (_dir, mut session) = script_session("03-typescript-std", "shell.ts");
    a_click_moves_the_keyboard_to_the_window(&mut session);
}

#[test]
fn example_03_a_python_shell_gives_a_new_window_the_keyboard() {
    let (_dir, mut session) = script_session("03-python-std", "shell.py");
    a_new_window_takes_the_keyboard(&mut session);
}

#[test]
fn example_03_a_python_shell_moves_the_keyboard_on_a_click() {
    let (_dir, mut session) = script_session("03-python-std", "shell.py");
    a_click_moves_the_keyboard_to_the_window(&mut session);
}

#[test]
fn example_01_a_new_window_takes_the_keyboard() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().minimal, &[], &[]);
    session.wait_serving();
    a_new_window_takes_the_keyboard(&mut session);
}
