//! The built-in lock card's seam with the session: when it is wanted, what it takes, and how a password it sends is
//! checked.
//!
//! The card itself (its drawing and its entry) is the composition root's: it is a dialog like the passphrase dialog
//! (`gnoblin-compositor`, `ipc/lock_card.rs`). This module is the part that has to live where the lock lives. It
//! decides when the card is wanted, draws it over the cover, takes every key while it is up, and runs the password
//! through the same PAM check the shell's `authenticate` call uses. The rules, each with its test:
//!
//! - **Only the compositor's own lock.** The card serves a lock taken through the `lock` node (the screensaver service,
//!   `gnoblin lock`): there the compositor owns the unlock. An `ext-session-lock-v1` client's lock belongs to that
//!   client, which draws its own screen and checks its own password; the card is never drawn over it.
//! - **Only when no shell holds the lock screen.** A shell that holds the tree and has `engaged` the lock draws the
//!   screen and takes the keys. With no shell attached the card is wanted at once. With one attached the card waits
//!   [`LOCK_UI_GRACE_MS`] for it to engage, and again after it lets go (a reload's generation swap), so a shell that is
//!   merely slow, or reloading, does not flash the card. A shell that detaches releases `engaged` here, so the next
//!   shell starts from a lock nobody holds and the card is back the moment the old one is gone.
//! - **Not scriptable.** The card is a [`Modal`] only the composition root can set, through [`ControlHandle`] and not
//!   through the tree. It is drawn after everything else in the frame, so no canvas is drawn over it, and while it is
//!   up every key and button goes to it and to nothing else: no shell node, no client and no binding hears the password.
//! - **The cover and the input gate are untouched.** The card is drawn over the cover and the shell's lock screen; the
//!   lock state machine, the embargo and the barrier never see it. It is cleared the moment the lock leaves the locked
//!   state, so no keystroke of the next session goes to a card that is gone.
//!
//! [`ControlHandle`]: crate::ControlHandle

use gnoblin_backend::InputEvent;
use gnoblin_lock::State as LockState;
use gnoblin_secret::SecretText;
use gnoblin_types::Tick;

use crate::log;
use crate::modal::{Modal, ModalEvent};
use crate::protocols::ext_session_lock_v1::internal_locker;
use crate::state::State;
use crate::tree_frame::TreeAnswer;

/// How long the shell has to hold the lock screen before the card is drawn over the cover, in session milliseconds.
pub const LOCK_UI_GRACE_MS: u64 = 1_000;

/// The token the card's password check runs under. A shell's tokens are its own counter, and the shell's checks
/// are refused when they name this one, so an answer under it is always the card's.
pub(crate) const CARD_TOKEN: u64 = u64::MAX;

/// How the compositor answered the card's password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockCardAnswer {
    /// PAM accepted it and the session is unlocking.
    Unlocked,
    /// The password was wrong.
    Denied,
    /// Too many wrong passwords: the next attempt can be made after this many milliseconds.
    RateLimited { milliseconds: u64 },
    /// The previous check is still running.
    Busy,
    /// The check could not be run, or the session was not locked.
    Failed,
}

impl LockCardAnswer {
    fn of(answer: &TreeAnswer) -> Self {
        match answer.error.as_str() {
            "" if answer.result == "ok" => Self::Unlocked,
            "denied" => Self::Denied,
            "rate-limited" => Self::RateLimited {
                milliseconds: answer.result.parse().unwrap_or(0),
            },
            "busy" => Self::Busy,
            _ => Self::Failed,
        }
    }
}

/// What the composition root takes from the session on each turn the card matters.
#[derive(Debug, Default)]
pub struct LockCardInput {
    /// Whether the card should be on screen now.
    pub wanted: bool,
    /// The keys and text the session took for the card since the last take, in order.
    pub events: Vec<ModalEvent>,
    /// The answers to the passwords the card sent since the last take.
    pub answers: Vec<LockCardAnswer>,
}

/// The session's lock card state.
#[derive(Default)]
pub(crate) struct LockCardTables {
    /// The card's overlay. Drawn and routed only while [`State::lock_card_active`] holds.
    card: Option<Modal>,
    events: Vec<ModalEvent>,
    answers: Vec<LockCardAnswer>,
    /// Since when no shell has held the lock screen while the lock stood.
    absent_since: Option<Tick>,
}

impl State {
    /// Whether the session is locked by the compositor's own lock and the cover is proven: the only state the card
    /// can serve.
    pub(crate) fn lock_card_serves(&self) -> bool {
        self.lock.state() == LockState::Locked && self.lock.locker() == Some(internal_locker())
    }

    /// Whether a shell holds the lock screen: it holds the tree and has engaged the lock.
    fn lock_ui_held(&self) -> bool {
        self.composer.shell_places() && self.composer.lock_ui_engaged()
    }

    /// Whether the card should be on screen now. See the module's rules.
    pub(crate) fn lock_card_wanted(&mut self) -> bool {
        if !self.lock_card_serves() || self.lock_ui_held() {
            self.lock_card.absent_since = None;
            return false;
        }
        if !self.composer.shell_places() {
            return true;
        }
        let since = *self.lock_card.absent_since.get_or_insert(self.clock);
        self.clock.since(since) >= LOCK_UI_GRACE_MS
    }

    /// Whether the card is set and the session is in the state it serves.
    pub(crate) fn lock_card_active(&self) -> Option<&Modal> {
        if self.lock_card_serves() {
            self.lock_card.card.as_ref()
        } else {
            None
        }
    }

    /// Sets or clears the card and draws the cover again. A card is refused unless the compositor's own lock covers
    /// the session; clearing is always allowed.
    pub(crate) fn set_lock_card(&mut self, card: Option<Modal>) {
        if card.is_some() && !self.lock_card_serves() {
            log::warn!(
                area = "lock",
                "refused a lock card set while the compositor's own lock does not cover the session"
            );
            return;
        }
        if card.is_none() && self.lock_card.card.is_none() {
            return;
        }
        self.lock_card.card = card;
        // The cover is described once per generation; the card is part of the frame that describes it.
        self.lock_tables.forget_painted();
    }

    /// Queues the key and text events an input edge becomes while the card is up. Returns `false` for an edge that is
    /// not one, which then routes as usual.
    pub(crate) fn route_to_lock_card(&mut self, event: &InputEvent) -> bool {
        let takes_text = self.lock_card.card.as_ref().is_some_and(Modal::takes_text);
        match self.modal_events_of(event, takes_text) {
            Some(events) => {
                self.lock_card.events.extend(events);
                true
            }
            None => false,
        }
    }

    /// Takes the card's input since the last take, and says whether the card is wanted now.
    pub(crate) fn take_lock_card_input(&mut self) -> LockCardInput {
        LockCardInput {
            wanted: self.lock_card_wanted(),
            events: std::mem::take(&mut self.lock_card.events),
            answers: std::mem::take(&mut self.lock_card.answers),
        }
    }

    /// Checks the password the card sent, through the same worker, rate limiter and unlock path as the shell's
    /// `authenticate` call. The answer is the card's to take; the session unlocks on an accepted one.
    pub(crate) fn lock_card_submit(&mut self, password: SecretText) {
        let locked = self.lock_is_held();
        let now = self.lock_auth.now_ms();
        if let Some(answer) = self.lock_auth.begin(CARD_TOKEN, password, locked, now) {
            self.lock_card_answer(&answer);
        }
    }

    /// Keeps the answer to a password the card sent.
    pub(crate) fn lock_card_answer(&mut self, answer: &TreeAnswer) {
        // An answer that arrives after the lock left the state the card serves (the accepted password unlocked it)
        // has no card to go to, and nothing of it may wait for the next lock.
        if self.lock_card_serves() {
            self.lock_card.answers.push(LockCardAnswer::of(answer));
        }
    }

    /// Forgets the card and everything it queued: the lock left the state it serves.
    pub(crate) fn lock_card_gone(&mut self) {
        self.lock_card.card = None;
        for event in &mut self.lock_card.events {
            *event = ModalEvent::Key {
                keycode: 0,
                state: gnoblin_types::KeyState::Released,
            };
        }
        self.lock_card.events.clear();
        self.lock_card.answers.clear();
        self.lock_card.absent_since = None;
    }

    /// A shell that no longer holds the tree no longer holds the lock screen: its `engaged` is released, so the card
    /// is wanted at once and the next shell starts from a lock nobody holds.
    pub(crate) fn release_dead_lock_screen(&mut self) {
        if !self.composer.shell_places() && self.composer.lock_ui_engaged() {
            self.composer.release_lock_engaged();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(result: &str, error: &str) -> TreeAnswer {
        TreeAnswer {
            token: CARD_TOKEN,
            result: result.to_owned(),
            error: error.to_owned(),
        }
    }

    #[test]
    fn an_answer_is_what_the_check_said() {
        assert_eq!(LockCardAnswer::of(&answer("ok", "")), LockCardAnswer::Unlocked);
        assert_eq!(LockCardAnswer::of(&answer("", "denied")), LockCardAnswer::Denied);
        assert_eq!(
            LockCardAnswer::of(&answer("1500", "rate-limited")),
            LockCardAnswer::RateLimited { milliseconds: 1500 }
        );
        assert_eq!(LockCardAnswer::of(&answer("", "busy")), LockCardAnswer::Busy);
        assert_eq!(
            LockCardAnswer::of(&answer("", "failed: no PAM")),
            LockCardAnswer::Failed
        );
        assert_eq!(LockCardAnswer::of(&answer("", "not-locked")), LockCardAnswer::Failed);
        assert_eq!(
            LockCardAnswer::of(&answer("ok", "denied")),
            LockCardAnswer::Denied,
            "an error is never an unlock, whatever the result says"
        );
    }
}
