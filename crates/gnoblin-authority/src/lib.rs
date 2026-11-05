//! Pure attribution, activation and grant rules.
//!
//! This crate decides who may take focus, who may act on a trusted input gesture
//! and who may capture or inject input, with no compositor in it: no Smithay
//! type, no socket, no `/proc` read and no clock. The compositor supplies every
//! [`gnoblin_types::Tick`], every identifier and every event, so a decision replays exactly.
//!
//! It replaces the `FocusContext`, `MenuContext`, `WindowDrag`, `SnapContext` and
//! `TextTarget` rules of `src/config/lua/gnoblin-lua-contexts.c` and
//! `src/control/gnoblin-control-focus.c` (design/rust-compositor.md, "Authority:
//! `gnoblin-authority`").
//!
//! # The four rules
//!
//! - **Launch attribution is a session leader plus an activation token.**
//!   [`Claimant::leader`] comes from `SO_PEERCRED` and [`Claimant::token`] from a
//!   launch this session authorised; both are required. An application id is a
//!   label an application chooses, and a bare process id is recycled, so neither
//!   ever grants focus alone. The other way to take focus is a trusted input
//!   context ([`ContextRequest::Focus`]), which needs a gesture the user just
//!   made and is subject to the rules below.
//! - **A token is accepted at most once and expires on the caller's tick.** A
//!   redeemed token moves to the spent list, so a second attempt is refused as
//!   reused rather than looking like a forgery, and a token cannot outlive
//!   [`gnoblin_types::Tick::saturating_add`] of its issue time no matter how long the machine is idle.
//! - **A trusted input context belongs to one gesture.** It carries the client
//!   and surface of the gesture that created it, is single use, and ends when the
//!   gesture ends, when it expires, or when the session locks.
//! - **Grants need consent from `portal.ui`.** A capture or input grant exists
//!   per caller and per capability with a [`Consent`] state, and D14 makes the
//!   missing-provider case a refusal: no client claiming `portal.ui` means no
//!   prompt, no grant and an error the caller can act on.
//!
//! # Failure modes first
//!
//! The plan's component card, D13 and D14 give this list, and each entry names
//! the test that covers it:
//!
//! 1. A token is redeemed twice: `a_token_is_accepted_once`.
//! 2. An input context outlives its gesture:
//!    `a_context_does_not_outlive_its_gesture`.
//! 3. A child process inherits authority it should not:
//!    `a_child_with_another_leader_inherits_nothing`.
//! 4. A client claims another's app id to get focus:
//!    `an_app_id_never_grants_focus`.
//! 5. A session leader or bare process id is offered as authority:
//!    `a_session_leader_alone_never_grants_focus`.
//! 6. A token is redeemed after it expired: `an_expired_token_is_refused`.
//! 7. A token from another launch is redeemed:
//!    `a_token_exists_only_in_the_session_that_minted_it`.
//! 8. The session is covered and a client asks for focus:
//!    `a_covered_session_refuses_activation`.
//! 9. A context is used twice: `a_context_is_single_use`.
//! 10. A context is used after it expired: `an_expired_context_is_refused`.
//! 11. A context is used for another client or surface:
//!     `a_context_is_bound_to_its_target`.
//! 12. A context survives a lock and is redeemed afterwards:
//!     `locking_ends_live_contexts`.
//! 13. No client claims `portal.ui`, so capture is requested anyway:
//!     `capture_fails_closed_without_a_portal_ui_provider`.
//! 14. Capture runs while the session is locked:
//!     `a_grant_stops_while_the_session_is_locked`.
//! 15. Capture runs before anyone consented: `consent_pending_grants_nothing`.
//! 16. Capture runs after the user refused:
//!     `a_refused_consent_stays_refused`.
//! 17. Something other than the provider answers a prompt:
//!     `only_the_provider_answers`.
//! 18. A grant outlives its requester:
//!     `a_grant_ends_with_its_requester`.
//! 19. A token value is issued twice: `a_token_value_is_issued_once`.
//! 20. The token table is exhausted: `token_capacity_fails_closed`.
//! 21. A prompt is never answered: `an_unanswered_prompt_expires_closed`.
//! 22. A gesture begins while the session is covered:
//!     `a_gesture_during_lock_authorises_nothing`.
//! 23. The provider leaves while prompts are waiting:
//!     `releasing_portal_ui_refuses_waiting_prompts`.
//! 24. A privileged request arrives with no gesture behind it:
//!     `a_context_request_without_a_gesture_is_refused`.
//!
//! `src/model.rs` adds model-based tests over random event sequences for the two
//! invariants no single scenario establishes: a token activates at most once,
//! and a covered session authorises no activation, no context request and no
//! grant use.

#![forbid(unsafe_code)]

mod authority;
mod denial;
mod effect;
mod event;
mod ids;
#[cfg(test)]
mod model;
#[cfg(test)]
mod tests;

pub use authority::{Authority, Consent, Limits};
pub use denial::{Decision, Denial};
pub use effect::Effect;
pub use event::{Answer, Capability, Claimant, ContextRequest, Event, Gesture, GestureKind, LaunchOrigin};
pub use ids::{AppId, GestureId, RequestId, RequesterId, SessionLeader, SurfaceId, TokenId};
