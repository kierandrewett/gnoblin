//! `org.gnome.Shell.Introspect` for the portal's window picker.
//!
//! This is `gnoblin-compat-gnome-shell-introspect` of `design/rust-compositor.md`
//! ("Compatibility crates" and the redirect map's `org.gnome.Shell.Introspect` row): the
//! adapter that answers "which windows are there, what are they called and how big is the
//! desktop". `xdg-desktop-portal-gnome`'s window picker is its consumer, and so is anything
//! else that needs a window list without a Wayland protocol for it.
//!
//! # What it is
//!
//! An IPC client with a D-Bus face. [`link`] subscribes to the compositor's object graph and
//! folds the `window` objects (and the enabled `output` objects, which is where `ScreenSize`
//! comes from); [`state`] turns them into the interface's dictionary; [`service`] serves the
//! upstream interface and raises the two change signals. The interface is vendored beside this
//! crate (`interfaces/org.gnome.Shell.Introspect.xml`, from gnome-shell 51.0, the release
//! `src/gnome-versions.json` pins).
//!
//! # What it is not
//!
//! - **Not a thumbnail server.** The interface's dictionary carries no pixels, and a picker's
//!   previews come from the capture path, not from here.
//! - **Not an application model.** `GetRunningApplications` is refused with the matrix's
//!   reason: the schema publishes no application object, and inventing one from window app ids
//!   would answer a different question from the one Mutter answers.
//! - **Not state-changing.** Every member is a read, so it answers in D13's default `portal`
//!   mode and while the session is locked, which is what a lock screen's own UI needs.
//!
//! # Running it
//!
//! ```no_run
//! use gnoblin_client::{UnixTransport, socket_path};
//! use gnoblin_compat_gnome_shell_introspect::{IntrospectLink, IntrospectService, export};
//! use gnoblin_proto::Version;
//! use zbus::Connection;
//!
//! # async fn run_it() -> Result<(), Box<dyn std::error::Error>> {
//! let path = socket_path(|name| std::env::var(name).ok())?;
//! let client = gnoblin_client::Client::connect(UnixTransport::connect(&path)?, Version::new(0, 2, 0))?;
//! let mut link = IntrospectLink::connect(client)?;
//! let connection = Connection::session().await?;
//! let service = IntrospectService::new()?;
//! export(&connection, &service).await?;
//! service.publish(link.state().clone()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/`.
//!
//! 1. **The list is stale.** It follows the graph's diffs and a window that opens is announced:
//!    `get_windows_lists_the_window_objects`.
//! 2. **A closed window stays in the list.** It leaves the dictionary and `WindowsChanged`
//!    fires once: `a_closed_window_leaves_the_list`.
//! 3. **An unsupported method fails silently.** `GetRunningApplications` answers with the
//!    matrix's rule: `an_unsupported_member_is_refused_by_name`.
//! 4. **The screen size is invented.** It is the union of the enabled outputs and is refused
//!    when nothing is enabled: `screen_size_is_the_union_of_the_enabled_outputs` and
//!    `a_graph_without_an_enabled_output_cannot_answer_the_screen_size`.
//! 5. **Two windows claim one identity.** Refused rather than merged:
//!    `two_windows_cannot_share_an_identity`.
//! 6. **The adapter registers twice.** The second export is refused:
//!    `an_adapter_registers_once`.

#![forbid(unsafe_code)]

gnoblin_log::component!("compat-introspect");

pub mod compat;
pub mod environment;
pub mod error;
pub mod link;
pub mod service;
pub mod state;

pub use environment::{link_from_environment, mode_from_environment};

pub use compat::{IntrospectAdmission, IntrospectCall, MATRIX, NAME, NAMES};
pub use error::IntrospectError;
pub use link::IntrospectLink;
pub use service::{BUS_NAME, INTERFACE, IntrospectService, PATH, VERSION, export};
pub use state::{IntrospectState, WindowInfo};
