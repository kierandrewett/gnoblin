//! The lock screen when the shell that holds it fails, against a real compositor.
//!
//! The stock shell draws the lock screen and takes the password (`data/js/gnoblin/lock.ts`); the compositor's own card
//! (`gnoblin-wayland/src/lock_card.rs`) takes it when no shell holds the screen. A shell holds the screen by a lease it
//! renews with the lock node's `heartbeat` call, and the compositor gives the screen to the card when the lease lapses
//! (`LOCK_UI_LEASE_MS`) and the grace after it has gone (`LOCK_UI_GRACE_MS`). The shell is another process and can fail
//! in every way a process can, so what each test pins is what the session does while it has:
//!
//! 1. **The session stays locked.** The cover is the compositor's and does not depend on the shell.
//! 2. **Somebody takes the password within a bound.** [`CARD_WITHIN`] after the last beat, and at once when the shell is
//!    gone, the card does, and the bound is the same whatever the shell is doing and whatever budget it has been given.
//! 3. **The password is still needed.** A wrong password and an empty one never unlock the session, whoever takes them,
//!    and the right one does.
//! 4. **The compositor does not wait for the shell.** Its tree answers a second connection and its card takes keys
//!    while the shell is inside a call for seconds.
//!
//! The failures are: a shell that hangs holding the screen, a frame of the screen that throws, a shell that crashes (its
//! role ends with an engine error and its connection closes, which is what a killed process does to the compositor;
//! `js_guardian.rs` kills the real process), a slow shell that renews in time and one that does not, a script that spins
//! in a frame handler, and a script that spins in a reload. The spins run under budgets far longer than the product's
//! (`FRAME_BUDGET` is 5 s and `STAGE_BUDGET` 30 s) on purpose: the bound is a property of the lease and not of the budget,
//! so a test that cut the spin at the budget would prove less.
//!
//! What stays unproven here: the card's pixels (no test reads the output's picture) and a real process hung by a
//! signal over a full socket buffer, which `js_guardian.rs` does with `SIGSTOP`. The password check is the scripted
//! `FakeAuthenticator`, not PAM.

use std::fs;
use std::time::{Duration, Instant};

mod support;
use support::Mapper;
use support::lock_session::*;

/// What the scripts of these tests reach into: the lock screen, the frame hook and the runtime.
const IMPORTS: &str = "import { gnoblin, lockScreen, onUiFrame } from \"gnoblin\";\n\
                       Object.assign(globalThis, { gnoblin, lockScreen, onUiFrame });\n";

/// How long a read of the tree may take while the shell is busy. It takes milliseconds when nothing is wrong; this is
/// what a machine that other tests keep busy may take, and well under the seconds a compositor that waited for the shell
/// would take.
const SERVE_WITHIN: Duration = Duration::from_secs(2);

/// A config directory whose `init.ts` is `IMPORTS` and `body`.
fn config_with(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a config directory");
    fs::write(dir.path().join("init.ts"), format!("{IMPORTS}{body}\n")).expect("init.ts");
    dir
}

/// Waits until the shell's lock screen is up and has taken the lock: painted in the tree, and `engaged`.
fn wait_for_the_screen(session: &Session) {
    wait_until("the shell's lock screen is up and engaged", session, |seen| {
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
}

/// A locked session whose shell is run by the test and holds the lock screen. It has run long enough to renew its lease.
fn locked_with_a_shell(body: &str) -> (Session, tempfile::TempDir, Driven) {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with(body);
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    shell.run_for(Duration::from_millis(700));
    assert_eq!(view(&session.ipc()).state, "locked");
    (session, config, shell)
}

/// What a locked session asks of whoever takes the keys, the shell's screen or the compositor's card: a wrong password
/// and an empty one never unlock it, and the right one does within `card_after + SLACK` of `since`. `card_after` is
/// when the card is surely up: [`CARD_WITHIN`] after a shell stopped renewing, nothing after one is gone. `wait` passes
/// the time between the attempts: it ticks a shell that is alive, and sleeps for one that is not. Returns how long after
/// `since` the session unlocked.
#[track_caller]
fn the_password_is_still_needed(
    session: &Session,
    since: Instant,
    card_after: Duration,
    mut wait: impl FnMut(Duration),
) -> Duration {
    let ipc = session.ipc();
    let still_locked = |when: &str| {
        let seen = view(&ipc);
        assert_eq!(
            seen.state, "locked",
            "{when}: a password that was not the right one unlocked: {seen:?}"
        );
    };
    // Before the card can be up: a wrong password and an empty one, typed at whoever holds the keys.
    type_a_wrong_password(session);
    wait(Duration::from_millis(300));
    still_locked("a wrong password");
    type_nothing(session);
    wait(Duration::from_millis(300));
    still_locked("an empty password");
    // Across the takeover, and then a wrong one when the card is surely up.
    while since.elapsed() < card_after + Duration::from_millis(500) {
        wait(Duration::from_millis(100));
        still_locked("the takeover");
    }
    type_a_wrong_password(session);
    wait(Duration::from_millis(300));
    still_locked("a wrong password at the card");
    // The right one, until it takes.
    let deadline = since + card_after + SLACK;
    loop {
        type_the_password(session);
        wait(Duration::from_millis(400));
        if view(&ipc).state == "not-locked" {
            return since.elapsed();
        }
        assert!(
            Instant::now() < deadline,
            "nobody took the right password within {:?} of the failure: {:?}",
            card_after + SLACK,
            view(&ipc)
        );
    }
}

// ---- a shell that hangs, throws, crashes ---------------------------------------------------------------------------

#[test]
fn a_shell_that_hangs_holding_the_lock_screen_leaves_the_session_covered_and_the_card_takes_the_password_in_time() {
    let (session, _config, shell) = locked_with_a_shell("");
    // The shell is attached and engaged, and never runs another frame: it renews nothing.
    let hung = Instant::now();
    let took = the_password_is_still_needed(&session, hung, CARD_WITHIN, std::thread::sleep);
    assert!(
        took <= CARD_WITHIN + SLACK,
        "the card took the password {took:?} after the shell hung"
    );
    drop(shell);
}

#[test]
fn a_shell_whose_lock_frame_throws_every_frame_still_gets_the_card_though_it_stays_attached_and_engaged() {
    let (session, _config, mut shell) = locked_with_a_shell("");
    shell.eval("__gnoblinHost.lock.takeAnswers = () => { throw new Error('the lock screen is broken'); }; 0");
    let broken = Instant::now();
    // The shell goes on running frames, and reports the failure once: it is alive and its screen is not.
    let took = the_password_is_still_needed(&session, broken, CARD_WITHIN, |window| shell.run_for(window));
    assert!(
        took <= CARD_WITHIN + SLACK,
        "the card took the password {took:?} after the frame broke"
    );
    assert_eq!(
        shell
            .problems
            .iter()
            .filter(|line| line.contains("LockScreen.frame threw"))
            .count(),
        1,
        "the failure is reported once and not every frame: {:?}",
        shell.problems
    );
}

#[test]
fn a_shell_that_crashes_leaves_the_session_covered_and_the_card_takes_the_password_at_once() {
    let (session, _config, mut shell) = locked_with_a_shell("");
    // The shell's own module fails: an entry point of the library throws, which is an engine error of the role and not
    // a listener's failure the shell reports and goes on from. The role ends, and its connection closes with it.
    shell.eval("__gnoblinHost.entry.ui = () => { throw new Error('the shell fails'); }; 0");
    let ended = shell.try_tick().expect_err("the frame fails and the role ends");
    drop(shell);
    let crashed = Instant::now();
    assert!(!ended.to_string().is_empty(), "the failure says what it was");
    // No shell holds the tree, so the card is wanted at once: there is no grace to wait out.
    let took = the_password_is_still_needed(&session, crashed, Duration::ZERO, std::thread::sleep);
    assert!(
        took <= SLACK,
        "the card took the password {took:?} after the shell was gone"
    );
    assert!(
        !view(&session.ipc()).engaged,
        "the dead shell's claim on the lock screen is released"
    );
}

// ---- a shell that is slow ------------------------------------------------------------------------------------------

#[test]
fn a_slow_shell_that_still_renews_its_lease_keeps_the_screen_and_the_card_stays_away() {
    let (session, _config, mut shell) = locked_with_a_shell("");
    // Frames 700 ms apart: more than ten times a frame's usual cost, and well inside the two seconds the lease lasts.
    let slow = |shell: &mut Driven, window: Duration| {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            shell.tick();
            std::thread::sleep(Duration::from_millis(700));
        }
    };
    // A wrong password, and the screen's own word for it: only the shell's screen can write that in the tree.
    type_a_wrong_password(&session);
    let deadline = Instant::now() + SLACK * 3;
    while !view(&session.ipc()).texts.iter().any(|text| text == "Wrong password.") {
        slow(&mut shell, Duration::from_millis(700));
        assert!(
            Instant::now() < deadline,
            "a slow shell never answered the wrong password on its screen: {:?}",
            view(&session.ipc())
        );
    }
    assert_eq!(view(&session.ipc()).state, "locked");
    // Long enough that a lease that did not hold would have lapsed and the card taken over: the screen is still the
    // shell's, and the right password is answered by it, which takes the screen down and releases the lock.
    slow(&mut shell, CARD_WITHIN + Duration::from_secs(1));
    assert!(
        view(&session.ipc()).engaged,
        "the slow shell still holds the lock screen"
    );
    type_the_password(&session);
    let deadline = Instant::now() + SLACK * 3;
    loop {
        slow(&mut shell, Duration::from_millis(700));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" && !seen.engaged {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the shell's own screen never unlocked: {seen:?}"
        );
    }
}

#[test]
fn a_shell_too_slow_to_renew_loses_the_keys_to_the_card_for_that_stretch_and_takes_the_screen_back_when_it_recovers() {
    let (session, _config, mut shell) = locked_with_a_shell("");
    // One frame that takes longer than the lease and the grace together: the shell renews nothing meanwhile.
    let stalled = Instant::now();
    while stalled.elapsed() < CARD_WITHIN + Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(50));
    }
    // A wrong password typed now goes to the card. If the shell's screen were still the key target, the shell would find
    // these keys when it recovers, and say "Wrong password." for them.
    type_a_wrong_password(&session);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(view(&session.ipc()).state, "locked");
    // The frame ends and the shell is quick again: it renews, and the card gives the screen back.
    shell.run_for(Duration::from_millis(1500));
    let seen = view(&session.ipc());
    assert!(
        seen.texts.iter().any(|text| text == "Enter your password.")
            && !seen.texts.iter().any(|text| text == "Wrong password."),
        "the keys typed while the shell was away were the card's, not the screen's: {seen:?}"
    );
    // And the screen is the shell's again: it answers a wrong password in the tree, where the card cannot.
    type_a_wrong_password(&session);
    let deadline = Instant::now() + SLACK * 2;
    while !view(&session.ipc()).texts.iter().any(|text| text == "Wrong password.") {
        shell.run_for(Duration::from_millis(100));
        assert!(
            Instant::now() < deadline,
            "the recovered shell never took the screen back: {:?}",
            view(&session.ipc())
        );
    }
    assert_eq!(view(&session.ipc()).state, "locked");
}

// ---- a script that runs away ---------------------------------------------------------------------------------------

/// How long a runaway script spins in these tests. It is longer than the bound by a margin that a machine under load
/// does not eat, so that a session that unlocks inside the bound did it while the script was still spinning.
const SPIN: Duration = Duration::from_secs(10);

/// The longest the time to the card may be and still be inside the spin.
fn inside_the_spin() -> Duration {
    CARD_WITHIN + SLACK
}

/// Reads the tree and types, while a script spins, as a user and a second client of the session would: every read of the
/// tree has to come back within [`SERVE_WITHIN`], and the password has to be needed and to work within the bound.
/// Returns how long the session took to unlock and the slowest read.
#[track_caller]
fn serve_while_it_spins(session: &Session, since: Instant) -> (Duration, Duration) {
    let ipc = session.ipc();
    let mut slowest = Duration::ZERO;
    let took = the_password_is_still_needed(session, since, CARD_WITHIN, |window| {
        let started = Instant::now();
        let _ = view(&ipc);
        let read = started.elapsed();
        slowest = slowest.max(read);
        assert!(
            read <= SERVE_WITHIN,
            "a read of the tree took {read:?} while the shell was inside a call"
        );
        std::thread::sleep(window);
    });
    (took, slowest)
}

/// A script whose toolkit frame hook spins once for [`SPIN`], inside the frame's own call, which the frame budget (5 s
/// in the product, 60 s here) is what would cut. `trigger` is the condition that starts the spin.
fn spinner(trigger: &str) -> String {
    format!(
        "let spun = false;\n\
         const loaded = Date.now();\n\
         onUiFrame(() => {{\n\
         \x20   if (spun || !({trigger})) {{ return; }}\n\
         \x20   spun = true;\n\
         \x20   const end = Date.now() + {};\n\
         \x20   while (Date.now() < end) {{}}\n\
         }});",
        SPIN.as_millis()
    )
}

/// What a spin in a frame handler must leave: the session served and unlocked within the bound, by the card, while the
/// script was still spinning.
#[track_caller]
fn assert_served_during_the_spin(session: &Session, since: Instant, what: &str) {
    let (took, slowest) = serve_while_it_spins(session, since);
    eprintln!("{what}: unlocked {took:?} after it began, the slowest read of the tree {slowest:?}");
    assert!(
        took <= inside_the_spin(),
        "{what}: the session unlocked {took:?} after the spin began; the card should take the password within {:?}",
        inside_the_spin()
    );
    assert!(
        took < SPIN,
        "{what}: the session unlocked only after the spin ended, so the card did not take it"
    );
}

#[test]
fn a_script_that_spins_in_a_frame_handler_cannot_hold_the_lock_screen_or_the_compositor_past_the_bound() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // The spin starts 2.5 s after the script loaded, with the lock screen up and renewing its lease.
    let config = config_with(&spinner("Date.now() - loaded > 2500"));
    let mut shell = Shell::start_with(&session, config.path(), |config| {
        config.watch = false;
        config.shell.frame_budget = Duration::from_secs(60);
    });
    wait_for_the_screen(&session);
    // The latest the spin can start: from here the shell is inside the hook, and beats nothing.
    let spinning = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < spinning {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_served_during_the_spin(&session, spinning, "a spin in a frame handler");
    let problems = shell.finish();
    assert!(
        problems.iter().all(|line| !line.contains("hunter2")),
        "the shell reported: {problems:?}"
    );
}

#[test]
#[ignore = "reproduced open defect: with a window mapped under the lock, a shell that then spins holding the lock screen \
            is not replaced by the card within the bound; the same hang with the window open before the lock and the \
            same spin with no window pass (design/js-critique-status.md, 'Found on the way')"]
fn a_script_that_spins_when_a_window_arrives_cannot_hold_the_lock_screen_or_the_compositor_past_the_bound() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // A window arriving is the trigger, so the spin starts at a moment the test knows.
    let config = config_with(&spinner("gnoblin.clients.length > 0"));
    let mut shell = Shell::start_with(&session, config.path(), |config| {
        config.watch = false;
        config.shell.frame_budget = Duration::from_secs(60);
    });
    wait_for_the_screen(&session);
    std::thread::sleep(Duration::from_millis(1200));
    let mut mapper = Mapper::connect(&session.wayland());
    let spinning = Instant::now();
    mapper.map_window("org.example.trigger", "Trigger");
    assert_served_during_the_spin(&session, spinning, "a spin when a window arrives");
    let _ = shell.finish();
}

#[test]
fn a_shell_that_hangs_under_a_lock_with_a_window_open_still_gets_the_card() {
    let session = Session::start();
    // A session as it is when a user locks it: a window is open before the lock, and the shell has placed it.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.open", "Open");
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with("");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    shell.run_for(Duration::from_millis(700));
    // The shell hangs holding the screen, with the window still open.
    let hung = Instant::now();
    let took = the_password_is_still_needed(&session, hung, CARD_WITHIN, std::thread::sleep);
    assert!(
        took <= CARD_WITHIN + SLACK,
        "the card took the password {took:?} after the shell hung"
    );
    drop(shell);
}

#[test]
fn a_script_that_spins_in_a_reload_cannot_hold_the_lock_screen_or_the_compositor_past_the_bound() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with("");
    // The reload stages the new file in a generation of its own, on the role's thread, under the stage budget (30 s in the
    // product, 60 s here), and no frame runs meanwhile. The watcher is the trigger.
    let mut shell = Shell::start_with(&session, config.path(), |config| {
        config.shell.js.stage_budget = Duration::from_secs(60);
    });
    wait_for_the_screen(&session);
    std::thread::sleep(Duration::from_millis(1200));
    let saved = Instant::now();
    fs::write(
        config.path().join("init.ts"),
        format!(
            "{IMPORTS}const end = Date.now() + {}; while (Date.now() < end) {{}}\n",
            SPIN.as_millis()
        ),
    )
    .expect("init.ts");

    let (took, slowest) = serve_while_it_spins(&session, saved);
    assert!(
        took <= inside_the_spin(),
        "the session unlocked {took:?} after the reload began; the card should take the password within {:?} of the last beat",
        inside_the_spin()
    );
    assert!(
        took < SPIN,
        "the session unlocked only after the reload ended, so the card did not take it"
    );
    eprintln!("a spin in a reload: unlocked {took:?} after the save, the slowest read of the tree {slowest:?}");
    let _ = shell.finish();
}
