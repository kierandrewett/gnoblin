//! `org.gnome.Mutter.DisplayConfig` for GNOME Settings and the stock portal.
//!
//! This is `gnoblin-compat-mutter-displayconfig` of `design/rust-compositor.md`
//! ("Compatibility crates" and the redirect map's `DisplayConfig` row): the adapter
//! that answers a caller asking what the desktop's monitors are and, when a user
//! allows state changes, applies a new arrangement.
//!
//! # What it is
//!
//! An IPC client with a D-Bus face. [`link`] subscribes to the compositor's object
//! graph and folds it into the monitors and logical monitors [`state`] reports;
//! [`service`] serves the upstream interface from that state and sends a write to each
//! `output` object's own `apply` method. The upstream interface is vendored beside this
//! crate (`interfaces/org.gnome.Mutter.DisplayConfig.xml`, from Mutter 51.0, the release
//! `data/gnome-versions.json` pins) so the signature a caller sees is the one this
//! adapter was written against rather than a paraphrase of it.
//!
//! # What it is not
//!
//! - **Not a second configuration store.** The layout lives in the compositor's output
//!   objects; this adapter reads them and asks them to change. It keeps one thing of its
//!   own, the serial a caller sends back, because Mutter's interface has one and a
//!   caller uses it to detect a hotplug.
//! - **Not the write path by default.** D13's default is "reads on; writes off", so
//!   `ApplyMonitorsConfig` is refused with `write-not-enabled` until
//!   `gnoblin.configure.compat.capture` is set to `unmediated`.
//! - **Not a mirror of Mutter's whole surface.** `GetResources`, the CRTC and backlight
//!   members and `SetOutputCTM` are refused with the matrix's reason. GNOME Settings
//!   needs `GetCurrentState` and `ApplyMonitorsConfig`; everything else in the interface
//!   belongs to Mutter's own configuration model, which this compositor does not have.
//!
//! # Running it
//!
//! ```no_run
//! use gnoblin_client::{UnixTransport, socket_path};
//! use gnoblin_compat_mutter_displayconfig::{DisplayConfigService, DisplayLink, export};
//! use gnoblin_model::Filter;
//! use gnoblin_proto::Version;
//! use zbus::Connection;
//!
//! # async fn run_it() -> Result<(), Box<dyn std::error::Error>> {
//! let path = socket_path(|name| std::env::var(name).ok())?;
//! let client = gnoblin_client::Client::connect(UnixTransport::connect(&path)?, Version::new(0, 2, 0))?;
//! let mut link = DisplayLink::connect(client)?;
//! let connection = Connection::session().await?;
//! let service = DisplayConfigService::new(Box::new(Recorded))?;
//! export(&connection, &service).await?;
//! service.publish(link.state().clone(), link.lock()).await?;
//! # Ok(())
//! # }
//! # struct Recorded;
//! # impl gnoblin_compat_mutter_displayconfig::OutputApplier for Recorded {
//! #     fn apply(&self, _output: &str, _fields: &gnoblin_model::Fields) -> Result<(), String> { Ok(()) }
//! # }
//! ```
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/`.
//!
//! 1. **An adapter call skips the lock embargo or the mode gate.** Every member admits
//!    through [`compat::DisplayConfigAdmission`] before it maps anything, so there is no
//!    path into the mapping that the gate has not seen:
//!    `the_gate_refuses_a_disabled_adapter`.
//! 2. **A write is allowed in the default mode.** `ApplyMonitorsConfig` is a write, and
//!    `portal` refuses it with `write-not-enabled` while
//!    `ApplyMonitorsConfigAllowed` says `false`:
//!    `an_apply_is_refused_in_the_default_mode`.
//! 3. **An unsupported method fails silently.** Every refused member answers with a
//!    D-Bus error that names the matrix's rule and reason, never an empty success:
//!    `an_unsupported_member_is_refused_by_name`.
//! 4. **Two views disagree about output names.** The connector is the schema's `name`
//!    verbatim, and two outputs claiming one name are refused rather than merged:
//!    `two_outputs_cannot_claim_one_connector` and `a_connector_is_reported_verbatim`.
//! 5. **An apply races a hotplug.** A configuration whose serial is not the current one
//!    is refused, naming both: `an_apply_with_a_stale_serial_is_refused`.
//! 6. **The adapter registers twice.** The second export is refused, not a second object
//!    server entry: `an_adapter_registers_once`.
//! 7. **A refused/unsupported matrix entry does not really refuse.** The matrix is read
//!    back and every `unsupported` member is called:
//!    `the_matrix_matches_what_the_adapter_refuses`.

#![forbid(unsafe_code)]

pub mod compat;
pub mod error;
pub mod link;
pub mod service;
pub mod state;

pub use compat::{DisplayConfigAdmission, DisplayConfigCall, MATRIX, NAME, NAMES};
pub use error::DisplayError;
pub use link::DisplayLink;
pub use service::{BUS_NAME, DisplayConfigService, INTERFACE, OutputApplier, PATH, VERSION, export};
pub use state::{ApplyMethod, ApplyPlan, DisplayOutput, DisplayState, LogicalMonitor, MonitorAssignment, MonitorMode};
