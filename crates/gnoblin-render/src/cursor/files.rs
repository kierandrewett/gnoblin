//! Theme file access, the only place a cursor provider reads from.
//!
//! Providers see a [`CursorFiles`] and never the filesystem. The production
//! implementation, [`FsFiles`], searches the directories a format is installed
//! in; [`MemoryFiles`] serves tests with an in-memory theme tree. Every path a
//! provider asks for is relative to one theme directory, and a path that could
//! leave that directory is refused before any IO happens: theme files are
//! untrusted input, and a manifest saying `../../etc/passwd` must not read it.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The largest theme file a provider will read, in bytes.
///
/// A larger file is a [`FileError::TooLarge`], which the provider reports as a
/// named failure and the chain moves past. This bounds the memory and the
/// rasterisation time one hostile file can take.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Which family of theme directories a read searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SearchPath {
    /// Xcursor themes: `XCURSOR_PATH`, the user and system icon directories,
    /// and `/usr/share/pixmaps`.
    Xcursor,
    /// Compiled Hyprcursor themes: the user and XDG data icon directories.
    Hyprcursor,
    /// `cursors_scalable` themes: the user and system icon directories.
    Scalable,
}

/// Why a theme file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileError {
    /// The file exists but could not be read; the message is the IO error.
    Unreadable(String),
    /// The file is larger than [`MAX_FILE_BYTES`].
    TooLarge,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(reason) => write!(f, "unreadable: {reason}"),
            Self::TooLarge => write!(f, "larger than {MAX_FILE_BYTES} bytes"),
        }
    }
}

/// Read access to theme directories, in the shape every provider needs.
pub trait CursorFiles: Send + Sync {
    /// The bytes of `relative` inside `theme`, from the first directory of
    /// `root` that has it. `Ok(None)` means no directory has the file.
    fn read(&self, root: SearchPath, theme: &str, relative: &str) -> Result<Option<Vec<u8>>, FileError>;

    /// The file names directly inside `relative` inside `theme`, across the
    /// directories of `root`. A missing directory lists as empty.
    fn list(&self, root: SearchPath, theme: &str, relative: &str) -> Result<Vec<String>, FileError>;

    /// Whether any directory of any family holds a theme with this name.
    fn theme_exists(&self, theme: &str) -> bool;
}

/// Whether a theme name or relative path stays inside its theme directory.
///
/// Every component must be a plain name: no separators inside a component, no
/// `.` or `..`, no root. An empty path is refused too.
#[must_use]
pub fn is_contained(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && !path.contains('\0')
}

/// The directories a theme is searched in, read from the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsFiles {
    xcursor: Vec<PathBuf>,
    hyprcursor: Vec<PathBuf>,
    scalable: Vec<PathBuf>,
}

impl FsFiles {
    /// Searches the standard directories, using `XCURSOR_PATH`, `HOME` and
    /// `XDG_DATA_DIRS` when they are set.
    #[must_use]
    pub fn from_environment() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let user_icons: Vec<PathBuf> = home
            .iter()
            .flat_map(|home| [home.join(".local/share/icons"), home.join(".icons")])
            .collect();
        let system_icons = PathBuf::from("/usr/share/icons");

        let mut xcursor: Vec<PathBuf> = std::env::var_os("XCURSOR_PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default();
        xcursor.extend(user_icons.iter().cloned());
        xcursor.push(system_icons.clone());
        xcursor.push(PathBuf::from("/usr/share/pixmaps"));

        let mut hyprcursor = user_icons.clone();
        let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .map(|dirs| std::env::split_paths(&dirs).collect::<Vec<_>>())
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]);
        hyprcursor.extend(xdg_data_dirs.iter().map(|dir| dir.join("icons")));

        let mut scalable = user_icons;
        scalable.push(system_icons);

        Self {
            xcursor: dedupe(xcursor),
            hyprcursor: dedupe(hyprcursor),
            scalable: dedupe(scalable),
        }
    }

    /// Builds a loader from explicit directory lists, for tests and tools.
    #[must_use]
    pub fn with_roots(xcursor: Vec<PathBuf>, hyprcursor: Vec<PathBuf>, scalable: Vec<PathBuf>) -> Self {
        Self {
            xcursor,
            hyprcursor,
            scalable,
        }
    }

    /// The directories searched for one kind of theme, in the order configured.
    fn roots(&self, root: SearchPath) -> &[PathBuf] {
        match root {
            SearchPath::Xcursor => &self.xcursor,
            SearchPath::Hyprcursor => &self.hyprcursor,
            SearchPath::Scalable => &self.scalable,
        }
    }
}

/// Removes repeated search directories, keeping the first occurrence so the
/// configured order still decides which copy of a theme wins.
fn dedupe(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = Vec::with_capacity(paths.len());
    for path in paths {
        if !seen.contains(&path) {
            seen.push(path);
        }
    }
    seen
}

impl CursorFiles for FsFiles {
    fn read(&self, root: SearchPath, theme: &str, relative: &str) -> Result<Option<Vec<u8>>, FileError> {
        if !is_contained(theme) || !is_contained(relative) {
            return Ok(None);
        }
        for directory in self.roots(root) {
            let path = directory.join(theme).join(relative);
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(FileError::Unreadable(error.to_string())),
            };
            if !metadata.is_file() {
                continue;
            }
            if metadata.len() > MAX_FILE_BYTES {
                return Err(FileError::TooLarge);
            }
            let file = fs::File::open(&path).map_err(|error| FileError::Unreadable(error.to_string()))?;
            let mut bytes = Vec::new();
            file.take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| FileError::Unreadable(error.to_string()))?;
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Err(FileError::TooLarge);
            }
            return Ok(Some(bytes));
        }
        Ok(None)
    }

    fn list(&self, root: SearchPath, theme: &str, relative: &str) -> Result<Vec<String>, FileError> {
        if !is_contained(theme) || !is_contained(relative) {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for directory in self.roots(root) {
            let Ok(entries) = fs::read_dir(directory.join(theme).join(relative)) else {
                continue;
            };
            for entry in entries {
                let entry = entry.map_err(|error| FileError::Unreadable(error.to_string()))?;
                let name = entry.file_name().into_string().ok();
                if let Some(name) = name.filter(|name| !names.contains(name)) {
                    names.push(name);
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn theme_exists(&self, theme: &str) -> bool {
        is_contained(theme)
            && [SearchPath::Xcursor, SearchPath::Hyprcursor, SearchPath::Scalable]
                .into_iter()
                .flat_map(|root| self.roots(root))
                .any(|directory| directory.join(theme).is_dir())
    }
}

/// An in-memory theme tree for tests.
///
/// Files are keyed by family and by `theme/relative/path`, so a test states a
/// theme exactly as it would sit on disk and reads it through the same trait.
#[derive(Clone, Debug, Default)]
pub struct MemoryFiles {
    files: BTreeMap<(SearchPath, String), Vec<u8>>,
}

impl MemoryFiles {
    /// An empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file at `theme/relative` in the family `root`.
    #[must_use]
    pub fn with_file(mut self, root: SearchPath, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.files.insert((root, path.to_owned()), bytes.into());
        self
    }
}

impl CursorFiles for MemoryFiles {
    fn read(&self, root: SearchPath, theme: &str, relative: &str) -> Result<Option<Vec<u8>>, FileError> {
        if !is_contained(theme) || !is_contained(relative) {
            return Ok(None);
        }
        let Some(bytes) = self.files.get(&(root, format!("{theme}/{relative}"))) else {
            return Ok(None);
        };
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(FileError::TooLarge);
        }
        Ok(Some(bytes.clone()))
    }

    fn list(&self, root: SearchPath, theme: &str, relative: &str) -> Result<Vec<String>, FileError> {
        let prefix = format!("{theme}/{relative}/");
        let mut names: Vec<String> = self
            .files
            .keys()
            .filter(|(family, path)| *family == root && path.starts_with(&prefix))
            .filter_map(|(_, path)| {
                let rest = &path[prefix.len()..];
                (!rest.contains('/')).then(|| rest.to_owned())
            })
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    fn theme_exists(&self, theme: &str) -> bool {
        let prefix = format!("{theme}/");
        self.files.keys().any(|(_, path)| path.starts_with(&prefix))
    }
}
