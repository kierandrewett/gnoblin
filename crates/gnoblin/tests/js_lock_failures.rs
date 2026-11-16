//! The lock screen when the shell that holds it fails, against a real compositor.
//!
//! The stock shell draws the lock screen and takes the password (`data/js/gnoblin-std/lock.ts`); the compositor's own card
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
//! in a frame handler, a script that spins in a reload, and one that spins when a window arrives. The spins run under
//! budgets far longer than the product's (`FRAME_BUDGET` is 5 s and `STAGE_BUDGET` 30 s) on purpose: the bound is a
//! property of the lease and not of the budget, so a test that cut the spin at the budget would prove less.
//!
//! A spin that does not happen proves nothing: the session unlocks through the shell's own screen all the same. So each
//! spin test reads [`Shell::stuck_for`], the time the shell's thread has been inside one tick, when the card has taken
//! the password, and fails unless the shell was still in the spin.
//!
//! The second half of the file is the windows: a window that arrives under the lock takes the keyboard with the stock
//! focus policy, and a healthy shell, which renews its lease on time, must not leave the session with nobody to type a
//! password to (`the_lock_screen_takes_the_keys`).
//!
//! What stays unproven here: the card's pixels (no test reads the output's picture) and a real process hung by a
//! signal over a full socket buffer, which `js_guardian.rs` does with `SIGSTOP`. The password check is the scripted
//! `FakeAuthenticator`, not PAM.

use std::fs;
use std::time::{Duration, Instant};

mod support;
use support::Mapper;
use support::lock_session::*;

/// What the scripts of these tests reach into: every export of the library on the global object (the lock screen, the
/// frame hook, the toolkit) next to the runtime, which is already a global and read-only.
const IMPORTS: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\n\
                       const { gnoblin: _runtime, ...exported } = lib;\n\
                       Object.assign(globalThis, exported);\n";

/// How long a read of the tree may take while the shell is busy. It takes milliseconds when nothing is wrong; this is
/// what a machine that other tests keep busy may take, and well under the seconds a compositor that waited for the shell
/// would take.
const SERVE_WITHIN: Duration = Duration::from_secs(2);

/// A config directory whose `shell.ts` is `IMPORTS` and `body`.
fn config_with(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a config directory");
    fs::write(dir.path().join("shell.ts"), format!("{IMPORTS}{body}\n")).expect("shell.ts");
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

/// What a spin must leave: the session served and unlocked within the bound, by the card, while the script was still
/// spinning. The shell's thread was inside one tick for the length of the spin when the card took the password, which
/// is read from outside ([`Shell::stuck_for`]): a script that never spun leaves the shell's own screen to take the
/// password, which passes every other assertion here.
#[track_caller]
fn assert_served_during_the_spin(session: &Session, shell: &Shell, since: Instant, what: &str) {
    let (took, slowest) = serve_while_it_spins(session, since);
    let stuck = shell.stuck_for();
    eprintln!(
        "{what}: unlocked {took:?} after it began, the shell inside one tick for {stuck:?}, the slowest read of the tree {slowest:?}"
    );
    assert!(
        took <= inside_the_spin(),
        "{what}: the session unlocked {took:?} after the spin began; the card should take the password within {:?}",
        inside_the_spin()
    );
    assert!(
        took < SPIN,
        "{what}: the session unlocked only after the spin ended, so the card did not take it"
    );
    assert!(
        stuck >= Duration::from_secs(1) && stuck + Duration::from_secs(2) >= took,
        "{what}: the shell had been inside one tick for only {stuck:?} when the session unlocked {took:?} after the spin \
         should have begun: the script did not spin, and the shell's own screen took the password"
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
    assert_served_during_the_spin(&session, &shell, spinning, "a spin in a frame handler");
    let problems = shell.finish();
    assert!(
        problems.iter().all(|line| !line.contains("hunter2")),
        "the shell reported: {problems:?}"
    );
    assert!(problems.is_empty(), "the script loaded and ran clean: {problems:?}");
}

#[test]
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
    assert_served_during_the_spin(&session, &shell, spinning, "a spin when a window arrives");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the script loaded and ran clean: {problems:?}");
}

#[test]
fn windows_that_arrive_and_go_while_the_shell_spins_do_not_move_the_card() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with(&spinner("Date.now() - loaded > 2500"));
    let mut shell = Shell::start_with(&session, config.path(), |config| {
        config.watch = false;
        config.shell.frame_budget = Duration::from_secs(60);
    });
    wait_for_the_screen(&session);
    let spinning = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < spinning {
        std::thread::sleep(Duration::from_millis(50));
    }
    // The shell is in the spin; windows come and go beneath it, a client that never draws among them. None of them is
    // heard by the shell, and none may postpone the card or leave the compositor waiting for the placement it asked for.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.first", "First");
    mapper.open_window_without_buffer("org.example.never", "Never");
    mapper.map_window("org.example.second", "Second");
    mapper.close_window(0);
    assert_served_during_the_spin(&session, &shell, spinning, "windows arriving and going in a spin");
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
        config.path().join("shell.ts"),
        format!(
            "{IMPORTS}const end = Date.now() + {}; while (Date.now() < end) {{}}\n",
            SPIN.as_millis()
        ),
    )
    .expect("shell.ts");

    assert_served_during_the_spin(&session, &shell, saved, "a spin in a reload");
    let _ = shell.finish();
}

// ---- a window that arrives under the lock ------------------------------------------------------------------------

/// How long a healthy shell may leave the keys without a taker after a window arrived: the screen takes the keyboard
/// back within a frame, and the card is the fallback after the compositor's grace. Nothing here waits for the grace.
const KEYS_BACK_WITHIN: Duration = Duration::from_secs(4);

/// The shell's own screen takes the keys: a wrong password is answered `Wrong password.` on it, in the tree, where only
/// the shell's screen can write it (the compositor's card answers on the card), and the right one unlocks. Each is
/// typed until it takes, as a user retries, within [`KEYS_BACK_WITHIN`].
#[track_caller]
fn the_lock_screen_takes_the_keys(session: &Session, what: &str) {
    let began = Instant::now();
    loop {
        type_a_wrong_password(session);
        std::thread::sleep(Duration::from_millis(300));
        let seen = view(&session.ipc());
        assert_eq!(seen.state, "locked", "{what}: a wrong password unlocked: {seen:?}");
        if seen.texts.iter().any(|text| text == "Wrong password.") {
            break;
        }
        assert!(
            began.elapsed() < KEYS_BACK_WITHIN,
            "{what}: no key typed in {KEYS_BACK_WITHIN:?} reached the shell's screen; the keyboard is with something else, \
             and the shell kept the lock screen with nobody to type into: {seen:?}"
        );
    }
    // Three wrong passwords in a row are rate limited, so the right one is typed until the limit has passed.
    let right = Instant::now();
    loop {
        type_the_password(session);
        std::thread::sleep(Duration::from_millis(400));
        if view(&session.ipc()).state == "not-locked" {
            return;
        }
        assert!(
            right.elapsed() < KEYS_BACK_WITHIN + SLACK,
            "{what}: the right password never unlocked the session: {:?}",
            view(&session.ipc())
        );
    }
}

/// A locked session whose shell, run in a thread, holds the lock screen.
fn locked_session_with_a_shell() -> (Session, tempfile::TempDir, Shell) {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with("");
    let shell = Shell::start(&session, config.path());
    wait_for_the_screen(&session);
    // Long enough for the screen to have renewed its lease: the shell is healthy from here on.
    std::thread::sleep(Duration::from_millis(700));
    (session, config, shell)
}

#[test]
fn a_window_that_arrives_under_the_lock_does_not_leave_the_session_with_nobody_to_type_a_password_to() {
    let (session, _config, mut shell) = locked_session_with_a_shell();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.arrival", "Arrival");
    the_lock_screen_takes_the_keys(&session, "a window arrived under the lock");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn many_windows_arriving_one_after_another_under_the_lock_do_not_either() {
    let (session, _config, mut shell) = locked_session_with_a_shell();
    let mut mapper = Mapper::connect(&session.wayland());
    for number in 0..5 {
        mapper.map_window("org.example.many", &format!("Window {number}"));
        std::thread::sleep(Duration::from_millis(120));
    }
    the_lock_screen_takes_the_keys(&session, "five windows arrived under the lock");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_window_that_arrives_and_is_closed_under_the_lock_leaves_the_keys_with_the_screen() {
    let (session, _config, mut shell) = locked_session_with_a_shell();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.gone", "Gone");
    std::thread::sleep(Duration::from_millis(400));
    // The window has had the focus, and the node that held it is destroyed: the seat's focus is left naming nothing.
    mapper.close_window(0);
    the_lock_screen_takes_the_keys(&session, "a window arrived under the lock and was closed");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_client_that_never_draws_does_not_take_the_keys_from_the_lock_screen() {
    let (session, _config, mut shell) = locked_session_with_a_shell();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.open_window_without_buffer("org.example.never", "Never");
    std::thread::sleep(Duration::from_millis(400));
    the_lock_screen_takes_the_keys(&session, "a client that never draws opened a window under the lock");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_client_that_never_commits_does_not_take_the_keys_from_the_lock_screen() {
    let (session, _config, mut shell) = locked_session_with_a_shell();
    let mut mapper = Mapper::connect(&session.wayland());
    // A toplevel the client created and then stopped on: it has no committed state at all, not even the empty first
    // commit of a client that has not drawn yet.
    mapper.open_window_without_commit("org.example.silent", "Silent");
    std::thread::sleep(Duration::from_millis(400));
    the_lock_screen_takes_the_keys(&session, "a client that never commits opened a window under the lock");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_shell_that_crashes_with_a_window_open_under_the_lock_leaves_the_card_to_take_the_password_at_once() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with("");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    let mut mapper = Mapper::connect(&session.wayland());
    let keys = mapper.keyboard();
    mapper.map_window("org.example.open", "Open");
    // The stock focus policy offers the window the focus, and the screen takes the keyboard back: the shell is healthy
    // with a window open under its lock when it fails.
    shell.run_for(Duration::from_millis(900));
    assert_eq!(view(&session.ipc()).state, "locked");
    shell.eval("__gnoblinHost.entry.ui = () => { throw new Error('the shell fails'); }; 0");
    shell.try_tick().expect_err("the frame fails and the role ends");
    drop(shell);
    let crashed = Instant::now();
    let took = the_password_is_still_needed(&session, crashed, Duration::ZERO, std::thread::sleep);
    assert!(
        took <= SLACK,
        "the card took the password {took:?} after the shell crashed with a window open"
    );
    mapper.roundtrip();
    assert!(
        keys.pressed().is_empty(),
        "the window under the lock heard the password typed at the card: {:?}",
        keys.pressed()
    );
}

#[test]
fn the_stock_focus_policy_racing_wrong_passwords_never_unlocks_and_leaves_somebody_to_type_to() {
    let (session, _config, mut shell) = locked_session_with_a_shell();
    let mut mapper = Mapper::connect(&session.wayland());
    // Each window maps while a wrong password is being typed: the policy writes the focus in the turn the keys arrive.
    for number in 0..2 {
        mapper.map_window("org.example.race", &format!("Race {number}"));
        type_a_wrong_password(&session);
        std::thread::sleep(Duration::from_millis(40));
        let seen = view(&session.ipc());
        assert_eq!(
            seen.state, "locked",
            "a wrong password raced a window and unlocked: {seen:?}"
        );
    }
    the_lock_screen_takes_the_keys(&session, "windows raced wrong passwords under the lock");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_window_that_maps_while_the_lock_is_being_taken_does_not_leave_the_session_with_nobody_to_type_a_password_to() {
    let session = Session::start();
    // The shell runs on an unlocked session, as it does when a user locks one.
    let config = config_with("");
    let mut shell = Shell::start(&session, config.path());
    std::thread::sleep(Duration::from_millis(1500));
    let mut mapper = Mapper::connect(&session.wayland());
    // The lock and the window race: the window maps in the turns in which the cover goes up and the screen engages.
    lock(&session);
    mapper.map_window("org.example.racing", "Racing");
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    wait_for_the_screen(&session);
    the_lock_screen_takes_the_keys(&session, "a window mapped as the lock was taken");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_window_that_was_open_before_the_lock_leaves_the_keys_with_the_lock_screen() {
    let session = Session::start();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("org.example.before", "Before");
    let config = config_with("");
    let mut shell = Shell::start(&session, config.path());
    std::thread::sleep(Duration::from_millis(1500));
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    wait_for_the_screen(&session);
    the_lock_screen_takes_the_keys(&session, "a window was open before the lock");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

#[test]
fn a_healthy_shell_that_keeps_the_keyboard_from_its_own_screen_still_gets_the_card() {
    // The shell is well: it renews its lease on time and never stops running frames. A script of the user's gives the
    // keyboard to a canvas outside the lock screen on every frame, after the screen has taken it back, so that the
    // keyboard is never on the screen when a key arrives. Nothing is spinning and nothing is hung: only the keys have
    // no taker, and the compositor's card takes them within its grace although the lease is fresh.
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = config_with(
        "const thief = new Box({ width: 10, height: 10 });\n\
         thief.mountTop();\n\
         onUiFrame(() => thief.focus());",
    );
    let mut shell = Shell::start(&session, config.path());
    wait_for_the_screen(&session);
    let lost = Instant::now();
    let (took, slowest) = {
        let ipc = session.ipc();
        let mut slowest = Duration::ZERO;
        let took = the_password_is_still_needed(
            &session,
            lost,
            Duration::from_millis(gnoblin_wayland::LOCK_UI_GRACE_MS),
            |window| {
                let started = Instant::now();
                let _ = view(&ipc);
                slowest = slowest.max(started.elapsed());
                std::thread::sleep(window);
            },
        );
        (took, slowest)
    };
    eprintln!(
        "a healthy shell that keeps the keyboard away: unlocked {took:?} after the screen came up, slowest read {slowest:?}"
    );
    assert!(
        took <= Duration::from_millis(gnoblin_wayland::LOCK_UI_GRACE_MS) + SLACK,
        "the card took the password {took:?} after the screen came up with the keyboard away from it"
    );
    assert!(
        shell.stuck_for() < Duration::from_secs(1),
        "the shell was never stuck, so it was the keys and not a spin that the card answered"
    );
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}
