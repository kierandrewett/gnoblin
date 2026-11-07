//! Opens the log file the `logging` section names. The composition root owns this so the session
//! binary and the portal backend open the same file the same way. The file itself is written by
//! `gnoblin-log`'s file sink.

use gnoblin_config::Logging;
use gnoblin_log::file::FileSink;

/// Opens the file sink when the `logging` section keeps the log, or returns `None` when it does
/// not. A file that cannot be opened is an error the caller reports once the log is installed.
pub fn open_log_file(logging: &Logging) -> Result<Option<FileSink>, String> {
    if !logging.file.unwrap_or(true) {
        return Ok(None);
    }
    gnoblin_log::file::open_from_settings(logging.dir.as_deref(), logging.max_size_mb, |key| {
        std::env::var(key).ok()
    })
    .map(Some)
}
