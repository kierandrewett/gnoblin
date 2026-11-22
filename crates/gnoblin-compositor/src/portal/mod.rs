//! Portal negotiation for `org.freedesktop.impl.portal.*` requests.
//!
//! Nothing here draws, reads a clock, touches D-Bus or spawns a process. The
//! services role and the compositor feed events in with an explicit `now`
//! ([`gnoblin_types::Tick`]) and act on the [`Effect`]s that come back. The
//! negotiation rules (fallback, requester-gone, first-terminal-wins), the queue
//! caps and the reply validation are `gnoblin-portal-core`'s (`negotiate`,
//! `queue`, `outcome`), re-exported here so they can be tested without a
//! session, and a UI provider that keeps its own queue names the same ones. The
//! built-in dialogs they hand off to are the models in [`gnoblin_portal`].
//!
//! This module is core logic, not part of the portal relay: the relay
//! (`xdg-desktop-portal-gnoblin`) stays a removable thin relay (D19).
//!
//! What it does not hold is the dialog the user answers. That is a [`UiProvider`]: the built-in screens are std's
//! (`gnoblin-std-impl`, registered through the C ABI and reached as a [`ForeignUi`](crate::providers::ForeignUi)), and a
//! session with none runs [`NoUi`], which refuses every request `no-ui`, so a consent request that cannot be shown is
//! never granted.

mod provider;
mod style;
mod validate;

pub use gnoblin_portal_core::{
    Admission, CancelReason, Effect, Event, InterfaceContract, Negotiation, PortalError, PortalProvider, Queue,
    Refusal, Stage, Verdict,
};
pub use gnoblin_portal_ui::provider::{SubmitOutcome, UiProvider};
pub use provider::NoUi;
pub(crate) use style::{dialog_style, report_dialog_style};
pub use validate::{Deviation, Request, validate_reply};
