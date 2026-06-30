//! Std-only shared helpers for gnoblin shell clients.

use std::error::Error;

pub type RuntimeError = Box<dyn Error + Send + Sync>;

pub fn runtime_error(message: impl Into<String>) -> RuntimeError {
    Box::new(std::io::Error::other(message.into()))
}

/// Last-modified time of a file, or `None` if the path is `None` or unreadable.
/// Layer-shell clients poll this to live-reload their config/pins on change.
pub fn file_mtime(path: Option<&std::path::Path>) -> Option<std::time::SystemTime> {
    path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// A cross-process boolean flag backed by the presence of a file in
/// `$XDG_RUNTIME_DIR`.
pub struct FileFlag {
    name: &'static str,
}

impl FileFlag {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    fn path(&self) -> Option<std::path::PathBuf> {
        std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|d| std::path::PathBuf::from(d).join(self.name))
    }

    /// Is the flag currently set?
    pub fn is_on(&self) -> bool {
        self.path().map(|p| p.exists()).unwrap_or(false)
    }

    /// Set or clear the flag.
    pub fn set(&self, on: bool) {
        let Some(p) = self.path() else { return };
        if on {
            let _ = std::fs::write(&p, b"");
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }

    /// Flip the flag, returning the new state.
    pub fn toggle(&self) -> bool {
        let next = !self.is_on();
        self.set(next);
        next
    }
}

pub mod args;
pub mod config;
pub mod dnd;
pub mod nightlight;
pub mod test_support;
pub mod xdg;

pub use args::ClientArgs;

/// A human-friendly app name from an app-id: drop `.desktop`, take the segment
/// after the last `.` (reverse-DNS tail), and capitalise.
pub fn prettify_app(app_id: &str) -> String {
    let base = app_id
        .strip_suffix(".desktop")
        .unwrap_or(app_id)
        .rsplit('.')
        .next()
        .unwrap_or(app_id);
    if base.is_empty() {
        return String::new();
    }
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
