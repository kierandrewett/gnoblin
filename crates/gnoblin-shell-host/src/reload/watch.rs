//! Watches the files a native shell is loaded from and turns a change into a reload request.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::watch::{ConfigWatcher, ReloadRequest, WatchError};

use super::{ReloadController, ReloadTrigger};

/// Watches the module file of a native shell (and, for a launcher file, the launcher and its target), so that a rebuild
/// reloads the shell.
///
/// Only those files matter: a shell's build directory (`target/debug`) is full of files a build writes that are not the
/// library, and a reload for each would be a reload for nothing. A symlink is watched at both ends, because the build
/// writes the target and the user may have replaced the link. The files need not exist yet: a build that has not finished
/// leaves none, and its arrival is the event.
pub struct ModuleWatch {
    watcher: ConfigWatcher,
}

impl std::fmt::Debug for ModuleWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleWatch")
            .field("dirs", &self.watcher.watched_dirs())
            .finish()
    }
}

impl ModuleWatch {
    /// Watches `files` (each a module, a launcher or a launcher's target) with `debounce` of quiet after an event.
    ///
    /// # Errors
    ///
    /// A file's directory cannot be resolved or watched.
    pub fn new(files: &[PathBuf], debounce: Duration) -> Result<Self, WatchError> {
        let (dirs, wanted) = Self::targets(files)?;
        let first = dirs.iter().next().cloned().ok_or_else(|| WatchError::Resolve {
            path: PathBuf::new(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file to watch"),
        })?;
        let modules: Vec<PathBuf> = wanted.iter().cloned().collect();
        let mut watcher = ConfigWatcher::new(&first, debounce, move |path| wanted.contains(path))?;
        watcher.watch_modules(&modules)?;
        Ok(Self { watcher })
    }

    /// The canonical directories to watch and the canonical paths of the files that count: each file as named and, when it is
    /// a link that resolves, the file it points at.
    fn targets(files: &[PathBuf]) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>), WatchError> {
        let mut dirs = BTreeSet::new();
        let mut wanted = BTreeSet::new();
        for file in files {
            let mut names = vec![file.clone()];
            if let Ok(resolved) = fs::canonicalize(file) {
                names.push(resolved);
            }
            for name in names {
                let Some(parent) = name.parent().map(|parent| {
                    if parent.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        parent
                    }
                }) else {
                    continue;
                };
                let parent = fs::canonicalize(parent).map_err(|source| WatchError::Resolve {
                    path: parent.to_path_buf(),
                    source,
                })?;
                if let Some(leaf) = name.file_name() {
                    wanted.insert(parent.join(leaf));
                }
                dirs.insert(parent);
            }
        }
        Ok((dirs, wanted))
    }

    /// Waits up to `wait` for a change of a watched file and returns the debounced request.
    pub fn next_request(&self, wait: Duration) -> Option<ReloadRequest> {
        self.watcher.next_request(wait)
    }

    /// Waits up to `wait` for a change and, if one comes, asks `controller` for a reload with the files that changed.
    /// Returns whether a request was made.
    pub fn forward_to(&self, controller: &ReloadController, wait: Duration) -> bool {
        match self.next_request(wait) {
            Some(request) => {
                controller.request(ReloadTrigger::Files(request.paths));
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const DEBOUNCE: Duration = Duration::from_millis(300);
    const PATIENCE: Duration = Duration::from_secs(10);
    const QUIET: Duration = Duration::from_millis(900);

    #[test]
    fn a_rebuild_of_the_module_is_a_request_and_its_neighbours_are_not() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the dir exists");
        let module = root.join("libshell.so");
        fs::write(&module, b"v1").expect("write");
        let watch = ModuleWatch::new(std::slice::from_ref(&module), DEBOUNCE).expect("a watch");

        fs::write(root.join("libother.so"), b"noise").expect("write");
        fs::write(root.join("build.log"), b"noise").expect("write");
        assert_eq!(watch.next_request(QUIET), None, "only the module counts");

        fs::write(&module, b"v2").expect("write");
        let request = watch.next_request(PATIENCE).expect("the rebuild is seen");
        assert_eq!(request.paths, vec![module]);
    }

    #[test]
    fn a_module_that_does_not_exist_yet_is_seen_when_the_build_creates_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the dir exists");
        let module = root.join("libshell.so");
        let watch = ModuleWatch::new(std::slice::from_ref(&module), DEBOUNCE).expect("a watch");
        fs::write(&module, b"v1").expect("write");
        assert!(watch.next_request(PATIENCE).is_some(), "its arrival is the event");
    }

    #[test]
    fn a_link_is_watched_at_both_ends() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the dir exists");
        let build = root.join("target");
        fs::create_dir(&build).expect("mkdir");
        let real = build.join("libshell.so");
        fs::write(&real, b"v1").expect("write");
        let link = root.join("shell.so");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let watch = ModuleWatch::new(std::slice::from_ref(&link), DEBOUNCE).expect("a watch");
        fs::write(&real, b"v2").expect("write");
        let request = watch.next_request(PATIENCE).expect("the build writes the target");
        assert_eq!(request.paths, vec![real]);
    }

    #[test]
    fn a_change_is_forwarded_to_the_controller_as_one_request() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the dir exists");
        let module = root.join("libshell.so");
        fs::write(&module, b"v1").expect("write");
        let watch = ModuleWatch::new(std::slice::from_ref(&module), DEBOUNCE).expect("a watch");
        let controller = ReloadController::new();
        assert!(!watch.forward_to(&controller, QUIET));
        fs::write(&module, b"v2").expect("write");
        assert!(watch.forward_to(&controller, PATIENCE));
        assert_eq!(controller.take_request(), Some(ReloadTrigger::Files(vec![module])));
    }

    #[test]
    fn watching_nothing_is_an_error() {
        let error = ModuleWatch::new(&[], DEBOUNCE).expect_err("no file");
        assert!(error.to_string().contains("no file to watch"), "{error}");
    }
}
