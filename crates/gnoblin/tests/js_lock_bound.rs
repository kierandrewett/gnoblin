//! The takeover bounds of `docs/session-lock.md`, measured against a real compositor.
//!
//! The page says the card takes the password at most 3 seconds (a 2 second lease and 1 second of grace) after the last
//! healthy beat of the shell, and at most 1 second after the keyboard left the shell's screen, each plus [`TAIL`]. The
//! other lock tests allow [`SLACK`] on top of the bounds so that a loaded machine does not fail them; these assert the
//! bounds themselves and print what they measured.
//!
//! How each time is taken, all from outside the compositor:
//!
//! - The last healthy beat is the instant the shell's link wrote its last `heartbeat` call (`Forged::beats`), taken
//!   before the write. The compositor cannot have seen the beat earlier, so the measured age is never smaller than the
//!   real one.
//! - The keyboard leaves the screen when the shell's link writes the first `focus` that names a node other than the
//!   entry it had given the focus to, again taken before the write. The first key is typed 150 ms after that, so that
//!   the write has reached the compositor: a key that beat it would find the keyboard on the screen and be the screen's.
//! - The password is typed every [`POLL`] and the session is read after each, so the instant it is seen unlocked is
//!   at most one poll and one read after the unlock.
//!
//! The end of each interval is the password accepted, as the tree shows it, and not the card's first frame, because the
//! card is not a node and no test reads its pixels. [`TAIL`] is what lies between the lease's lapse and that moment:
//! the session's clock moves on a loop turn of at most 100 ms, and the lapse and the grace are each noticed on a turn;
//! the card's pump runs every 50 ms and draws the card, takes the keys and sends the check; the answer is applied on
//! the next turn; and the test polls.
//!
//! Each scenario runs `GNOBLIN_LOCK_BOUND_RUNS` times (three by default) on a fresh session, and prints the minimum,
//! the median and the maximum. Run it in a release build, one test at a time, for numbers worth quoting:
//! `GNOBLIN_LOCK_BOUND_RUNS=20 cargo test --release -p gnoblin --test js_lock_bound -- --nocapture --test-threads=1`.

use std::fs;
use std::time::{Duration, Instant};

mod support;
use support::lock_session::*;

/// How often the right password is typed while the card is awaited.
const POLL: Duration = Duration::from_millis(25);

/// What the measurement adds when it asks whether the card came too early: one poll, one read of the tree and the
/// scheduling of the test's own thread.
const RESOLUTION: Duration = Duration::from_millis(125);

/// What the card may take beyond the lease and the grace, as the page states it: half a second in a release build,
/// which is the product's, and a second in a debug build, which is what `cargo test` runs. Twenty runs in a release
/// build ended 0.31 to 0.40 s after the lease and the grace, and 0.19 to 0.31 s after the grace alone; one debug run
/// took 0.51 s.
const TAIL: Duration = Duration::from_millis(if cfg!(debug_assertions) { 1_000 } else { 500 });

/// The card takes the password this long after the last healthy beat at the latest, before [`TAIL`].
const AFTER_THE_BEAT: Duration =
    Duration::from_millis(gnoblin_wayland::LOCK_UI_LEASE_MS + gnoblin_wayland::LOCK_UI_GRACE_MS);

/// The card takes the password this long after the keyboard left the screen at the latest, before [`TAIL`].
const AFTER_THE_KEYBOARD: Duration = Duration::from_millis(gnoblin_wayland::LOCK_UI_GRACE_MS);

fn runs() -> usize {
    std::env::var("GNOBLIN_LOCK_BOUND_RUNS")
        .ok()
        .and_then(|runs| runs.parse().ok())
        .unwrap_or(3)
}

/// Prints the minimum, the median and the maximum of `took`, in the form the docs quote them.
fn report(what: &str, bound: Duration, mut took: Vec<Duration>) {
    took.sort();
    let (min, median, max) = (took[0], took[took.len() / 2], took[took.len() - 1]);
    eprintln!(
        "{what}: {} runs, min {min:?}, median {median:?}, max {max:?}; bound {bound:?} + {TAIL:?} ({})",
        took.len(),
        if cfg!(debug_assertions) {
            "debug build"
        } else {
            "release build"
        }
    );
}

/// A locked session whose shell, driven by the test, has engaged the lock and renewed its lease.
fn locked_with_a_driven_shell(init: &str) -> (Session, tempfile::TempDir, Driven) {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), init).expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    // Long enough for the screen to have renewed its lease a few times: the shell is healthy up to here.
    shell.run_for(Duration::from_millis(1200));
    assert_eq!(view(&session.ipc()).state, "locked");
    (session, config, shell)
}

/// Types the right password every [`POLL`] until the session is seen unlocked, and returns when. `between` passes the
/// time: it ticks a shell that is alive, and sleeps for one that is not.
#[track_caller]
fn until_unlocked(session: &Session, patience: Duration, mut between: impl FnMut(Duration)) -> Instant {
    let deadline = Instant::now() + patience;
    loop {
        type_the_password(session);
        between(POLL);
        if view(&session.ipc()).state == "not-locked" {
            return Instant::now();
        }
        assert!(
            Instant::now() < deadline,
            "nobody took the right password in {patience:?}"
        );
    }
}

/// Fails unless every run in `took` is within `bound` and [`TAIL`]. It is called after the report, so a run that is over
/// still has its numbers printed with the others.
#[track_caller]
fn within_the_bound(what: &str, bound: Duration, took: &[Duration]) {
    for (run, took) in took.iter().enumerate() {
        assert!(
            *took <= bound + TAIL,
            "run {run}: the card took the password {took:?} {what}; the page says at most {bound:?} + {TAIL:?}"
        );
    }
}

#[test]
fn the_card_takes_the_password_within_the_documented_bound_after_the_last_healthy_beat() {
    let mut measured = Vec::new();
    for run in 0..runs() {
        let (session, _config, shell) = locked_with_a_driven_shell("export {};\n");
        let beat = *shell
            .forged
            .borrow()
            .beats
            .last()
            .expect("a healthy shell renewed its lease");
        // The shell is attached and engaged and never runs another frame: it renews nothing, and the card is the only
        // one left to take the password.
        let unlocked = until_unlocked(&session, AFTER_THE_BEAT + Duration::from_secs(5), std::thread::sleep);
        let took = unlocked.duration_since(beat);
        eprintln!("run {run}: the card took the password {took:?} after the last healthy beat");
        // It cannot have come before the lease lapsed: a card that took the screen from a shell that is renewing would
        // be the failure the other way.
        assert!(
            took + RESOLUTION >= Duration::from_millis(gnoblin_wayland::LOCK_UI_LEASE_MS),
            "run {run}: the card took the password {took:?} after the last beat, before the lease could have lapsed"
        );
        measured.push(took);
        drop(shell);
    }
    report("the card after the last healthy beat", AFTER_THE_BEAT, measured.clone());
    within_the_bound("after the last healthy beat", AFTER_THE_BEAT, &measured);
}

#[test]
fn the_card_takes_the_password_within_the_documented_bound_after_the_keyboard_leaves_the_screen() {
    // A healthy shell: it ticks throughout and renews its lease on time. A script of the user's gives the keyboard to a
    // canvas outside the lock screen on every frame once told to, after the screen has taken it back.
    const INIT: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\n\
                        const { gnoblin: _runtime, ...exported } = lib;\n\
                        Object.assign(globalThis, exported);\n\
                        const thief = new Box({ width: 10, height: 10 });\n\
                        thief.mountTop();\n\
                        let stealing = false;\n\
                        (globalThis as any).steal = () => { stealing = true; };\n\
                        onUiFrame(() => { if (stealing) thief.focus(); });\n";
    let mut measured = Vec::new();
    for run in 0..runs() {
        let (session, _config, mut shell) = locked_with_a_driven_shell(INIT);
        let entry = shell
            .forged
            .borrow()
            .focus_writes
            .last()
            .expect("the screen gave the keyboard to its entry")
            .1;
        let beats_before = shell.forged.borrow().beats.len();
        shell.eval("steal(); 0");
        let left_at = |shell: &Driven| {
            shell
                .forged
                .borrow()
                .focus_writes
                .iter()
                .find(|(_, node)| *node != entry)
                .map(|(at, _)| *at)
        };
        shell.until("the script took the keyboard from the entry", |shell| {
            left_at(shell).is_some()
        });
        let left = left_at(&shell).expect("the keyboard left the entry");
        // The write has reached the compositor before the first key is typed: a key that beat it would find the
        // keyboard on the screen and be the screen's, which is no failure of the screen.
        shell.run_for(Duration::from_millis(150));
        let unlocked = until_unlocked(&session, AFTER_THE_KEYBOARD + Duration::from_secs(5), |window| {
            shell.run_for(window)
        });
        let took = unlocked.duration_since(left);
        eprintln!("run {run}: the card took the password {took:?} after the keyboard left the screen");
        // The shell was healthy the whole time, so it was the keys and not a lapsed lease that the card answered.
        assert!(
            shell.forged.borrow().beats.len() > beats_before,
            "run {run}: the shell stopped renewing its lease, so the lease and not the keyboard was measured"
        );
        measured.push(took);
    }
    report(
        "the card after the keyboard left the screen",
        AFTER_THE_KEYBOARD,
        measured.clone(),
    );
    within_the_bound("after the keyboard left the screen", AFTER_THE_KEYBOARD, &measured);
}
