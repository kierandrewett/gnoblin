//! The lookup: seven tiers, first one that yields exactly one shell wins.
//!
//! | Tier | Source |
//! | ---- | ------ |
//! | 1 | the command line: a `PATH`, or `--shell NAME` |
//! | 2 | `GNOBLIN_SHELL`: a path when it contains `/`, else a name |
//! | 3 | `$XDG_CONFIG_HOME/gnoblin/config.toml`: `[shell] default = "name"` or `[shell] path = "..."` |
//! | 4 | `$XDG_CONFIG_HOME/gnoblin/shell.{so,py,js,mjs,ts,mts}` |
//! | 5 | `$XDG_DATA_HOME/gnoblin/shells/` |
//! | 6 | each `$XDG_DATA_DIRS` entry's `gnoblin/shells/`, in order |
//! | 7 | the packaged default, `<prefix>/share/gnoblin/default-shell/shell.<ext>` |
//!
//! Tiers 1 to 3 are explicit requests: a request that cannot be satisfied is an error, and the lookup never falls
//! through to a lower tier, because a person who named a shell and got a different one has been misled. A tier with
//! nothing in it is skipped. A tier with several shells is an ambiguity error, except that inside a tier 5, 6 or 7
//! directory a shell named `default` wins over the others. The first `XDG_DATA_DIRS` directory with any shell
//! decides, so an earlier directory shadows a later one, which is XDG precedence.
//!
//! A name given to `--shell`, `GNOBLIN_SHELL` or `[shell] default` is looked up in tiers 5, 6 and 7 only, in that
//! order. The packaged default is the shell named `default`.

use std::path::{Path, PathBuf};

use crate::candidate::{
    Candidate, DEFAULT_NAME, DirectoryProblem, PathOutcome, candidates_at, conventional_candidates,
    packaged_candidates, scan_shells_directory,
};
use crate::environment::Environment;
use crate::error::{DiscoveryError, Installed, Remedies, Searched, nearest};
use crate::language::Language;
use crate::shell_config::{ConfigError, ShellConfig, read_shell_config};
use crate::tier::{Request, Source, Tier};
use crate::trace::Trace;

/// Something worth telling the user that did not stop the lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// The file the warning is about.
    pub source: PathBuf,
    /// What it says, one line.
    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.source.display(), self.message)
    }
}

/// The shell that was selected, and everything the launcher needs to start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The shell's name: the file stem, the directory name or the launcher's name.
    pub name: String,
    /// The file the loader opens. For a launcher it is the file the launcher names.
    pub path: PathBuf,
    /// How to load it.
    pub language: Language,
    /// The launcher file that named [`path`](Self::path), when one did.
    pub launcher: Option<PathBuf>,
    /// The launcher's `description`.
    pub description: Option<String>,
    /// The tier that decided. For `[shell] default = "name"` it is [`Tier::Config`], and
    /// [`found_in`](Self::found_in) says where the name was found.
    pub chosen_by: Tier,
    /// The tier the shell itself was found in.
    pub found_in: Tier,
    /// What was found in the tier: the file, the directory, the launcher or the link.
    pub location: PathBuf,
    /// `[shell] isolate` from `config.toml`, which is read whichever tier chose the shell.
    pub isolate: Option<bool>,
    /// Things that did not stop the lookup: an unknown key, a `config.toml` that was ignored.
    pub warnings: Vec<Warning>,
}

impl std::fmt::Display for Selection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} ({}) {}, chosen by tier {} ({})",
            self.name,
            self.language,
            self.path.display(),
            self.chosen_by.number(),
            self.chosen_by.as_str()
        )
    }
}

/// What `config.toml` came to.
enum ConfigState {
    /// There is no usable home, so there is no config directory.
    Unavailable,
    /// The file is not there.
    Missing,
    /// The file was read.
    Loaded(ShellConfig),
    /// The file is there and cannot be used.
    Broken(ConfigError),
}

/// A directory the lookup can list shells from.
pub(crate) struct Place {
    pub(crate) tier: Tier,
    pub(crate) directory: PathBuf,
}

/// The shells of a place, or why it cannot be listed.
pub(crate) fn scan_place(place: &Place, environment: &Environment) -> Result<Vec<Candidate>, DirectoryProblem> {
    match place.tier {
        Tier::Packaged => packaged_candidates(&place.directory),
        _ => scan_shells_directory(&place.directory, place.tier, environment),
    }
}

/// The directories names are looked up in, in order: tier 5, tier 6 and tier 7.
pub(crate) fn nameable_places(environment: &Environment) -> Vec<Place> {
    let user = environment.user_shells_dir().map(|directory| Place {
        tier: Tier::UserShells,
        directory,
    });
    let system = environment.system_shells_dirs().into_iter().map(|directory| Place {
        tier: Tier::SystemShells,
        directory,
    });
    let packaged = environment.packaged_default_dir.clone().map(|directory| Place {
        tier: Tier::Packaged,
        directory,
    });
    user.into_iter().chain(system).chain(packaged).collect()
}

/// What a set of candidates in one directory comes to.
pub(crate) enum Pick {
    One(Candidate),
    Many(Vec<Candidate>),
}

/// The shell a directory's candidates name when nobody asked for one by name: the only one; else the one named
/// `default`; else none of them, and the caller reports an ambiguity. `None` for an empty list.
///
/// Two forms of one name (`foo.so` and `foo.py`) are two candidates and so an ambiguity, as are two forms of
/// `default`.
pub(crate) fn pick_default(candidates: Vec<Candidate>) -> Option<Pick> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next().map(Pick::One);
    }
    let defaults: Vec<Candidate> = candidates
        .iter()
        .filter(|candidate| candidate.name == DEFAULT_NAME)
        .cloned()
        .collect();
    match defaults.len() {
        0 => Some(Pick::Many(candidates)),
        1 => defaults.into_iter().next().map(Pick::One),
        _ => Some(Pick::Many(defaults)),
    }
}

/// Everything one lookup shares: the environment and what `config.toml` said.
pub(crate) struct Context<'a> {
    pub(crate) environment: &'a Environment,
    config_path: Option<PathBuf>,
    config: ConfigState,
}

impl<'a> Context<'a> {
    pub(crate) fn new(environment: &'a Environment) -> Context<'a> {
        let config_path = environment.config_toml();
        let config = match &config_path {
            None => ConfigState::Unavailable,
            Some(path) => match read_shell_config(path) {
                Ok(None) => ConfigState::Missing,
                Ok(Some(config)) => ConfigState::Loaded(config),
                Err(error) => ConfigState::Broken(error),
            },
        };
        Context {
            environment,
            config_path,
            config,
        }
    }

    fn remedies(&self) -> Remedies {
        Remedies::new(self.config_path.as_deref())
    }

    /// The `[shell]` table, when `config.toml` was read.
    pub(crate) fn shell_config(&self) -> Option<&ShellConfig> {
        match &self.config {
            ConfigState::Loaded(config) => Some(config),
            _ => None,
        }
    }

    /// The path of `config.toml`, when there is a config directory.
    pub(crate) fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    fn isolate(&self) -> Option<bool> {
        match &self.config {
            ConfigState::Loaded(config) => config.isolate,
            _ => None,
        }
    }

    /// The warnings `config.toml` contributes. A broken file only warns when a higher tier chose the shell: then
    /// `isolate` could not be read, and nothing else depended on the file.
    fn config_warnings(&self, chosen_by: Tier) -> Vec<Warning> {
        match (&self.config, &self.config_path) {
            (ConfigState::Loaded(config), Some(path)) => config
                .warnings
                .iter()
                .map(|message| Warning {
                    source: path.clone(),
                    message: message.clone(),
                })
                .collect(),
            (ConfigState::Broken(error), _) if chosen_by < Tier::Config => vec![Warning {
                source: error.path.clone(),
                message: format!("ignored: {}", error.message),
            }],
            _ => Vec::new(),
        }
    }

    /// The selection for a candidate that can be used, or the error that says why it cannot.
    fn select(
        &self,
        candidate: &Candidate,
        chosen_by: Tier,
        source: Option<Source>,
    ) -> Result<Selection, DiscoveryError> {
        let target = match &candidate.target {
            Ok(target) => target,
            Err(problem) => {
                return Err(DiscoveryError::Unusable {
                    source,
                    name: candidate.name.clone(),
                    location: candidate.location.clone(),
                    problem: problem.clone(),
                });
            }
        };
        let language_override = match (&self.config, chosen_by) {
            (ConfigState::Loaded(config), Tier::Config) => config.language.clone(),
            _ => None,
        };
        let mut warnings = self.config_warnings(chosen_by);
        let warning_source = target.launcher.clone().unwrap_or_else(|| candidate.location.clone());
        warnings.extend(target.warnings.iter().map(|message| Warning {
            source: warning_source.clone(),
            message: message.clone(),
        }));
        Ok(Selection {
            name: candidate.name.clone(),
            path: target.path.clone(),
            language: language_override.unwrap_or_else(|| target.language.clone()),
            launcher: target.launcher.clone(),
            description: target.description.clone(),
            chosen_by,
            found_in: candidate.tier,
            location: candidate.location.clone(),
            isolate: self.isolate(),
            warnings,
        })
    }

    fn ambiguous(&self, candidates: Vec<Candidate>, trace: Option<Trace>) -> DiscoveryError {
        DiscoveryError::Ambiguous {
            candidates,
            trace,
            remedies: self.remedies(),
        }
    }

    /// The absolute path an explicit request names: relative to the working directory for the command line and
    /// `GNOBLIN_SHELL`, relative to the directory of `config.toml` for `[shell] path`, which also expands `~`.
    /// `None` when `~` has no home to expand to.
    pub(crate) fn resolve_path(&self, given: &Path, source: &Source) -> Option<PathBuf> {
        match source {
            Source::Config(config) => {
                let expanded = self.environment.expand_home(&given.to_string_lossy())?;
                let directory = config.parent().unwrap_or(Path::new("/"));
                Some(self.environment.absolute(&directory.join(expanded)))
            }
            _ => Some(self.environment.absolute(given)),
        }
    }

    /// An explicit path: from the command line, from `GNOBLIN_SHELL`, or from `[shell] path`.
    fn select_path(&self, given: &Path, source: Source) -> Result<Selection, DiscoveryError> {
        let Some(resolved) = self.resolve_path(given, &source) else {
            return Err(DiscoveryError::PathUnusable {
                source,
                path: given.to_path_buf(),
                reason: "starts with `~` and there is no home directory".to_owned(),
            });
        };
        let shown = match source {
            Source::Config(_) => resolved.clone(),
            _ => given.to_path_buf(),
        };
        match candidates_at(&resolved, source.tier(), self.environment) {
            PathOutcome::Missing => Err(DiscoveryError::PathMissing {
                bare_word: source == Source::CommandLine && !given.is_absolute() && given.components().count() == 1,
                source,
                path: shown,
            }),
            PathOutcome::Unusable(reason) => Err(DiscoveryError::PathUnusable {
                source,
                path: shown,
                reason,
            }),
            PathOutcome::Found(mut candidates) if candidates.len() == 1 => {
                let candidate = candidates.remove(0);
                let tier = source.tier();
                self.select(&candidate, tier, Some(source))
            }
            PathOutcome::Found(candidates) => Err(self.ambiguous(candidates, None)),
        }
    }

    /// A name: looked up in tiers 5, 6 and 7 in order; the first directory that has it decides.
    fn select_name(&self, name: &str, source: Source) -> Result<Selection, DiscoveryError> {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(DiscoveryError::InvalidName {
                source,
                name: name.to_owned(),
            });
        }
        let mut installed: Vec<Installed> = Vec::new();
        let mut searched: Vec<Searched> = Vec::new();
        for place in nameable_places(self.environment) {
            let candidates = match scan_place(&place, self.environment) {
                Ok(candidates) => candidates,
                Err(problem) => {
                    searched.push(Searched {
                        directory: place.directory,
                        exists: !matches!(problem, DirectoryProblem::Missing | DirectoryProblem::NotADirectory),
                    });
                    continue;
                }
            };
            searched.push(Searched {
                directory: place.directory.clone(),
                exists: true,
            });
            for candidate in &candidates {
                if !installed.iter().any(|shell| shell.name == candidate.name) {
                    installed.push(Installed {
                        name: candidate.name.clone(),
                        location: candidate.location.clone(),
                    });
                }
            }
            let mut matching: Vec<Candidate> = candidates
                .into_iter()
                .filter(|candidate| candidate.name == name)
                .collect();
            match matching.len() {
                0 => {}
                1 => {
                    let candidate = matching.remove(0);
                    let tier = source.tier();
                    return self.select(&candidate, tier, Some(source));
                }
                _ => return Err(self.ambiguous(matching, None)),
            }
        }
        installed.sort_by(|a, b| a.name.cmp(&b.name));
        Err(DiscoveryError::NameNotFound {
            nearest: nearest(name, installed.iter().map(|shell| shell.name.as_str())),
            source,
            name: name.to_owned(),
            installed,
            searched,
        })
    }

    /// One directory of tier 5, 6 or 7 when nobody asked for a name: `Ok(None)` when it has nothing.
    fn select_in_place(
        &self,
        place: &Place,
        index: usize,
        trace: &mut Trace,
    ) -> Result<Option<Selection>, DiscoveryError> {
        let absent = |problem: &DirectoryProblem| match (place.tier, problem) {
            (Tier::Packaged, DirectoryProblem::Missing) => "not installed".to_owned(),
            _ => problem.describe(),
        };
        let candidates = match scan_place(place, self.environment) {
            Ok(candidates) => candidates,
            Err(problem) => {
                trace.set_place(place.tier, index, absent(&problem));
                return Ok(None);
            }
        };
        match pick_default(candidates) {
            None => {
                trace.set_place(
                    place.tier,
                    index,
                    if place.tier == Tier::Packaged {
                        "not installed"
                    } else {
                        "empty"
                    },
                );
                Ok(None)
            }
            Some(Pick::One(candidate)) => self.select(&candidate, place.tier, None).map(Some),
            Some(Pick::Many(candidates)) => {
                trace.set_place(place.tier, index, format!("{} shells", candidates.len()));
                Err(self.ambiguous(candidates, Some(trace.clone())))
            }
        }
    }
}

/// Select the shell to run.
///
/// # Errors
///
/// [`DiscoveryError`] when nothing is found, when several shells are equally preferred, when an explicit request
/// cannot be satisfied (an explicit request never falls through to a lower tier), or when `config.toml` is needed
/// and cannot be used. Its `Display` is the text to print after `gnoblin: `.
pub fn discover(environment: &Environment, request: &Request) -> Result<Selection, DiscoveryError> {
    let context = Context::new(environment);
    let mut trace = Trace::new(environment);

    // Tier 1.
    match request {
        Request::Path(path) => return context.select_path(path, Source::CommandLine),
        Request::Name(name) => return context.select_name(name, Source::CommandLine),
        Request::Discover => trace.set(Tier::CommandLine, "not given"),
    }

    // Tier 2.
    if let Some(value) = environment.gnoblin_shell.as_ref().filter(|value| !value.is_empty()) {
        return if value.as_encoded_bytes().contains(&b'/') {
            context.select_path(Path::new(value), Source::Environment)
        } else {
            context.select_name(&value.to_string_lossy(), Source::Environment)
        };
    }
    trace.set(Tier::Environment, "not set");

    // Tier 3.
    match (&context.config, &context.config_path) {
        (ConfigState::Broken(error), _) => return Err(DiscoveryError::Config(error.clone())),
        (ConfigState::Loaded(config), Some(path)) => {
            let source = Source::Config(path.clone());
            if let Some(name) = &config.default {
                return context.select_name(name, source);
            }
            if let Some(file) = &config.path {
                return context.select_path(Path::new(file), source);
            }
            let status = if config.section {
                "no default or path in [shell]"
            } else {
                "no [shell] section"
            };
            trace.set(Tier::Config, status);
        }
        (ConfigState::Missing, _) => trace.set(Tier::Config, "missing"),
        _ => trace.set(Tier::Config, "no home directory"),
    }

    // Tier 4.
    match environment.config_dir() {
        Some(directory) => match pick_default(conventional_candidates(&directory)) {
            None => trace.set(Tier::UserConfig, "none present"),
            Some(Pick::One(candidate)) => return context.select(&candidate, Tier::UserConfig, None),
            Some(Pick::Many(candidates)) => {
                trace.set(Tier::UserConfig, format!("{} shells", candidates.len()));
                return Err(context.ambiguous(candidates, Some(trace)));
            }
        },
        None => trace.set(Tier::UserConfig, "no home directory"),
    }

    // Tiers 5, 6 and 7, each place in order.
    let places = nameable_places(environment);
    if environment.user_shells_dir().is_none() {
        trace.set(Tier::UserShells, "no home directory");
    }
    if environment.system_shells_dirs().is_empty() {
        trace.set(Tier::SystemShells, "no absolute entry in XDG_DATA_DIRS");
    }
    if environment.packaged_default_dir.is_none() {
        trace.set(Tier::Packaged, "location unknown");
    }
    let mut counts = [0_usize; 8];
    for place in &places {
        let index = counts[usize::from(place.tier.number())];
        counts[usize::from(place.tier.number())] += 1;
        if let Some(selection) = context.select_in_place(place, index, &mut trace)? {
            return Ok(selection);
        }
    }
    Err(DiscoveryError::NoShell {
        trace,
        remedies: context.remedies(),
    })
}
