//! The built-in lock card against the real compositor: a locked session whose shell does not hold the lock screen
//! still takes a password and unlocks.
//!
//! Nothing here is a fake but the password check, which is the scripted `FakeAuthenticator` the other lock tests use in
//! place of PAM (`accept("kieran", "hi")`). The session is [`Compositor::run`] on a headless backend; the lock is taken
//! through the tree (an observer's `lock` call, as the screensaver service does); the keys are injected on the seat path
//! of a real keyboard (`InputTap`); the lock's state is read back over a tree observer connection. The shell is a raw
//! tree connection that holds the tree, makes a canvas under the lock node with the seat's keyboard focus on it and
//! writes the lock node's `engaged`, as the stock lock screen does, because this crate does not link the JavaScript
//! shell (`crates/gnoblin/tests/js_lock.rs` runs that one).
//!
//! The card's pixels are not read here, and nothing in the tree says it is up: what these tests prove is its effect.
//! Keys typed at a locked session with no shell go nowhere unless the card takes them, so a password that unlocks the
//! session was typed into the card; and a card that is gone takes nothing. Typing is repeated until the state changes,
//! because the card appears a turn or two after the lock is proven and a key typed before that is dropped.
//!
//! 1. No shell: the right password unlocks.
//! 2. A wrong password keeps the session locked, and the card still takes the right one after it.
//! 3. A shell that holds the lock screen keeps the card away, and one that dies while locked brings it up.
//! 4. A shell that attaches while the card is up takes over: the card is gone, and the shell's own `authenticate` unlocks.
//! 5. A shell that engaged the lock and then stops renewing its lease (hung, or its screen failing) loses the lock UI:
//!    the card takes the password although the shell is still attached and still `engaged`.
//! 6. A shell that renews its lease on time but whose keyboard is on a node outside its lock screen (a window that
//!    arrived under the lock took the focus, or a script moved it) loses the lock UI too, within the grace: nobody
//!    could type a password into a screen no key reaches.
//! 7. The card gives the keys back when the shell takes the keyboard to its screen again.
//! 8. A client that stops reading stalls neither the session nor the card: the dispatcher never waits for a socket. It
//!    drops a client that took nothing for five seconds, or that has more than 32 MiB waiting, and a client that reads
//!    late but inside the limit hears every change, in order (`ipc/outbox.rs` holds the unit tests of the outbox).

mod support;

use std::time::{Duration, Instant};

use gnoblin_proto::tree::op;
use support::Session;
use support::lock::{
    KEY_H, KEY_I, KEY_N, KEY_O, PASSWORD, Peer, attach, lock, start, type_and_enter, type_right_until_unlocked,
};

#[test]
fn with_no_shell_the_card_takes_the_right_password_and_unlocks_the_session() {
    let session = start();
    let mut observer = lock(&session);
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(15)),
        "a locked session with no shell never unlocked: the lock cover has nothing to type into"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_wrong_password_keeps_the_session_locked_and_the_card_still_takes_the_right_one() {
    let session = start();
    let mut observer = lock(&session);
    // Wrong passwords for a few seconds. The check's limiter answers `rate-limited` after three, which the card shows.
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        type_and_enter(&session, &[KEY_N, KEY_O]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(observer.state, "locked", "a wrong password never unlocks");
    }
    // After the limiter's wait the right one gets through.
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(20)),
        "the card took no password after a wrong one"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_that_holds_the_lock_screen_keeps_the_card_away_and_one_that_dies_brings_it_up() {
    let session = start();
    let mut holder = attach(&session, "holder");
    holder.hold_the_keyboard_on_a_screen();
    holder.engage(true);
    let mut observer = lock(&session);
    // The shell's screen is up: the card is not, so a typed password goes to the shell's (absent) entry and nowhere else.
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        holder.heartbeat();
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must not draw over a shell that holds the lock screen"
        );
    }
    // The shell dies while the session is locked: the card is up at once and takes the password.
    drop(holder);
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(15)),
        "the shell died and nothing took its place: the locked session has nothing to type into"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_that_attaches_while_the_card_is_up_takes_over_and_unlocks_through_its_own_call() {
    let session = start();
    let mut observer = lock(&session);
    // The card has been up since the cover was proven; give the session a few turns to show it.
    observer.watch(Duration::from_millis(1500));
    let mut holder = attach(&session, "holder");
    holder.hold_the_keyboard_on_a_screen();
    holder.engage(true);
    observer.watch(Duration::from_millis(500));
    // The card is gone: the right password typed at the keyboard now goes to the shell's (absent) entry, not the card.
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        holder.heartbeat();
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must not outlive the shell's takeover"
        );
    }
    // The shell unlocks through the call the stock lock screen makes.
    let reply = holder.call("authenticate", PASSWORD);
    assert!(reply.applied, "the holder asks for the check: {reply:?}");
    observer.wait_for_state("not-locked");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_that_engaged_the_lock_and_stopped_renewing_its_lease_loses_it_to_the_card() {
    let session = start();
    let mut holder = attach(&session, "holder");
    holder.hold_the_keyboard_on_a_screen();
    holder.engage(true);
    let mut observer = lock(&session);
    // While it renews, the shell's screen is the lock UI and the card stays away.
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        holder.heartbeat();
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must not draw over a shell with a live lease"
        );
    }
    // The shell stays attached and `engaged` and sends nothing more: its lease lapses, and a second later the card
    // takes the password. A shell that only claims the lock UI holds nothing.
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(20)),
        "the shell stopped renewing and the card never took the lock UI over: the session has nothing to type into"
    );
    drop(holder);
    session.stop().expect("the session stops cleanly");
}

/// How long the card may take to take the keys from a shell whose lease is fresh but whose screen has no keyboard: the
/// compositor's grace, and the turns of the session loop and of the card's pump on either side of it.
const KEYS_LOST_WITHIN: Duration = Duration::from_millis(gnoblin_wayland::LOCK_UI_GRACE_MS + 1_500);

#[test]
fn a_shell_that_renews_its_lease_but_has_lost_the_keyboard_loses_the_lock_ui_to_the_card() {
    let session = start();
    let mut holder = attach(&session, "holder");
    holder.hold_the_keyboard_on_a_screen();
    holder.engage(true);
    let mut observer = lock(&session);
    // With the keyboard on its screen the shell holds the lock UI and the card stays away.
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        holder.heartbeat();
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must not draw over a shell that holds the keyboard"
        );
    }
    // A window arrives under the lock and the stock policy gives it the focus, or a script moves the focus: the shell
    // gives the keyboard to a node that is no part of its lock screen, and goes on renewing its lease on time. A healthy
    // shell is no guarantee that anybody can type a password, so the card takes the keys within the grace.
    let root = holder.root;
    let elsewhere = holder.canvas_under(root);
    holder.focus(elsewhere);
    let lost = Instant::now();
    let mut unlocked = false;
    while lost.elapsed() < KEYS_LOST_WITHIN + Duration::from_secs(5) && !unlocked {
        holder.heartbeat();
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        unlocked = observer.state == "not-locked";
    }
    assert!(
        unlocked,
        "the shell renewed its lease and held no keyboard, and the card never took the keys: nobody can unlock"
    );
    assert!(
        lost.elapsed() <= KEYS_LOST_WITHIN + Duration::from_secs(1),
        "the card took the keys {:?} after the shell lost the keyboard",
        lost.elapsed()
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_card_that_took_the_keys_gives_them_back_when_the_shell_takes_the_keyboard_again() {
    let session = start();
    let mut holder = attach(&session, "holder");
    let entry = holder.hold_the_keyboard_on_a_screen();
    holder.engage(true);
    let mut observer = lock(&session);
    let root = holder.root;
    let elsewhere = holder.canvas_under(root);
    holder.focus(elsewhere);
    // The card is up by now and nothing is typed at it.
    let until = Instant::now() + KEYS_LOST_WITHIN;
    while Instant::now() < until {
        holder.heartbeat();
        observer.watch(Duration::from_millis(300));
    }
    assert_eq!(observer.state, "locked");
    // The shell's screen has the keyboard again: the card is gone, so the right password goes to the shell's (absent)
    // entry and unlocks nothing.
    holder.focus(entry);
    observer.watch(Duration::from_millis(500));
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        holder.heartbeat();
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must give the keys back to a shell that holds the keyboard again"
        );
    }
    session.stop().expect("the session stops cleanly");
}

/// How long a client may take nothing of what the session holds for it before the session drops the connection
/// (`ipc/outbox.rs`, `STALL_LIMIT`).
const STALL_LIMIT: Duration = Duration::from_secs(5);

/// The longest one request may wait for its answer while a client that stopped reading is being fed. The dispatcher that
/// serves every connection, the lock card among them, must never wait for a client's socket, so this is a few turns of
/// the dispatcher's own tick (50 ms) and not the seconds a write to a full socket takes.
const STALL_BOUND: Duration = Duration::from_millis(1_500);

/// The longest a client that reads is given to hear everything the session kept for it. A client that takes something
/// is never dropped for [`STALL_LIMIT`], so this bounds only the test when a change really was lost.
const HEARING_LIMIT: Duration = Duration::from_secs(30);

/// A paint list of 60 long texts, a little under the 1 MiB a `content` may hold. The first text says which list it is,
/// so no two are equal and each is a change the compositor publishes to every observer.
fn large_paint_list(which: u16) -> Vec<u8> {
    use gnoblin_paint::{PaintList, PaintOp, Rgba8};
    let text = |first: String| PaintOp::Text {
        x: 0.0,
        y: 0.0,
        size: 13.0,
        colour: Rgba8::new(10, 20, 30, 255),
        font: "sans".to_owned(),
        text: first + &"x".repeat(16_000),
    };
    let mut ops = vec![text(format!("{which:05}"))];
    ops.extend((1..60).map(|_| text(String::new())));
    gnoblin_paint::encode(&PaintList { ops })
}

/// What feeding a canvas told: the slowest answer to one change, and the serial each change was applied at.
struct Fed {
    slowest: Duration,
    serials: Vec<u64>,
}

/// Changes the canvas `fed` `changes` times (960 KB each) through the holder, each answered before the next, as a shell
/// that draws does, while the locker's own observer keeps reading: only a client that does not read may fill. `slow`,
/// when there is one, takes 64 KB of what is held for it after each change, so that it is never without progress.
fn feed(holder: &mut Peer, fed: u64, reading: &mut Peer, changes: u16, mut slow: Option<&mut Peer>) -> Fed {
    let mut told = Fed {
        slowest: Duration::ZERO,
        serials: Vec::new(),
    };
    for which in 0..changes {
        let asked = Instant::now();
        holder.apply(&[op::Properties {
            kind: "set".to_owned(),
            node: fed,
            key: "content".to_owned(),
            value_kind: "bytes".to_owned(),
            value_bytes: large_paint_list(which),
            ..Default::default()
        }]);
        told.slowest = told.slowest.max(asked.elapsed());
        told.serials.push(holder.serial);
        reading.watch(Duration::from_millis(20));
        if let Some(slow) = slow.as_deref_mut() {
            slow.wire.take(64 * 1024);
        }
    }
    told
}

/// A locked session with the locker's observer, a second observer that is about to stop reading, and a holder with a
/// canvas to feed.
fn session_with_a_client_to_stall() -> (Session, Peer, Peer, Peer, u64) {
    let session = start();
    let observer = lock(&session);
    // The second observer subscribes to every change, and the test never reads from it until it says so.
    let stalled = attach(&session, "observer");
    let mut holder = attach(&session, "holder");
    let root = holder.root;
    let fed = holder.canvas_under(root);
    (session, observer, stalled, holder, fed)
}

#[test]
fn a_client_that_stopped_reading_stalls_neither_the_session_nor_the_card_and_is_dropped_once_the_stall_limit_has_passed()
 {
    let (session, mut observer, mut stalled, mut holder, fed) = session_with_a_client_to_stall();
    let began = Instant::now();
    let told = feed(&mut holder, fed, &mut observer, 12, None);
    eprintln!(
        "a client that stopped reading: 12 changes of 960 KB in {:?}, the slowest answer {:?}",
        began.elapsed(),
        told.slowest
    );
    assert!(
        told.slowest <= STALL_BOUND,
        "a request waited {:?} for the compositor while a client that stopped reading was being fed",
        told.slowest
    );
    // The card still takes the password: the session never stood still long enough to lose it.
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(5)),
        "the card took no password after a client stopped reading"
    );
    // The client has taken nothing since the first changes. It is read last, because reading makes the room the session
    // waits for: once the limit has passed the session has dropped it, and what it reads is what its socket held and
    // then the end of the stream.
    let silent_until = began + STALL_LIMIT + Duration::from_millis(1_500);
    while Instant::now() < silent_until {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        stalled.wire.read_until_eof(Duration::from_secs(10)),
        "the session still held a client that took nothing for {:?}",
        began.elapsed()
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_client_that_reads_but_too_little_is_dropped_once_more_than_the_session_holds_is_waiting_for_it() {
    let (session, mut observer, mut slow, mut holder, fed) = session_with_a_client_to_stall();
    let began = Instant::now();
    // 50 changes of 960 KB are 48 MB, more than the 32 MiB the session holds for one client. The client takes 64 KB with
    // each of them, so it is never without progress for the stall limit: if the session drops it, the size bound did.
    let told = feed(&mut holder, fed, &mut observer, 50, Some(&mut slow));
    eprintln!(
        "a client that reads too little: 50 changes of 960 KB in {:?}, the slowest answer {:?}",
        began.elapsed(),
        told.slowest
    );
    assert!(
        told.slowest <= STALL_BOUND,
        "a request waited {:?} for the compositor while a client that reads too little was being fed",
        told.slowest
    );
    assert!(
        slow.wire.read_until_eof(Duration::from_secs(5)),
        "the session still held a client with 48 MB queued for it and 3 MB taken, {:?} in",
        began.elapsed()
    );
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(5)),
        "the card took no password after a client was dropped"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_client_that_reads_late_but_inside_the_stall_limit_loses_nothing_and_hears_every_change_in_order() {
    let (session, mut observer, mut late, mut holder, fed) = session_with_a_client_to_stall();
    let feeding = Instant::now();
    let told = feed(&mut holder, fed, &mut observer, 10, None);
    assert!(
        told.slowest <= STALL_BOUND,
        "a request waited {:?} for the compositor while a client that reads late was being fed",
        told.slowest
    );
    // Two seconds of not reading is inside the limit: what the session keeps for it, it keeps in order. The client has
    // taken nothing since the first change, so the two seconds are counted from there and not from when the feeding ended:
    // feeding a loaded machine takes seconds, and a client that waited those and two more would be past the limit.
    std::thread::sleep((feeding + Duration::from_secs(2)).saturating_duration_since(Instant::now()));
    // What it kept is about 10 MB, and the session offers the socket more of it on each tick of its dispatcher, so how long
    // it takes to hand over depends on how busy the machine is. The client reads until the last change has come, and not
    // for a time that a loaded machine can overrun; a change the session lost never comes, and the loop below says which.
    let last = *told.serials.last().expect("the canvas was fed");
    late.watch_until(HEARING_LIMIT, |late| late.batches.contains(&last));
    assert!(
        late.batches.windows(2).all(|pair| pair[0] < pair[1]),
        "the changes arrived out of order: {:?}",
        late.batches
    );
    for serial in &told.serials {
        assert!(
            late.batches.contains(serial),
            "the change applied at serial {serial} never reached a client that read late: {:?}",
            late.batches
        );
    }
    session.stop().expect("the session stops cleanly");
}
