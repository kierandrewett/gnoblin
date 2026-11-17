//! Seeing an edit: the files a generation imported, and the request for a reload.
//!
//! The loader (`gnoblin/_loader.py`) knows every file the serving generation was made from (the script and each module it
//! imported, however deep) and records what each looked like when it was read. The runtime looks at them again a few times a second
//! and, when one differs, asks the host for a reload. It does not reload by itself: the host serves the request between turns through
//! the loader every language shares, so a script reloads with the same protocol, counters and log lines as a native shell.
//!
//! A file counts as changed when its modification time, size or inode differ, so an edit that keeps the size and lands in the same
//! second is seen, and so is an editor's write-then-rename. A file that is gone is a change too, and one that stays gone is not a
//! second one.

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_core_abi::GnoblinCtx;

use crate::generation::{self, with_state};

/// How often the files are looked at.
const INTERVAL: Duration = Duration::from_millis(100);

/// What a file looked like: modification time in nanoseconds, size and inode; all `-1`/`0` for a file that is not there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    mtime_ns: i128,
    size: u64,
    inode: u64,
}

impl Stamp {
    fn of(path: &Path) -> Stamp {
        match std::fs::metadata(path) {
            Ok(meta) => Stamp {
                mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
                size: meta.size(),
                inode: meta.ino(),
            },
            Err(_) => Stamp {
                mtime_ns: -1,
                size: 0,
                inode: 0,
            },
        }
    }
}

/// The files of the serving generation and the state of the request for a reload.
#[derive(Default)]
pub(crate) struct Watch {
    files: Vec<(PathBuf, Stamp)>,
    last: Option<Instant>,
    /// Whether a reload was asked for and has not started yet.
    requested: bool,
}

impl Watch {
    /// Watches `files`, each with the stamp the loader read it at.
    pub(crate) fn replace(&mut self, files: Vec<(String, i128, u64, u64)>) {
        self.files = files
            .into_iter()
            .map(|(path, mtime_ns, size, inode)| (PathBuf::from(path), Stamp { mtime_ns, size, inode }))
            .collect();
        self.requested = false;
    }

    /// Stops watching: no generation serves.
    pub(crate) fn clear(&mut self) {
        self.files.clear();
        self.requested = false;
    }

    /// The first watched file that no longer looks as it did.
    fn changed(&self) -> Option<&Path> {
        self.files
            .iter()
            .find(|(path, stamp)| Stamp::of(path) != *stamp)
            .map(|(path, _)| path.as_path())
    }
}

/// Looks at the files if it is time to, and asks the host for a reload when one changed.
pub(crate) fn poll(ctx: GnoblinCtx) {
    let asked = with_state(|state| {
        if state.serving != Some(ctx) || state.watch.requested {
            return None;
        }
        let now = Instant::now();
        if state.watch.last.is_some_and(|last| now.duration_since(last) < INTERVAL) {
            return None;
        }
        state.watch.last = Some(now);
        let changed = state.watch.changed()?.to_path_buf();
        state.watch.requested = true;
        state.handles.get(&ctx).cloned().map(|handle| (handle, changed))
    })
    .flatten();
    if let Some((handle, changed)) = asked {
        if let Err(error) = handle.request_reload() {
            generation::log(
                gnoblin_core::LogLevel::Warn,
                &format!("python: the reload for {} was refused: {error}", changed.display()),
            );
            with_state(|state| state.watch.requested = false);
        } else {
            generation::log(
                gnoblin_core::LogLevel::Info,
                &format!("python: {} changed, asking the host to reload", changed.display()),
            );
        }
    }
}
