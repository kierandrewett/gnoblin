//! XML protocol `keyboard_shortcuts_inhibit_unstable_v1` (`<protocol name="keyboard_shortcuts_inhibit_unstable_v1">`); interfaces served: `zwp_keyboard_shortcuts_inhibit_manager_v1`.
//!
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

use smithay::input::{Seat, SeatHandler};
use smithay::reexports::wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::server::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
};
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor, KeyboardShortcutsInhibitorSeat,
    KeyboardShortcutsInhibitorUserData,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-keyboard-shortcuts-inhibit-v1",
    interfaces: &[ServedInterface::new("zwp_keyboard_shortcuts_inhibit_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor.
pub trait Host {
    /// Whether the session is unlocked. A covered session never inhibits.
    fn session_unlocked(&self) -> bool;

    /// The inhibitors that were granted and are still held.
    fn shortcuts_held(&mut self) -> &mut Vec<KeyboardShortcutsInhibitor>;
}

/// Creates the `zwp_keyboard_shortcuts_inhibit_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> KeyboardShortcutsInhibitState
where
    D: GlobalDispatch<ZwpKeyboardShortcutsInhibitManagerV1, GlobalData>
        + Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, GlobalData>
        + Dispatch<ZwpKeyboardShortcutsInhibitorV1, KeyboardShortcutsInhibitorUserData>
        + 'static,
{
    KeyboardShortcutsInhibitState::new::<D>(registrar.display())
}

/// A client created an inhibitor.
pub fn new_inhibitor<H: Host>(host: &mut H, inhibitor: KeyboardShortcutsInhibitor) {
    if !host.session_unlocked() {
        // The inhibitor object stays alive and inert: the client is told nothing,
        // which is the protocol's way of saying the request was refused. Destroying
        // it would be a second, unasked-for answer.
        return;
    }
    inhibitor.activate();
    host.shortcuts_held().push(inhibitor);
}

/// A client destroyed an inhibitor.
pub fn inhibitor_destroyed<H: Host>(host: &mut H, inhibitor: &KeyboardShortcutsInhibitor) {
    host.shortcuts_held().retain(|held| held != inhibitor);
}

/// What the compositor asks of the inhibitors it holds.
pub trait ShortcutInhibitExt: Host {
    /// Revokes every inhibitor, sending `inactive` to each client.
    ///
    /// The session lock calls this when the cover comes up. Revoking is the compositor's
    /// side of the protocol: a client that is told `inactive` knows its shortcuts are not
    /// in effect, so it can show that state instead of silently losing keys.
    fn revoke_shortcut_inhibitors(&mut self) {
        for inhibitor in std::mem::take(self.shortcuts_held()) {
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
    fn shortcuts_inhibited(&self, seat: &Seat<Self>) -> bool
    where
        Self: SeatHandler + Sized + 'static,
    {
        if !self.session_unlocked() {
            return false;
        }
        seat.keyboard_shortcuts_inhibited()
    }
}

impl<T: Host> ShortcutInhibitExt for T {}
