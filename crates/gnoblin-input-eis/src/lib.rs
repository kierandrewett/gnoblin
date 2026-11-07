//! The EIS server: libei's server side, the one input path for remote desktop and for
//! the test harness.
//!
//! `design/rust-compositor.md` states the rule twice: "one input injection path (EIS)
//! serves remote desktop and the test harness" ("Principles") and "input goes in
//! through EIS on every target" ("Parity and testing"). A remote desktop session, a
//! parity scenario and an integration test therefore all produce input the same way,
//! through an EI client that this module answers, so a grab, a binding or the session
//! lock's input gate cannot behave differently for one of them.
//!
//! The server is its own crate because it is a distinct feature with its own dependency
//! weight. It pulls in `reis`, `rustix`, `gnoblin-authority` and `gnoblin-log`, which the
//! routing in `gnoblin-input` (chords, gestures, policy) never needs. The split inside
//! this crate is the project's usual one:
//!
//! - [`service`] is the state machine. It has no socket, no thread, no clock and no
//!   protocol type: a command in, a list of [`Action`]s out. Every rule below is
//!   decided there and is tested without a peer.
//! - [`server`] is the wiring. It binds the socket, speaks EI through the `reis`
//!   crate, and turns the state machine's actions back into protocol requests and
//!   [`gnoblin_backend::InputEvent`]s.
//!
//! The wiring cannot live in `gnoblin-wayland`, because the dependency rules forbid an
//! engine-to-engine edge there: `gnoblin-wayland` may reach another engine crate only
//! through `gnoblin-backend` traits, and `cargo xtask deps` enforces it. What
//! `gnoblin-wayland` provides instead is the other half of the seam: an `InputSink`
//! that accepts events exactly as the backend would deliver them.
//!
//! # Consent
//!
//! A connected client is not trusted to inject anything yet. A session starts with no
//! capabilities until a [`Grant`] arrives from the caller (the portal path in a real
//! session: the remote desktop request the user approved), and the seat advertises only
//! what the grant covers. [`Grant::pending`] is the default everywhere, and
//! [`Grant::refused`] ends the session. The state of the decision is `gnoblin-authority`'s
//! [`Consent`], so a portal prompt and the input it unlocks answer with one vocabulary;
//! the grant is that answer plus the capabilities it covers.
//!
//! # Injected input obeys the same gate as hardware input
//!
//! Nothing here decides whether an event may reach a client. The events leave this
//! module as [`InputEvent`](gnoblin_backend::InputEvent)s, the same type a libinput
//! device produces, and the seat applies the same rules: the session lock's input gate
//! (`gnoblin-lock`), grabs, bindings and focus all run as they do for a device. A
//! locked session therefore delivers injected input to the lock surface or to nothing,
//! never to a normal client.
//!
//! # Failure modes
//!
//! Each is covered by a test named after it in `tests/eis.rs`, except where the entry
//! names the real-client suite in `tests/eis_real.rs`.
//!
//! 1. **A held key survives its client.** A disconnected, refused or misbehaving
//!    client has every held key, button and touch released exactly once, before the
//!    connection ends: `a_disconnect_releases_every_held_edge_once`.
//! 2. **Injection bypasses the lock gate.** Injected input takes the seat's own path,
//!    so a locked session delivers it to the lock surface only:
//!    `injected_input_reaches_only_the_locker_while_locked`.
//! 3. **A client escalates its capabilities.** The seat advertises the grant, a bind
//!    beyond it is refused, and a device is created only for a granted capability:
//!    `a_bind_beyond_the_grant_creates_no_device` and, against a real client,
//!    `a_capability_the_grant_excludes_gets_no_device`.
//! 4. **An absolute event lands outside every region.** A position outside every
//!    region, or any absolute event on a device that was created without one, is
//!    refused instead of being clamped to an output: `an_absolute_event_outside_every_region_is_refused`.
//! 5. **A client floods the seat.** More events between two frames than the quota
//!    allows ends the session rather than the frame queue growing without bound:
//!    `a_flood_ends_the_session`.
//! 6. **An event arrives before the device may send.** Motion, keys, buttons and
//!    touches before `start_emulating`, and a second `start_emulating` without a
//!    `stop_emulating`, are refused: `a_device_event_before_start_emulating_is_refused`.
//! 7. **A client sends an event for a device it does not have.** An unknown device
//!    identity, and an event for a capability the device was not created with, are
//!    refused: `an_event_for_an_unknown_device_is_refused`.
//! 8. **A keycode outside the range reaches the seat.** A code Linux does not define
//!    ends the session rather than being masked into some other key:
//!    `a_key_code_outside_the_linux_range_is_refused`.
//! 9. **A press is duplicated or a release is orphaned.** A key or button pressed
//!    twice within one frame is a protocol error, a press for an already-held edge is
//!    refused, and an orphaned release is dropped and counted rather than delivered:
//!    `a_duplicate_edge_in_one_frame_is_refused` and
//!    `an_orphaned_release_is_dropped`.
//! 10. **A touch contact is invented or forgotten.** A contact that is not down cannot
//!     move or lift, a down for a contact that is down is refused, and more contacts
//!     than the quota allows ends the session:
//!     `a_touch_contact_must_be_down_before_it_moves`.
//! 11. **A refusal leaves the client attached.** Every refusal ends the connection with
//!     the reason, so the client learns why instead of hanging:
//!     `a_refusal_ends_the_connection_with_its_reason` and, against a real client,
//!     `malformed_events_are_refused_and_the_server_keeps_serving`.
//! 12. **A layout change moves an existing device's regions.** A device's region list
//!     is fixed when the client is told it, so a new layout reaches new devices only
//!     and an existing device keeps hit-testing against the layout it was created
//!     with: `a_layout_change_does_not_move_an_existing_devices_regions`.
//!
//! # Determinism
//!
//! Nothing here reads a clock, a random source, the environment or a filesystem.
//! Contexts, devices and the sets of held edges are ordered, so the same command
//! sequence always produces the same action sequence, and the real-client suite drives
//! the same state machine over a real socket.

pub mod server;
pub mod service;

// The logger keeps the `input` target the module logged under before the move, so the
// `[eis]` lines land in the same log stream.
gnoblin_log::logger!(log = "input");

pub use gnoblin_authority::Consent;
pub use server::{Endpoint, Options};
pub use service::{
    Action, Caps, Context, ContextId, ContextType, DeviceId, EiDevice, EiRequest, Ending, Grant, Injection, Origin,
    Quota, Region, Service, Violation,
};
