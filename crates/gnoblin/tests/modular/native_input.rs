//! Example 28, run for real from Rust and from Python: the shipped binary on a headless backend serving a shell that binds key chords on the
//! seat's input context of the core. What is asserted is what the shell logs when the compositor tells it of a pressed chord, and that a
//! reload hands a chord from one generation to the next:
//!
//! - `Super+a` has two bindings. The one of priority 5 declines, so a press runs it and then the one of priority 0, in that order;
//! - the two share one claim, so three bindings stand on two claims;
//! - `Super+b` is handled by generation 1. After the shell is rebuilt (or its file saved again) generation 2 handles it, and generation 1
//!   does not hear it any more: the claim was handed over, with the compositor never holding the chord for nobody or for both.
//!
//! The tiers (device, layout, context), a handler object, scopes, contexts of a shell's own and the devices are tested against a fake
//! compositor in `crates/gnoblin-core-impl/tests/input.rs`, which also replays the fixtures of the TypeScript matcher.

use std::fs;
use std::path::Path;

use crate::support::native_shell::{
    Gnoblin, Home, KEY_A, KEY_B, Launch, Start, Typist, input_generations, shells, start_script,
};
use crate::{copy_example, save, serving};

/// How many log lines satisfy `matches`.
fn count(session: &Gnoblin, matches: impl Fn(&str) -> bool) -> usize {
    session.log().iter().filter(|line| matches(&line.text)).count()
}

/// The position of the `nth` (from 0) log line that contains `needle`.
fn position(session: &Gnoblin, needle: &str, nth: usize) -> Option<usize> {
    session
        .log()
        .iter()
        .enumerate()
        .filter(|(_, line)| line.text.contains(needle))
        .nth(nth)
        .map(|(at, _)| at)
}

/// How many claims the three bindings of the shell stand on: the two of `Super+a` share one.
const CLAIMS: usize = 2;

/// The shell of generation `n` (from 1) has bound its chords and the compositor holds them: the shell says how many claims its three bindings
/// stand on and that it runs, and the compositor says it applied the claims of every generation so far and the releases of the ones before.
///
/// The shell's own lines are not enough. A script runs while its generation is only staged, so it says "running" before the old generation
/// ended, and what the new one claims goes to the compositor when it takes over, in one transaction behind the old one's releases, which
/// the compositor takes in a frame of its own. A chord pressed on the shell's line is the old generation's, or nobody's.
#[track_caller]
fn assert_generation_runs(session: &mut Gnoblin, n: usize) {
    let holds = session.wait_log("the shell counts its claims", |line| {
        line.contains(&format!("input: generation {n} holds"))
    });
    assert!(
        holds.text.contains(&format!("holds {CLAIMS} claims for 3 bindings")),
        "{}",
        holds.text
    );
    session.wait_log("the shell is running", |line| {
        line.contains(&format!("input: generation {n} is running"))
    });
    session.wait_claims(CLAIMS * n, CLAIMS * (n - 1));
}

/// A press of `Super+a` runs the binding that declines and then the one that handles it, for the `nth` time (from 0).
#[track_caller]
fn assert_a_press_runs_both(session: &mut Gnoblin, typist: &mut Typist, nth: usize) {
    typist.super_plus(KEY_A);
    session.wait_for("the press runs both bindings", |session| {
        position(session, "input: low handles Super+a", nth).is_some()
    });
    let high = position(session, "input: high declines Super+a", nth).expect("the first binding ran");
    let low = position(session, "input: low handles Super+a", nth).expect("the second binding ran");
    assert!(
        high < low,
        "the binding of the higher priority runs first\n{}",
        session.log_text()
    );
}

/// A press of `Super+b` is handled by generation `n`, once.
#[track_caller]
fn assert_generation_handles(session: &mut Gnoblin, typist: &mut Typist, n: u32) {
    let line = format!("input: generation {n} handles Super+b");
    let before = count(session, |text| text.contains(&line));
    typist.super_plus(KEY_B);
    session.wait_for("the generation handles the chord", |session| {
        count(session, |text| text.contains(&line)) > before
    });
    assert_eq!(
        count(session, |text| text.contains(&line)),
        before + 1,
        "one press, one run"
    );
}

/// The assertions both languages share: generation 1, a reload, generation 2.
fn assert_the_chords_are_handed_over(session: &mut Gnoblin, reload: impl FnOnce()) {
    assert_generation_runs(session, 1);
    let mut typist = Typist::connect(&session.wayland());
    assert_a_press_runs_both(session, &mut typist, 0);
    assert_generation_handles(session, &mut typist, 1);

    reload();
    assert_generation_runs(session, 2);
    assert_generation_handles(session, &mut typist, 2);
    assert_eq!(
        count(session, |text| text.contains("input: generation 1 handles Super+b")),
        1,
        "generation 1 does not hear the chord again\n{}",
        session.log_text()
    );
    assert_a_press_runs_both(session, &mut typist, 1);
    assert_eq!(
        count(session, |text| text.contains("input: low handles Super+a")),
        2,
        "the new generation's bindings run once for each press, and the old generation's not at all"
    );
    assert!(session.exited().is_none(), "{}", session.log_text());
}

/// Replaces `live` with `from` the way a linker's last step does: a whole file appears under the name at once.
fn swap_in(from: &Path, live: &Path) {
    let part = live.with_extension("part");
    fs::copy(from, &part).expect("the replacement is written");
    fs::rename(&part, live).expect("the replacement takes the name");
}

#[test]
fn example_28_a_rust_shell_binds_chords_and_a_reload_hands_one_to_the_next_generation() {
    let built = input_generations();
    // The shell the workspace build made is generation 1 of the same source.
    assert!(shells().input.exists(), "the example builds with the other shells");
    let home = Home::new();
    let live = home.copy(&built.v1, "live/libnative_input.so");
    let mut launch = Launch::compositor(&["--watch"]);
    launch.args.push(live.clone().into_os_string());
    let mut session = Gnoblin::launch(home, launch);
    session.wait_serving();
    assert_the_chords_are_handed_over(&mut session, || swap_in(&built.v2, &live));
}

#[test]
fn example_28_a_python_shell_binds_chords_and_a_reload_hands_one_to_the_next_generation() {
    let dir = copy_example("28-native-input-py");
    let path = dir.path().join("shell.py");
    let mut session = start_script(Start::Compositor, &path, &[]);
    serving(&mut session);
    let text = fs::read_to_string(&path).expect("the shell");
    assert_the_chords_are_handed_over(&mut session, || {
        let next = text.replace("GENERATION = 1", "GENERATION = 2");
        assert_ne!(next, text, "the example has the constant this test edits");
        save(&path, &next);
    });
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
