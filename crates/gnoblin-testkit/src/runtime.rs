//! A private runtime directory for one test session.
//!
//! Wayland sockets, D-Bus bus files, PipeWire sockets and lock files all live in
//! `XDG_RUNTIME_DIR`, and two parity targets that shared one would find each
//! other's sockets and fail in ways that have nothing to do with the behaviour
//! under test. Each run therefore gets its own directory, created mode `0700`
//! the way the XDG base directory specification requires, and removed when the
//! fixture drops unless the test asked to keep it.
//!
//! The directory cannot be exported by setting the variable in this process:
//! `std::env::set_var` is unsafe under edition 2024 and this workspace forbids
//! unsafe. Callers pass [`RuntimeDir::path`] to the child they spawn with
//! `Command::env`, which is safe and keeps the isolation per child.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A unique directory under the system temporary directory, owned by one test.
#[derive(Debug)]
pub struct RuntimeDir {
    root: PathBuf,
    keep: bool,
}

impl RuntimeDir {
    /// Create `$TMPDIR/gnoblin-<label>-<pid>-<n>`.
    ///
    /// The label plus a process id plus a per-process counter makes the path
    /// unique without a clock, so two tests running in one process never share a
    /// directory and the path is still recognisable in a failure message.
    pub fn new(label: &str) -> io::Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("gnoblin-{label}-{}-{sequence}", std::process::id()));
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&root)?;
        // `mode` applies only to directories this call created, so set it again
        // for a path that already existed.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(Self { root, keep: false })
    }

    /// The directory itself, for `XDG_RUNTIME_DIR`.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// An entry inside the directory, for a socket or bus path that already
    /// exists but cannot be created yet.
    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.root.join(name)
    }

    /// Leave the directory on disk when the fixture drops. A failing run keeps
    /// its sockets and logs so the failure can be read afterwards.
    pub fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        if !self.keep {
            // A failure to remove is reported nowhere useful inside `drop`, and
            // the directory is under the temporary directory, so the system
            // cleans it up eventually.
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

impl fmt::Display for RuntimeDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.root.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_exists_with_a_private_mode() {
        let dir = RuntimeDir::new("runtime-mode").expect("runtime dir");
        let metadata = fs::metadata(dir.path()).expect("metadata");
        assert!(metadata.is_dir(), "{} is not a directory", dir.path().display());
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            0o700,
            "a runtime directory must not be readable by other users"
        );
    }

    #[test]
    fn two_fixtures_get_different_directories() {
        let first = RuntimeDir::new("runtime-unique").expect("first");
        let second = RuntimeDir::new("runtime-unique").expect("second");
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn the_directory_is_removed_on_drop() {
        let path = {
            let dir = RuntimeDir::new("runtime-drop").expect("runtime dir");
            let path = dir.path().to_path_buf();
            fs::write(dir.join("wayland-0"), b"stale").expect("write socket stand-in");
            path
        };
        assert!(!path.exists(), "{} survived the fixture", path.display());
    }

    #[test]
    fn a_kept_directory_survives_the_fixture() {
        let mut dir = RuntimeDir::new("runtime-keep").expect("runtime dir");
        dir.keep();
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(path.exists(), "keeping the directory must leave it on disk");
        fs::remove_dir_all(&path).expect("clean up the kept directory");
    }
}
