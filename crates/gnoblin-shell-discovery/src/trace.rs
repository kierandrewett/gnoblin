//! The record of where the lookup looked, which the error text prints as "Looked in this order".

use std::path::Path;

use crate::environment::{Environment, SHELL_STEM};
use crate::language::KNOWN_EXTENSIONS;
use crate::tier::Tier;

/// The label of tiers 1 and 2 is padded to this many columns so their statuses line up.
const LABEL_WIDTH: usize = 21;

/// What one tier said: the places it looked and what it found there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceLine {
    /// The tier.
    pub tier: Tier,
    /// Each place the tier looked, with what was found. Every tier has one place except tier 6, which has one for
    /// each `XDG_DATA_DIRS` entry.
    pub places: Vec<(String, String)>,
}

/// Where the lookup looked, one line for each tier in precedence order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    /// The seven tiers' lines.
    pub lines: Vec<TraceLine>,
}

impl Trace {
    /// A trace for `environment` in which no tier has been reached yet.
    pub(crate) fn new(environment: &Environment) -> Trace {
        let config_dir = environment.config_dir();
        let config_dir_text = directory_text(config_dir.as_deref(), "$XDG_CONFIG_HOME/gnoblin");
        let user_shells = environment.user_shells_dir();
        let system = environment.system_shells_dirs();
        let places = |tier: Tier, subjects: Vec<String>| TraceLine {
            tier,
            places: subjects
                .into_iter()
                .map(|subject| (subject, "not reached".to_owned()))
                .collect(),
        };
        let extensions = KNOWN_EXTENSIONS.join(",");
        Trace {
            lines: vec![
                places(Tier::CommandLine, vec!["command line".to_owned()]),
                places(Tier::Environment, vec!["GNOBLIN_SHELL".to_owned()]),
                places(Tier::Config, vec![format!("{config_dir_text}/config.toml")]),
                places(
                    Tier::UserConfig,
                    vec![format!("{config_dir_text}/{SHELL_STEM}.{{{extensions}}}")],
                ),
                places(
                    Tier::UserShells,
                    vec![format!(
                        "{}/",
                        directory_text(user_shells.as_deref(), "$XDG_DATA_HOME/gnoblin/shells")
                    )],
                ),
                places(
                    Tier::SystemShells,
                    if system.is_empty() {
                        vec!["$XDG_DATA_DIRS/gnoblin/shells/".to_owned()]
                    } else {
                        system.iter().map(|dir| format!("{}/", dir.display())).collect()
                    },
                ),
                places(
                    Tier::Packaged,
                    vec![match &environment.packaged_default_dir {
                        Some(dir) => format!("{}/{SHELL_STEM}.*", dir.display()),
                        None => format!("<prefix>/share/gnoblin/default-shell/{SHELL_STEM}.*"),
                    }],
                ),
            ],
        }
    }

    /// Set the status of the only place of `tier`.
    pub(crate) fn set(&mut self, tier: Tier, status: impl Into<String>) {
        self.set_place(tier, 0, status);
    }

    /// Set the status of the `index`th place of `tier`.
    pub(crate) fn set_place(&mut self, tier: Tier, index: usize, status: impl Into<String>) {
        if let Some(place) = self
            .lines
            .iter_mut()
            .find(|line| line.tier == tier)
            .and_then(|line| line.places.get_mut(index))
        {
            place.1 = status.into();
        }
    }

    /// The numbered lines, as the error text prints them, without a trailing newline.
    pub fn render(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            if !text.is_empty() {
                text.push('\n');
            }
            let places = &line.places;
            let rendered = match line.tier {
                Tier::CommandLine | Tier::Environment => {
                    let (label, status) = &places[0];
                    format!("{label:<LABEL_WIDTH$}   {status}")
                }
                Tier::Config | Tier::UserConfig | Tier::Packaged => {
                    let (subject, status) = &places[0];
                    format!("{subject}   {status}")
                }
                Tier::UserShells | Tier::SystemShells => places
                    .iter()
                    .map(|(subject, status)| format!("{subject}  {status}"))
                    .collect::<Vec<_>>()
                    .join(";  "),
            };
            text.push_str(&format!("  {}. {rendered}", line.tier.number()));
        }
        text
    }
}

/// A directory as text, or the XDG expression it would have been built from when there is no usable home.
fn directory_text(directory: Option<&Path>, fallback: &str) -> String {
    directory.map_or_else(|| fallback.to_owned(), |dir| dir.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn environment() -> Environment {
        Environment {
            home: Some(PathBuf::from("/home/k")),
            packaged_default_dir: Some(PathBuf::from("/usr/share/gnoblin/default-shell")),
            ..Environment::default()
        }
    }

    #[test]
    fn a_fresh_trace_has_every_tier_unreached() {
        let trace = Trace::new(&environment());
        assert_eq!(trace.lines.len(), 7);
        assert!(
            trace
                .lines
                .iter()
                .flat_map(|line| &line.places)
                .all(|(_, status)| status == "not reached")
        );
    }

    #[test]
    fn tier_six_has_a_place_for_each_data_dir() {
        let trace = Trace::new(&environment());
        assert_eq!(trace.lines[5].places.len(), 2);
    }

    #[test]
    fn setting_an_unknown_place_changes_nothing() {
        let mut trace = Trace::new(&environment());
        let before = trace.clone();
        trace.set_place(Tier::CommandLine, 5, "x");
        assert_eq!(trace, before);
    }

    #[test]
    fn without_a_home_the_trace_names_the_xdg_expressions() {
        let trace = Trace::new(&Environment::default());
        let text = trace.render();
        assert!(text.contains("$XDG_CONFIG_HOME/gnoblin/config.toml"), "{text}");
        assert!(text.contains("$XDG_DATA_HOME/gnoblin/shells/"), "{text}");
        assert!(text.contains("<prefix>/share/gnoblin/default-shell/shell.*"), "{text}");
    }
}
