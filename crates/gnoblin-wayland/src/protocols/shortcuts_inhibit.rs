//! `zwp_keyboard_shortcuts_inhibit_manager_v1`: a client asks to see the keys the
//! compositor would otherwise take.
//!
//! A virtual machine viewer and a remote-desktop client both need it: while their window
//! has focus, `Super` combinations must reach the guest instead of the shell. The
//! protocol leaves the decision to the compositor (`new_inhibitor` exists for exactly
//! that), and this module makes two of them:
//!
//! - **A covered session never inhibits.** While the session lock is up, the keys are
//!   the lock's: an inhibitor held by a window underneath the cover would take the
//!   keys the lock screen needs, which is a security bug rather than a convenience. A
//!   request that arrives while covered is not activated, and a client that already
//!   holds one is deactivated when the cover comes up.
//! - **An inhibitor is revocable.** The compositor can deactivate it at any time, and
//!   the seat's routing reads it per edge, so the revocation reaches the next key rather
//!   than the next request.
//!
//! The inhibitor is per surface and per seat, which is Smithay's bookkeeping: this module
//! is the policy on top — who may hold one, and when it stops counting.
//!
//! # Failure modes
//!
//! 1. **An inhibitor is granted while the session is locked.** The session lock's
//!    activation deactivates every inhibitor and a new request while covered is refused,
//!    so a covered session's keys always reach the lock:
//!    `a_shortcuts_inhibitor_is_refused_while_locked` in `tests/input.rs`.
//! 2. **An inhibitor keeps working after the compositor revoked it.** Deactivating sends
//!    `inactive` and clears the flag the routing reads, so the next key goes to the
//!    compositor's bindings again: `revoking_an_inhibitor_restores_the_binding`.
//! 3. **An inhibitor for a destroyed surface suppresses shortcuts.** The inhibitor is
//!    removed with its object and the seat's lookup is per surface, so a dead surface
//!    cannot inhibit.

use smithay::input::Seat;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
    KeyboardShortcutsInhibitorSeat,
};

use crate::State;

/// Creates the `zwp_keyboard_shortcuts_inhibit_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> KeyboardShortcutsInhibitState {
    KeyboardShortcutsInhibitState::new::<State>(display)
}

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        if !self.shortcuts_may_be_inhibited() {
            // The inhibitor object stays alive and inert: the client is told nothing,
            // which is the protocol's way of saying the request was refused. Destroying
            // it would be a second, unasked-for answer.
            return;
        }
        inhibitor.activate();
        self.shortcuts_held.push(inhibitor);
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.shortcuts_held.retain(|held| held != &inhibitor);
    }
}

impl State {
    /// Whether a client may inhibit the compositor's shortcuts right now.
    ///
    /// Never while the session is covered: the keys belong to the lock screen, and a
    /// window under the cover must not be able to claim them.
    #[must_use]
    pub(crate) fn shortcuts_may_be_inhibited(&self) -> bool {
        self.lock.state() == gnoblin_lock::State::Unlocked
    }

    /// Revokes every inhibitor, sending `inactive` to each client.
    ///
    /// The session lock calls this when the cover comes up. Revoking is the compositor's
    /// side of the protocol: a client that is told `inactive` knows its shortcuts are not
    /// in effect, so it can show that state instead of silently losing keys.
    pub(crate) fn revoke_shortcut_inhibitors(&mut self) {
        for inhibitor in std::mem::take(&mut self.shortcuts_held) {
            if inhibitor.is_active() {
                inhibitor.inactivate();
            }
        }
    }

    /// Whether the focused surface's client is inhibiting the compositor's shortcuts.
    ///
    /// The answer is the seat's, read per edge by the input router: a revocation
    /// therefore reaches the very next key, not the next focus change.
    #[must_use]
    pub(crate) fn shortcuts_inhibited(&self, seat: &Seat<Self>) -> bool {
        if !self.shortcuts_may_be_inhibited() {
            return false;
        }
        seat.keyboard_shortcuts_inhibited()
    }
}
