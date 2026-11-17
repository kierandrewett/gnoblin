//! A native shell in the compositor process, when it fails (`design/modular-framework.md`, M3.5 and M3.6).
//!
//! The shell runs on a thread of the compositor process, so it can fail in ways a separate process cannot take the compositor
//! down with. What each test pins is what the session does while it has:
//!
//! 1. **A hung callback does not stall the compositor or the lock.** The shell's thread is stuck inside a callback for good. The
//!    compositor's tree keeps answering, and the session lock, which the shell held by renewing a lease, goes to the built-in
//!    card when the lease lapses. The time is measured from this test, on its own clock, from the moment the shell wrote that
//!    it hung to the moment the card's line was read: it is not the shell's own measure (the shell cannot be asked anything).
//! 2. **A panic in a callback is contained.** The shell's next callback runs, and the next window is placed by it.
//! 3. **A call from a retired generation does nothing.** A thread the shell started calls the core after the shell asked to
//!    exit and was retired; the core answers `Retired` and the session goes on.
//! 4. **A shell that ends does not end the session.** Its tree is detached and the compositor's own window management takes the
//!    windows; the guardian is told, as a warning, and does not end the session or restart the compositor.
//! 5. **A shell that cannot load is the same.** The `init` that fails is logged with its reason and the session runs without it.

use std::time::{Duration, Instant};

mod support;
use support::lock_session::{CARD_WITHIN, SLACK, view};
use support::native_shell::*;
use support::{Mapper, scene, take_lock};

/// What a read of the compositor's tree may take while the shell is stuck. It takes milliseconds when nothing is wrong; this is
/// what a machine that other tests keep busy may take, and well under the seconds a compositor that waited for the shell would.
const SERVE_WITHIN: Duration = Duration::from_secs(2);

/// Where the fixture shell puts the nth window it places.
fn shell_place(n: u32) -> (f64, f64) {
    (300.0 + 20.0 * f64::from(n), 200.0)
}

fn placed_by_the_shell(window: &support::Window, n: u32) -> bool {
    let (x, y) = shell_place(n);
    (window.x - x).abs() < 0.5 && (window.y - y).abs() < 0.5
}

fn started(start: Start, prepared: &[(&str, &str)]) -> Gnoblin {
    let mut session = Gnoblin::start(start, &shells().fault, &[], prepared);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    session.wait_noted("started");
    session
}

#[test]
fn a_hung_callback_lets_the_lock_lease_lapse_and_the_card_takes_over_within_the_bound() {
    let mut session = started(Start::Compositor, &[]);
    let ipc = session.ipc();

    // The shell holds the lock screen: it engages on the lock and renews its lease, as the stock screen does.
    take_lock(&ipc);
    session.wait_for("the shell holds the lock screen", |_| {
        let seen = view(&ipc);
        seen.state == "locked" && seen.engaged
    });
    std::thread::sleep(Duration::from_millis(900));
    assert!(
        session
            .find(|line| line.contains("the built-in lock card is up"))
            .is_none(),
        "the card is up while the shell holds the screen:\n{}",
        session.log_text()
    );

    // The callback never returns. `since` is read after the shell wrote that it hung, so it is never earlier than the hang.
    session.command("hang", "");
    session.wait_noted("hanging");
    let since = Instant::now();

    // The compositor does not wait for the shell: its tree answers a new connection, and a client maps a window, while the
    // shell's thread is inside the callback.
    let answered = Instant::now();
    let windows = scene(&ipc);
    assert!(
        answered.elapsed() < SERVE_WITHIN,
        "the tree took {:?} to answer",
        answered.elapsed()
    );
    assert!(windows.is_empty(), "{windows:?}");
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("during", "During the hang");
    session.wait_for(
        "the compositor shows the window the hung shell cannot place",
        |session| scene(&session.ipc()).iter().any(|window| window.app_id == "during"),
    );

    // The lease lapses two seconds after the last beat (the shell beat at most 400 ms before it hung) and the card follows a second
    // after the lapse: between the two seconds, and the bound with the slack a loaded machine needs.
    let card = session.wait_log("the built-in card takes the lock", |line| {
        line.contains("no shell holds the lock screen: the built-in lock card is up")
    });
    let took = card.at.duration_since(since);
    assert!(
        took >= Duration::from_secs(2),
        "the card took over {took:?} after the hang, before the lease could have lapsed"
    );
    assert!(
        took <= CARD_WITHIN + SLACK,
        "the card took {took:?} after the hang, past the bound of {:?}",
        CARD_WITHIN + SLACK
    );
    assert_eq!(
        view(&ipc).state,
        "locked",
        "the session stays locked while the shell hangs"
    );
    assert!(
        session.exited().is_none(),
        "the compositor runs on:\n{}",
        session.log_text()
    );
}

#[test]
fn a_panic_in_a_callback_is_contained_and_the_next_event_is_delivered() {
    let mut session = started(Start::Compositor, &[]);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("before", "Before");
    wait_for_window(
        &mut session,
        "the shell places the window before the panic",
        "before",
        |window| placed_by_the_shell(window, 0),
    );
    let before: u64 = session.noted("ticks").and_then(|ticks| ticks.parse().ok()).unwrap_or(0);

    session.command("panic", "");
    session.wait_noted("panicking");
    session.wait_log("the core records the contained panic", |line| {
        line.contains("panicked; contained, the next event is still delivered")
    });

    // The timer that panicked fires again, and the shell still places a window that opens afterwards.
    session.wait_for("the shell's timer still runs after the panic", |session| {
        session
            .noted("ticks")
            .and_then(|ticks| ticks.parse::<u64>().ok())
            .is_some_and(|ticks| ticks > before + 3)
    });
    mapper.map_window("after", "After");
    wait_for_window(
        &mut session,
        "the shell still places windows after the panic",
        "after",
        |window| placed_by_the_shell(window, 1),
    );
    assert!(
        session.exited().is_none(),
        "the compositor runs on:\n{}",
        session.log_text()
    );
}

#[test]
fn a_call_from_a_generation_that_retired_is_refused_and_the_session_goes_on() {
    let mut session = started(Start::Compositor, &[]);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("before", "Before");
    wait_for_window(&mut session, "the shell places the window", "before", |window| {
        placed_by_the_shell(window, 0)
    });

    // The shell asks to exit cleanly, which retires its generation. A thread it started waits for that and then calls the core.
    session.command("exit", "0");
    let late = session.wait_noted("late-call");
    assert!(
        late.contains("Retired"),
        "the late call was answered with `{late}`, not Retired"
    );
    session.wait_log("the generation was retired", |line| line.contains("retired generation"));

    // The core did not take the call, and the compositor serves: the shell is gone from the tree, so the compositor's own window
    // management places the next window, and it is not where the shell would have put it.
    mapper.map_window("after", "After");
    wait_for_window(
        &mut session,
        "the compositor places the window the shell no longer sees",
        "after",
        |window| !placed_by_the_shell(window, 1) && !placed_by_the_shell(window, 0),
    );
    assert!(
        session.exited().is_none(),
        "the compositor runs on:\n{}",
        session.log_text()
    );
}

#[test]
fn a_shell_that_ends_does_not_end_the_session_and_the_guardian_is_told() {
    let mut session = started(Start::Guardian, &[]);
    session.wait_log("the session is ready", |line| line.contains("[guardian] session ready"));
    session.wait_log("the shell accepted its configuration", |line| {
        line.contains("[guardian] configuration revision 0 accepted")
    });
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("before", "Before");
    wait_for_window(&mut session, "the shell places the window", "before", |window| {
        placed_by_the_shell(window, 0)
    });

    // The shell fails: it asks to exit with a code that is an error. The thread ends, the tree is detached and the guardian is told.
    session.command("exit", "3");
    session.wait_log("the compositor logs the shell's end", |line| {
        line.contains(
            "the in-process shell ended with an error, so the tree is detached and the session continues without it",
        )
    });
    session.wait_log("the guardian is told the shell failed", |line| {
        line.contains(
            "[guardian] configuration revision 0 rejected: the shell failed: the shell asked to exit with code 3",
        )
    });

    // The session is still up: the compositor was not restarted and nothing ended the session, and it places the next window.
    mapper.map_window("after", "After");
    wait_for_window(&mut session, "the compositor places the window", "after", |window| {
        !placed_by_the_shell(window, 1) && !placed_by_the_shell(window, 0)
    });
    let log = session.log_text();
    assert_eq!(
        log.matches("started compositor (").count(),
        1,
        "the compositor was restarted:\n{log}"
    );
    assert!(!log.contains("session ended"), "the session ended:\n{log}");
    assert!(
        !log.contains("[guardian] compositor exited"),
        "the compositor exited:\n{log}"
    );
    assert!(session.exited().is_none(), "the guardian runs on:\n{log}");
}

#[test]
fn a_shell_that_cannot_load_is_logged_with_its_reason_and_the_session_runs_without_it() {
    let mut session = Gnoblin::start(Start::Guardian, &shells().fault, &[], &[("fail-init", "")]);
    session.wait_log("the session is ready", |line| line.contains("[guardian] session ready"));
    session.wait_log("the shell's init failure is logged with its reason", |line| {
        line.contains("init failed") && line.contains("the fixture was told to fail its init")
    });
    session.wait_log("the guardian is told", |line| {
        line.contains("[guardian] configuration revision 0 rejected: the shell failed:")
    });
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("window", "Window");
    wait_for_window(&mut session, "the compositor places the window", "window", |window| {
        !placed_by_the_shell(window, 0)
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}
