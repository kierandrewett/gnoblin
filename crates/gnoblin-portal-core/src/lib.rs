//! The portal dialog models.
//!
//! - [`model`]: interface names, typed payloads, and the dialog models. Plain data,
//!   no drawing, no clock, no D-Bus. The compositor links this module for its
//!   negotiation types, and the dialog UI crates build their screens on it.
//!
//! - [`negotiate`], [`outcome`] and [`queue`]: the half of the portal that decides which provider serves a request, how
//!   it ends and whether it is admitted. The compositor drives them for every request, and a UI provider that holds its
//!   own queue (std's dialogs) uses the same rules, so both name them here and neither carries a copy.
//!
//! The crate names no engine crate: no egui and no renderer. It names the primitive
//! and schema crates only. The egui shell and the `DialogUi` trait live in
//! `gnoblin-portal-ui`, which depends on this crate; the dialog screens depend on that.

/// The dialog models and their state, independent of any renderer.
pub mod model;
/// Per-request negotiation: which provider serves a request, and the first terminal verdict wins.
pub mod negotiate;
/// What a provider or the built-in UI returns, and what the caller finally gets.
pub mod outcome;
/// Admission for built-in UI requests: queue caps, a per-app rate limit and the lock embargo.
pub mod queue;
/// The byte form dialogs, verdicts and keys take across the provider ABI.
pub mod wire;

pub use negotiate::{Effect, Event, InterfaceContract, Negotiation, PortalProvider, Stage};
pub use outcome::{CancelReason, PortalError, Verdict};
pub use queue::{Admission, Queue, Refusal};
