//! Test fixtures: a temporary directory that removes itself, and writers for
//! OpenXR manifests and fake executables.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

/// A directory under the system temporary directory, removed on drop.
pub struct TempDir {
    root: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("xr-runtime-{label}-{}-{n}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test directory");
        Self { root }
    }

    pub fn join(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
    fs::write(path, contents).expect("write file");
}

/// Writes a version 1.0.0 runtime manifest that names `library_path`.
pub fn write_manifest(path: &Path, library_path: &str) {
    write_file(
        path,
        &format!(r#"{{"file_format_version":"1.0.0","runtime":{{"name":"Test","library_path":"{library_path}"}}}}"#),
    );
}

/// Writes an empty file with the execute bits set.
#[cfg(unix)]
pub fn write_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    write_file(path, "");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}
