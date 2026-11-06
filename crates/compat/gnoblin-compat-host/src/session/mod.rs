//! The compositor role's wiring: the compat extensions, the XSETTINGS owner, and the settings the
//! guardian exports into the session environment.
//!
//! [`compat`] registers the adapters on the session configuration; [`settings`] exports the Qt and
//! dconf settings into the environment every child inherits.

mod compat;
mod settings;

pub use compat::configure;
pub use settings::{export_dconf_profile, export_guardian_environment, export_qt_settings};
