//! The `logging` section, read before the log is installed.
//!
//! A process reads this section before its subscriber exists, so it cannot report a problem
//! with the configuration through the log. [`logging_from_environment`] therefore returns the
//! problem as an error, and the caller reports it once logging is up.

use crate::{Logging, SearchPaths, Store, locate};

/// The `logging` section of the last accepted configuration.
///
/// With no state directory or no configuration file there is nothing to read, and the defaults
/// apply. A configuration that exists but cannot be read is an error: the defaults still apply,
/// and the caller says why.
///
/// # Errors
///
/// A human-readable description when the last accepted configuration cannot be read.
pub fn logging_from_environment() -> Result<Logging, String> {
    let Some(store) = Store::from_env() else {
        return Ok(Logging::default());
    };
    let Ok(path) = locate(&SearchPaths::from_env()) else {
        return Ok(Logging::default());
    };
    match store.load(path.path()) {
        Ok(stored) => Ok(stored.config.logging.unwrap_or_default()),
        Err(error) if error.is_absent() => Ok(Logging::default()),
        Err(error) => Err(format!("configuration not readable: {error}")),
    }
}
