//! Portal negotiation for `org.freedesktop.impl.portal.*` requests (design/rust-compositor.md, D19).
//!
//! Nothing here draws, reads a clock, touches D-Bus or spawns a process. The
//! services role and the compositor feed events in with an explicit `now`
//! ([`gnoblin_types::Tick`]) and act on the [`Effect`]s that come back. The
//! negotiation rules (fallback, requester-gone, first-terminal-wins), the queue
//! caps and the reply validation live here so they can be tested without a
//! session. The built-in dialogs they hand off to are the models in
//! [`gnoblin_portal`].
//!
//! This module is core logic, not part of the portal relay: the relay
//! (`xdg-desktop-portal-gnoblin`) stays a removable thin relay (D19).

mod negotiate;
mod outcome;
mod queue;
mod ui;
mod validate;

pub use negotiate::{Effect, Event, InterfaceContract, Negotiation, Provider, Stage};
pub use outcome::{CancelReason, PortalError, Verdict};
pub use queue::{Admission, Queue, Refusal};
pub use ui::{PortalUi, SubmitOutcome};
pub use validate::{Deviation, Request, validate_reply};
