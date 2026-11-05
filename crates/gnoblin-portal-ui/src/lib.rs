//! Pure portal negotiation and built-in portal UI state (design/rust-compositor.md, D19).
//!
//! Nothing here draws, reads a clock, touches D-Bus or spawns a process. The
//! services role and the compositor feed events in with an explicit `now`
//! ([`gnoblin_types::Tick`]) and act on the [`Effect`]s that come back. The
//! negotiation rules (fallback, requester-gone, first-terminal-wins), the queue
//! caps and the dialog models live here so they can be tested without a session.
//!
//! Depends only on `gnoblin-types` and `gnoblin-schema` (D19).

mod dialog;
mod interface;
mod negotiate;
mod outcome;
mod queue;
mod validate;

pub use dialog::{
    ARM_DELAY_MS, AppChoice, AppChooser, Confirmation, Consent, ConsentDecision, ConsentFocus, ConsentInput,
    ConsentTarget, DialogError, FileChooser, FileChooserMode, FileEntry, FileFilter, Key,
};
pub use interface::{Interface, UnknownInterface};
pub use negotiate::{Effect, Event, Negotiation, Provider, Stage};
pub use outcome::{CancelReason, Grant, PortalError, Verdict};
pub use queue::{Admission, Queue, Refusal};
pub use validate::{Deviation, Request, validate_reply};
