//! Example 30, run for real from Rust and from Python: the shipped binary on a headless backend serving a shell that binds a pointer drag and
//! a click on the seat's input context of the core. A virtual keyboard holds `Super` and a virtual pointer presses, moves and lets go of a
//! button, as a device would. What is asserted is what the shell logs:
//!
//! - a drag with the left button is one function called with `start`, `update` and `end`, in that order;
//! - the right button with no motion is a click, and a click is not a drag;
//! - a reload while a drag is going cancels the drag in generation 1, and generation 2 hears nothing of it;
//! - generation 2 then follows a drag of its own.
//!
//! The tiers, scopes, a window that closes under a drag, the threshold and the objects a shell implements are tested against a fake
//! compositor in `crates/gnoblin-core-impl/tests/input.rs`, which also replays the fixtures of the TypeScript matcher.

use std::fs;
use std::path::Path;

use crate::support::native_shell::{
    Gnoblin, Home, Launch, Pointer, Start, Typist, pointer_generations, screen_size, shells, start_script,
};
use crate::{copy_example, save, serving};

/// The evdev codes of the buttons the test presses.
const BTN_LEFT: u32 = 272;
const BTN_RIGHT: u32 = 273;

/// How many log lines contain `needle`.
fn count(session: &Gnoblin, needle: &str) -> usize {
    session.log().iter().filter(|line| line.text.contains(needle)).count()
}

/// How many claims the shell's three bindings stand on: the drag and the click on a button each, and the swipe.
const CLAIMS: usize = 3;

/// The shell of generation `n` (from 1) has bound its triggers and the compositor holds them: the shell says its three bindings stand on three
/// claims and that it runs, and the compositor says it applied the claims of every generation so far and the releases of the ones before.
///
/// The shell's own lines are not enough. A script runs while its generation is only staged, so it says "running" before the old generation
/// ended, and what the new one claims goes to the compositor when it takes over, in one transaction behind the old one's releases, which
/// the compositor takes in a frame of its own. A press on the shell's line is the old generation's, or nobody's.
#[track_caller]
fn assert_generation_runs(session: &mut Gnoblin, n: usize) {
    let holds = session.wait_log("the shell counts its claims", |line| {
        line.contains(&format!("pointer: generation {n} holds"))
    });
    assert!(
        holds.text.contains(&format!("holds {CLAIMS} claims for 3 bindings")),
        "{}",
        holds.text
    );
    session.wait_log("the shell is running", |line| {
        line.contains(&format!("pointer: generation {n} is running"))
    });
    session.wait_claims(CLAIMS * n, CLAIMS * (n - 1));
}

/// Waits until the log has `needle` `times` times.
#[track_caller]
fn wait_count(session: &mut Gnoblin, needle: &str, times: usize) {
    session.wait_for(needle, |session| count(session, needle) >= times);
}
/// The keys and the pointer of the test: `Super` is held on the virtual keyboard, and the virtual pointer sits at `at`.
struct Hands {
    keys: Typist,
    pointer: Pointer,
    at: (u32, u32),
}

impl Hands {
    /// Moves the pointer `dx` to the right of where it is.
    fn drag_by(&mut self, dx: u32) {
        self.at.0 += dx;
        self.pointer.move_to(self.at.0, self.at.1);
    }
}

/// A drag of generation `n` with the left button: it starts, updates and ends, in that order, and the click binding hears nothing.
#[track_caller]
fn assert_a_drag(session: &mut Gnoblin, hands: &mut Hands, n: u32) {
    let start = format!("pointer: generation {n} drag start");
    let update = format!("pointer: generation {n} drag update");
    let end = format!("pointer: generation {n} drag end");
    let (starts, updates, ends) = (count(session, &start), count(session, &update), count(session, &end));
    hands.pointer.button(BTN_LEFT, true);
    hands.drag_by(20);
    hands.drag_by(10);
    wait_count(session, &update, updates + 1);
    hands.pointer.button(BTN_LEFT, false);
    wait_count(session, &end, ends + 1);
    assert_eq!(count(session, &start), starts + 1, "one press, one start");
    let (first, last) = (
        session.log().iter().rposition(|line| line.text.contains(&start)),
        session.log().iter().rposition(|line| line.text.contains(&end)),
    );
    assert!(first < last, "the drag starts before it ends\n{}", session.log_text());
}

/// The right button, pressed and let go with no motion, is a click of generation `n`.
#[track_caller]
fn assert_a_click(session: &mut Gnoblin, hands: &mut Hands, n: u32) {
    let click = format!("pointer: generation {n} click");
    let before = count(session, &click);
    hands.pointer.click(BTN_RIGHT);
    wait_count(session, &click, before + 1);
    assert_eq!(count(session, &click), before + 1, "one press, one click");
}

/// The assertions both languages share: generation 1, a drag in flight at a reload, generation 2.
fn assert_the_drag_is_cancelled_by_a_reload(session: &mut Gnoblin, reload: impl FnOnce()) {
    assert_generation_runs(session, 1);
    // The virtual pointer moves the pointer over every other client's windows, so the session offers it on its own sockets only.
    let (width, height) = screen_size(&session.ipc());
    let mut hands = Hands {
        keys: Typist::connect(&session.wayland_privileged()),
        pointer: Pointer::connect(&session.wayland_privileged(), (width as u32, height as u32)),
        at: (width as u32 / 3, height as u32 / 2),
    };
    hands.pointer.move_to(hands.at.0, hands.at.1);
    hands.keys.hold_super();
    assert_a_drag(session, &mut hands, 1);
    assert_a_click(session, &mut hands, 1);
    assert_eq!(count(session, "drag cancel"), 0, "a drag that ended was not cancelled");

    // A drag is going when the shell is replaced.
    let updates = count(session, "pointer: generation 1 drag update");
    hands.pointer.button(BTN_LEFT, true);
    hands.drag_by(20);
    wait_count(session, "pointer: generation 1 drag update", updates + 1);
    reload();
    assert_generation_runs(session, 2);
    session.wait_log("generation 1 is told its drag ended", |line| {
        line.contains("pointer: generation 1 drag cancel")
    });
    assert_eq!(
        count(session, "pointer: generation 1 drag cancel"),
        1,
        "generation 1 is told once\n{}",
        session.log_text()
    );

    // The button is let go after the reload: nobody hears the end of a drag that ended already.
    hands.pointer.button(BTN_LEFT, false);
    assert_a_drag(session, &mut hands, 2);
    assert_a_click(session, &mut hands, 2);
    assert_eq!(
        count(session, "pointer: generation 1 drag end"),
        1,
        "generation 1 heard only the end of its first drag"
    );
    assert_eq!(count(session, "pointer: generation 2 drag cancel"), 0);
    hands.keys.release_super();
    assert!(session.exited().is_none(), "{}", session.log_text());
}

/// Replaces `live` with `from` the way a linker's last step does: a whole file appears under the name at once.
fn swap_in(from: &Path, live: &Path) {
    let part = live.with_extension("part");
    fs::copy(from, &part).expect("the replacement is written");
    fs::rename(&part, live).expect("the replacement takes the name");
}

#[test]
fn example_30_a_rust_shell_follows_a_drag_and_a_reload_cancels_the_drag_in_flight() {
    let built = pointer_generations();
    assert!(shells().pointer.exists(), "the example builds with the other shells");
    let home = Home::new();
    let live = home.copy(&built.v1, "live/libnative_pointer.so");
    let mut launch = Launch::compositor(&["--watch"]);
    launch.args.push(live.clone().into_os_string());
    let mut session = Gnoblin::launch(home, launch);
    session.wait_serving();
    assert_the_drag_is_cancelled_by_a_reload(&mut session, || swap_in(&built.v2, &live));
}

#[test]
fn example_30_a_python_shell_follows_a_drag_and_a_reload_cancels_the_drag_in_flight() {
    let dir = copy_example("30-native-pointer");
    let path = dir.path().join("shell.py");
    let mut session = start_script(Start::Compositor, &path, &[]);
    serving(&mut session);
    let text = fs::read_to_string(&path).expect("the shell");
    assert_the_drag_is_cancelled_by_a_reload(&mut session, || {
        let next = text.replace("GENERATION = 1", "GENERATION = 2");
        assert_ne!(next, text, "the example has the constant this test edits");
        save(&path, &next);
    });
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
