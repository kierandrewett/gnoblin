//! `org.gnome.Shell.Screenshot`, served so that capture is refused in one place.
//!
//! This is `gnoblin-compat-gnome-shell-screenshot` of `design/rust-compositor.md`
//! ("Compatibility crates" and the redirect map's `org.gnome.Shell.Screenshot` row). It is the
//! adapter with the least to do and the most to say: every member is capture, D13's default
//! `portal` mode refuses capture, and the IPC publishes no image-capture member, so what a
//! caller gets is a refusal that names which of those three reasons applies.
//!
//! # What it is
//!
//! An IPC client for one fact and a D-Bus face for one answer. [`link`] subscribes to the
//! compositor's object graph for the `lock` object, which is what the gate's embargo is built
//! from; [`service`] serves the upstream interface and refuses every member. The interface is
//! vendored beside this crate (`interfaces/org.gnome.Shell.Screenshot.xml`, from gnome-shell
//! 51.0, the release `src/gnome-versions.json` pins).
//!
//! # What it is not
//!
//! - **Not a screenshot path.** Capture after consent is the Gnoblin portal's, and the IPC has
//!   no image-capture member for an adapter to reach. Serving the name is still worth doing:
//!   without it a caller's `Screenshot` fails with "no such name", which tells it nothing, and
//!   with it the refusal names the portal.
//! - **Not a UI.** `SelectArea`, `InteractiveScreenshot` and `FlashArea` are the shell's own
//!   drawing, and this adapter draws nothing.
//! - **Not reachable through the lock.** The embargo is the compositor's: the lock object is
//!   read on every call, so a session that locks stops answering a request that was legal a
//!   moment before.
//!
//! # Running it
//!
//! ```no_run
//! use gnoblin_client::{UnixTransport, socket_path};
//! use gnoblin_compat_gnome_shell_screenshot::{ScreenshotLink, ScreenshotService, export};
//! use gnoblin_proto::Version;
//! use zbus::Connection;
//!
//! # async fn run_it() -> Result<(), Box<dyn std::error::Error>> {
//! let path = socket_path(|name| std::env::var(name).ok())?;
//! let client = gnoblin_client::Client::connect(UnixTransport::connect(&path)?, Version::new(0, 2, 0))?;
//! let mut link = ScreenshotLink::connect(client)?;
//! let connection = Connection::session().await?;
//! let service = ScreenshotService::new()?;
//! export(&connection, &service).await?;
//! service.publish(link.lock());
//! # Ok(())
//! # }
//! ```
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/`.
//!
//! 1. **Capture is allowed in the default mode.** Refused with `capture-portal-only`:
//!    `a_screenshot_is_refused_in_portal_mode`.
//! 2. **Capture is allowed while the session is locked.** Refused with `locked-capture` even in
//!    `unmediated` mode: `a_screenshot_is_refused_while_the_session_is_locked`.
//! 3. **An unsupported method fails silently.** Every member answers `unsupported` in
//!    `unmediated` mode: `a_screenshot_is_unsupported_when_capture_is_unmediated` and
//!    `every_method_is_refused_by_name`.
//! 4. **The embargo is guessed.** A graph with no lock object is refused at connect:
//!    `a_graph_without_a_lock_object_is_refused`.
//! 5. **The adapter registers twice.** The second export is refused: `an_adapter_registers_once`.

#![forbid(unsafe_code)]

pub mod compat;
pub mod error;
pub mod link;
pub mod service;

pub use compat::{MATRIX, NAME, NAMES, ScreenshotAdmission, ScreenshotCall};
pub use error::ScreenshotError;
pub use link::ScreenshotLink;
pub use service::{BUS_NAME, INTERFACE, PATH, ScreenshotService, VERSION, export};
