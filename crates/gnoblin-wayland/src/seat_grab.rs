//! The shell's hold on the seat's input: a grab of the pointer, the keyboard or both, and claims on every button of a
//! named pointer.
//!
//! A **grab** (`Seat.grab`) takes the seat's input for one node of the shell's own. The keyboard grab is a grab in the
//! router (`gnoblin-input`'s `begin_grab`): it comes before every binding and claim and before the client, so every
//! key edge is the grab's and goes to the node as its `key` event. The router sees keys and the five buttons a chord
//! can name; motion, the wheel and the other buttons never reach it, so the pointer grab is this module's: while it
//! holds, every button edge, every motion and every wheel step goes to the node, in the node's own space, and the
//! client that had the pointer is told it left. The compositor's own dialogs (the lock card, a consent dialog) take the
//! keys and buttons before `State::input` gets to a grab, so no shell can sit between the user and one of them.
//!
//! A **button claim** (`Seat.claim_pointer`) takes every button of the pointers a device name selects, the way
//! `claim_device` takes a keyboard's keys: the press is withheld from clients and nodes and the owner hears it as
//! `pointer_button`. It is the claim for a device whose buttons are not chords (a pad of side buttons, a trackball's
//! four).
//!
//! # Who may grab
//!
//! Only the shell that holds the tree, on the main seat: an observer's call is refused by the tree's own rules, and a
//! grab on any other seat or for a node that is not the shell's own (a window, a compositor node) is refused here.
//! Nothing else may start one: the tree is the owner's socket (`SO_PEERCRED`), so no client of the session can.
//!
//! # Failure modes
//!
//! 1. **The shell goes while holding a grab.** A grab outlives no connection: detaching releases it with the shell's
//!    claims, the keys and the pointer go back to the clients, and `Seat.grabbed` reads empty for the next shell:
//!    `a_shell_that_detaches_gives_the_grab_back` in `tests/protocols.rs`.
//! 2. **The grab's node is destroyed.** The grab ends at the next input or frame, not later: a node that is gone
//!    cannot hear anything, and a seat whose input vanished into it would be dead:
//!    `a_grab_ends_with_its_node`.
//! 3. **The session locks.** A grab ends at once, and a grab is refused, unless the compositor's own lock covers the
//!    session and the shell has its lock screen up and the grab's node is part of it: the password must reach the screen
//!    that checks it, and nothing else of the shell may listen to the lock screen:
//!    `a_grab_is_refused_while_locked_and_ends_when_the_session_locks`.
//! 4. **A press that came before the grab.** Its release goes where its press went (the router's rule for keys, the
//!    button's own record for buttons), so the client is never left with a key or button down:
//!    `a_grab_does_not_take_the_release_of_a_key_that_was_already_down`.
//! 5. **A press the grab took outlives the grab.** Its release is withheld from the client, which never saw the press:
//!    `a_button_the_grab_took_is_released_to_nobody_when_the_grab_has_ended`.
//! 6. **A pointer leaves mid-press.** The node is sent the release the pointer can no longer make, and the claim's
//!    owner too, so neither is left holding a button.
//! 7. **A second grab.** One at a time: the second is refused and the first stands.

use std::collections::BTreeSet;

use gnoblin_input::{DeviceSelector, GrabId, Scope, Target};
use gnoblin_lock::State as LockState;
use gnoblin_render::transaction::TreeCall;
use gnoblin_types::{DeviceId, Keysym, NodeId, PointF};
use smithay::backend::input::InputTime;

use crate::State;
use crate::keyboard::claim_name;
use crate::session_lock::internal_locker;
use crate::tree_frame::{PointerButtonEvent, TreeInput};

use crate::log;

/// The seat's input, held by one node of the shell.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SeatGrab {
    /// The seat node the call was made on.
    pub(crate) seat: NodeId,
    /// The id the shell allocated for the grab, in the number space of its claims.
    pub(crate) id: i64,
    /// The node that hears the input.
    pub(crate) node: NodeId,
    /// What the grab takes.
    pub(crate) scope: Scope,
    /// The grab in the router, which decides every key.
    router: GrabId,
}

/// A claim on every button of the pointers a device name selects.
#[derive(Clone, Debug)]
pub(crate) struct ButtonClaim {
    /// The seat node the claim was made on, and its owner's.
    pub(crate) node: NodeId,
    /// The id the shell allocated.
    pub(crate) id: i64,
    /// The pointers it takes.
    pub(crate) device: DeviceSelector,
}

/// The grab and the button claims the shell holds, and the presses each took.
#[derive(Debug, Default)]
pub(crate) struct SeatInputTables {
    /// The grab, when the shell holds one.
    pub(crate) grab: Option<SeatGrab>,
    /// The buttons a grab took the press of, by pointer and `evdev` code, so their release is the grab's too.
    pub(crate) grab_buttons: BTreeSet<(DeviceId, u32)>,
    /// The button claims, by their name (`claim_name` of the seat and the id).
    pub(crate) claims: std::collections::BTreeMap<String, ButtonClaim>,
    /// The buttons a claim took the press of, by pointer and `evdev` code, with the claim's name: their release is the
    /// claim's whatever else changes, and the owner hears it as long as the claim stands.
    pub(crate) pressed: std::collections::BTreeMap<(DeviceId, u32), String>,
}

/// The scope a call's text names.
fn parse_scope(text: &str) -> Option<Scope> {
    match text {
        "pointer" => Some(Scope::Pointer),
        "keyboard" => Some(Scope::Keyboard),
        "both" => Some(Scope::Both),
        _ => None,
    }
}

/// The name `Seat.grabbed` publishes for a scope.
const fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Pointer => "pointer",
        Scope::Keyboard => "keyboard",
        Scope::Both => "both",
    }
}

impl State {
    /// Applies one seat `grab` call: the node `args[1]` takes the seat's input as `text` names it, or the call is
    /// refused (logged; `Seat.grabbed` stays as it was). A grab with the id of the one held replaces it.
    pub(crate) fn begin_seat_grab(&mut self, call: &TreeCall, id: i64) {
        let Some(node) = call
            .args
            .get(1)
            .and_then(|raw| u64::try_from(*raw).ok())
            .map(NodeId::new)
        else {
            log::warn!(area = "grab", grab = id, "a seat grab names no node");
            return;
        };
        let Some(scope) = parse_scope(call.text.expose()) else {
            log::warn!(
                area = "grab",
                grab = id,
                "a seat grab's scope is not pointer, keyboard or both"
            );
            return;
        };
        let refusal = if self.composer.main_seat_node() != Some(call.node) {
            Some("not the main seat")
        } else if !self.composer.holder_attached() {
            Some("no shell holds the tree")
        } else if self
            .input_tables
            .grab
            .is_some_and(|held| held.seat != call.node || held.id != id)
        {
            Some("a grab is already held")
        } else if !self.composer.is_shell_node(node) {
            Some("the node is not one of the shell's own")
        } else if !self.grab_allowed_by_lock(node) {
            Some("the session is locked")
        } else {
            None
        };
        if let Some(reason) = refusal {
            log::warn!(area = "grab", grab = id, reason = reason, "a seat grab is refused");
            return;
        }
        self.end_seat_grab();
        let router = self.keyboard_tables.router.begin_grab(scope);
        self.input_tables.grab = Some(SeatGrab {
            seat: call.node,
            id,
            node,
            scope,
            router,
        });
        if scope.covers(Scope::Pointer) {
            // The client that had the pointer is told it left, and the shell's own hover ends: the node has it now.
            self.shell_pointer(None);
            self.clear_pointer_focus();
        }
        self.composer.publish_main_seat_grab(scope_name(scope));
        // The client's cursor is not the pointer's while the shell holds the pointer, and is again when it lets go.
        self.update_cursor(true);
        log::debug!(
            area = "grab",
            grab = id,
            scope = scope_name(scope),
            "the shell took the seat's input"
        );
    }

    /// Ends the grab the shell holds, if any, and gives the pointer back to the surface under it.
    pub(crate) fn end_seat_grab(&mut self) {
        let Some(grab) = self.input_tables.grab.take() else {
            return;
        };
        self.keyboard_tables.router.end_grab(grab.router);
        self.composer.publish_main_seat_grab("");
        // The client's cursor is not the pointer's while the shell holds the pointer, and is again when it lets go.
        self.update_cursor(true);
        if grab.scope.covers(Scope::Pointer) {
            self.move_pointer(self.pointer_position, InputTime::now());
        }
        log::debug!(area = "grab", grab = grab.id, "the shell's grab ended");
    }

    /// Ends the grab `id` made on `seat`, when that is the grab held.
    pub(crate) fn end_seat_grab_of(&mut self, seat: NodeId, id: i64) {
        if self
            .input_tables
            .grab
            .is_some_and(|grab| grab.seat == seat && grab.id == id)
        {
            self.end_seat_grab();
        }
    }

    /// Whether the session's lock lets a grab for `node` stand: an unlocked session does, and so does one the
    /// compositor's own lock covers while the shell has its lock screen up and `node` is part of it. Every other lock
    /// (an `ext-session-lock-v1` client's, a lock with no lock screen up) owns the input outright.
    fn grab_allowed_by_lock(&self, node: NodeId) -> bool {
        match self.lock.state() {
            LockState::Unlocked => true,
            LockState::Locked => self.lock.locker() == Some(internal_locker()) && self.composer.lock_screen_holds(node),
            _ => false,
        }
    }

    /// Ends the grab when it can no longer stand: the shell no longer holds the tree, its node is gone, or the session
    /// is locked in a way that takes the input. Called before input is routed and as a frame begins.
    pub(crate) fn refresh_seat_grab(&mut self) {
        let Some(grab) = self.input_tables.grab else {
            return;
        };
        if !self.composer.holder_attached()
            || !self.composer.is_shell_node(grab.node)
            || !self.grab_allowed_by_lock(grab.node)
        {
            self.end_seat_grab();
        }
    }

    /// The node a pointer grab sends the pointer to, with the pointer in its local space. `None` when no grab holds
    /// the pointer, and for a node that has gone.
    pub(crate) fn pointer_grab_target(&self) -> Option<(NodeId, PointF)> {
        let grab = self.input_tables.grab?;
        if !grab.scope.covers(Scope::Pointer) {
            return None;
        }
        let position = self.pointer_position;
        let local = self
            .composer
            .shell_node_local(grab.node, PointF::new(position.x, position.y))?;
        Some((grab.node, local))
    }

    /// Whether a pointer grab holds the pointer.
    pub(crate) fn pointer_grabbed(&self) -> bool {
        self.input_tables
            .grab
            .is_some_and(|grab| grab.scope.covers(Scope::Pointer))
    }

    /// Sends a key edge the router gave to the grab `target` to the grab's node. An edge for a grab that has ended
    /// reaches nobody: it is still withheld from the client, which never saw the press.
    pub(crate) fn route_grab_key(&mut self, target: Target, pressed: bool, symbol: u32) {
        let Target::Grab(id) = target else {
            return;
        };
        let Some(grab) = self.input_tables.grab.filter(|grab| grab.router == id) else {
            return;
        };
        self.tree.input(
            grab.node,
            TreeInput::Key {
                pressed,
                keysym: Keysym::new(symbol),
            },
        );
    }

    /// Routes one pointer button edge through the grab and the button claims, and says whether it was consumed: `None`
    /// means neither has it, and the edge goes on to the chords and the clients.
    ///
    /// A press is the grab's while a pointer grab holds, else a claim's while the session is unlocked and the pointer's
    /// name is one a claim selects. Its release is the same one's, whatever has changed since.
    pub(crate) fn route_seat_button(
        &mut self,
        device: DeviceId,
        code: u32,
        pressed: bool,
        time: InputTime,
    ) -> Option<bool> {
        let held = (device, code);
        if !pressed {
            if self.input_tables.grab_buttons.remove(&held) {
                // A grab that has ended leaves the release to nobody: the client never saw the press.
                self.send_grab_button(code, false);
                return Some(true);
            }
            let name = self.input_tables.pressed.remove(&held)?;
            self.send_claimed_button(&name, code, false, time);
            return Some(true);
        }
        if self.pointer_grabbed() {
            self.input_tables.grab_buttons.insert(held);
            self.send_grab_button(code, true);
            return Some(true);
        }
        if self.lock.state() != LockState::Unlocked {
            return None;
        }
        let name = self.button_claim_name(device)?;
        self.input_tables.pressed.insert(held, name.clone());
        self.send_claimed_button(&name, code, true, time);
        Some(true)
    }

    /// Tells the grab's node a button went down or up, at the pointer.
    fn send_grab_button(&mut self, code: u32, pressed: bool) {
        let Some((node, local)) = self.pointer_grab_target() else {
            return;
        };
        let (x, y) = (local.x, local.y);
        let input = if pressed {
            TreeInput::Press { x, y, button: code }
        } else {
            TreeInput::Release { x, y, button: code }
        };
        self.tree.input(node, input);
    }

    /// The claim that takes the buttons of `device`, by name: one whose selector matches the name the pointer reported.
    fn button_claim_name(&self, device: DeviceId) -> Option<String> {
        let name = self.keyboard_tables.router.device_name(device)?;
        self.input_tables
            .claims
            .iter()
            .find(|(_, claim)| claim.device.matches(name))
            .map(|(claim_name, _)| claim_name.clone())
    }

    /// Reports a button of a claimed pointer to the claim's owner, while the claim stands.
    fn send_claimed_button(&mut self, name: &str, code: u32, pressed: bool, time: InputTime) {
        let Some(claim) = self.input_tables.claims.get(name) else {
            return;
        };
        let (node, id) = (claim.node, claim.id);
        self.tree.pointer_button(
            node,
            PointerButtonEvent {
                claim: id,
                button: code,
                pressed,
                x: self.pointer_position.x,
                y: self.pointer_position.y,
                time: time.millis(),
            },
        );
    }

    /// Applies one seat `claim_pointer` call: the pointers the call's text names have every button taken from clients
    /// from now on. A name the selector refuses is logged and the claim is not made.
    pub(crate) fn claim_pointer(&mut self, call: &TreeCall, id: i64) {
        let device = match DeviceSelector::new(call.text.expose()) {
            Ok(device) => device,
            Err(error) => {
                log::warn!(area = "claim", claim = id, %error, "a seat pointer claim's name is refused");
                return;
            }
        };
        self.input_tables.claims.insert(
            claim_name(call.node, id),
            ButtonClaim {
                node: call.node,
                id,
                device,
            },
        );
    }

    /// Gives up the button claim `name`. A press it took keeps its release withheld from the client.
    pub(crate) fn release_button_claim(&mut self, name: &str) {
        self.input_tables.claims.remove(name);
    }

    /// Drops the grab and the button claims made on the seats `released` names: a claim outlives no connection.
    pub(crate) fn release_seat_input_where(&mut self, released: impl Fn(NodeId) -> bool) {
        if self.input_tables.grab.is_some_and(|grab| released(grab.seat)) {
            self.end_seat_grab();
        }
        self.input_tables.claims.retain(|_, claim| !released(claim.node));
    }

    /// Lets the shell know of the buttons a pointer that left still held: its node and its claim's owner are sent the
    /// release the pointer can no longer make, so neither is left holding a button.
    pub(crate) fn release_seat_buttons_of(&mut self, device: DeviceId, time: InputTime) {
        let grabbed: Vec<u32> = self
            .input_tables
            .grab_buttons
            .iter()
            .filter(|(owner, _)| *owner == device)
            .map(|(_, code)| *code)
            .collect();
        for code in grabbed {
            self.input_tables.grab_buttons.remove(&(device, code));
            self.send_grab_button(code, false);
        }
        let claimed: Vec<(u32, String)> = self
            .input_tables
            .pressed
            .iter()
            .filter(|((owner, _), _)| *owner == device)
            .map(|((_, code), name)| (*code, name.clone()))
            .collect();
        for (code, name) in claimed {
            self.input_tables.pressed.remove(&(device, code));
            self.send_claimed_button(&name, code, false, time);
        }
    }
}
