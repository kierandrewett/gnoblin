//! The roles this executable has, and its command line.
//!
//! Design/rust-compositor.md, "Process model", gives the executable its roles in one binary, and
//! `design/modular-framework.md` section 7.1 gives the command line:
//!
//! ```text
//! gnoblin [--shell NAME] [--watch] [--isolate] [--list-shells] [PATH]
//! ```
//!
//! `PATH` is always a path (so `./myshell.so` and `myshell.py` both work); `--shell` is always a name; the two together are a
//! usage error. Everything else is the guardian's own business: `--role` and `--language` are what the guardian starts its
//! children with, and they are the only two flags that are not about choosing and running a shell. A role is a word a supervisor
//! passes, not a configuration surface, and every other knob belongs to the configuration a role reads.
//!
//! A role name this build does not have is a command line error, the same as any other unknown word: the supervisor's exit 2
//! names the roles that exist.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

/// One role of `design/rust-compositor.md`'s process model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The default: session start and end, supervision, environment and recovery. It resolves the shell.
    Guardian,
    /// Wayland, rendering, input, outputs, the policy core, the object store and the IPC socket. It runs the shell in its
    /// own process unless the shell is isolated.
    Compositor,
    /// The shell host in a process of its own: what `--isolate` starts, connected to the compositor over its socket.
    Shell,
    /// The D-Bus agents: polkit, GeoClue, prompts, privacy, screen saver, idle.
    Services,
    /// The dry run of a shell module over a tree with nothing in it, which is what `gnoblinctl check` asks for. It prints its verdict
    /// and exits; it starts no session.
    Check,
}

impl Role {
    /// Every role, in the order the plan lists them.
    pub const ALL: [Role; 5] = [
        Role::Guardian,
        Role::Compositor,
        Role::Shell,
        Role::Services,
        Role::Check,
    ];

    /// The word the command line uses.
    pub fn name(self) -> &'static str {
        match self {
            Role::Guardian => "guardian",
            Role::Compositor => "compositor",
            Role::Shell => "shell",
            Role::Services => "services",
            Role::Check => "check",
        }
    }

    /// The role a command line word names, if it names one.
    pub fn from_name(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.name() == name)
    }

    /// Whether this role loads a shell module, and so takes `--watch`, `--language` and a `PATH`.
    pub fn loads_shell(self) -> bool {
        matches!(self, Role::Guardian | Role::Compositor | Role::Shell | Role::Check)
    }
}

/// The text `--help` ends with: where the shell comes from and how to say which.
const AFTER_HELP: &str = "\
Which shell runs, first match wins (a tier that names a shell that is not there is an error, never a fall-through):
  1. the command line: PATH, or --shell NAME
  2. GNOBLIN_SHELL: a path when it contains '/', else a name
  3. [shell] default = \"NAME\" or path = \"...\" in $XDG_CONFIG_HOME/gnoblin/config.toml
  4. $XDG_CONFIG_HOME/gnoblin/shell.{so,py,js,mjs,ts,mts}
  5. $XDG_DATA_HOME/gnoblin/shells/
  6. each $XDG_DATA_DIRS entry's gnoblin/shells/, in order
  7. the packaged default shell, <prefix>/share/gnoblin/default-shell/

Several equally preferred shells in one tier are an error that lists them (a shell named `default` wins inside a shells/ \
directory). `gnoblin --list-shells` prints every shell in every tier and marks the one that would run.
With no shell found, gnoblin says where it looked and exits 1 before any compositor starts.";

/// The command line.
///
/// `--version` prints the version the IPC serves. `--role` and `--language` are the guardian's children's flags and are shown
/// under their own heading, not among the shell options.
pub fn command() -> Command {
    Command::new("gnoblin")
        .about("Gnoblin session entry point: starts the compositor and the shell you choose.")
        .version(version_line())
        .after_help(AFTER_HELP)
        .arg(
            Arg::new("shell")
                .value_name("PATH")
                .value_parser(value_parser!(PathBuf))
                .help(
                    "Run this shell: a module (`./libmyshell.so`, `shell.py`, `shell.ts`, ...), a directory holding \
                     `shell.<ext>`, or a `name.toml` launcher. Relative to the working directory.",
                ),
        )
        .arg(
            Arg::new("named")
                .long("shell")
                .value_name("NAME")
                .conflicts_with("shell")
                .help("Run the installed shell called NAME (see --list-shells). Not a path: use PATH for that."),
        )
        .arg(
            Arg::new("watch")
                .long("watch")
                .action(ArgAction::SetTrue)
                .help(
                    "Reload the shell when its files change: the module (and a launcher's target) for a native shell, every \
                     file the script imported for a script. Watches the shell discovery selects when no PATH is given.",
                ),
        )
        .arg(
            Arg::new("isolate")
                .long("isolate")
                .action(ArgAction::SetTrue)
                .help(
                    "Run the shell in a process of its own, connected to the compositor over its socket, so a crash in \
                     shell code cannot take the compositor with it. `[shell] isolate = true` in config.toml does the same.",
                ),
        )
        .arg(
            Arg::new("list")
                .long("list-shells")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["watch", "isolate"])
                .help("Print every shell discovery can see, in precedence order, and mark the one that would run."),
        )
        .next_help_heading("Roles (what the session starts its children as)")
        .arg(
            Arg::new("role")
                .long("role")
                .value_name("ROLE")
                .default_value("guardian")
                .value_parser(Role::ALL.map(Role::name))
                .help(
                    "The role this process plays. The default, `guardian`, supervises the session; `compositor` runs it \
                     in the foreground (with PATH, running that shell in its process; with none, alone); `shell` is the \
                     isolated shell host, `services` the D-Bus agents and `check` the dry run of a shell module \
                     (what `gnoblinctl check` asks for).",
                ),
        )
        .arg(
            Arg::new("language")
                .long("language")
                .value_name("LANGUAGE")
                .help(
                    "How to load the module, instead of detecting it: `native`, `python`, `js`, `ts`, or any language \
                     with a `libgnoblin-runtime-<LANGUAGE>.so`. The guardian passes it for a launcher file's `language =`.",
                ),
        )
}

/// The shell module the command line names by path, if it names one.
pub fn path_from_matches(matches: &ArgMatches) -> Option<PathBuf> {
    matches.get_one::<PathBuf>("shell").cloned()
}

/// The installed shell the command line names with `--shell`, if it does.
pub fn name_from_matches(matches: &ArgMatches) -> Option<String> {
    matches.get_one::<String>("named").cloned()
}

/// Whether a boolean flag was given.
pub fn flag(matches: &ArgMatches, name: &str) -> bool {
    matches.get_flag(name)
}

/// The `--language` the command line forces, if any.
pub fn language_from_matches(matches: &ArgMatches) -> Option<String> {
    matches.get_one::<String>("language").cloned()
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
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        format!("{major}.{minor}.{patch} (build {})", env!("CARGO_PKG_VERSION"))
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
    use super::{
        Role, command, flag, from_matches, language_from_matches, name_from_matches, path_from_matches, version_line,
    };

    #[test]
    fn every_role_round_trips_through_its_name() {
        for role in Role::ALL {
            assert_eq!(Role::from_name(role.name()), Some(role));
        }
        assert_eq!(Role::from_name("compositorr"), None);
        assert_eq!(
            Role::from_name("runtime"),
            None,
            "the script runtime role is the shell role now"
        );
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
    fn a_positional_argument_is_the_shell_path() {
        let matches = command()
            .try_get_matches_from(["gnoblin", "./libshell.so"])
            .expect("a path is accepted");
        assert_eq!(from_matches(&matches), Ok(Role::Guardian));
        assert_eq!(
            path_from_matches(&matches),
            Some(std::path::PathBuf::from("./libshell.so"))
        );
        assert_eq!(name_from_matches(&matches), None);
        let none = command().try_get_matches_from(["gnoblin"]).expect("no flag is fine");
        assert_eq!(path_from_matches(&none), None);
    }

    #[test]
    fn the_shell_flags_are_read() {
        let matches = command()
            .try_get_matches_from(["gnoblin", "--shell", "tiler", "--watch", "--isolate"])
            .expect("the flags are accepted");
        assert_eq!(name_from_matches(&matches).as_deref(), Some("tiler"));
        assert!(flag(&matches, "watch") && flag(&matches, "isolate") && !flag(&matches, "list"));
        assert_eq!(path_from_matches(&matches), None);
        let matches = command()
            .try_get_matches_from(["gnoblin", "--list-shells"])
            .expect("the list flag is accepted");
        assert!(flag(&matches, "list"));
        let matches = command()
            .try_get_matches_from(["gnoblin", "--role", "shell", "--language", "python", "a.py"])
            .expect("the children's flags are accepted");
        assert_eq!(language_from_matches(&matches).as_deref(), Some("python"));
    }

    #[test]
    fn a_path_and_a_name_together_are_a_usage_error() {
        let error = command()
            .try_get_matches_from(["gnoblin", "--shell", "tiler", "./x.so"])
            .expect_err("a path and a name together");
        assert!(error.use_stderr(), "a usage error, not help: {error}");
    }

    #[test]
    fn listing_does_not_take_the_flags_that_run_a_shell() {
        for flags in [["--watch"], ["--isolate"]] {
            let mut words = vec!["gnoblin", "--list-shells"];
            words.extend(flags);
            assert!(command().try_get_matches_from(words).is_err());
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
        let plan = gnoblin_session::Plan::standard(false, false, false, Vec::new()).expect("the standard plan");
        let children: Vec<&str> = plan.children().iter().map(|child| child.name.as_str()).collect();
        for child in ["compositor", "services", "shell"] {
            assert!(
                children.contains(&child),
                "`{child}` is in the standard plan: {children:?}"
            );
        }
    }
}
