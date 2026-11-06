//! `org.gnome.Mutter.IdleMonitor` for screensavers, power management and idle policy.
//!
//! This is `gnoblin-compat-mutter-idlemonitor` of `design/rust-compositor.md`
//! ("Compatibility crates" and the redirect map's `IdleMonitor` row): the adapter that
//! answers "how long has the user been away" and fires when a caller's threshold is
//! reached. Gnoblin's own `src/idle.c` talks to this interface today, and so do `gsd-power`
//! and every GNOME screensaver, which is why it is one of the adapters that is on by
//! default.
//!
//! # What it is
//!
//! An IPC client with a D-Bus face. [`link`] subscribes to the compositor's object graph and
//! folds the `idle` object's counter (and the `lock` object's embargo value); [`watches`] is
//! the per-caller watch registry, which holds no clock; [`service`] serves the upstream
//! interface and fires `WatchFired` when the counter crosses a threshold. The interface is
//! vendored beside this crate (`interfaces/org.gnome.Mutter.IdleMonitor.xml`, from Mutter
//! 51.0, the release `data/gnome-versions.json` pins).
//!
//! # What it is not
//!
//! - **Not an idle clock.** The compositor measures the idle time and publishes it. Two
//!   clocks in one session disagree, and the one a policy reads has to be the one the
//!   compositor's own screensaver behaviour uses.
//! - **Not a state-changing adapter.** Every member is a read, so it answers in D13's
//!   default `portal` mode; the one member that would change the session, `ResetIdletime`,
//!   is refused by the matrix because a caller that could reset the counter could hold the
//!   session awake forever.
//! - **Not a global watch list.** A watch belongs to the bus identity that added it and ends
//!   with that connection ([`gnoblin_compat_core::CallerWatcher`]), so one process cannot cancel
//!   another's policy.
//!
//! # Running it
//!
//! ```no_run
//! use gnoblin_client::{UnixTransport, socket_path};
//! use gnoblin_compat_mutter_idlemonitor::{IdleLink, IdleMonitorService, export, watch_callers};
//! use gnoblin_proto::Version;
//! use zbus::Connection;
//!
//! # async fn run_it() -> Result<(), Box<dyn std::error::Error>> {
//! let path = socket_path(|name| std::env::var(name).ok())?;
//! let client = gnoblin_client::Client::connect(UnixTransport::connect(&path)?, Version::new(0, 2, 0))?;
//! let mut link = IdleLink::connect(client)?;
//! let connection = Connection::session().await?;
//! let service = IdleMonitorService::new()?;
//! export(&connection, &service).await?;
//! service.publish(link.state()).await?;
//! watch_callers(connection.clone(), service.clone()).await;
//! # Ok(())
//! # }
//! ```
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/`.
//!
//! 1. **A watch survives its caller.** The registry drops a caller's watches when its bus
//!    identity vanishes: `a_watch_ends_with_its_callers_connection`.
//! 2. **A watch is removed by the wrong caller.** `RemoveWatch` from anyone but the caller
//!    is refused: `a_watch_belongs_to_its_caller`.
//! 3. **An unsupported method fails silently.** `ResetIdletime` answers with the matrix's
//!    rule and reason: `an_unsupported_member_is_refused_by_name`.
//! 4. **A watch never fires because the counter never moved.** A crossing fires it, a
//!    second advance at the same value does not: `a_watch_fires_when_the_counter_crosses`.
//! 5. **A user-active watch fires twice.** It is removed when it fires:
//!    `an_active_watch_fires_once`.
//! 6. **A graph with no idle object reports zero.** Refused at connect:
//!    `a_graph_without_an_idle_counter_is_refused`.

#![forbid(unsafe_code)]

pub mod compat;
pub mod error;
pub mod link;
pub mod service;
pub mod watches;

pub use compat::{IdleMonitorAdmission, IdleMonitorCall, MATRIX, NAME, NAMES};
pub use error::IdleError;
pub use link::{IdleLink, IdleState};
pub use service::{BUS_NAME, INTERFACE, IdleMonitorService, PATH, VERSION, export, watch_callers};
pub use watches::{Watch, WatchKind, Watches};
