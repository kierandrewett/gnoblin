//! What a locked session keeps from windows and from readers of the password, against a real compositor.
//!
//! The stock lock screen takes the password in a field the JavaScript shell draws, so the password passes through the
//! script engine (`docs/session-lock.md`, "What this protects, and what it cannot"). The lock exists to stop other people
//! at the keyboard, and not to sandbox the shell, which is the user's own process: so the boundary these tests pin is the
//! one between the locked session and everything that is not the shell or the compositor's card.
//!
//! - **A window hears no key.** A client that has the keyboard before the lock is given nothing while the session is
//!   locked, whether the password is typed into the shell's screen or into the card, and hears keys again once the session
//!   is unlocked. The password never reaches a Wayland client.
//! - **The password is in no node of the tree.** The shell writes the lock node's `engaged` and calls `authenticate`; the
//!   password is the call's secret text and never a property, so no reader of the tree (a second connection, `gnoblinctl`)
//!   sees it, typed or checked.
//!
//! What is pinned elsewhere, by name, is in `docs/session-lock.md`, "Tests": the capture embargo, the input gate and the log
//! (`a_failed_check_logs_one_warning_without_the_password`). The pointer is not exercised here.

use std::fs;
use std::time::{Duration, Instant};

mod support;
use support::Mapper;
use support::lock_session::*;

/// evdev codes of the letters of the password the secrecy test types.
const HUNTER2: [u32; 7] = [35, 22, 49, 20, 18, 19, 3];

/// Dispatches the client until it has heard `code` pressed, or the time is up.
fn heard(mapper: &mut Mapper, keys: &support::KeyLog, code: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        mapper.roundtrip();
        if keys.pressed().contains(&code) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Types the right password until the session unlocks, dispatching the client between the attempts.
fn unlock(session: &Session, mapper: &mut Mapper) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_the_password(session);
        std::thread::sleep(Duration::from_millis(400));
        mapper.roundtrip();
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            return;
        }
        assert!(Instant::now() < deadline, "the session never unlocked: {seen:?}");
    }
}

/// A client with a mapped window and the keyboard, and the log of what it hears.
fn client_with_the_keyboard(session: &Session) -> (Mapper, support::KeyLog) {
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.app", "App");
    let keys = mapper.keyboard();
    // The window has the keyboard when a key typed at the session reaches it.
    key(session, KEY_A);
    assert!(
        heard(&mut mapper, &keys, KEY_A),
        "the client never heard a key typed before the lock"
    );
    (mapper, keys)
}

#[test]
fn a_window_that_had_the_keyboard_hears_nothing_while_the_shells_lock_screen_takes_the_password() {
    let session = Session::start();
    let (mut mapper, keys) = client_with_the_keyboard(&session);
    // The shell runs before the lock is taken, as it does when a user locks a running session: its screen then finds the
    // window that has the keyboard and gives the keyboard back to it. (A shell that starts under a lock finds no window
    // to give it back to; the compositor's card restores the focus itself.)
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Shell::start(&session, config.path());
    std::thread::sleep(Duration::from_millis(1500));
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    wait_until("the shell's lock screen is up and engaged", &session, |seen| {
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });

    // A wrong password is answered on the shell's screen, and the right one unlocks. Neither is heard by the window.
    type_a_wrong_password(&session);
    wait_until("a wrong password answered on the lock screen", &session, |seen| {
        seen.texts.iter().any(|text| text == "Wrong password.")
    });
    mapper.roundtrip();
    assert_eq!(
        keys.pressed(),
        [KEY_A],
        "the window heard the password typed into the lock screen"
    );
    type_the_password(&session);
    wait_until("the session unlocked", &session, |seen| seen.state == "not-locked");
    mapper.roundtrip();
    assert_eq!(keys.pressed(), [KEY_A], "the window heard the right password too");

    // The window that had the keyboard has it again: the unlock through the shell's screen gives it back as the card's
    // does (the next test). The shell's screen moved the seat's focus onto its own entry while it was up, and the
    // compositor kept the window as the focus the session comes back to.
    key(&session, KEY_B);
    assert!(
        heard(&mut mapper, &keys, KEY_B),
        "the window never heard a key after the unlock through the shell's lock screen"
    );
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_window_that_had_the_keyboard_hears_nothing_while_the_compositors_card_takes_the_password() {
    let session = Session::start();
    let (mut mapper, keys) = client_with_the_keyboard(&session);
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // No shell: the card takes the keys. A key typed before it is up is dropped by the lock gate, so the password is
    // typed until it works, and every attempt is a set of keys the window must not hear.
    unlock(&session, &mut mapper);
    assert_eq!(
        keys.pressed(),
        [KEY_A],
        "the window heard a key typed at the card: {:?}",
        keys.pressed()
    );
    key(&session, KEY_B);
    assert!(
        heard(&mut mapper, &keys, KEY_B),
        "the window never heard a key after the unlock"
    );
}

/// A second client, with a keyboard, that maps a window under the lock. Returns it with what it hears.
fn arrives_under_the_lock(session: &Session) -> (Mapper, support::KeyLog) {
    let mut arrival = Mapper::connect(&session.wayland());
    let heard_by_arrival = arrival.keyboard();
    arrival.map_window("org.example.arrival", "Arrival");
    // Long enough for the stock focus policy (when a shell runs) to have offered it the focus, and for the shell's
    // screen to have taken the keyboard back.
    std::thread::sleep(Duration::from_millis(700));
    arrival.roundtrip();
    (arrival, heard_by_arrival)
}

#[test]
fn a_window_that_arrived_under_the_lock_leaves_the_keyboard_with_the_one_that_had_it_after_an_unlock_through_the_shells_screen()
 {
    let session = Session::start();
    let (mut mapper, keys) = client_with_the_keyboard(&session);
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Shell::start(&session, config.path());
    std::thread::sleep(Duration::from_millis(1500));
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    wait_until("the shell's lock screen is up and engaged", &session, |seen| {
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    let (mut arrival, heard_by_arrival) = arrives_under_the_lock(&session);
    unlock(&session, &mut mapper);
    arrival.roundtrip();
    assert_eq!(keys.pressed(), [KEY_A], "the first window heard the password");
    assert!(
        heard_by_arrival.pressed().is_empty(),
        "the window that arrived under the lock heard the password: {:?}",
        heard_by_arrival.pressed()
    );
    // Nothing under the lock moved the keyboard: the window that had it before the lock has it again.
    key(&session, KEY_B);
    assert!(
        heard(&mut mapper, &keys, KEY_B),
        "the window that had the keyboard before the lock never heard a key after the unlock"
    );
    arrival.roundtrip();
    assert!(
        heard_by_arrival.pressed().is_empty(),
        "the window that arrived under the lock took the keyboard: {:?}",
        heard_by_arrival.pressed()
    );
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_window_that_arrived_under_the_lock_leaves_the_keyboard_with_the_one_that_had_it_after_an_unlock_through_the_card()
{
    let session = Session::start();
    let (mut mapper, keys) = client_with_the_keyboard(&session);
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let (mut arrival, heard_by_arrival) = arrives_under_the_lock(&session);
    unlock(&session, &mut mapper);
    arrival.roundtrip();
    assert_eq!(keys.pressed(), [KEY_A], "the first window heard the password");
    assert!(
        heard_by_arrival.pressed().is_empty(),
        "the window that arrived under the lock heard the password: {:?}",
        heard_by_arrival.pressed()
    );
    key(&session, KEY_B);
    assert!(
        heard(&mut mapper, &keys, KEY_B),
        "the window that had the keyboard before the lock never heard a key after the unlock"
    );
    arrival.roundtrip();
    assert!(
        heard_by_arrival.pressed().is_empty(),
        "the window that arrived under the lock took the keyboard: {:?}",
        heard_by_arrival.pressed()
    );
}

#[test]
fn a_window_that_arrived_under_the_lock_has_the_keyboard_after_the_unlock_when_no_window_had_it_before() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Shell::start(&session, config.path());
    std::thread::sleep(Duration::from_millis(1500));
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    wait_until("the shell's lock screen is up and engaged", &session, |seen| {
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    let (mut arrival, heard_by_arrival) = arrives_under_the_lock(&session);
    unlock(&session, &mut arrival);
    assert!(
        heard_by_arrival.pressed().is_empty(),
        "the window that arrived under the lock heard the password: {:?}",
        heard_by_arrival.pressed()
    );
    // The session has a window and nobody to give the keyboard to but it: it must hear a key, or the user comes back
    // to a desktop that takes none.
    key(&session, KEY_B);
    assert!(
        heard(&mut arrival, &heard_by_arrival, KEY_B),
        "no window heard a key after the unlock: the window that arrived under the lock never got the keyboard"
    );
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

/// Whether any property of any node in the compositor's tree holds `needle`, as text, in a list of texts or in bytes.
fn tree_holds(ipc: &std::path::Path, needle: &str) -> bool {
    support::snapshot(ipc).iter().any(|node| {
        node.props.iter().any(|prop| {
            prop.value_text.contains(needle)
                || prop.value_texts.iter().any(|text| text.contains(needle))
                || prop
                    .value_bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
        })
    })
}

#[test]
fn the_password_is_in_no_node_of_the_compositors_tree_typed_or_checked() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Shell::start(&session, config.path());
    wait_until("the shell's lock screen is up and engaged", &session, |seen| {
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });

    // Typed, character by character: the entry draws a mask, and the tree is read after each key.
    for code in HUNTER2 {
        key(&session, code);
        assert!(
            !tree_holds(&session.ipc(), "hunter2"),
            "the whole password is in the tree while it is typed"
        );
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !tree_holds(&session.ipc(), "hunter"),
        "a part of the password is in the tree"
    );
    // Checked: the call carries it as secret text and the answer is `denied`, which the screen says.
    key(&session, KEY_ENTER);
    wait_until("the wrong password answered on the lock screen", &session, |seen| {
        seen.texts.iter().any(|text| text == "Wrong password.")
    });
    assert!(
        !tree_holds(&session.ipc(), "hunter"),
        "the password is in the tree after it was checked"
    );
    assert_eq!(view(&session.ipc()).state, "locked");
    let problems = shell.finish();
    assert!(
        problems.iter().all(|line| !line.contains("hunter")),
        "the shell reported the password: {problems:?}"
    );
}
