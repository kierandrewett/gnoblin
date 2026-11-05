//! Pure input policy: the chord grammar, the binding table, the key and button delivery
//! order, touchpad gesture recognition and routing, the script policy deadline and idle
//! activity.
//!
//! This crate is the "Input" card of `design/rust-compositor.md`. It owns everything in
//! the input path that is a decision rather than a device:
//!
//! ```text
//! device event -> libinput settings (mechanism layer)
//!              -> lock gate (gnoblin-lock)
//!              -> [this crate] grabs, then bindings, then client
//!              -> [this crate] optional script decision with a deadline
//!              -> seat delivery
//!              -> [this crate] idle and activity
//! ```
//!
//! There are no libinput, xkbcommon or Smithay types anywhere: modifiers and keysyms are
//! plain values, contacts are numbers, and time is the caller's [`Tick`]. The one thing
//! this crate cannot know on its own is which keysym a key *name* means, because that is
//! a table rather than a rule, so [`chord::Keysyms`] is implemented by the mechanism
//! layer over `xkb_keysym_from_name`. Everything else in the grammar, including the
//! `XF86` prefix rule and the canonical written order, is decided here.
//!
//! Not in this increment: libinput device settings, keymaps and layouts, the IBus bridge,
//! the EIS server and IME. They are the same crate's job in the plan and need the
//! mechanism layer to exist first.
//!
//! # Failure modes
//!
//! These are the plan's "failure modes first" for input plus the ones this increment
//! exists to prevent. Each is covered by a test named after it in
//! `tests/failure_modes.rs`, and by the invariants in `tests/model.rs`.
//!
//! 1. **A chord canonicalised two ways.** Aliases, case, modifier order, the `XF86`
//!    prefix and key-code case all fold into one identity and one written form, so the
//!    same key combination cannot become two bindings. Covered by
//!    `a_chord_is_canonicalised_one_way`.
//! 2. **Two bindings silently shadow each other.** A chord belongs to one binding for
//!    both edges; a second declaration is a reported conflict that leaves the table
//!    untouched, and a later declaration with the same name replaces in place. Covered
//!    by `two_bindings_never_shadow_each_other`.
//! 3. **A gesture is claimed twice.** Bindings that could reserve one gesture are
//!    refused when the router is built, so one movement has one owner; a reserved
//!    gesture whose movement does not match runs nothing and is not passed on. Covered
//!    by `a_gesture_is_claimed_once_or_not_at_all`.
//! 4. **A late script decision blocks input.** The queue never waits; a request past its
//!    deadline is retired with the default decision, the expiry is counted, and a late
//!    reply is refused. Covered by `a_late_decision_never_blocks_and_is_recorded`.
//! 5. **A key release is lost during a grab.** A grab that starts while a key is held
//!    does not take that key's release: the release follows its press. Covered by
//!    `a_grab_start_does_not_steal_a_held_keys_release`.
//! 6. **A consumed press leaks its release.** A grab that ends, or a binding removed by a
//!    reload, while its press is held still consumes the release rather than delivering
//!    an orphan release to the client. Covered by `a_consumed_press_keeps_its_release`.
//! 7. **A held key at grab start.** A key pressed before a grab exists keeps the target
//!    its press chose, and a key pressed during a grab keeps the grab. Covered by
//!    `a_held_key_keeps_the_target_its_press_chose`.
//! 8. **A layout change under a held key.** Ownership is recorded by hardware key code,
//!    so a release whose keysym changed with the layout still reaches its press's target.
//!    Covered by `a_layout_change_under_a_held_key_keeps_its_target`.
//! 9. **A device is removed mid-gesture.** The gesture is cancelled once, the reservation
//!    is released, and no end event is invented. Covered by
//!    `a_removed_device_cancels_the_gesture_once`.
//! 10. **A lost terminal event leaves a gesture waiting forever.** A stream that stops
//!     advancing expires and is cancelled; a gather whose contacts go quiet is dropped,
//!     so the next gesture starts cleanly. Covered by
//!     `a_lost_terminal_event_ends_the_gesture`.
//! 11. **An out-of-order tick regresses idle time.** The activity clock only moves
//!     forwards, and idle time saturates rather than wrapping. Covered by
//!     `a_late_tick_does_not_move_the_idle_clock_backwards`.
//! 12. **A policy reply arrives after a reload.** A reload retires every waiting request
//!     in submission order and the reply that follows cannot act on it. Covered by
//!     `a_reload_retires_requests_before_their_replies`.
//!
//! # Determinism
//!
//! Nothing here reads a clock, a random source, the environment or a filesystem. Bindings
//! are kept in declaration order and held edges in key order, so the same event sequence
//! always produces the same decisions. `tests/model.rs` replays each generated sequence
//! twice and requires an identical decision stream.

pub mod binding;
pub mod chord;
pub mod gesture;
pub mod gesture_router;
pub mod idle;
pub mod name;
pub mod policy;
pub mod router;
pub mod tick;

pub use binding::{Binding, BindingId, BindingTable, Conflict, Insertion, Trigger};
pub use chord::{Chord, ChordKind, Key, KeyCode, Keysym, Keysyms, Modifiers, ParseError, Symbol};
pub use gesture::{
    CancelReason, Contact, ContactPhase, Edge, EdgeBand, GestureConfig, GestureEvent, GestureKind, GesturePath,
    GesturePhase, Recogniser,
};
pub use gesture_router::{
    Context, GestureAction, GestureBinding, GestureError, GestureRoute, GestureRouter, GestureSpec, GestureStats,
    PinchDirection, When,
};
pub use idle::Activity;
pub use name::{Name, NameError};
pub use policy::{Decision, Expired, MAX_PENDING, PolicyQueue, PolicyStats, Reply, RequestId, Submitted};
pub use router::{Event, GrabId, Phase, Routed, Router, Scope, ShortcutInhibit, Stats, Target};
pub use tick::Tick;
