//! The portal dialogs drawn by egui: one screen per dialog, each a thin wrapper
//! over the dialog kind's UI crate.

mod screen;

/// The screen that hosts one dialog, and the mapping from dialog kind to its UI.
pub use screen::{PortalScreen, ui_for};

/// The passphrase dialog. It is not a portal screen (no portal request opens it), but it is a dialog crate like the
/// others, so the compositor reaches it through here and not by naming the crate (D19).
pub use gnoblin_portal_consent::{PolkitEvent, PolkitUi, SecretResult, SecretUi};

/// The lock card the compositor draws over its lock cover when no shell holds the lock screen. Like the passphrase
/// dialog it is no portal screen, and the compositor reaches it through here.
pub use gnoblin_portal_consent::{LockNotice, LockUi};
