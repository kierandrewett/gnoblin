//! The slot vocabulary, re-exported from the model.
//!
//! `design/rust-compositor.md` ("Providers and slots"): every optional capability is
//! a named slot with one contract and one provider. The registry needs two readers —
//! this role, which claims the defaults and publishes them, and the compositor, which
//! routes an offer to whoever provides the slot — so the vocabulary, the claim rule
//! and the publication fields live in [`gnoblin_model`], beside the request broker
//! that offers into them. This module is the services role's name for that, because a
//! second copy here would be a second answer to "who draws this prompt".
//!
//! The crate's own use of it is [`crate::Agent`], which claims `auth.polkit` before it
//! offers anything and cancels what a previous provider was holding when the claim
//! changes.

pub use gnoblin_model::{CapabilitySlot, ClaimError, Provider, Slots, UnknownSlot};
