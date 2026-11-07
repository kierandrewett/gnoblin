//! The last-good store: the document that was accepted last, kept so a broken
//! edit cannot cost the session its settings.
//!
//! At login a config that cannot be used at all (a syntax error, a value the
//! loader cannot finish) falls back to the last accepted document for that config
//! path, and only then to the embedded defaults. This module is that store.
//!
//! Three properties matter.
//!
//! *Atomic.* A save writes a temporary file in the state directory, syncs it,
//! renames it over the old one and syncs the directory. A crash mid-save leaves
//! the previous document readable, never half a document: a torn file would be
//! refused anyway, but a refused last-good turns a recoverable start into a fresh
//! one.
//!
//! *Private.* The state directory is set to `0700` and the file to `0600`, and
//! the load refuses a file or directory that is readable by another user. The
//! document can name an autostart command or a prompter policy, which is not
//! something another account should be able to read or rewrite.
//!
//! *Distrustful.* The stored document is re-validated on load and tagged with the
//! config path it came from. A document written by an older schema can hold a key
//! this version no longer accepts; it is then refused and reported, so the
//! session starts from its defaults instead of running on settings nobody meant.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::document::Config;
use crate::problem::Report;
use crate::value::Value;

/// The format of the stored envelope.
///
/// Bumped only when the envelope's own shape changes. A change to the *document*
/// schema does not need a bump: the stored document is re-validated, so a stale
/// key is reported as a problem rather than applied.
pub const FORMAT: u64 = 1;

/// The file name inside the state directory.
const FILE_NAME: &str = "last-good-config.json";

/// The directory Gnoblin owns under the state home.
const DIRECTORY_NAME: &str = "gnoblin";

/// Where the last accepted document lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    directory: PathBuf,
}

impl Store {
    /// A store under `state_home`, which is `$XDG_STATE_HOME` or
    /// `$HOME/.local/state`.
    pub fn new(state_home: &Path) -> Store {
        Store {
            directory: state_home.join(DIRECTORY_NAME),
        }
    }

    /// A store from the process environment.
    ///
    /// `None` when neither `XDG_STATE_HOME` nor `HOME` is set: there is nowhere
    /// to write, and the session simply has no last-good document.
    pub fn from_env() -> Option<Store> {
        let variable = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|value| !value.as_os_str().is_empty())
        };
        if let Some(state_home) = variable("XDG_STATE_HOME").filter(|path| path.is_absolute()) {
            return Some(Store::new(&state_home));
        }
        variable("HOME")
            .filter(|path| path.is_absolute())
            .map(|home| Store::new(&home.join(".local").join("state")))
    }

    /// The directory the document is stored in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The stored document's path.
    pub fn path(&self) -> PathBuf {
        self.directory.join(FILE_NAME)
    }

    /// Store `config` as the document accepted from `source`.
    ///
    /// The document is validated first: the store never becomes a way to smuggle
    /// a document past validation, and a caller cannot save a config it built by
    /// hand.
    pub fn save(&self, source: &Path, config: &Config) -> Result<(), StoreError> {
        let source = source.to_str().ok_or_else(|| StoreError::Unrepresentable {
            path: source.to_path_buf(),
        })?;
        let envelope = serde_json::json!({
            "format": FORMAT,
            "source": source,
            "document": config.document().to_json(),
        });
        let bytes = serde_json::to_vec(&envelope).map_err(|error| StoreError::Malformed {
            detail: error.to_string(),
        })?;
        write_atomically(&self.path(), &bytes)
    }

    /// Read back the document accepted from `source`.
    pub fn load(&self, source: &Path) -> Result<StoredDocument, StoreError> {
        let path = self.path();
        check_private(&self.directory, true)?;
        check_private(&path, false)?;
        let bytes = fs::read(&path).map_err(|error| StoreError::Io {
            action: "read",
            path: path.clone(),
            error,
        })?;
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| StoreError::Malformed {
            detail: error.to_string(),
        })?;
        let format = envelope
            .get("format")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| StoreError::Malformed {
                detail: "the envelope has no format".to_owned(),
            })?;
        if format != FORMAT {
            return Err(StoreError::Format { found: format });
        }
        let stored_source = envelope
            .get("source")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| StoreError::Malformed {
                detail: "the envelope has no source".to_owned(),
            })?;
        if Path::new(stored_source) != source {
            return Err(StoreError::OtherSource {
                stored: PathBuf::from(stored_source),
            });
        }
        let document = envelope
            .get("document")
            .and_then(Value::from_json)
            .ok_or_else(|| StoreError::Malformed {
                detail: "the envelope holds no document".to_owned(),
            })?;
        let config = Config::from_value(document).map_err(StoreError::Rejected)?;
        Ok(StoredDocument {
            config,
            format,
            source: PathBuf::from(stored_source),
        })
    }

    /// Accept a freshly built document, saving it or falling back to the stored
    /// one.
    ///
    /// This is the whole recovery rule in one place, so the compositor, the
    /// runtime and a test cannot each implement it differently.
    pub fn accept(&self, source: &Path, built: Result<Config, Report>) -> Resolution {
        match built {
            Ok(config) => {
                // A failed save does not make the document unusable; it only
                // means the next start cannot recover from a broken config. The
                // reason is kept so the session can log it.
                let save_error = self.save(source, &config).err();
                Resolution {
                    config: Some(config),
                    status: Status::Current,
                    save_error,
                }
            }
            Err(reason) => match self.load(source) {
                Ok(stored) => Resolution {
                    config: Some(stored.config),
                    status: Status::LastGood { reason },
                    save_error: None,
                },
                Err(last_good) => Resolution {
                    config: None,
                    status: Status::Unusable { reason, last_good },
                    save_error: None,
                },
            },
        }
    }
}

/// What a load found.
#[derive(Debug)]
pub struct StoredDocument {
    /// The document, re-validated.
    pub config: Config,
    /// The envelope version it was written with, always [`FORMAT`].
    pub format: u64,
    /// The config path it was accepted from.
    pub source: PathBuf,
}

/// Why a document could not be stored or read back.
#[derive(Debug)]
pub enum StoreError {
    /// The filesystem refused an operation.
    Io {
        /// What was attempted: `create`, `read`, `write`, `sync`, `rename`.
        action: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The operating system's reason.
        error: io::Error,
    },
    /// The state directory or the file is reachable by another user.
    Insecure {
        /// The path that is not private to this user.
        path: PathBuf,
    },
    /// The file is not a readable envelope: a torn write, or a foreign file.
    Malformed {
        /// What the reader objected to.
        detail: String,
    },
    /// The envelope is a version this build does not read.
    Format {
        /// The version that was found.
        found: u64,
    },
    /// The stored document belongs to a different config path.
    OtherSource {
        /// The path it was written for.
        stored: PathBuf,
    },
    /// The stored document does not validate against this version's schema.
    Rejected(Report),
    /// The config path is not representable as UTF-8, so it cannot be recorded.
    Unrepresentable {
        /// The path involved.
        path: PathBuf,
    },
}

impl StoreError {
    /// True when there is no stored document at all: the state directory or the
    /// file does not exist. That is the normal state of a first boot, not a fault.
    #[must_use]
    pub fn is_absent(&self) -> bool {
        matches!(self, StoreError::Io { error, .. } if error.kind() == io::ErrorKind::NotFound)
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io { action, path, error } => {
                write!(formatter, "could not {action} {}: {error}", path.display())
            }
            StoreError::Insecure { path } => write!(
                formatter,
                "{} is not private to this user, so the saved configuration was not used",
                path.display()
            ),
            StoreError::Malformed { detail } => {
                write!(formatter, "the saved configuration is malformed: {detail}")
            }
            StoreError::Format { found } => write!(
                formatter,
                "the saved configuration uses format {found}, and this build reads {FORMAT}"
            ),
            StoreError::OtherSource { stored } => write!(
                formatter,
                "the saved configuration was accepted for {}, not for this config path",
                stored.display()
            ),
            StoreError::Rejected(report) => {
                write!(formatter, "the saved configuration no longer validates: {report}")
            }
            StoreError::Unrepresentable { path } => {
                write!(
                    formatter,
                    "{} is not valid UTF-8, so it cannot be recorded",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// What a start or a reload decided.
#[derive(Debug)]
pub struct Resolution {
    /// The document to apply, or `None` when the caller must use its own
    /// defaults.
    pub config: Option<Config>,
    /// Why this document was chosen, as the object model publishes it.
    pub status: Status,
    /// Why the accepted document could not be stored, when it could not.
    pub save_error: Option<StoreError>,
}

impl Resolution {
    /// The accepted document, when there is one.
    pub fn config(&self) -> Option<&Config> {
        self.config.as_ref()
    }

    /// The status name a shell or `gnoblinctl` shows.
    pub fn status_name(&self) -> &'static str {
        self.status.as_str()
    }
}

/// Why the applied document is the one that is applied.
#[derive(Debug)]
pub enum Status {
    /// The configuration on disk was accepted.
    Current,
    /// The configuration on disk failed entirely, so the last accepted document
    /// for the same path was used instead.
    LastGood {
        /// Why the configuration on disk was refused.
        reason: Report,
    },
    /// Neither the configuration on disk nor a stored document could be used, so
    /// the session runs on its built-in defaults.
    Unusable {
        /// Why the configuration on disk was refused.
        reason: Report,
        /// Why no stored document was used.
        last_good: StoreError,
    },
}

impl Status {
    /// The name the object model publishes: `current`, `using-last-good` or
    /// `unusable`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Current => "current",
            Status::LastGood { .. } => "using-last-good",
            Status::Unusable { .. } => "unusable",
        }
    }

    /// Why the on-disk configuration was refused, when it was.
    pub fn reason(&self) -> Option<&Report> {
        match self {
            Status::Current => None,
            Status::LastGood { reason } | Status::Unusable { reason, .. } => Some(reason),
        }
    }

    /// Why no stored document was used, when one was expected.
    pub fn last_good_error(&self) -> Option<&StoreError> {
        match self {
            Status::Unusable { last_good, .. } => Some(last_good),
            _ => None,
        }
    }
}

/// Create the state directory private to this user.
fn ensure_private_directory(directory: &Path) -> Result<(), StoreError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|error| StoreError::Io {
            action: "create",
            path: directory.to_path_buf(),
            error,
        })?;
    // `mode` applies only to directories this call created, so set it again for
    // one that already existed.
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|error| StoreError::Io {
        action: "secure",
        path: directory.to_path_buf(),
        error,
    })
}

/// Write `bytes` to `path` so a reader sees the old file or the new one.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().unwrap_or(Path::new("."));
    ensure_private_directory(directory)?;
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map_or_else(|| FILE_NAME.into(), |name| name.to_os_string());
    let temporary = directory.join(format!(
        ".{}.tmp-{}-{sequence}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let written = write_and_sync(&temporary, bytes).and_then(|()| {
        fs::rename(&temporary, path).map_err(|error| StoreError::Io {
            action: "rename",
            path: path.to_path_buf(),
            error,
        })
    });
    if let Err(error) = written {
        // Leave no temporary file behind: the next load would have to explain it.
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    // Syncing the directory makes the rename durable, which is the difference
    // between "the file is new" and "the file is new after a power cut".
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| StoreError::Io {
            action: "sync",
            path: directory.to_path_buf(),
            error,
        })
}

/// Write a temporary file that only this user can read, and flush it.
fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| StoreError::Io {
            action: "create",
            path: path.to_path_buf(),
            error,
        })?;
    file.write_all(bytes).map_err(|error| StoreError::Io {
        action: "write",
        path: path.to_path_buf(),
        error,
    })?;
    file.sync_all().map_err(|error| StoreError::Io {
        action: "sync",
        path: path.to_path_buf(),
        error,
    })
}

/// Refuse a path that another user could read, write or replace.
///
/// A stored document is only trustworthy when both the directory and the file
/// are private to this user and owned by this user: a directory another account
/// created would let that account swap the document between the check and the
/// read. The process's own id comes from `/proc/self`, because reading it needs
/// `geteuid`, which is a C call this workspace forbids without an FFI shim;
/// Gnoblin runs on Linux, and a system without `/proc` fails closed.
fn check_private(path: &Path, directory: bool) -> Result<(), StoreError> {
    let our_id = effective_uid().ok_or_else(|| StoreError::Insecure {
        path: path.to_path_buf(),
    })?;
    let metadata = fs::symlink_metadata(path).map_err(|error| StoreError::Io {
        action: "inspect",
        path: path.to_path_buf(),
        error,
    })?;
    let kind_ok = if directory {
        metadata.is_dir()
    } else {
        // `symlink_metadata` does not follow a link, so a symlink is not a file
        // here and is refused rather than followed.
        metadata.is_file()
    };
    let private = metadata.permissions().mode() & 0o077 == 0;
    let ours = std::os::unix::fs::MetadataExt::uid(&metadata) == our_id;
    if kind_ok && private && ours {
        return Ok(());
    }
    Err(StoreError::Insecure {
        path: path.to_path_buf(),
    })
}

/// The process's effective user id, from `/proc/self`.
fn effective_uid() -> Option<u32> {
    let metadata = fs::metadata("/proc/self").ok()?;
    Some(std::os::unix::fs::MetadataExt::uid(&metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_names_what_the_shell_shows() {
        assert_eq!(Status::Current.as_str(), "current");
        assert_eq!(
            Status::LastGood {
                reason: Report::new(vec![crate::problem::Problem::new(
                    crate::path::Path::root(),
                    crate::problem::Fault::NotFinite
                )]),
            }
            .as_str(),
            "using-last-good"
        );
    }
}
