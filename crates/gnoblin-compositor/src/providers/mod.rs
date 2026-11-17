//! The provider slots: where a library that is not the core (std) puts the desktop policy the compositor asks it for
//! (`design/modular-framework.md`, 9.4 and 28).
//!
//! Core holds mechanism and a minimal safe default. Three things are decided by convention and are therefore a provider's:
//!
//! - the **window manager** that places and focuses windows while no shell holds the tree. With none registered the session
//!   runs [`NullWm`](gnoblin_wm_api::NullWm): windows at the origin, no focus policy;
//! - the **portal dialog UI**. With none registered a request that needs a dialog is refused `no-ui`, so a consent that cannot
//!   be shown is never granted;
//! - the **configuration observer**, which turns the accepted document's `window-management`, `accessibility` and `compositor`
//!   sections into the compositor's settings (`setting_set`). With none registered those settings keep their defaults.
//!
//! [`ProviderSlots`] is the compositor's side of all three. It holds no policy and names no std crate. The providers arrive as
//! the Rust traits the compositor already drives ([`gnoblin_wm_api::WindowManager`], [`gnoblin_portal_ui::provider::UiProvider`], [`ConfigObserver`]); how a library that is not
//! this one turned a C vtable into them is the core library's business (`gnoblin-core-impl`, over `gnoblin-provider-wire`). A
//! test that wants the stock behaviour registers a provider of its own the same way.
//!
//! Registration can happen before the compositor serves (the core library registers std while it starts) or after (a shell
//! that uses std registers at its `init`), from any thread, and ends with [`ProviderSlots::remove`] or the death of whoever
//! registered. Everything registered early waits in the slots and takes effect when the session attaches.

mod settings;
mod slots;
mod ui;

pub use settings::{SettingError, SettingValue};
pub use slots::{Busy, ConfigObserver, ProviderId, ProviderSlots};
pub use ui::SlotUi;
