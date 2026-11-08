//! The Gnoblin desktop portal backend: `org.freedesktop.impl.portal.desktop.gnoblin`.
//!
//! Design/rust-compositor.md's D19 has Gnoblin serve every interface a desktop needs. This
//! crate is the relay for them. Today it implements the set `gnoblin.portal` lists: ScreenCast,
//! Screenshot, RemoteDesktop, InputCapture, GlobalShortcuts, Clipboard, Settings, Inhibit and
//! Access (the accessibility consent dialog). FileChooser, AppChooser, Print, Account, Email,
//! Notification, Wallpaper, Background, Lockdown and DynamicLauncher are not implemented here
//! yet: `data/portals.conf` routes them past Gnoblin to `gtk` or `kde`, and `gnoblin.portal`
//! does not list them. Secret is routed to Gnoblin alone and refused, and OpenURI and Trash
//! stay with a generic backend.
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
//! | [`PORTALS_CONF`] | `portals.conf`, checked in as data so a routing mistake is reviewable |
//!
//! # What is real, and what is a seam
//!
//! Real and tested: the session lifecycle of every interface this crate serves, the consent
//! flow with its D14 refusal, expiry, cancellation, requester-gone and lock rules, the
//! `Settings` interface end to end (its values come from the graph and its changes are
//! signals), and `Inhibit`'s request and monitoring lifetimes.
//!
//! A seam: every call into the compositor except the screenshot. A screenshot is the
//! compositor's `capture.screen` member, served by [`capture_relay`], which writes the
//! picture to the user's Pictures folder. The object schema declares no member for a stream,
//! a colour pick, an EIS connection, a shortcut binding, a clipboard transfer or an
//! inhibition, so [`backends::Missing`] refuses each one with a sentence naming what is
//! missing, and the composition root supplies a real implementation when the members exist.
//! No interface returns success for work it did not do.
//!
//! # How it starts
//!
//! Two ways, and both end in the same [`Portal`]:
//!
//! - **Bus activation.** `data/org.freedesktop.impl.portal.desktop.gnoblin.service` names the
//!   binary, so the first call to the bus name starts it. `data/gnoblin.portal` lists the
//!   interfaces it serves and `data/portals.conf` routes to it. Installed under
//!   `/usr/share/dbus-1/services` and `/usr/share/xdg-desktop-portal/portals`. An activated
//!   process has no privileged IPC connection, so it runs with [`host::NoShell`]: consent is
//!   refused, and `Settings` still answers.
//! - **The services role.** Starts the same binary with the IPC connection as an inherited
//!   descriptor, which is what lets a capture be offered to the shell at all.
//!
//! `tests/activation.rs` starts the built binary on a private bus, with no IPC, and checks
//! the name, `Settings`, the refusals and the exit on bus loss.
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

/// Compositor-side calls each portal request ends in; all refuse by default.
pub mod backends;
/// Screenshots: the compositor's `capture.screen` member, stored in the Pictures folder.
pub mod capture_relay;
/// The consent core: portal sessions, grants and the `portal.ui` offer.
pub mod consent;
/// The privileged IPC connection: offers out, answers and graph diffs in.
pub mod host;
/// The D-Bus objects, one per served `org.freedesktop.impl.portal.*` interface.
pub mod iface;
/// Screenshot file storage: the Pictures folder, the file name and the `file://` URI.
pub mod picture;
/// The `settings` object's properties as the namespaces toolkits read.
pub mod settings;

/// Backend traits and the refusing default set, re-exported from [`backends`].
pub use backends::{Backends, CaptureBackend, ClipboardBackend, EisBackend, InhibitBackend, ShortcutsBackend};
/// Consent types and the guard, re-exported from [`consent`].
pub use consent::{Consent, ConsentOutcome, Guard, Host, Interface, PortalCapability, Refusal, Session, State};
/// The IPC host and its read loop, re-exported from [`host`].
pub use host::{IpcHost, IpcLoop};
/// The bus name, object path and object root, re-exported from [`iface`].
pub use iface::{BUS_NAME, PATH, Portal};
/// Settings table types, re-exported from [`settings`].
pub use settings::{Change, Setting, Table};

gnoblin_log::logger!(log = "portal");

/// The `portals.conf` this backend ships, checked in beside the source.
///
/// It is data rather than generated text for the reason the plan's "Contracts are data"
/// gives: a routing change is a reviewable line in a file, and an interface that is served
/// by nobody is visible in the diff.
pub const PORTALS_CONF: &str = include_str!("../data/portals.conf");
