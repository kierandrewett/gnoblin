//! The stock lock screen (`data/js/gnoblin-std/lock.ts`) over a fake compositor.
//!
//! The cases are the retired lock screen's (`the retired retired corpus`, the cases named `lock`), run on
//! the JavaScript shell: the screen engages when the session locks, waits for a screen to cover, shows itself to a shell
//! that attaches to a locked session, comes down on an unlock it did not make and goes up again on a later lock, sends
//! the typed password to the compositor as the call's secret text and never as an argument, waits for the answer, comes
//! down on `ok`, and on a wrong or rate-limited attempt stays up and says why. The cases the retired screen had no test
//! for: the clock, the keyboard focus it takes and gives back, an Enter that is not an attempt, a key typed after the
//! unlock, and the password never being drawn, stored in the tree or kept in the entry once it is sent.
//!
//! The compositor is the shared fake of `tests/common`: an in-memory `TreeLink` that applies the ops the way the real
//! one does, answers nothing by itself (a test answers a password check under its token) and keeps what a real
//! compositor would hold. `Comp` is what these tests add to it: the lock's state, the keyboard routed to whatever holds
//! it, and the lock screen's canvases found by their place under the `lock` node. What the fake cannot show (the cover,
//! the keys the compositor routes, PAM) is `crates/gnoblin/tests/js_lock.rs`'s.

use std::ops::{Deref, DerefMut};
use std::time::Duration;

use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

mod common;
use common::{CanvasText, FakeCompositor, ROOT, SentCall, rig_setup, text};

const LOCK: u64 = 2;
const SEAT: u64 = 3;
const SCREEN: u64 = 4;
const WINDOW: u64 = 5;

// ---- the compositor ---------------------------------------------------------------------------------------------

/// The shared fake compositor, scripted as the lock tests need it.
#[derive(Clone)]
struct Comp(FakeCompositor);

impl Deref for Comp {
    type Target = FakeCompositor;

    fn deref(&self) -> &FakeCompositor {
        &self.0
    }
}

#[derive(Clone, Copy, Debug)]
struct Screen {
    scrim: u64,
    card: u64,
    clock: u64,
    entry: u64,
    message: u64,
}

impl Comp {
    /// The compositor's lock moves to `state`.
    fn lock_state(&self, state: &str) {
        self.0.external_set(LOCK, "state", text(state));
    }

    /// A screen comes up after the shell attached.
    fn add_screen(&self) {
        self.0
            .create_screen(SCREEN, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
    }

    /// The keyboard focus, as the seat holds it.
    fn focus(&self) -> Option<u64> {
        match self.0.value(SEAT, "focus") {
            Some(MirrorValue::Node(id)) if id != 0 => Some(id),
            _ => None,
        }
    }

    /// A key reaches the node that holds the keyboard, as the compositor routes it; nothing happens with no focus.
    fn key(&self, key: &str, typed: &str, pressed: bool) {
        let Some(target) = self.focus() else { return };
        self.key_to(target, key, typed, pressed);
    }

    fn key_to(&self, target: u64, key: &str, typed: &str, pressed: bool) {
        self.0.key(target, key, typed, pressed);
    }

    /// Every call the shell sent, with the secret text it carried, in order.
    fn calls(&self) -> Vec<SentCall> {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter_map(|op| match op {
                TxOp::Call {
                    node,
                    method,
                    args,
                    token,
                    text,
                } => Some(SentCall {
                    node: *node,
                    method: method.clone(),
                    args: args.clone(),
                    token: *token,
                    text: text.expose().to_owned(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The password checks the shell sent, in order.
    fn checks(&self) -> Vec<SentCall> {
        self.0.authentications()
    }

    /// The strings a canvas paints, in order.
    fn texts(&self, id: u64) -> Vec<String> {
        self.0.text_of(id)
    }

    fn rect(&self, id: u64) -> (f64, f64, f64, f64) {
        let number = |key: &str| self.0.number(id, key);
        (number("x"), number("y"), number("width"), number("height"))
    }

    /// The lock screen's canvases: the scrim under the lock node, the card, then the clock, entry and message.
    fn screen(&self) -> Option<Screen> {
        let scrim = *self.0.children(LOCK).first()?;
        let card = *self.0.children(scrim).first()?;
        let parts = self.0.children(card);
        let [clock, entry, message] = parts[..] else {
            return None;
        };
        Some(Screen {
            scrim,
            card,
            clock,
            entry,
            message,
        })
    }
}

// ---- the rig ------------------------------------------------------------------------------------------------------

/// A shell over a session, and the compositor it is scripted by. The rest of the rig is the shared one's.
struct Rig {
    inner: common::Rig,
    comp: Comp,
}

impl Deref for Rig {
    type Target = common::Rig;

    fn deref(&self) -> &common::Rig {
        &self.inner
    }
}

impl DerefMut for Rig {
    fn deref_mut(&mut self) -> &mut common::Rig {
        &mut self.inner
    }
}

impl Rig {
    /// A shell over a session with a lock in `state`, a seat (focused on `focus`), a window and, when `screen` is set, a
    /// screen.
    fn session(state: &str, screen: bool, focus: Option<u64>) -> Self {
        let inner = rig_setup(
            "import { LockScreen } from \"gnoblin-std\";\n(globalThis as any).screen = new LockScreen(LockScreen.defaultLockStyle(), { now: () => (globalThis as any).fakeNow ?? Date.now() });\n",
            "",
            |comp| {
                comp.add_node(LOCK, ROOT, "lock", vec![("state", text(state))]);
                let seat = focus.map_or_else(Vec::new, |id| vec![("focus", MirrorValue::Node(id))]);
                comp.add_node(SEAT, ROOT, "seat", seat);
                comp.add_node(WINDOW, ROOT, "window", vec![("title", text("Editor"))]);
                if screen {
                    comp.add_screen(SCREEN, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
                }
            },
        );
        assert!(inner.load_diagnostics.is_empty(), "{:?}", inner.load_diagnostics);
        let comp = Comp(inner.comp.clone());
        Self { inner, comp }
    }

    /// Runs frames until the shell and the compositor have nothing left to say, asserting that nothing went wrong.
    fn settle(&mut self) {
        let diagnostics = self.inner.settle();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// Types `typed` one character at a time to whatever holds the keyboard, then lets the shell see it.
    fn type_text(&mut self, typed: &str) {
        for character in typed.chars() {
            let name = character.to_string();
            self.comp.key(&name, &name, true);
            self.comp.key(&name, "", false);
        }
        self.settle();
    }

    fn enter(&mut self) {
        self.comp.key("Return", "", true);
        self.comp.key("Return", "", false);
        self.settle();
    }

    /// The token of the one password check the shell has sent that nothing answered yet.
    fn check_token(&self) -> u64 {
        self.comp.checks().last().expect("a password check was sent").token
    }
}

/// A shell on a session that is not locked yet, with a screen and the window focused.
fn unlocked() -> Rig {
    let mut rig = Rig::session("not-locked", true, Some(WINDOW));
    rig.settle();
    rig
}

/// A shell on a session whose lock is `pending`, with the lock screen up.
fn locked() -> Rig {
    let mut rig = unlocked();
    rig.comp.lock_state("pending");
    rig.settle();
    rig.comp.lock_state("locked");
    rig.settle();
    rig
}

// ---- the screen comes up and goes down ----------------------------------------------------------------------------

#[test]
fn a_lock_request_puts_the_screen_up_and_engages_the_lock() {
    let mut rig = unlocked();
    assert!(rig.comp.screen().is_none(), "an unlocked session has no lock screen");
    assert_ne!(rig.comp.flag(LOCK, "engaged"), Some(true));
    rig.comp.lock_state("pending");
    rig.settle();
    let screen = rig.comp.screen().expect("the lock screen is up");
    assert_eq!(
        rig.comp.flag(LOCK, "engaged"),
        Some(true),
        "the lock records that its screen is up"
    );
    assert_eq!(
        rig.comp.rect(screen.scrim),
        (0.0, 0.0, 1000.0, 800.0),
        "the scrim covers the screen"
    );
    let (x, y, width, _) = rig.comp.rect(screen.card);
    assert_eq!(
        (x, y, width),
        (320.0, 266.0, 360.0),
        "the card is centred, a third of the way down"
    );
    assert_eq!(rig.comp.focus(), Some(screen.entry), "the entry holds the keyboard");
}

#[test]
fn a_lock_requested_before_the_tree_has_a_screen_waits_for_one() {
    let mut rig = Rig::session("pending", false, None);
    rig.settle();
    assert!(
        rig.comp.screen().is_none(),
        "no lock screen can be up before there is a screen to cover"
    );
    assert_ne!(
        rig.comp.flag(LOCK, "engaged"),
        Some(true),
        "and the lock is not engaged"
    );
    rig.comp.add_screen();
    // The screen that waits for a covering screen looks again on the shell's clock, so a moment of it passes.
    rig.advance(1000);
    rig.settle();
    assert!(
        rig.comp.screen().is_some(),
        "the screen comes up once there is a screen"
    );
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn a_shell_that_attaches_to_a_locked_session_shows_the_lock_screen() {
    // The request is one-shot: a shell that comes up after it hears nothing, so the state is the request.
    let mut rig = Rig::session("locked", true, None);
    rig.settle();
    assert!(
        rig.comp.screen().is_some(),
        "a locked session shows its lock screen on attach"
    );
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn a_lock_that_unlocks_before_the_screen_exists_leaves_no_screen_behind() {
    let mut rig = Rig::session("pending", false, None);
    rig.settle();
    rig.comp.lock_state("not-locked");
    rig.settle();
    rig.comp.add_screen();
    rig.settle();
    assert!(rig.comp.screen().is_none(), "an unlocked session gets no lock screen");
    assert_ne!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn an_unlock_the_screen_did_not_make_takes_it_down_and_a_later_lock_puts_it_up_again() {
    let mut rig = locked();
    let first = rig.comp.screen().expect("the lock screen is up");
    rig.comp.lock_state("not-locked"); // unlocked through the protocol while the screen is up
    rig.settle();
    assert!(rig.comp.screen().is_none(), "an unlock takes the screen down");
    assert!(!rig.comp.exists(first.scrim), "and destroys its canvases");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(false), "and disengages the lock");
    assert_eq!(
        rig.comp.focus(),
        Some(WINDOW),
        "the keyboard goes back to the window that had it"
    );
    rig.comp.lock_state("pending");
    rig.settle();
    assert!(rig.comp.screen().is_some(), "a later lock draws the screen again");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn focus_is_not_taken_back_from_a_window_that_took_it_while_the_screen_went_down() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(SCREEN)); // something else took the keyboard
    rig.comp.lock_state("not-locked");
    rig.settle();
    assert_eq!(
        rig.comp.focus(),
        Some(SCREEN),
        "the screen held it no longer, so it gives nothing back"
    );
    assert!(!rig.comp.exists(screen.entry));
}

#[test]
#[ignore = "vrnw: compositor-owned on exit: closing the native generation queues the removal of the lock screen's canvases, and nothing sends it once no generation serves; the compositor reaps a holder's nodes when its connection detaches (gnoblin-compositor src/ipc/tree_host.rs `detach`). Whether the lock disengages on detach is not checked"]
fn retiring_the_shell_takes_the_screen_down() {
    let mut rig = locked();
    assert!(rig.comp.screen().is_some());
    // The generation goes with its core, as the loader ends it.
    let core = rig.inner.core.as_mut().expect("the rig has a core");
    core.close(&rig.inner.shell);
    // No frame follows: the generation is gone, and a frame of the same generation would be a new lock screen.
    assert!(
        rig.comp.screen().is_none(),
        "a generation that goes leaves no canvases on the cover"
    );
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(false));
}

// ---- the clock ----------------------------------------------------------------------------------------------------

#[test]
fn the_screen_shows_the_time_of_day_and_follows_it() {
    let mut rig = unlocked();
    rig.eval("globalThis.fakeNow = 1700000000000; 0");
    rig.comp.lock_state("locked");
    rig.settle();
    let clock = rig.comp.screen().expect("the lock screen is up").clock;
    // The expected text is worked out in the script from the same local time zone, one digit pair at a time.
    let at = "((ms) => { const d = new Date(ms); \
              return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0'); })";
    let expected = rig.eval(&format!("{at}(1700000000000)"));
    assert_eq!(
        rig.comp.texts(clock),
        std::slice::from_ref(&expected),
        "the clock shows HH:MM of the moment"
    );
    assert!(
        expected.len() == 5 && expected.as_bytes()[2] == b':',
        "the time is two digits, a colon and two digits: {expected}"
    );
    rig.eval("globalThis.fakeNow += 60000; 0");
    // The screen reads the clock on the shell's clock, not on every turn: a second of it passes.
    rig.advance(1000);
    rig.settle();
    let expected_later = rig.eval(&format!("{at}(1700000060000)"));
    assert_eq!(
        rig.comp.texts(clock),
        std::slice::from_ref(&expected_later),
        "a minute later the clock shows the next minute"
    );
    assert_ne!(expected, expected_later);
}

// ---- the password -------------------------------------------------------------------------------------------------

#[test]
fn enter_sends_the_typed_password_to_the_compositor_as_the_calls_secret_text() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("hunter2");
    assert_eq!(
        rig.comp.texts(screen.entry),
        ["*******"],
        "the entry draws one mask character for each key"
    );
    assert!(rig.comp.checks().is_empty(), "nothing is sent before Enter");
    rig.enter();
    let checks = rig.comp.checks();
    assert_eq!(checks.len(), 1, "Enter sends one check");
    assert_eq!(checks[0].node, LOCK, "the call is the lock node's");
    assert_eq!(checks[0].text, "hunter2", "the typed password reaches the compositor");
    assert!(checks[0].args.is_empty(), "the password is never an argument");
}

#[test]
fn the_password_is_never_drawn_and_never_held_in_the_tree_or_the_entry() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("hunter2");
    assert!(!rig.comp.holds("hunter2"), "the tree holds no copy while it is typed");
    rig.enter();
    assert!(!rig.comp.holds("hunter2"), "nor once it is sent");
    assert_eq!(
        rig.comp.texts(screen.entry),
        [""],
        "the entry was emptied as soon as the call was made, before any answer"
    );
}

#[test]
fn typing_edits_with_backspace_and_ignores_keys_that_type_nothing() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("abc");
    rig.comp.key("BackSpace", "", true);
    rig.comp.key("Shift_L", "", true);
    rig.comp.key("Tab", "\t", true);
    rig.settle();
    assert_eq!(
        rig.comp.texts(screen.entry),
        ["**"],
        "one character was deleted and the others typed nothing"
    );
    rig.type_text("d");
    rig.enter();
    assert_eq!(rig.comp.checks()[0].text, "abd");
}

#[test]
fn a_key_that_came_up_types_nothing_and_a_character_outside_ascii_is_typed_whole() {
    let mut rig = locked();
    rig.comp.key("e", "\u{e9}", true);
    rig.comp.key("e", "\u{e9}", false);
    rig.comp.key("x", "\u{1f511}", true);
    rig.settle();
    rig.enter();
    assert_eq!(rig.comp.checks()[0].text, "\u{e9}\u{1f511}");
}

#[test]
fn an_empty_entry_is_not_an_attempt() {
    let mut rig = locked();
    rig.enter();
    assert!(rig.comp.checks().is_empty(), "a bare Enter sends nothing");
}

#[test]
fn an_enter_pressed_while_the_compositor_checks_is_not_a_second_attempt() {
    let mut rig = locked();
    rig.type_text("one");
    rig.enter();
    rig.type_text("two");
    rig.enter();
    assert_eq!(rig.comp.checks().len(), 1, "one check runs at a time");
    let token = rig.check_token();
    rig.comp.answer(token, "", "denied");
    rig.settle();
    rig.type_text("three");
    rig.enter();
    let checks = rig.comp.checks();
    assert_eq!(checks.len(), 2, "after the answer the user may try again");
    assert_eq!(checks[1].text, "three", "and what was typed while it ran was not kept");
}

#[test]
fn an_ok_answer_takes_the_screen_down_and_disengages_the_lock() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    assert!(
        rig.comp.screen().is_some(),
        "the screen waits for the compositor's answer"
    );
    rig.comp.answer(token, "ok", "");
    rig.settle();
    assert!(rig.comp.screen().is_none(), "an ok answer takes the screen down");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(false));
    assert_eq!(rig.comp.focus(), Some(WINDOW));
    // The compositor's state moves in the same breath: the screen stays down.
    rig.comp.lock_state("not-locked");
    rig.settle();
    assert!(rig.comp.screen().is_none());
}

#[test]
fn a_screen_that_was_accepted_does_not_come_back_before_the_state_says_unlocked() {
    // The answer can arrive before the state change that goes with it: the screen must not flash up again.
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "ok", "");
    rig.settle();
    rig.settle();
    assert!(
        rig.comp.screen().is_none(),
        "the state still reads locked and the screen stays down"
    );
    rig.comp.lock_state("not-locked");
    rig.settle();
    rig.comp.lock_state("pending");
    rig.settle();
    assert!(rig.comp.screen().is_some(), "a later lock puts it up again");
}

#[test]
fn a_wrong_password_keeps_the_screen_up_clears_the_entry_and_says_so() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("nope");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "denied");
    rig.settle();
    assert!(rig.comp.screen().is_some(), "a denial keeps the lock screen up");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
    assert_eq!(rig.comp.texts(screen.message), ["Wrong password."]);
    assert_eq!(rig.comp.texts(screen.entry), [""], "the entry is empty again");
    assert_eq!(rig.comp.focus(), Some(screen.entry), "and still holds the keyboard");
    rig.type_text("again");
    assert_eq!(
        rig.comp.texts(screen.entry),
        ["*****"],
        "the next attempt starts from nothing"
    );
}

#[test]
fn a_rate_limited_attempt_keeps_the_screen_up_and_says_how_long_to_wait() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("again");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "1500", "rate-limited");
    rig.settle();
    assert!(rig.comp.screen().is_some());
    assert_eq!(
        rig.comp.texts(screen.message),
        ["Too many attempts. Try again in 2 s."],
        "1500 ms rounds up to 2 s"
    );
}

#[test]
fn a_busy_or_failed_check_says_what_it_can() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("a");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "busy");
    rig.settle();
    assert_eq!(rig.comp.texts(screen.message), ["Checking the previous attempt."]);
    rig.type_text("b");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "failed: no PAM");
    rig.settle();
    assert_eq!(rig.comp.texts(screen.message), ["The password could not be checked."]);
    assert!(rig.comp.screen().is_some(), "anything but ok keeps the lock");
}

#[test]
fn an_answer_to_another_call_is_not_the_checks() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token + 1000, "ok", "");
    rig.settle();
    assert!(
        rig.comp.screen().is_some(),
        "an answer under a token the check never had changes nothing"
    );
}

// ---- keys after the unlock ----------------------------------------------------------------------------------------

#[test]
fn keys_typed_after_the_unlock_are_ignored() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "ok", "");
    rig.settle();
    let sent = rig.comp.calls().len();
    // The compositor may still have the keys in flight for the entry it knew: they go nowhere.
    rig.comp.key_to(screen.entry, "x", "x", true);
    rig.comp.key_to(screen.entry, "Return", "", true);
    rig.settle();
    assert_eq!(
        rig.comp.calls().len(),
        sent,
        "no call was made for a key typed after the unlock"
    );
    assert!(rig.comp.screen().is_none());
}

#[test]
fn keys_that_reach_another_node_are_not_the_screens() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.comp.key_to(WINDOW, "Return", "", true);
    rig.settle();
    assert!(
        rig.comp.checks().is_empty(),
        "an Enter for the window is not an Enter in the entry"
    );
}

#[test]
fn a_late_answer_after_an_external_unlock_is_dropped() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.lock_state("not-locked");
    rig.settle();
    rig.comp.answer(token, "", "denied");
    rig.settle();
    assert!(
        rig.comp.screen().is_none(),
        "a screen that is down has nothing to say about an answer"
    );
}

/// Turns until `done` holds, a moment of the shell's clock apart: what the compositor says reaches the screen in the core's turn.
#[track_caller]
fn run_until(rig: &mut Rig, what: &str, mut done: impl FnMut(&mut Rig) -> bool) {
    for _ in 0..20 {
        rig.inner.frame();
        if done(rig) {
            return;
        }
        rig.advance(100);
    }
    panic!("never reached: {what}");
}

// ---- change gating ------------------------------------------------------------------------------------------------
//
// A quiet turn runs no frame (`tests/frame_gating.rs`). The lock screen is the one part that must not rest while it is
// up: it renews the compositor's lease and follows the clock and the entry every frame, and the compositor puts its own
// card up when the lease lapses. So a screen that is up keeps every turn running, and a session without one is quiet.

#[test]
fn a_session_with_no_lock_screen_is_quiet_and_the_locks_state_wakes_the_next_frame() {
    let mut rig = unlocked();
    rig.quiet();
    for _ in 0..25 {
        assert!(!rig.next_frame_runs(), "an unlocked session woke a frame");
    }
    assert_eq!(rig.shell.next_wake(), None);
    rig.comp.lock_state("pending");
    // Before: the state woke the very next engine frame, and the screen was up in it. The core's turn puts it up now, in the turns
    // that follow.
    run_until(&mut rig, "the screen came up", |rig| rig.comp.screen().is_some());
}

#[test]
#[ignore = "vrnw: TS internal: the engine's frame counter (`next_frame_runs`) and `next_wake` showed that the TypeScript lock screen kept every engine turn running while it was up; the screen is the core's now, which asks for its own turns, and the engine's frame does not run for it"]
fn a_screen_that_is_up_runs_every_turn() {
    let mut rig = locked();
    for _ in 0..10 {
        assert_eq!(
            rig.shell.next_wake(),
            Some(Duration::ZERO),
            "the screen is due every turn"
        );
        assert!(rig.next_frame_runs(), "a screen that is up skipped a turn");
    }
}

#[test]
fn the_unlock_that_takes_the_screen_down_lets_the_loop_rest() {
    let mut rig = locked();
    rig.comp.lock_state("not-locked");
    run_until(&mut rig, "the unlock took the screen down", |rig| {
        rig.comp.screen().is_none()
    });
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None, "with the screen gone nothing is due");
}

#[test]
fn a_password_answer_is_read_by_the_frame_it_arrives_in() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("nope");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "denied");
    // Before: the very next engine frame read the answer. The core's turns read it now.
    run_until(&mut rig, "the answer was read", |rig| {
        rig.comp.texts(screen.message) == ["Wrong password."]
    });
    rig.type_text("hunter2");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "ok", "");
    run_until(&mut rig, "an ok answer took the screen down", |rig| {
        rig.comp.screen().is_none()
    });
}

#[test]
fn a_screen_that_was_accepted_rests_until_the_state_moves_and_the_next_lock_wakes_a_frame() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "ok", "");
    assert!(rig.next_frame_runs());
    rig.quiet();
    assert_eq!(
        rig.shell.next_wake(),
        None,
        "the state still reads locked, but an accepted password has nothing left to draw"
    );
    rig.comp.lock_state("not-locked");
    assert!(rig.next_frame_runs(), "the state's move did not wake a frame");
    rig.quiet();
    rig.comp.lock_state("pending");
    run_until(&mut rig, "a later lock puts the screen up again", |rig| {
        rig.comp.screen().is_some()
    });
}
