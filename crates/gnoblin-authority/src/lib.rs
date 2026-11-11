//! Pure attribution, activation and grant rules.
//!
//! This crate decides who may take focus and who may capture or inject input,
//! with no compositor in it: no Smithay
//! type, no socket, no `/proc` read and no clock. The compositor supplies every
//! [`gnoblin_types::Tick`], every identifier and every event, so a decision replays exactly.
//!
//! It replaces the `FocusContext`, `MenuContext`, `WindowDrag`, `SnapContext` and
//! `TextTarget` rules of `src/config/lua/gnoblin-lua-contexts.c` and
//! `src/control/gnoblin-control-focus.c` (design/rust-compositor.md, "Authority:
//! `gnoblin-authority`").
//!
//! # The three rules
//!
//! - **Launch attribution is a session leader plus an activation token.**
//!   [`Claimant::leader`] comes from `SO_PEERCRED` and [`Claimant::token`] from a
//!   launch this session authorised; both are required. An application id is a
//!   label an application chooses, and a bare process id is recycled, so neither
//!   ever grants focus alone.
//! - **A token is accepted at most once and expires on the caller's tick.** A
//!   redeemed token moves to the spent list, so a second attempt is refused as
//!   reused rather than looking like a forgery, and a token cannot outlive
//!   [`gnoblin_types::Tick::saturating_add`] of its issue time no matter how long the machine is idle.
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
//! 2. A child process inherits authority it should not:
//!    `a_child_with_another_leader_inherits_nothing`.
//! 3. A client claims another's app id to get focus:
//!    `an_app_id_never_grants_focus`.
//! 4. A session leader or bare process id is offered as authority:
//!    `a_session_leader_alone_never_grants_focus`.
//! 5. A token is redeemed after it expired: `an_expired_token_is_refused`.
//! 6. A token from another launch is redeemed:
//!    `a_token_exists_only_in_the_session_that_minted_it`.
//! 7. The session is covered and a client asks for focus:
//!    `a_covered_session_refuses_activation`.
//! 8. No client claims `portal.ui`, so capture is requested anyway:
//!    `capture_fails_closed_without_a_portal_ui_provider`.
//! 9. Capture runs while the session is locked:
//!    `a_grant_stops_while_the_session_is_locked`.
//! 10. Capture runs before anyone consented: `consent_pending_grants_nothing`.
//! 11. Capture runs after the user refused:
//!     `a_refused_consent_stays_refused`.
//! 12. Something other than the provider answers a prompt:
//!     `only_the_provider_answers`.
//! 13. A grant outlives its requester: `a_grant_ends_with_its_requester`.
//! 14. A token value is issued twice: `a_token_value_is_issued_once`.
//! 15. The token table is exhausted: `token_capacity_fails_closed`.
//! 16. A prompt is never answered: `an_unanswered_prompt_expires_closed`.
//! 17. The provider leaves while prompts are waiting:
//!     `releasing_portal_ui_refuses_waiting_prompts`.
//!
//! `src/model.rs` adds model-based tests over random event sequences for the two
//! invariants no single scenario establishes: a token activates at most once,
//! and a covered session authorises no activation and no grant use.

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

/// The machine, how far a grant has got ([`Consent`]), and the caps on what it tracks ([`Limits`]).
pub use authority::{Authority, Consent, Limits};
/// The decision a request reaches and the denial that explains a refusal.
pub use denial::{Decision, Denial};
/// An instruction authority gives the compositor or the portal, in the order it must happen.
pub use effect::Effect;
/// The events the authority answers, and the launch and claimant types they carry.
pub use event::{Answer, Claimant, Event, LaunchOrigin, PortalCapability};
/// Identifiers for apps, requests, requesters, leaders, surfaces and tokens.
pub use ids::{AppId, RequestId, RequesterId, SessionLeader, SurfaceId, TokenId};
