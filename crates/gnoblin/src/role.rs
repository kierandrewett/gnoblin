//! The roles this executable has.
//!
//! Design/rust-compositor.md, "Process model", gives the executable four roles and
//! keeps them in one binary. The command line is therefore one flag and nothing else:
//! a role is a word a supervisor passes, not a configuration surface, and every
//! other knob belongs to the configuration document a role reads.
//!
//! A role name this build does not have is a command line error, the same as any
//! other unknown word: the supervisor's exit 2 names the roles that exist.

use clap::{Arg, ArgMatches, Command};

/// One role of `design/rust-compositor.md`'s process model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The default: session start and end, supervision, environment and recovery.
    Guardian,
    /// Wayland, rendering, input, outputs, the policy core, the object store and the
    /// IPC socket.
    Compositor,
    /// The Lua configuration and the event handlers.
    Runtime,
    /// The D-Bus agents: polkit, GeoClue, prompts, privacy, screen saver, idle.
    Services,
}

impl Role {
    /// Every role, in the order the plan lists them.
    pub const ALL: [Role; 4] = [Role::Guardian, Role::Compositor, Role::Runtime, Role::Services];

    /// The word the command line uses.
    pub fn name(self) -> &'static str {
        match self {
            Role::Guardian => "guardian",
            Role::Compositor => "compositor",
            Role::Runtime => "runtime",
            Role::Services => "services",
        }
    }

    /// The role a command line word names, if it names one.
    pub fn from_name(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.name() == name)
    }
}

/// The command line.
///
/// `--role` is documented and defaulted; `--version` prints the version the IPC
/// serves. Nothing else is a flag: a role reads the configuration document, so an
/// option here would be a second, undocumented place for the same setting.
pub fn command() -> Command {
    Command::new("gnoblin")
        .about("Gnoblin session entry point: one executable, one role per process.")
        .version(version_line())
        .arg(
            Arg::new("role")
                .long("role")
                .value_name("ROLE")
                .default_value("guardian")
                .value_parser(Role::ALL.map(Role::name))
                .help(
                    "The role this process plays. The default, `guardian`, supervises the session; \
                     `compositor` runs it in the foreground; `runtime` serves the scripts and `services` \
                     the D-Bus agents.",
                ),
        )
}

/// The version `--version` prints.
///
/// The triple is the IPC version, which the plan makes exactly the Gnoblin version;
/// the build's own package version follows it, so a candidate (`0.2.0-alpha.N`) and
/// the release it becomes are told apart in a bug report without either hiding the
/// numbers a client compares.
///
/// It is built once and kept: clap wants a value that outlives the command, and a
/// leaked string per `command()` call would be a leak in a process that builds the
/// command line once.
fn version_line() -> &'static str {
    static VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "{} (build {})",
            gnoblin_compositor::version(),
            env!("CARGO_PKG_VERSION")
        )
    });
    VERSION.as_str()
}

/// The role the command line asked for.
///
/// # Errors
///
/// The word clap accepted that no [`Role`] answers to. The value parser is built from
/// [`Role::ALL`], so this cannot happen for a command line clap accepted; it is
/// reported rather than unwrapped because a role this build does not have must be a
/// command line error, not a panic.
pub fn from_matches(matches: &ArgMatches) -> Result<Role, String> {
    let name = matches
        .get_one::<String>("role")
        .map(String::as_str)
        .unwrap_or("guardian");
    Role::from_name(name).ok_or_else(|| format!("`{name}` is not a role"))
}

#[cfg(test)]
mod tests {
    use super::{Role, command, from_matches, version_line};

    #[test]
    fn every_role_round_trips_through_its_name() {
        for role in Role::ALL {
            assert_eq!(Role::from_name(role.name()), Some(role));
        }
        assert_eq!(Role::from_name("compositorr"), None);
    }

    #[test]
    fn the_command_line_defaults_to_the_guardian() {
        let matches = command().try_get_matches_from(["gnoblin"]).expect("no flag is fine");
        assert_eq!(from_matches(&matches), Ok(Role::Guardian));
    }

    #[test]
    fn the_command_line_reaches_every_role() {
        for role in Role::ALL {
            let matches = command()
                .try_get_matches_from(["gnoblin", "--role", role.name()])
                .expect("the role is one this build has");
            assert_eq!(from_matches(&matches), Ok(role));
        }
    }

    #[test]
    fn an_unknown_role_names_the_ones_that_exist() {
        let error = command()
            .try_get_matches_from(["gnoblin", "--role", "daemon"])
            .expect_err("no such role");
        let message = error.to_string();
        for role in Role::ALL {
            assert!(message.contains(role.name()), "{message}");
        }
    }

    #[test]
    fn the_version_is_the_triple_and_the_build() {
        let line = version_line();
        let version = gnoblin_compositor::version();
        assert!(line.starts_with(&version.to_string()), "{line}");
        assert!(line.contains(env!("CARGO_PKG_VERSION")), "{line}");
        assert!(line.matches('.').count() >= 2, "a full triple: {line}");
    }

    #[test]
    fn the_guardian_supervises_the_standard_plan() {
        // Built from the real plan, so this is the list a session starts today rather than a
        // description of it. The guardian role itself is `gnoblin-session`'s supervisor.
        let plan = gnoblin_session::Plan::standard(false, false, Vec::new()).expect("the standard plan");
        let children: Vec<&str> = plan.children().iter().map(|child| child.name.as_str()).collect();
        for child in ["compositor", "services", "runtime"] {
            assert!(
                children.contains(&child),
                "`{child}` is in the standard plan: {children:?}"
            );
        }
    }
}
