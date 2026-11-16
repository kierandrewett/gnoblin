//! Candidates: the shells a tier can offer, found by looking at the filesystem.
//!
//! In a `shells/` directory a shell is one of
//!
//! - a module file: `name.so`, `name.py`, `name.js`, `name.mjs`, `name.ts` or `name.mts`;
//! - a directory `name/` that contains `shell.<ext>`, for a shell of several files;
//! - a launcher file `name.toml` (see [`crate::launcher`]);
//! - a symlink to any of these. The name is the link's own name, so a link can rename what it points to.
//!
//! The name is the file stem (the directory name for the directory form). Dot files and any other file are not
//! shells and are not mentioned. A candidate whose target cannot be used (a launcher with a missing target, a
//! dangling symlink) is still a candidate with the reason attached: it takes part in the ambiguity rule, and
//! choosing it is an error that says what is wrong, not a silent skip to the next one.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::environment::{Environment, SHELL_STEM};
use crate::language::{KNOWN_EXTENSIONS, Language};
use crate::launcher;
use crate::tier::Tier;

/// The name of the packaged default shell.
pub const DEFAULT_NAME: &str = "default";

/// The extension of a launcher file.
const LAUNCHER_EXTENSION: &str = "toml";

/// How a candidate is laid out on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A module file or script: `name.so`, `name.py`, ...
    Module,
    /// A directory `name/` holding `shell.<ext>`.
    Directory,
    /// A launcher file `name.toml`.
    Launcher,
}

/// The module or script a candidate runs, with everything the loader needs to know about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The file to load. For a launcher it is the file the launcher names, not the launcher.
    pub path: PathBuf,
    /// How to load it.
    pub language: Language,
    /// The launcher file that named the target, when one did.
    pub launcher: Option<PathBuf>,
    /// The launcher's `description`.
    pub description: Option<String>,
    /// One line for each key of the launcher that is not known.
    pub warnings: Vec<String>,
}

/// One shell a tier offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The name `--shell NAME` and `[shell] default` use.
    pub name: String,
    /// The tier the candidate was found in.
    pub tier: Tier,
    /// What was found in the directory: the file, the directory, the launcher or the symlink.
    pub location: PathBuf,
    /// How it is laid out.
    pub form: Form,
    /// What it runs, or why it cannot be run.
    pub target: Result<Target, String>,
}

impl Candidate {
    /// The file the loader opens, when the candidate can be used.
    pub fn path(&self) -> Option<&Path> {
        self.target.as_ref().ok().map(|target| target.path.as_path())
    }

    /// The language, when the candidate can be used.
    pub fn language(&self) -> Option<&Language> {
        self.target.as_ref().ok().map(|target| &target.language)
    }
}

/// Why a directory cannot be searched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryProblem {
    /// There is no such directory.
    Missing,
    /// The path exists and is not a directory.
    NotADirectory,
    /// The directory cannot be read.
    Unreadable(String),
}

impl DirectoryProblem {
    /// The word the trace prints.
    pub(crate) fn describe(&self) -> String {
        match self {
            DirectoryProblem::Missing => "missing".to_owned(),
            DirectoryProblem::NotADirectory => "not a directory".to_owned(),
            DirectoryProblem::Unreadable(error) => format!("unreadable: {error}"),
        }
    }
}

/// The shells of one `shells/` directory, sorted by file name so the answer does not depend on the filesystem's
/// listing order.
pub(crate) fn scan_shells_directory(
    directory: &Path,
    tier: Tier,
    environment: &Environment,
) -> Result<Vec<Candidate>, DirectoryProblem> {
    let reader = fs::read_dir(directory).map_err(|error| match error.kind() {
        ErrorKind::NotFound => DirectoryProblem::Missing,
        ErrorKind::NotADirectory => DirectoryProblem::NotADirectory,
        _ => DirectoryProblem::Unreadable(error.to_string()),
    })?;
    let mut names: Vec<_> = reader
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| !name.to_string_lossy().starts_with('.'))
        .collect();
    names.sort();
    let mut candidates = Vec::new();
    for name in names {
        let location = directory.join(&name);
        candidates.extend(entry_candidates(&location, tier, environment));
    }
    Ok(candidates)
}

/// The candidates of one directory entry: none when it is not a shell, one for a file, one for each `shell.<ext>`
/// of a directory.
fn entry_candidates(location: &Path, tier: Tier, environment: &Environment) -> Vec<Candidate> {
    let Some(file_name) = location.file_name().map(|name| name.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    match fs::metadata(location) {
        Ok(metadata) if metadata.is_dir() => shell_files_in(location)
            .into_iter()
            .map(|file| directory_candidate(&file, location, &file_name, tier))
            .collect(),
        Ok(metadata) if metadata.is_file() => file_candidate(location, tier, environment).into_iter().collect(),
        Ok(_) => Vec::new(),
        // A symlink to nothing. It is a shell when its name says it was meant to be one.
        Err(_) => dangling_candidate(location, tier).into_iter().collect(),
    }
}

/// The candidate for a regular file (after symlinks), or `None` when its extension is not one a shell has.
fn file_candidate(location: &Path, tier: Tier, environment: &Environment) -> Option<Candidate> {
    let extension = location.extension()?.to_str()?;
    let name = location.file_stem()?.to_string_lossy().into_owned();
    if extension == LAUNCHER_EXTENSION {
        let target = launcher::resolve(location, environment);
        return Some(Candidate {
            name,
            tier,
            location: location.to_path_buf(),
            form: Form::Launcher,
            target,
        });
    }
    Language::of_known_extension(extension)?;
    Some(Candidate {
        name,
        tier,
        location: location.to_path_buf(),
        form: Form::Module,
        target: Ok(module_target(location)),
    })
}

/// A broken symlink whose name ends in a shell extension or `.toml`.
fn dangling_candidate(location: &Path, tier: Tier) -> Option<Candidate> {
    fs::symlink_metadata(location).ok()?.is_symlink().then_some(())?;
    let extension = location.extension()?.to_str()?;
    if extension != LAUNCHER_EXTENSION && Language::of_known_extension(extension).is_none() {
        return None;
    }
    let destination = fs::read_link(location)
        .map(|link| link.display().to_string())
        .unwrap_or_default();
    Some(Candidate {
        name: location.file_stem()?.to_string_lossy().into_owned(),
        tier,
        location: location.to_path_buf(),
        form: if extension == LAUNCHER_EXTENSION {
            Form::Launcher
        } else {
            Form::Module
        },
        target: Err(format!("the symlink points to {destination}, which does not exist")),
    })
}

/// The candidate for a `shell.<ext>` inside the directory `name/`.
fn directory_candidate(file: &Path, directory: &Path, name: &str, tier: Tier) -> Candidate {
    Candidate {
        name: name.to_owned(),
        tier,
        location: directory.to_path_buf(),
        form: Form::Directory,
        target: Ok(module_target(file)),
    }
}

/// A plain module or script.
fn module_target(file: &Path) -> Target {
    Target {
        path: file.to_path_buf(),
        language: Language::detect(file),
        launcher: None,
        description: None,
        warnings: Vec::new(),
    }
}

/// The `shell.<ext>` files of a directory that are there, in the order of [`KNOWN_EXTENSIONS`].
pub(crate) fn shell_files_in(directory: &Path) -> Vec<PathBuf> {
    KNOWN_EXTENSIONS
        .iter()
        .map(|extension| directory.join(format!("{SHELL_STEM}.{extension}")))
        .filter(|file| file.is_file())
        .collect()
}

/// What a path given by a person names.
pub(crate) enum PathOutcome {
    /// A file, or the `shell.<ext>` files of a directory.
    Found(Vec<Candidate>),
    /// There is nothing at the path.
    Missing,
    /// There is something and it is not a shell: the sentence says what.
    Unusable(String),
}

/// Resolve a path from the command line, `GNOBLIN_SHELL` or `[shell] path`: a module or script, a launcher file, or a
/// directory holding `shell.<ext>`. Its name is the file stem or directory name.
pub(crate) fn candidates_at(path: &Path, tier: Tier, environment: &Environment) -> PathOutcome {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return PathOutcome::Missing;
        }
        Err(error) => return PathOutcome::Unusable(format!("cannot be read: {error}")),
    };
    let Some(name) = path
        .file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return PathOutcome::Unusable("has no file name".to_owned());
    };
    if metadata.is_dir() {
        let directory_name = path
            .file_name()
            .map_or_else(|| name.clone(), |name| name.to_string_lossy().into_owned());
        let candidates: Vec<Candidate> = shell_files_in(path)
            .iter()
            .map(|file| directory_candidate(file, path, &directory_name, tier))
            .collect();
        return if candidates.is_empty() {
            PathOutcome::Unusable(format!(
                "is a directory with no {SHELL_STEM}.{{{}}} in it",
                KNOWN_EXTENSIONS.join(",")
            ))
        } else {
            PathOutcome::Found(candidates)
        };
    }
    if !metadata.is_file() {
        return PathOutcome::Unusable("is not a regular file".to_owned());
    }
    let is_launcher = path
        .extension()
        .is_some_and(|extension| extension == LAUNCHER_EXTENSION);
    let (form, target) = if is_launcher {
        (Form::Launcher, launcher::resolve(path, environment))
    } else {
        (Form::Module, Ok(module_target(path)))
    };
    PathOutcome::Found(vec![Candidate {
        name,
        tier,
        location: path.to_path_buf(),
        form,
        target,
    }])
}

/// The packaged default: every `shell.<ext>` of the directory, as candidates named `default`.
pub(crate) fn packaged_candidates(directory: &Path) -> Result<Vec<Candidate>, DirectoryProblem> {
    match fs::metadata(directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(DirectoryProblem::NotADirectory),
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(DirectoryProblem::Missing),
        Err(error) => return Err(DirectoryProblem::Unreadable(error.to_string())),
    }
    Ok(shell_files_in(directory)
        .iter()
        .map(|file| Candidate {
            name: DEFAULT_NAME.to_owned(),
            tier: Tier::Packaged,
            location: file.clone(),
            form: Form::Module,
            target: Ok(module_target(file)),
        })
        .collect())
}

/// The conventional user shells: every `shell.<ext>` of the config directory that is there.
pub(crate) fn conventional_candidates(directory: &Path) -> Vec<Candidate> {
    shell_files_in(directory)
        .iter()
        .map(|file| Candidate {
            name: SHELL_STEM.to_owned(),
            tier: Tier::UserConfig,
            location: file.clone(),
            form: Form::Module,
            target: Ok(module_target(file)),
        })
        .collect()
}
