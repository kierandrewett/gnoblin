//! Opens the log store the `logging` section names. The composition root owns this because it is
//! the only library that may name `gnoblin-store` among the crates a binary links: the session
//! binary calls it through here and does not depend on the store itself. The opening itself lives
//! in `gnoblin-store`, so the portal backend opens the same store the same way.

use gnoblin_config::Logging;

/// Opens the store the `logging` section names and returns its log. The directory is the
/// configured `store-path`, or the state directory's `gnoblin/store` when that is empty.
pub fn open_log_store(logging: &Logging) -> Result<gnoblin_store::Logs, String> {
    gnoblin_store::open_logs_from_settings(
        logging.store_path.as_deref(),
        logging.store_max_age_days,
        logging.store_max_size_mb,
        |key| std::env::var(key).ok(),
    )
}

/// Opens the store the key-value verbs of the IPC socket write into. It is the same store the
/// `logging` section names, so one directory holds the log and the keys, and a plain client never
/// sees a path it did not configure.
pub fn open_kv_store(logging: &Logging) -> Result<gnoblin_store::Store, String> {
    gnoblin_store::open_store_from_settings(logging.store_path.as_deref(), |key| std::env::var(key).ok())
}
