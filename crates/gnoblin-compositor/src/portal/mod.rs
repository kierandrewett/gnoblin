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
//!
//! What it does not hold is the dialog the user answers. That is a [`UiProvider`]: the built-in screens are the stock
//! provider (`std_providers::portal_ui`, behind the `std-providers` feature), and a session with none runs [`NoUi`],
//! which refuses every request `no-ui`, so a consent request that cannot be shown is never granted
//! (design/modular-framework.md, 9.3).

mod negotiate;
mod outcome;
mod provider;
mod queue;
mod validate;

pub use negotiate::{Effect, Event, InterfaceContract, Negotiation, PortalProvider, Stage};
pub use outcome::{CancelReason, PortalError, Verdict};
pub use provider::{NoUi, SubmitOutcome, UiProvider};
pub use queue::{Admission, Queue, Refusal};
pub use validate::{Deviation, Request, validate_reply};

use gnoblin_portal_ui::style::DialogStyle;

/// The look the compositor's own dialogs (the portal's, the lock card, the secret and polkit prompts) are drawn with,
/// from a committed configuration. Reading `dialogs.style` is the stock provider's; a core with none draws the stock
/// look.
pub(crate) fn dialog_style(config: &gnoblin_config::Config) -> DialogStyle {
    #[cfg(feature = "std-providers")]
    {
        crate::std_providers::portal_ui::dialog_style(config)
    }
    #[cfg(not(feature = "std-providers"))]
    {
        let _ = config;
        DialogStyle::default()
    }
}
