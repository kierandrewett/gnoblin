//! The session lock against a real compositor, with and without the JavaScript shell.
//!
//! The session, the shell and the way the tests type and read the tree are `support/lock_session.rs`: the session is
//! [`Compositor::run`] on a headless backend with its sockets in a temporary directory, its lock check is the scripted
//! `FakeAuthenticator` the other lock tests use in place of PAM (`accept("kieran", "hi")`), the lock is taken through
//! the tree as the screensaver service takes it, and the keys are injected on the seat path of a real keyboard
//! (`InputTap`): they go through the lock gate, which gives them to the shell's focused node under the compositor's own
//! lock, or to the built-in card, and to nothing else. What the tests assert is what the compositor's tree holds, read
//! over a second connection.
//!
//! What is proven here, and what is not:
//!
//! - With no shell, the right password typed at the locked session unlocks it: the compositor's built-in card took the
//!   keys and the PAM test double checked them.
//! - A shell that attaches to the locked session draws its lock screen under the `lock` node and engages the lock (the
//!   clock and the entry are painted in the tree), a wrong password is answered "Wrong password." in that screen and
//!   the session stays locked, and the right one unlocks it, the screen comes down and the lock is disengaged.
//! - A transaction the compositor refuses `stale-base` with no op named (the shell's base is ahead of, or too far behind,
//!   the compositor's serial: the test forges it on the way out, because the harness has no other way to make the real
//!   compositor's base stale between the shell's read and its send) is sent again whole: an `Output.apply` in it still
//!   gets its answer, and a password check in it, whose text is not kept, is answered `failed` on the lock screen, which
//!   takes the next attempt and unlocks. The shell runs on the test's thread for those, which ticks it.
//! - A shell that stays attached and `engaged` but stops ticking (a hang) loses the lock UI to the compositor's card
//!   when its lease lapses.
//! - Not read: the pixels. The compositor draws the shell's lock screen and the card over the cover, but no test here
//!   reads the output's picture; the tree's paint lists are what is read.
//!
//! The ways the shell can fail while it holds the lock screen (it hangs, crashes, is killed, is slow, throws every frame,
//! runs away in a frame handler or a reload) are `js_lock_failures.rs`; what the lock keeps from windows and from the
//! password's readers is `js_lock_boundary.rs`.

use std::fs;
use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::{TxOp, TxReply};

mod support;
use support::lock_session::*;

// ---- the tests ----------------------------------------------------------------------------------------------------

#[test]
fn with_no_shell_the_right_password_typed_at_the_locked_session_unlocks_it() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // A key typed before the built-in card is up is dropped by the lock gate, so the password is typed until it works.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        std::thread::sleep(Duration::from_millis(300));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a locked session with no shell never unlocked: {seen:?}"
        );
    }
}

#[test]
fn a_js_shell_that_attaches_to_a_locked_session_takes_over_from_the_card_and_unlocks_it() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // The card has been up since the cover was proven: nobody holds the lock screen.
    std::thread::sleep(Duration::from_millis(1500));

    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Shell::start(&session, config.path());

    let screen = wait_until("the shell's lock screen engaged and painted", &session, |seen| {
        seen.engaged
            && seen.texts.iter().any(|text| text == "Enter your password.")
            && seen
                .texts
                .iter()
                .any(|text| text.len() == 5 && text.as_bytes()[2] == b':')
    });
    assert_eq!(
        screen.state, "locked",
        "the shell's screen replaced the card without unlocking"
    );

    // A wrong password, typed at the keyboard the shell's entry holds, is answered in the shell's screen.
    type_and_enter(&session, &[KEY_N, KEY_O]);
    wait_until("a wrong password answered on the lock screen", &session, |seen| {
        seen.texts.iter().any(|text| text == "Wrong password.")
    });
    assert_eq!(view(&session.ipc()).state, "locked", "a wrong password never unlocks");

    // The right one unlocks; the screen comes down and the lock is disengaged.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        std::thread::sleep(Duration::from_millis(400));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the shell's screen never unlocked the session: {seen:?}"
        );
    }
    let after = wait_until("the lock screen came down", &session, |seen| {
        !seen.engaged && seen.texts.is_empty()
    });
    assert_eq!(after.state, "not-locked");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

// ---- a shell over a link the compositor can be made to refuse ----------------------------------------------------------

#[test]
fn an_output_apply_refused_stale_base_with_no_op_named_goes_again_and_is_answered() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell sees the compositor's screen", |shell| {
        shell.eval("String(gnoblin.outputs.length)") != "0"
    });

    shell.forged.borrow_mut().forge = Some(Forge::NextBaseAhead);
    shell.eval("globalThis.app = gnoblin.outputs.at(0).apply({ scale: 1 }); 0");
    shell.until("the apply is answered", |shell| {
        shell.eval("String(app.done)") == "true"
    });

    let forged = shell.forged.borrow();
    let refused: Vec<&TxReply> = forged.replies.iter().filter(|reply| !reply.applied).collect();
    assert_eq!(refused.len(), 1, "the compositor refused one transaction: {refused:?}");
    assert_eq!(
        (refused[0].code.as_str(), refused[0].failed_op),
        ("stale-base", -1),
        "the real compositor's word for a base it cannot check"
    );
    let tokens: Vec<u64> = forged
        .sent
        .iter()
        .flatten()
        .filter_map(|op| match op {
            TxOp::Call { method, token, .. } if method == "apply" => Some(*token),
            _ => None,
        })
        .collect();
    assert_eq!(tokens.len(), 2, "the apply went out, was refused and went out again");
    assert_eq!(tokens[0], tokens[1], "under the same token");
}

#[test]
fn a_password_check_the_compositor_refused_stale_is_answered_failed_on_the_lock_screen_which_takes_the_next_attempt() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });

    // The check goes out with a base the compositor cannot take: it refuses the transaction `stale-base` and the
    // password, which is wiped once sent, cannot go again. The screen is told, and does not wait on "Checking...".
    shell.forged.borrow_mut().forge = Some(Forge::NextCallBaseAhead("authenticate"));
    type_and_enter(&session, &[KEY_H, KEY_I]);
    shell.until("the lost check is answered on the lock screen", |_| {
        view(&session.ipc())
            .texts
            .iter()
            .any(|text| text == "The password could not be checked.")
    });
    assert_eq!(view(&session.ipc()).state, "locked", "a lost check never unlocks");
    let refused: Vec<(String, i64)> = shell
        .forged
        .borrow()
        .replies
        .iter()
        .filter(|reply| !reply.applied)
        .map(|reply| (reply.code.clone(), reply.failed_op))
        .collect();
    assert_eq!(refused, [("stale-base".to_owned(), -1)]);

    // The screen takes another attempt, and the right password unlocks.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        shell.run_for(Duration::from_millis(400));
        if view(&session.ipc()).state == "not-locked" {
            break;
        }
        assert!(Instant::now() < deadline, "the screen took no second attempt");
    }
    let secrets: Vec<&String> = shell
        .problems
        .iter()
        .filter(|line| line.contains("carries a secret"))
        .collect();
    assert_eq!(
        secrets.len(),
        1,
        "the lost check was reported once: {:?}",
        shell.problems
    );
}

#[test]
fn a_js_shell_that_stays_engaged_but_stops_ticking_loses_the_lock_ui_to_the_card() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    // While it ticks, it renews its lease: its screen is the lock UI and the card stays away.
    shell.run_for(Duration::from_millis(1500));
    assert_eq!(view(&session.ipc()).state, "locked");

    // The shell hangs: it stays attached and engaged and sends nothing. Its lease lapses and the card takes over.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        std::thread::sleep(Duration::from_millis(400));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a shell that hung while holding the lock screen left the session with nothing to type into: {seen:?}"
        );
    }
}
