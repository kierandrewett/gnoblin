//! The seven tiers of the lookup and the way a request reached it.

use std::path::PathBuf;

/// One step of the lookup, in precedence order. The first tier that yields exactly one shell wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Tier 1: the command line, a `PATH` or `--shell NAME`.
    CommandLine = 1,
    /// Tier 2: `GNOBLIN_SHELL`.
    Environment = 2,
    /// Tier 3: `[shell] default` or `[shell] path` in `$XDG_CONFIG_HOME/gnoblin/config.toml`.
    Config = 3,
    /// Tier 4: `$XDG_CONFIG_HOME/gnoblin/shell.<ext>`.
    UserConfig = 4,
    /// Tier 5: `$XDG_DATA_HOME/gnoblin/shells/`.
    UserShells = 5,
    /// Tier 6: `<each XDG_DATA_DIRS entry>/gnoblin/shells/`.
    SystemShells = 6,
    /// Tier 7: the packaged default, `<prefix>/share/gnoblin/default-shell/shell.<ext>`.
    Packaged = 7,
}

impl Tier {
    /// Every tier in precedence order.
    pub const ALL: [Tier; 7] = [
        Tier::CommandLine,
        Tier::Environment,
        Tier::Config,
        Tier::UserConfig,
        Tier::UserShells,
        Tier::SystemShells,
        Tier::Packaged,
    ];

    /// The tier's number, 1 to 7, as the error text and `--list-shells` print it.
    pub fn number(self) -> u8 {
        self as u8
    }

    /// The name the object model and logs use for the tier.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::CommandLine => "command-line",
            Tier::Environment => "environment",
            Tier::Config => "config",
            Tier::UserConfig => "user-config",
            Tier::UserShells => "user-shells",
            Tier::SystemShells => "system-shells",
            Tier::Packaged => "packaged",
        }
    }

    /// Whether a shell can be picked by name from this tier's directory. Tier 4 files are all called `shell`, and a
    /// name found nowhere else is not a shell.
    pub(crate) fn holds_named_shells(self) -> bool {
        matches!(self, Tier::UserShells | Tier::SystemShells | Tier::Packaged)
    }
}

/// Who asked for a shell by path or by name, for the error text that says where the request came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The command line.
    CommandLine,
    /// `GNOBLIN_SHELL`.
    Environment,
    /// The `[shell]` section of this `config.toml`.
    Config(PathBuf),
}

impl Source {
    /// The tier a request from this source belongs to.
    pub fn tier(&self) -> Tier {
        match self {
            Source::CommandLine => Tier::CommandLine,
            Source::Environment => Tier::Environment,
            Source::Config(_) => Tier::Config,
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::CommandLine => formatter.write_str("the command line"),
            Source::Environment => formatter.write_str("GNOBLIN_SHELL"),
            Source::Config(path) => write!(formatter, "[shell] in {}", path.display()),
        }
    }
}

/// What the command line asks for. The command line rejects a path and a name together before it gets here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Request {
    /// Nothing: run the lookup from tier 2.
    #[default]
    Discover,
    /// `gnoblin PATH`, used as given (relative to the working directory).
    Path(PathBuf),
    /// `gnoblin --shell NAME`, looked up as an installed shell.
    Name(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_are_numbered_in_precedence_order() {
        let numbers: Vec<u8> = Tier::ALL.iter().map(|tier| tier.number()).collect();
        assert_eq!(numbers, vec![1, 2, 3, 4, 5, 6, 7]);
        assert!(Tier::CommandLine < Tier::Packaged);
    }

    #[test]
    fn only_the_data_tiers_are_nameable() {
        let nameable: Vec<Tier> = Tier::ALL.into_iter().filter(|tier| tier.holds_named_shells()).collect();
        assert_eq!(nameable, vec![Tier::UserShells, Tier::SystemShells, Tier::Packaged]);
    }

    #[test]
    fn a_source_belongs_to_its_tier() {
        assert_eq!(Source::CommandLine.tier(), Tier::CommandLine);
        assert_eq!(Source::Environment.tier(), Tier::Environment);
        assert_eq!(Source::Config(PathBuf::from("/c/config.toml")).tier(), Tier::Config);
        assert_eq!(
            Source::Config(PathBuf::from("/c/config.toml")).to_string(),
            "[shell] in /c/config.toml"
        );
    }
}
