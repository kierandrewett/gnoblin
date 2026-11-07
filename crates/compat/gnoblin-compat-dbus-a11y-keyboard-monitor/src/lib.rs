//! `org.freedesktop.a11y.KeyboardMonitor`, the D-Bus face of the accessibility key path.
//!
//! An assistive technology (Orca, a braille driver, a switch device) asks the compositor
//! to tell it about keys and to take keys away from clients. This crate serves that
//! object on the session bus. It is a compat adapter: a process that does not want the
//! interface simply does not link it.
//!
//! # What it owns
//!
//! Only the D-Bus object. Method calls arrive as [`gnoblin_a11y::Event`]s; key reports
//! go back out as `KeyEvent` signals. The table that maps a bus name to an
//! [`gnoblin_a11y::ids::OwnerId`] lives here too, because a bus name is a D-Bus concept.
//!
//! # What it does not own
//!
//! - **The policy.** The grab table, the sticky and slow keys decisions and the effects
//!   all live in `gnoblin-a11y`, behind `A11y::apply`.
//! - **The key path.** The compositor loop calls `A11y::apply` synchronously for each key
//!   event, before the focused client sees that key. That is why the decision stays
//!   in-process: a bus round trip per key would let a client race the grab.
//!
//! # Failure modes first
//!
//! 1. **The bus is unreachable.** [`start`] returns an error and the caller runs without
//!    the service, so keys pass through untouched.
//! 2. **A monitor leaves without unwatching.** The owner watch sends
//!    `Event::MonitorDropped`, so its grabs end with it.
//! 3. **A late report names a departed monitor.** Owners are never reused, and a report
//!    for an owner that is gone is dropped rather than sent to its successor.

#![forbid(unsafe_code)]

/// The D-Bus object, the owner table and the threads that serve them.
pub mod service;

/// The well-known name the compositor serves the interface on.
pub use service::{BUS_NAME, INTERFACE, KeyReport, KeyReporter, KeyboardMonitorService, PATH, start};
