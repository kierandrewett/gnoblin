//! Pure session lock and protection cover state machine.
//!
//! This crate is the policy of `ext-session-lock-v1` and of the protection
//! covers that nested sessions need (design/rust-compositor.md, "Hosting
//! sessions" and D4), with no compositor in it: no Smithay type, no actor, no
//! seat, no protocol object and no clock. `gnoblin-wayland` translates protocol
//! requests and render results into [`Event`]s, `Lock::apply` returns
//! [`Effect`]s in the order they must happen, and `gnoblin-compositor` performs
//! them.
//!
//! # What it guarantees
//!
//! - The covered set is a set of [`Subject`]s, not "every surface": one
//!   [`Subject::Session`] for a plain desktop, one [`Subject::Client`] per
//!   hosted session under D4. A cover can hide one client while another stays
//!   visible, and the rest of the machine never branches on which it is.
//! - Nothing is `Locked` until a frame newer than the current barrier has
//!   reached **every** output. A frame carries the generation it was armed
//!   with, so a frame queued before the cover was raised proves nothing.
//! - Input is routed by one table ([`Lock::gate_input`]) covering every device
//!   class, input methods and their popups. Only the owner's mapped lock surface
//!   is ever a destination while covered, and a miss is consumed rather than
//!   falling through to a covered client.
//! - Capture and clipboard are embargoes ([`Lock::capture`],
//!   [`Lock::gate_clipboard`]) that a granted capture stream can only satisfy
//!   with the presented lock scene, and only from `Locked`.
//! - Every failure ends the same way: the cover and the embargo stay, and
//!   nothing is revealed. A locker crash, a locker that abandons its request, a
//!   frame that cannot be produced, an empty output layout and a configuration
//!   reload that withdraws the protocol all land in [`State::CoverFailed`].
//!
//! # Failure modes first
//!
//! The plan's component card, `src/protocols/session-lock/README.md` and the
//! compositor invariants there give this list, and each entry names the test
//! that covers it:
//!
//! 1. A cover cannot be produced. The screen stays covered and nothing unlocks:
//!    `cover_failure_keeps_the_cover`.
//! 2. The locker crashes while the cover is being established:
//!    `locker_crash_while_locking_keeps_the_cover`.
//! 3. The locker crashes while locked:
//!    `locker_crash_while_locked_keeps_the_cover`.
//! 4. The locker destroys its lock object before `locked`:
//!    `abandoning_before_locked_is_not_an_unlock`.
//! 5. A frame queued before the cover was raised proves the lock:
//!    `stale_presentation_never_proves_the_cover`.
//! 6. Output hotplug while locked exposes an output or admits input early:
//!    `hotplug_while_locked_withholds_input_until_reproven`.
//! 7. Every output is removed while locked:
//!    `removing_every_output_never_reaches_locked`.
//! 8. The runtime restarts while locked:
//!    `runtime_restart_changes_nothing`.
//! 9. A reload disables the protocol while locked, which must not unlock:
//!    `disabling_the_protocol_keeps_the_cover`.
//! 10. Capture is requested during the lock transition:
//!     `capture_during_locking_is_refused`.
//! 11. Capture runs while locked: `granted_stream_sees_only_the_lock_scene`.
//! 12. An input method pops up, or any input reaches a covered client:
//!     `input_for_a_covered_client_is_consumed`.
//! 13. Clipboard, drag and data-control reach a covered client:
//!     `clipboard_belongs_to_the_owner_only`.
//! 14. Remote input arrives during the transition or in cover-failed:
//!     `remote_input_waits_for_the_presented_scene`.
//! 15. A client that is not the owner unlocks: `only_the_owner_unlocks`.
//! 16. The owner unlocks before `locked`: `unlock_before_locked_is_refused`.
//! 17. A stray reveal completion unlocks while covered:
//!     `a_stray_reveal_complete_unlocks_nothing`.
//! 18. A concurrent lock request is granted: `concurrent_requests_are_finished`.
//! 19. A lock is requested with no output, so no frame can prove it:
//!     `locking_without_outputs_fails_closed`.
//! 20. A replacement locker inherits the old proof:
//!     `takeover_requires_a_fresh_cover`.
//! 21. The owner is told it is locked twice, or after hotplug:
//!     `locked_is_announced_once_across_hotplug`.
//! 22. A newly covered subject (a nested session) is shown while covered:
//!     `a_new_subject_is_covered_before_it_is_shown`.
//! 23. A locker that never mapped a lock surface receives input:
//!     `an_unmapped_lock_surface_receives_no_input`.
//! 24. A locker crashes halfway through an authorised unlock:
//!     `a_crash_during_the_reveal_completes_it`.
//!
//! `src/model.rs` adds model-based tests over random event sequences for the two
//! invariants that no single scenario can establish: while the session is not
//! unlocked, no subject except the owner receives input, clipboard or desktop
//! capture; and `locked` arrives at most once per owner.

#![forbid(unsafe_code)]

mod effect;
mod event;
mod ids;
mod lock;
#[cfg(test)]
mod model;
mod state;
#[cfg(test)]
mod tests;

pub use effect::{Effect, Published, RejectReason};
pub use event::{CoverFailure, Event};
pub use ids::{ClientId, OutputId, Subject};
pub use lock::Lock;
pub use state::{Capture, CaptureStream, Delivery, Embargo, InputSource, State};
