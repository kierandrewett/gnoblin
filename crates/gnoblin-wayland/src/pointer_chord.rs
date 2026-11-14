//! Pointer-button chord claims: a shell claims `<Super>Button1` and hears the whole drag.
//!
//! A claim whose chord ends in a button (`Button1` to `Button9`) binds in the same router table as a key chord
//! claim, so a competing claim of the same chord for the same device is refused the same way. What differs is the
//! delivery. A key chord reports its press and release; a pointer chord is a capture. The press that matches the
//! claim (the chord's exact modifiers held, the session unlocked) is withheld from clients and from the shell's
//! nodes, the owner is told `start`, every later pointer motion until the release is reported as `update`, and the
//! release is `end`. The capture holds the pointer: no client hears the motion either, so a window that moves
//! under the drag does not see the pointer cross it. When the capture ends the pointer is given the surface it is
//! over.
//!
//! # Failure modes
//!
//! 1. **A capture outlives its owner.** A claim goes with the connection that made it, and the capture goes with
//!    the claim: no `end` is sent to a shell that is not there, the capture is dropped, and the button's release,
//!    which the router still owns, is withheld from clients as its press was. The client never saw the press, so a
//!    release it did see would be one for a button it never held:
//!    `a_shell_that_detaches_mid_drag_leaves_no_capture_and_no_stuck_button` in `tests/protocols.rs`.
//! 2. **The session locks mid-drag.** The owner gets `cancel` once and nothing after it, and the release is
//!    withheld: `a_lock_cancels_a_capture_in_progress`.
//! 3. **A modifier is let go first.** The capture is the button's, not the chord's: it ends with the button
//!    (`the_modifier_released_first_does_not_end_the_capture`).
//! 4. **A press while the session is locked.** It is not the claim's: it goes to the client as any press does, and
//!    so does its release, whichever of the lock and the claim goes first (`forwarded_buttons`).
//! 5. **A burst of motion.** `update` is held until the frame and carries the latest place, so a shell that is
//!    slower than the pointer is told where the pointer is, not where it was.

use gnoblin_input::{Event as RouterEvent, ShortcutInhibit, Target};
use gnoblin_lock::State as LockState;
use gnoblin_types::{ButtonState, DeviceId, NodeId};
use smithay::backend::input::InputTime;

use crate::State;
use crate::devices::LOCKS;
use crate::keyboard::{Claim, claim_name, modifiers_of};
use crate::tree_frame::{PointerChordEvent, PointerChordPhase};

/// The `evdev` codes of the buttons a pointer chord can name, from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

/// The number a chord gives an `evdev` button: X11's and GTK's (1 left, 2 middle, 3 right, 8 back, 9 forward).
/// A button no chord can name (the wheel has none, and the rest of the mouse's are not claimable) is `None`, and
/// the router never sees it.
pub(crate) const fn button_number(code: u32) -> Option<u8> {
    match code {
        BTN_LEFT => Some(1),
        BTN_MIDDLE => Some(2),
        BTN_RIGHT => Some(3),
        BTN_SIDE => Some(8),
        BTN_EXTRA => Some(9),
        _ => None,
    }
}

/// One drag a claim holds: from the press of the chord's button to its release.
#[derive(Clone, Debug)]
pub(crate) struct PointerCapture {
    /// The router binding name of the claim, which `unclaim` and a replaced claim name.
    name: String,
    /// The claim, for its owner's seat node and its id.
    claim: Claim,
    /// The chord's button number.
    button: u8,
    /// The chord's modifiers as the chord spells them, `<Super>` for `<Super>Button1`.
    modifiers: String,
    /// Where the press was.
    start: (f64, f64),
    /// Where the pointer was last put, so a refresh that moves nothing says nothing.
    last: (f64, f64),
    /// The latest place and time not yet reported, sent by the frame.
    pending: Option<((f64, f64), u32)>,
}

impl PointerCapture {
    fn new(claim: Claim, button: u8, start: (f64, f64)) -> Self {
        // The chord's text is canonical, modifiers in a fixed order and then the button, so the modifiers are
        // what comes before it.
        let modifiers = claim
            .chord
            .strip_suffix(&format!("Button{button}"))
            .unwrap_or_default()
            .to_owned();
        Self {
            name: claim_name(claim.node, claim.id),
            claim,
            button,
            modifiers,
            start,
            last: start,
            pending: None,
        }
    }

    /// The signal for `phase` with the pointer at `at`.
    fn event(&self, phase: PointerChordPhase, at: (f64, f64), time: u32) -> PointerChordEvent {
        PointerChordEvent {
            phase,
            claim: self.claim.id,
            x: at.0,
            y: at.1,
            dx: at.0 - self.start.0,
            dy: at.1 - self.start.1,
            button: self.button,
            modifiers: self.modifiers.clone(),
            time,
        }
    }

    /// Notes that the pointer is now at `at`. Only the latest place is kept until [`Self::take_update`].
    fn moved(&mut self, at: (f64, f64), time: u32) {
        if at == self.last {
            return;
        }
        self.last = at;
        self.pending = Some((at, time));
    }

    /// The `update` for the motion since the last one, if there was any.
    fn take_update(&mut self) -> Option<PointerChordEvent> {
        let (at, time) = self.pending.take()?;
        Some(self.event(PointerChordPhase::Update, at, time))
    }

    /// Whether this capture belongs to the claim `name`.
    pub(crate) fn is_of(&self, name: &str) -> bool {
        self.name == name
    }

    /// The seat node the claim's owner made the claim on.
    pub(crate) const fn node(&self) -> NodeId {
        self.claim.node
    }
}

impl State {
    /// Whether a shell's drag holds the pointer, so no client is told of where it goes.
    pub(crate) fn pointer_captured(&self) -> bool {
        !self.keyboard_tables.captures.is_empty()
    }

    /// Routes one pointer button edge through the claims and says whether it was consumed. A consumed edge goes
    /// nowhere else: not to a client and not to the shell's nodes.
    ///
    /// Every edge of a button a chord can name goes through the router, the press and the release, so the router
    /// keeps each press's release with it however the claims change while the button is held.
    pub(crate) fn route_pointer_button(&mut self, device: DeviceId, code: u32, pressed: bool, time: InputTime) -> bool {
        self.cancel_captures_when_locked(time.millis());
        let Some(button) = button_number(code) else {
            return false;
        };
        let modifiers = modifiers_of(&self.keyboard.modifier_state()).remove(LOCKS);
        let inhibit = if self.focused_shortcuts_inhibited() {
            ShortcutInhibit::Inhibited
        } else {
            ShortcutInhibit::Clear
        };
        let routed = self.keyboard_tables.router.route(
            RouterEvent::Button {
                device,
                button,
                modifiers,
                state: if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
            },
            inhibit,
        );
        let held = (device, button);
        if !pressed {
            // A press the lock held back went to the client, so its release does too.
            if self.keyboard_tables.forwarded_buttons.remove(&held) {
                return false;
            }
            if let Some(capture) = self.keyboard_tables.captures.remove(&held) {
                let at = (self.pointer_position.x, self.pointer_position.y);
                let end = capture.event(PointerChordPhase::End, at, time.millis());
                self.tree.pointer_chord(capture.claim.node, end);
                // The pointer was the shell's for the drag; the surface under it is the client's again.
                self.move_pointer(self.pointer_position, time);
                return true;
            }
            // A press the router owns for a claim that has since gone is withheld as it was; an edge the router
            // owned for nobody (a press a modal took) is the client's as before.
            return routed.followed_press && routed.target != Target::Client;
        }
        let Some(claim) = self.claim_of(routed.target) else {
            return routed.target != Target::Client;
        };
        if self.lock.state() != LockState::Unlocked {
            self.keyboard_tables.forwarded_buttons.insert(held);
            return false;
        }
        let at = (self.pointer_position.x, self.pointer_position.y);
        let capture = PointerCapture::new(claim, button, at);
        let start = capture.event(PointerChordPhase::Start, at, time.millis());
        self.tree.pointer_chord(capture.claim.node, start);
        if let Some(replaced) = self.keyboard_tables.captures.insert(held, capture) {
            // Two presses of one button on one device without a release between: the first drag is over.
            let cancel = replaced.event(PointerChordPhase::Cancel, at, time.millis());
            self.tree.pointer_chord(replaced.claim.node, cancel);
        }
        true
    }

    /// Records that the pointer moved while a drag holds it. `update` goes out with the frame.
    pub(crate) fn note_capture_motion(&mut self, time: InputTime) {
        let at = (self.pointer_position.x, self.pointer_position.y);
        for capture in self.keyboard_tables.captures.values_mut() {
            capture.moved(at, time.millis());
        }
    }

    /// Reports the motion each drag made since the last frame, once, with the latest place. Called as the frame
    /// begins, before the frame's tree work, so the shell hears it in the batch of the frame that follows.
    pub(crate) fn flush_pointer_captures(&mut self) {
        self.cancel_captures_when_locked(InputTime::now().millis());
        let updates: Vec<(NodeId, PointerChordEvent)> = self
            .keyboard_tables
            .captures
            .values_mut()
            .filter_map(|capture| capture.take_update().map(|update| (capture.node(), update)))
            .collect();
        for (node, update) in updates {
            self.tree.pointer_chord(node, update);
        }
    }

    /// Ends every drag with `cancel` when the session is no longer unlocked: the lock's cover owns the pointer.
    fn cancel_captures_when_locked(&mut self, time: u32) {
        if self.keyboard_tables.captures.is_empty() || self.lock.state() == LockState::Unlocked {
            return;
        }
        for (_, capture) in std::mem::take(&mut self.keyboard_tables.captures) {
            let cancel = capture.event(PointerChordPhase::Cancel, capture.last, time);
            self.tree.pointer_chord(capture.claim.node, cancel);
        }
    }

    /// Ends the drags a pointer that left was making with `cancel`: its release will never come.
    pub(crate) fn cancel_captures_of(&mut self, device: DeviceId) {
        let time = InputTime::now().millis();
        let gone: Vec<(DeviceId, u8)> = self
            .keyboard_tables
            .captures
            .keys()
            .filter(|(owner, _)| *owner == device)
            .copied()
            .collect();
        for held in gone {
            if let Some(capture) = self.keyboard_tables.captures.remove(&held) {
                let cancel = capture.event(PointerChordPhase::Cancel, capture.last, time);
                self.tree.pointer_chord(capture.claim.node, cancel);
            }
        }
        self.keyboard_tables
            .forwarded_buttons
            .retain(|(owner, _)| *owner != device);
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_types::NodeId;

    use super::{PointerCapture, button_number};
    use crate::keyboard::Claim;
    use crate::tree_frame::PointerChordPhase;

    fn capture(chord: &str, button: u8) -> PointerCapture {
        PointerCapture::new(
            Claim {
                node: NodeId::new(7),
                id: 3,
                chord: chord.to_owned(),
            },
            button,
            (100.0, 50.0),
        )
    }

    #[test]
    fn the_buttons_a_chord_names_are_x11s_numbers() {
        assert_eq!(button_number(0x110), Some(1), "BTN_LEFT");
        assert_eq!(button_number(0x112), Some(2), "BTN_MIDDLE");
        assert_eq!(button_number(0x111), Some(3), "BTN_RIGHT");
        assert_eq!(button_number(0x113), Some(8), "BTN_SIDE is the back button");
        assert_eq!(button_number(0x114), Some(9), "BTN_EXTRA is the forward button");
        assert_eq!(button_number(0x115), None);
        assert_eq!(button_number(0), None);
    }

    #[test]
    fn a_start_has_the_press_place_and_no_movement() {
        let capture = capture("<Super>Button1", 1);
        let start = capture.event(PointerChordPhase::Start, (100.0, 50.0), 42);
        assert_eq!(
            (start.claim, start.button, start.modifiers.as_str(), start.time),
            (3, 1, "<Super>", 42)
        );
        assert_eq!((start.x, start.y, start.dx, start.dy), (100.0, 50.0, 0.0, 0.0));
    }

    #[test]
    fn the_modifiers_are_what_the_chord_spells_before_its_button() {
        assert_eq!(capture("<Control><Super>Button3", 3).modifiers, "<Control><Super>");
        assert_eq!(
            capture("Button8", 8).modifiers,
            "",
            "a bare back button holds no modifier"
        );
    }

    #[test]
    fn a_burst_of_motion_is_one_update_with_the_latest_place_measured_from_the_press() {
        let mut capture = capture("<Super>Button1", 1);
        for step in 1..=10 {
            capture.moved((100.0 + f64::from(step), 50.0 + f64::from(step) * 2.0), step as u32);
        }
        let update = capture.take_update().expect("the burst is one update");
        assert_eq!(update.phase, PointerChordPhase::Update);
        assert_eq!((update.x, update.y), (110.0, 70.0));
        assert_eq!((update.dx, update.dy), (10.0, 20.0));
        assert_eq!(update.time, 10);
        assert!(capture.take_update().is_none(), "the update is sent once");
    }

    #[test]
    fn a_refresh_that_moves_nothing_says_nothing() {
        let mut capture = capture("<Super>Button1", 1);
        capture.moved((100.0, 50.0), 5);
        assert!(capture.take_update().is_none());
        capture.moved((101.0, 50.0), 6);
        capture.take_update().expect("a real move");
        capture.moved((101.0, 50.0), 7);
        assert!(capture.take_update().is_none(), "the same place twice is one move");
    }
}
