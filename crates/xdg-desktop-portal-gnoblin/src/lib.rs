//! The Gnoblin desktop portal backend: `org.freedesktop.impl.portal.desktop.gnoblin`.
//!
//! Design/rust-compositor.md's "Portals" card splits `xdg-desktop-portal`'s backends by what
//! an interface needs. This crate is the compositor-bound half: ScreenCast, Screenshot,
//! RemoteDesktop, InputCapture, GlobalShortcuts, Clipboard, Settings and Inhibit. The
//! toolkit-bound half (FileChooser, AppChooser, Print, Account, Email, Notification,
//! Wallpaper, Lockdown) is not here at all, and `data/portals.conf` is what routes each
//! interface to a backend that serves it.
//!
//! # The one rule this crate is built around
//!
//! No app gets a capture or an injection without a user's answer, and the answer comes from
//! the shell (D13, D14, D15):
//!
//! - **The portal owns the request.** The session table, the request handle, the response
//!   code, the cancellation and the lifetime are this crate's ([`consent`]).
//! - **The shell owns the dialogue.** Every choice is a `Request` offered to the
//!   `portal.ui` slot over the privileged IPC connection ([`host`]). A shell that claims
//!   the slot draws the prompt; this crate never does.
//! - **No shell, no capture.** Without a provider the offer is refused before it is made,
//!   and the caller learns that `portal.ui` has no provider. That is D14's fail-closed
//!   answer, and it is why the refusal names the slot.
//! - **The compositor owns the pixels.** A granted request ends in one call the compositor
//!   has to make ([`backends`]). Those calls are traits with one default implementation
//!   that refuses and says what is missing; nothing here pretends a stream exists.
//!
//! # Pieces
//!
//! | Module | Owns |
//! | --- | --- |
//! | [`consent`] | the session table, the grant model, the `portal.ui` offer and its answer |
//! | [`host`] | the privileged IPC connection: offers out, answers and graph diffs in |
//! | [`settings`] | the `settings` object's properties as the namespaces toolkits read |
//! | [`backends`] | the compositor calls, each a trait, all refusing by default |
//! | [`iface`] | the `org.freedesktop.impl.portal.*` objects, and their D-Bus types |
//! | [`data`] | `portals.conf`, checked in as data so a routing mistake is reviewable |
//!
//! # What is real, and what is a seam
//!
//! Real and tested: the session lifecycle of every interface this crate serves, the consent
//! flow with its D14 refusal, expiry, cancellation, requester-gone and lock rules, the
//! `Settings` interface end to end (its values come from the graph and its changes are
//! signals), and `Inhibit`'s request and monitoring lifetimes.
//!
//! A seam: every call into the compositor. The object schema declares no member for a
//! screenshot, a stream, an EIS connection, a shortcut binding, a clipboard transfer or an
//! inhibition, so [`backends::Missing`] refuses each one with a sentence naming what is
//! missing, and the composition root supplies a real implementation when the members exist.
//! No interface returns success for work it did not do.
//!
//! # Failure modes first
//!
//! Covered by `tests/failure_modes.rs` (the consent state machine, against a fake host):
//! a grant, a refusal, an expiry, a requester leaving, a provider leaving, and a repeat ask
//! that is answered from the grant. Not yet covered by a test in this crate: capture with
//! no `portal.ui` provider, a late answer after the requester left, session ownership
//! between callers, the sandboxed option set, and the settings write path. Those rules are
//! enforced in the code paths named in their modules; a test for each is still owed.

#![forbid(unsafe_code)]

pub mod backends;
pub mod consent;
pub mod host;
pub mod iface;
pub mod settings;

pub use backends::{Backends, CaptureBackend, ClipboardBackend, EisBackend, InhibitBackend, ShortcutsBackend};
pub use consent::{Consent, ConsentOutcome, Guard, Host, Interface, PortalCapability, Refusal, Session, State};
pub use host::{IpcHost, IpcLoop};
pub use iface::{BUS_NAME, PATH, Portal};
pub use settings::{Change, Setting, Table};

/// The `portals.conf` this backend ships, checked in beside the source.
///
/// It is data rather than generated text for the reason the plan's "Contracts are data"
/// gives: a routing change is a reviewable line in a file, and an interface that is served
/// by nobody is visible in the diff.
pub const PORTALS_CONF: &str = include_str!("../data/portals.conf");
