//! Watches the config directory and turns bursts of file events into one [`ReloadRequest`].
//!
//! An editor save is several events: vim writes `4913` to test the directory, writes a `~` backup,
//! renames, then writes `init.ts`; an atomic save replaces the inode. So the watcher
//!
//! - watches parent *directories*, non-recursively, because a watch on a file dies with its inode;
//! - drops editor temp files and anything that is not a module file (`.ts`, `.mts`, `.js`, `.mjs`);
//! - waits for a quiet period (the debounce) after the first event, and returns one request with every
//!   changed path, with a cap so a file that is rewritten forever cannot postpone the reload forever.
//!
//! Which directories are watched comes from the module graph of the last generation
//! ([`JsGeneration::module_paths`](crate::JsGeneration::module_paths)), so an imported `./lib/x.ts` in a
//! sub-directory is observed and a file the config never imports can only matter if it sits beside one
//! that it does. Any module file in a watched directory triggers a reload, imported or not: a new
//! `./x.ts` can shadow an import that resolved to `./x.js`, and a file that failed to load is not in
//! any graph until it is fixed.
//!
//! A watcher error, and the kernel's queue overflow, mean events may have been lost. Both surface as a
//! request with no paths, which reloads, because "possibly changed" must not be "unchanged".

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;

/// A debounced request to reload the config.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReloadRequest {
    /// The module files that changed, sorted and without duplicates. Empty when the watcher may have
    /// lost events and cannot say which files changed.
    pub paths: Vec<PathBuf>,
}

/// A failure to set up or re-arm the watcher.
#[derive(Debug, Error)]
pub enum JsWatchError {
    /// The directory does not exist or cannot be resolved.
    #[error("cannot resolve the config directory {}: {source}", path.display())]
    Resolve {
        /// The directory as given.
        path: PathBuf,
        /// The file system error.
        source: std::io::Error,
    },
    /// The operating system's watcher refused a directory, for example when `max_user_watches` is spent.
    #[error("cannot watch {}: {source}", path.display())]
    Watch {
        /// The directory that could not be watched.
        path: PathBuf,
        /// The watcher's error.
        source: notify::Error,
    },
    /// The operating system's watcher could not be created.
    #[error("cannot start the file watcher: {0}")]
    Start(#[source] notify::Error),
}

/// What the notify thread sends the reload loop.
#[derive(Debug)]
enum WatchMessage {
    /// Module files that changed.
    Changed(Vec<PathBuf>),
    /// Events may have been lost.
    Rescan,
}

/// A debouncing watcher over a config directory and the directories of its imported modules.
pub struct ConfigWatcher {
    watcher: RecommendedWatcher,
    events: Receiver<WatchMessage>,
    root: PathBuf,
    /// The directories currently watched.
    dirs: BTreeSet<PathBuf>,
    debounce: Duration,
    max_burst: Duration,
}

impl std::fmt::Debug for ConfigWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigWatcher")
            .field("root", &self.root)
            .field("dirs", &self.dirs)
            .field("debounce", &self.debounce)
            .finish_non_exhaustive()
    }
}

impl ConfigWatcher {
    /// The quiet time that ends a burst of events, when the caller does not choose one.
    pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(100);

    /// A watcher over `root`, the config directory, that waits `debounce` of quiet after an event
    /// before it reports. A burst that never goes quiet is reported after 20 debounces.
    ///
    /// Only `root` is watched until [`watch_modules`](Self::watch_modules) adds the directories of
    /// imported modules.
    pub fn new(root: &Path, debounce: Duration) -> Result<Self, JsWatchError> {
        let root = fs::canonicalize(root).map_err(|source| JsWatchError::Resolve {
            path: root.to_path_buf(),
            source,
        })?;
        let (sender, events) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let message = match result {
                Ok(event) => match classify(&event) {
                    Some(message) => message,
                    None => return,
                },
                // An error from the backend means events were possibly dropped.
                Err(_) => WatchMessage::Rescan,
            };
            // The receiver is gone when the watcher is being dropped; nothing is left to tell.
            let _ = sender.send(message);
        })
        .map_err(JsWatchError::Start)?;
        let mut this = Self {
            watcher,
            events,
            root,
            dirs: BTreeSet::new(),
            debounce,
            max_burst: debounce.saturating_mul(20),
        };
        let root = this.root.clone();
        this.watch_dir(&root)?;
        Ok(this)
    }

    /// The config directory, canonical.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directories being watched, sorted.
    pub fn watched_dirs(&self) -> Vec<PathBuf> {
        self.dirs.iter().cloned().collect()
    }

    /// Points the watcher at the directories of `modules`, the module graph of the current generation,
    /// plus the config directory. Directories that are no longer needed are released.
    ///
    /// Call it after every generation, so a newly imported sub-directory is observed.
    pub fn watch_modules(&mut self, modules: &[PathBuf]) -> Result<(), JsWatchError> {
        let mut wanted = BTreeSet::from([self.root.clone()]);
        wanted.extend(
            modules
                .iter()
                .filter_map(|module| module.parent())
                .map(Path::to_path_buf),
        );
        for dir in self.dirs.difference(&wanted).cloned().collect::<Vec<_>>() {
            // A directory that was deleted is already unwatched by the kernel; nothing to undo.
            let _ = self.watcher.unwatch(&dir);
            self.dirs.remove(&dir);
        }
        let added: Vec<PathBuf> = wanted.difference(&self.dirs).cloned().collect();
        let mut first_error = None;
        for dir in &added {
            if let Err(error) = self.watch_dir(dir) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn watch_dir(&mut self, dir: &Path) -> Result<(), JsWatchError> {
        self.watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|source| JsWatchError::Watch {
                path: dir.to_path_buf(),
                source,
            })?;
        self.dirs.insert(dir.to_path_buf());
        Ok(())
    }

    /// Waits up to `wait` for a relevant change and returns the debounced request, or `None` when
    /// nothing relevant happened.
    ///
    /// After the first event it keeps reading until `debounce` passes with no further event, so an
    /// editor's several writes are one request. `wait` bounds only the wait for that first event.
    pub fn next_request(&self, wait: Duration) -> Option<ReloadRequest> {
        let mut changed = BTreeSet::new();
        match self.events.recv_timeout(wait) {
            Ok(message) => absorb(&mut changed, message),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
        }
        let burst_end = Instant::now() + self.max_burst;
        loop {
            let left = burst_end.saturating_duration_since(Instant::now());
            let quiet = self.debounce.min(left);
            if quiet.is_zero() {
                break;
            }
            match self.events.recv_timeout(quiet) {
                Ok(message) => absorb(&mut changed, message),
                Err(_) => break,
            }
        }
        Some(ReloadRequest {
            paths: changed.into_iter().collect(),
        })
    }
}

/// Folds one message into the set of changed paths. A rescan adds nothing: an empty request is the
/// "possibly everything" signal, and paths found alongside it are still reported.
fn absorb(changed: &mut BTreeSet<PathBuf>, message: WatchMessage) {
    match message {
        WatchMessage::Changed(paths) => changed.extend(paths),
        WatchMessage::Rescan => {}
    }
}

/// The message a raw event is worth, or `None` for an event that cannot change a module.
fn classify(event: &Event) -> Option<WatchMessage> {
    if event.need_rescan() {
        return Some(WatchMessage::Rescan);
    }
    // Reads and opens cannot change a file; every write also arrives as a modify.
    if matches!(event.kind, EventKind::Access(_)) {
        return None;
    }
    let paths: Vec<PathBuf> = event
        .paths
        .iter()
        .filter(|path| is_module_file(path))
        .cloned()
        .collect();
    (!paths.is_empty()).then_some(WatchMessage::Changed(paths))
}

/// Whether `path` names a file the loader can run, and is not an editor's scratch file.
pub(crate) fn is_module_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if is_editor_junk(name) {
        return false;
    }
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("ts" | "mts" | "js" | "mjs")
    )
}

/// Editor temp, backup and lock file names.
///
/// - `init.ts~`: backup (vim, emacs, gedit).
/// - `.init.ts.swp`, `.swo`, `.swx`: vim swap files.
/// - `4913` and its successors: vim's "can I write here" probe, only digits.
/// - `.#init.ts`: emacs lock link. `#init.ts#`: emacs auto-save.
fn is_editor_junk(name: &str) -> bool {
    name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swo")
        || name.ends_with(".swx")
        || name.starts_with(".#")
        || (name.starts_with('#') && name.ends_with('#'))
        || (!name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{ConfigWatcher, is_editor_junk, is_module_file};

    /// The quiet time that ends a burst. A test writes its burst in microseconds, but a loaded machine can stall the
    /// writing thread for far longer than a short debounce, and the stalled burst would be reported as two requests.
    /// The debounce is longer than any stall a test machine is likely to see.
    const DEBOUNCE: Duration = Duration::from_secs(1);
    /// Long enough for inotify to deliver and the debounce to pass on a loaded CI machine.
    const PATIENCE: Duration = Duration::from_secs(10);
    /// Long enough that an event already on its way has arrived, so a test that expects none sees none. It never
    /// decides whether a burst was one request: the debounce does.
    const QUIET: Duration = Duration::from_millis(600);

    fn config_dir() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = fs::canonicalize(dir.path()).expect("the temp dir exists");
        (dir, path)
    }

    fn watcher(root: &Path) -> ConfigWatcher {
        ConfigWatcher::new(root, DEBOUNCE).expect("a watcher")
    }

    #[test]
    fn junk_names_are_recognised() {
        for junk in [
            "init.ts~",
            ".init.ts.swp",
            ".init.ts.swx",
            "4913",
            "5036",
            ".#init.ts",
            "#init.ts#",
        ] {
            assert!(is_editor_junk(junk), "{junk}");
        }
        for real in ["init.ts", "x.mts", "a.js", "4913.ts", ".hidden.ts"] {
            assert!(!is_editor_junk(real), "{real}");
        }
    }

    #[test]
    fn only_module_files_are_relevant() {
        for yes in ["/c/init.ts", "/c/lib/x.mts", "/c/a.js", "/c/b.mjs"] {
            assert!(is_module_file(Path::new(yes)), "{yes}");
        }
        for no in [
            "/c/notes.txt",
            "/c/init.ts~",
            "/c/.init.ts.swp",
            "/c/4913",
            "/c/dir",
            "/c/x.d.ts.bak",
        ] {
            assert!(!is_module_file(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn a_write_to_init_ts_is_one_request_with_its_path() {
        let (_dir, root) = config_dir();
        fs::write(root.join("init.ts"), "1;\n").expect("init.ts");
        let watcher = watcher(&root);
        fs::write(root.join("init.ts"), "2;\n").expect("init.ts is rewritten");
        let request = watcher.next_request(PATIENCE).expect("a request");
        assert_eq!(request.paths, vec![root.join("init.ts")]);
        assert_eq!(watcher.next_request(QUIET), None, "one burst, one request");
    }

    #[test]
    fn a_vim_style_save_is_one_request_and_names_only_the_module() {
        let (_dir, root) = config_dir();
        fs::write(root.join("init.ts"), "1;\n").expect("init.ts");
        let watcher = watcher(&root);
        // The sequence vim produces: probe file, backup, swap file, then the real write.
        fs::write(root.join("4913"), "").expect("probe");
        fs::remove_file(root.join("4913")).expect("probe removed");
        fs::write(root.join(".init.ts.swp"), "swap").expect("swap");
        fs::rename(root.join("init.ts"), root.join("init.ts~")).expect("backup");
        fs::write(root.join("init.ts"), "2;\n").expect("new init.ts");
        fs::remove_file(root.join(".init.ts.swp")).expect("swap removed");
        let request = watcher.next_request(PATIENCE).expect("a request");
        assert_eq!(request.paths, vec![root.join("init.ts")]);
        assert_eq!(watcher.next_request(QUIET), None);
    }

    #[test]
    fn junk_and_non_module_files_alone_do_not_fire() {
        let (_dir, root) = config_dir();
        let watcher = watcher(&root);
        for junk in ["4913", ".init.ts.swp", "init.ts~", ".#init.ts", "notes.txt"] {
            fs::write(root.join(junk), "x").expect("junk file");
        }
        assert_eq!(watcher.next_request(QUIET), None);
    }

    #[test]
    fn fifty_rapid_writes_are_one_request() {
        let (_dir, root) = config_dir();
        let watcher = watcher(&root);
        for n in 0..50 {
            fs::write(root.join("init.ts"), format!("{n};\n")).expect("init.ts");
        }
        let request = watcher.next_request(PATIENCE).expect("a request");
        assert_eq!(request.paths, vec![root.join("init.ts")]);
        assert_eq!(watcher.next_request(QUIET), None, "no second request");
    }

    #[test]
    fn an_imported_module_in_a_subdirectory_is_watched_once_registered() {
        let (_dir, root) = config_dir();
        fs::create_dir(root.join("lib")).expect("lib");
        fs::write(root.join("init.ts"), "import './lib/x.ts';\n").expect("init.ts");
        fs::write(root.join("lib/x.ts"), "export {};\n").expect("x.ts");
        let mut watcher = watcher(&root);

        // Not registered yet: the root is watched, `lib` is not.
        fs::write(root.join("lib/x.ts"), "export const a = 1;\n").expect("x.ts");
        assert_eq!(watcher.next_request(QUIET), None);

        watcher
            .watch_modules(&[root.join("init.ts"), root.join("lib/x.ts")])
            .expect("modules register");
        assert_eq!(watcher.watched_dirs(), vec![root.clone(), root.join("lib")]);
        fs::write(root.join("lib/x.ts"), "export const a = 2;\n").expect("x.ts");
        let request = watcher.next_request(PATIENCE).expect("a request");
        assert_eq!(request.paths, vec![root.join("lib/x.ts")]);

        // Dropping the import releases the directory.
        watcher
            .watch_modules(&[root.join("init.ts")])
            .expect("modules re-register");
        assert_eq!(watcher.watched_dirs(), vec![root.clone()]);
        fs::write(root.join("lib/x.ts"), "export const a = 3;\n").expect("x.ts");
        assert_eq!(watcher.next_request(QUIET), None);
    }

    #[test]
    fn writes_to_two_modules_in_one_burst_name_both() {
        let (_dir, root) = config_dir();
        let watcher = watcher(&root);
        fs::write(root.join("b.ts"), "1;\n").expect("b.ts");
        fs::write(root.join("a.ts"), "1;\n").expect("a.ts");
        let request = watcher.next_request(PATIENCE).expect("a request");
        assert_eq!(request.paths, vec![root.join("a.ts"), root.join("b.ts")]);
    }

    #[test]
    fn a_missing_config_directory_is_an_error() {
        let (_dir, root) = config_dir();
        let error = ConfigWatcher::new(&root.join("absent"), DEBOUNCE).expect_err("no such directory");
        assert!(error.to_string().contains("absent"), "{error}");
    }
}
