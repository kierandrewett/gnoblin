//! A locked session whose shell has no lock screen: the proof of decision D7 of `design/modular-framework.md` (the lock
//! card stays in core), against the real compositor.
//!
//! A shell that imports only `gnoblin-core` gets no stock lock screen: that one is `gnoblin-std/lock`. The session must
//! not be a black screen with no way back whatever the shell does, so the compositor's own card takes the password when
//! nobody else holds the lock screen. What a core-only shell has to know to be safe is nothing: it does not draw a lock
//! screen, engage the lock or call `authenticate`, and the card appears.
//!
//! The rig is the one of `js_lock.rs` (see `support/lock_session.rs`): [`Compositor::run`] on a headless backend, the
//! scripted `FakeAuthenticator` in place of PAM, the lock taken through the tree as the screensaver service takes it,
//! the keys injected on the seat path of a real keyboard, a real Wayland client with the keyboard, and what is asserted
//! read from the compositor's tree over a second connection. The shell's entry is [`CORE_ONLY_SHELL`]: it imports
//! `gnoblin-core` and nothing else, and places the client's window, which the tree shows, so a pass is not a shell that
//! failed to attach.
//!
//! 1. **Core-only shell.** The shell holds the tree. The session is locked and nobody draws a lock screen: no canvas under
//!    the lock node, the lock never engaged. The card takes the password: an empty and a wrong one never unlock, the
//!    right one does. The client hears none of the keys typed while the session is locked, and hears keys again once it
//!    is unlocked.
//! 2. **No shell.** The same, with nobody attached.
//! 3. **The password never reaches a core-only shell.** The shell is run by the test over a recording link. Every message
//!    the compositor sent it, whatever its kind, is searched for the password and for the bytes of it, and the tree is
//!    searched after every key. The same searches find the lock's own state in the traffic, so they can find text.
//!
//! A stock shell whose lock screen throws, hangs or crashes is `js_lock_failures.rs`; the takeover times of both
//! scenarios are `js_lock_bound.rs`.

use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::TxOp;

mod support;
use support::Mapper;
use support::lock_session::*;

/// What a session that is locked asks of the card, with a client that has the keyboard and a shell (or none) that does
/// not hold the lock screen. Returns how long after the cover was proven the session unlocked.
#[track_caller]
fn the_card_takes_the_password(
    session: &Session,
    mapper: &mut Mapper,
    keys: &support::KeyLog,
    card_after: Duration,
) -> Duration {
    // The lock screen is nobody's: the lock is not engaged and no canvas is drawn under it.
    lock(session);
    wait_until("the session locked", session, |seen| seen.state == "locked");
    let proven = Instant::now();
    let seen = view(&session.ipc());
    assert!(
        !seen.engaged && seen.texts.is_empty(),
        "a lock screen is up although no shell draws one: {seen:?}"
    );

    // An empty password and a wrong one never unlock; the right one does. The client is dispatched between the
    // attempts, so it hears whatever the compositor gives it.
    let took = the_password_is_still_needed(session, proven, card_after, |window| {
        mapper.roundtrip();
        std::thread::sleep(window);
    });
    mapper.roundtrip();
    assert_eq!(
        keys.pressed(),
        [KEY_A],
        "the window heard a key typed at the locked session: {:?}",
        keys.pressed()
    );

    // The keyboard goes back to the window that had it: the card is gone and the session takes keys again.
    assert_eq!(view(&session.ipc()).state, "not-locked");
    key(session, KEY_B);
    assert!(
        heard(mapper, keys, KEY_B),
        "the window never heard a key after the unlock through the card"
    );
    took
}

#[test]
fn a_shell_that_imports_only_gnoblin_core_and_holds_the_tree_leaves_the_card_to_take_the_password() {
    let session = Session::start();
    let (mut mapper, keys) = client_with_the_keyboard(&session);
    let (mut shell, _config) = core_only_shell_holding_the_tree(&session);

    let grace = Duration::from_millis(gnoblin_wayland::LOCK_UI_GRACE_MS);
    let took = the_card_takes_the_password(&session, &mut mapper, &keys, grace);
    eprintln!("core-only shell: the session unlocked through the card {took:?} after the cover was proven");
    assert!(
        took <= grace + SLACK,
        "the card took the password {took:?} after the cover, of a shell that draws no lock screen"
    );

    // The shell was alive and holding the tree throughout: it did not end or fail.
    assert!(!shell.ended(), "the shell ended");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn with_no_shell_attached_the_card_takes_the_password() {
    let session = Session::start();
    let (mut mapper, keys) = client_with_the_keyboard(&session);
    let took = the_card_takes_the_password(&session, &mut mapper, &keys, Duration::ZERO);
    eprintln!("no shell: the session unlocked through the card {took:?} after the cover was proven");
}

/// The password is typed at the locked session of a shell that never touches the lock, and nothing it is told carries it.
#[test]
fn the_password_never_reaches_a_shell_that_imports_only_gnoblin_core() {
    let session = Session::start_accepting(HUNTER2_TEXT);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window(PLACED_APP, "App");
    let config = tempfile::tempdir().expect("a config directory");
    std::fs::write(config.path().join("shell.ts"), CORE_ONLY_SHELL).expect("shell.ts");
    let mut shell = Driven::start(&session, config.path());
    wait_until_placed(&session, || shell.tick());
    shell.forged.borrow_mut().incoming = Some(Vec::new());

    lock(&session);
    shell.until("the session locked", |_| view(&session.ipc()).state == "locked");
    // The card is up after the grace. Typed at it: the password's letters with a wrong last one, then the right one.
    shell.run_for(Duration::from_millis(gnoblin_wayland::LOCK_UI_GRACE_MS + 700));
    let wrong: Vec<u32> = HUNTER2.iter().copied().take(6).chain([4]).collect();
    for code in &wrong {
        key(&session, *code);
        mapper.roundtrip();
        assert!(
            !tree_holds(&session.ipc(), "hunter"),
            "a part of the password is in the tree while it is typed"
        );
        shell.run_for(Duration::from_millis(20));
    }
    key(&session, KEY_ENTER);
    shell.run_for(Duration::from_millis(500));
    assert_eq!(
        view(&session.ipc()).state,
        "locked",
        "a wrong password unlocked the session"
    );

    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &HUNTER2);
        shell.run_for(Duration::from_millis(400));
        assert!(
            !tree_holds(&session.ipc(), "hunter"),
            "the password is in the tree after it was checked"
        );
        if view(&session.ipc()).state == "not-locked" {
            break;
        }
        assert!(Instant::now() < deadline, "the card never took the right password");
    }
    shell.run_for(Duration::from_millis(500));

    // What the compositor told the shell, whatever the kind of message: its text, and its bytes as the debug form of a byte
    // list writes them.
    let forged = shell.forged.borrow();
    let told: Vec<String> = forged
        .incoming
        .as_ref()
        .expect("the link was recording")
        .iter()
        .map(|message| format!("{message:?}"))
        .collect();
    assert!(!told.is_empty(), "the shell was told nothing");
    assert!(
        told.iter().any(|message| message.contains("\"locked\""))
            && told.iter().any(|message| message.contains("\"not-locked\"")),
        "the recording does not hold the lock's own state, so it cannot be trusted to hold a password: {told:?}"
    );
    for needle in ["hunter", "unter2", "unter3", "nter2"] {
        let bytes = needle
            .bytes()
            .map(|byte| byte.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        for message in &told {
            assert!(!message.contains(needle), "the shell was told {needle:?}: {message}");
            assert!(
                !message.contains(&bytes),
                "the shell was told the bytes of {needle:?}: {message}"
            );
        }
    }

    // And the shell sent nothing of it, nor touched the lock.
    let calls: Vec<&str> = forged
        .sent
        .iter()
        .flatten()
        .filter_map(|op| match op {
            TxOp::Call { method, .. } => Some(method.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !calls
            .iter()
            .any(|method| matches!(*method, "authenticate" | "heartbeat" | "lock" | "unlock")),
        "the core-only shell touched the lock: {calls:?}"
    );
    assert!(
        !format!("{:?}", forged.sent).contains("hunter"),
        "the shell sent the password"
    );
    drop(forged);
    assert!(shell.problems.is_empty(), "the shell reported: {:?}", shell.problems);
}
