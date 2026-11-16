//! Why no shell was selected, and the text that says so.
//!
//! The text is one block meant to be printed after `gnoblin: ` with the compositor never started, so the guardian
//! sees an exit and a reason instead of a blank screen. A failure lists where the lookup looked, how to name a shell
//! explicitly, how to configure a default and which candidates were found. [`DiscoveryError`]'s `Display` is the
//! text after `gnoblin: `; the tests pin it word for word.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::candidate::Candidate;
use crate::shell_config::ConfigError;
use crate::tier::Source;
use crate::trace::Trace;

/// The three ways to say which shell, printed under every failure that is about finding one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remedies {
    /// The file a `[shell] default` goes in, as text.
    pub config_toml: String,
}

impl Remedies {
    pub(crate) fn new(config_toml: Option<&Path>) -> Remedies {
        Remedies {
            config_toml: config_toml.map_or_else(
                || "$XDG_CONFIG_HOME/gnoblin/config.toml".to_owned(),
                |path| path.display().to_string(),
            ),
        }
    }

    fn render(&self) -> String {
        format!(
            "Run a shell directly:     gnoblin ./myshell.so\n\
             Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)\n\
             Set a default:            [shell] default = \"NAME\"  in {}",
            self.config_toml
        )
    }
}

/// An installed shell, for the list a missing name prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// Its name.
    pub name: String,
    /// Where it was found.
    pub location: PathBuf,
}

/// A directory a name was looked up in, for the list a missing name prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Searched {
    /// The directory.
    pub directory: PathBuf,
    /// Whether it is there.
    pub exists: bool,
}

/// Why no shell was selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    /// Every tier was empty and there is no packaged default.
    NoShell {
        /// Where the lookup looked.
        trace: Trace,
        /// How to fix it.
        remedies: Remedies,
    },
    /// A tier, a directory or a name had several equally preferred shells, and none of them is named `default`.
    Ambiguous {
        /// The shells that are equally preferred, in the order found.
        candidates: Vec<Candidate>,
        /// Where the lookup looked. `None` for an explicit request, which looked nowhere else.
        trace: Option<Trace>,
        /// How to fix it.
        remedies: Remedies,
    },
    /// A path that was asked for is not there. Discovery does not fall through on an explicit request.
    PathMissing {
        /// Who asked.
        source: Source,
        /// The path, as it was written.
        path: PathBuf,
        /// The path is one word with no `/`: probably meant as a name.
        bare_word: bool,
    },
    /// A path that was asked for is there and is not a shell.
    PathUnusable {
        /// Who asked.
        source: Source,
        /// The path, as it was written.
        path: PathBuf,
        /// What is wrong with it, a phrase that follows the path.
        reason: String,
    },
    /// A name that was asked for cannot be a name.
    InvalidName {
        /// Who asked.
        source: Source,
        /// The name.
        name: String,
    },
    /// A name that was asked for is not installed.
    NameNotFound {
        /// Who asked.
        source: Source,
        /// The name.
        name: String,
        /// The nearest installed name, when one is close.
        nearest: Option<String>,
        /// The shells that are installed, by name, each at the place that wins.
        installed: Vec<Installed>,
        /// The directories the name was looked up in.
        searched: Vec<Searched>,
    },
    /// The shell that was selected cannot be used: a launcher whose target is gone, a dangling symlink.
    Unusable {
        /// Who asked, or `None` when the lookup chose it.
        source: Option<Source>,
        /// The shell's name.
        name: String,
        /// The launcher, link or file.
        location: PathBuf,
        /// What is wrong, one line.
        problem: String,
    },
    /// `config.toml` cannot be used.
    Config(ConfigError),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::NoShell { trace, remedies } => write!(
                formatter,
                "no shell found.\n\nLooked in this order:\n{}\n\n{}",
                trace.render(),
                remedies.render()
            ),
            DiscoveryError::Ambiguous {
                candidates,
                trace,
                remedies,
            } => {
                write!(
                    formatter,
                    "several shells are equally preferred:\n{}",
                    candidate_lines(candidates)
                )?;
                if let Some(trace) = trace {
                    write!(formatter, "\n\nLooked in this order:\n{}", trace.render())?;
                }
                write!(formatter, "\n\n{}", remedies.render())
            }
            DiscoveryError::PathMissing {
                source,
                path,
                bare_word,
            } => {
                write!(formatter, "{}: no such file (from {source}).", path.display())?;
                if *bare_word {
                    let word = path.display();
                    write!(
                        formatter,
                        "\nDid you mean --shell {word}?    (a path needs a `/`, as in ./{word})"
                    )?;
                }
                Ok(())
            }
            DiscoveryError::PathUnusable { source, path, reason } => {
                write!(formatter, "{}: {reason} (from {source}).", path.display())
            }
            DiscoveryError::InvalidName { source, name } => write!(
                formatter,
                "\"{name}\" is not a shell name (from {source}): a name is one word with no `/`; to run a file, give its path."
            ),
            DiscoveryError::NameNotFound {
                source,
                name,
                nearest,
                installed,
                searched,
            } => {
                write!(formatter, "no shell named \"{name}\" (from {source}).")?;
                if let Some(nearest) = nearest {
                    write!(formatter, "\nDid you mean \"{nearest}\"?")?;
                }
                if installed.is_empty() {
                    formatter.write_str("\n\nNo shells are installed.")?;
                } else {
                    formatter.write_str("\n\nInstalled shells:")?;
                    let width = installed
                        .iter()
                        .map(|shell| shell.name.chars().count())
                        .max()
                        .unwrap_or(0);
                    for shell in installed {
                        write!(formatter, "\n  {:<width$}  {}", shell.name, shell.location.display())?;
                    }
                }
                formatter.write_str("\n\nLooked in:")?;
                for place in searched {
                    write!(formatter, "\n  {}/", place.directory.display())?;
                    if !place.exists {
                        formatter.write_str("  (missing)")?;
                    }
                }
                Ok(())
            }
            DiscoveryError::Unusable {
                source,
                name,
                location,
                problem,
            } => {
                write!(formatter, "the shell \"{name}\" cannot be used")?;
                if let Some(source) = source {
                    write!(formatter, " (from {source})")?;
                }
                write!(formatter, ":\n  {}: {problem}", location.display())
            }
            DiscoveryError::Config(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// The candidate list of an ambiguity: name, where it is (and what it launches) and how it is loaded.
fn candidate_lines(candidates: &[Candidate]) -> String {
    let width = candidates
        .iter()
        .map(|candidate| candidate.name.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = String::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if index > 0 {
            lines.push('\n');
        }
        let _ = write!(lines, "  {:<width$}  {}", candidate.name, candidate.location.display());
        match &candidate.target {
            Ok(target) => {
                if target.path != candidate.location {
                    let _ = write!(lines, " -> {}", target.path.display());
                }
                let _ = write!(lines, "  ({})", target.language);
            }
            Err(problem) => {
                let _ = write!(lines, "  (unusable: {problem})");
            }
        }
    }
    lines
}

/// The installed name closest to `wanted`, when it is close enough to be a typo: at most two edits, and fewer
/// than the length of the shorter word. Ties go to the name that sorts first.
pub(crate) fn nearest<'a>(wanted: &str, names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for name in names {
        let distance = edit_distance(wanted, name);
        let close = distance <= 2 && distance < wanted.chars().count().min(name.chars().count());
        if !close {
            continue;
        }
        if best.is_none_or(|(best_distance, best_name)| (distance, name) < (best_distance, best_name)) {
            best = Some((distance, name));
        }
    }
    best.map(|(_, name)| name.to_owned())
}

/// The Levenshtein distance between two words.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (row, left) in a.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right) in b.iter().enumerate() {
            let substitution = previous[column] + usize::from(left != *right);
            current.push(substitution.min(previous[column + 1] + 1).min(current[column] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_counts_single_edits() {
        assert_eq!(edit_distance("tilr", "tiler"), 1);
        assert_eq!(edit_distance("tiler", "tiler"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn the_nearest_name_is_a_close_one() {
        assert_eq!(nearest("tilr", ["lines", "tiler"]), Some("tiler".to_owned()));
        assert_eq!(nearest("zzz", ["lines", "tiler"]), None);
        assert_eq!(nearest("a", ["b"]), None, "a one-letter name has no typo");
        assert_eq!(
            nearest("tiler", ["tilers", "tilert"]),
            Some("tilers".to_owned()),
            "ties sort first"
        );
    }
}
