//! The roles this executable has, and what the ones this build does not serve need.
//!
//! Design/rust-compositor.md, "Process model", gives the executable five roles and
//! keeps them in one binary. The command line is therefore one flag and nothing else:
//! a role is a word a supervisor passes, not a configuration surface, and every
//! other knob belongs to the configuration document a role reads.
//!
//! The refusals are data, not prose sprinkled at the call site: [`not_implemented`]
//! and [`guardian_needs`] each return the list of what the role would run, so the
//! message a supervisor logs and the list a later lane works from are the same list.

use clap::{Arg, ArgMatches, Command};
use gnoblin_session::Plan;

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
    /// The login greeter, which bsm drives.
    Greeter,
}

impl Role {
    /// Every role, in the order the plan lists them.
    pub const ALL: [Role; 5] = [
        Role::Guardian,
        Role::Compositor,
        Role::Runtime,
        Role::Services,
        Role::Greeter,
    ];

    /// The word the command line uses.
    pub fn name(self) -> &'static str {
        match self {
            Role::Guardian => "guardian",
            Role::Compositor => "compositor",
            Role::Runtime => "runtime",
            Role::Services => "services",
            Role::Greeter => "greeter",
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
                    "The role this process plays. `compositor` runs the Wayland session in the foreground; \
                     `runtime`, `services` and `greeter` are roles this build does not serve yet.",
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
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        format!(
            "{} (build {})",
            gnoblin_compositor::version(),
            env!("CARGO_PKG_VERSION")
        )
    })
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

/// What a role this build does not serve would run.
pub fn not_implemented(role: Role) -> Vec<String> {
    match role {
        Role::Guardian => guardian_needs(),
        Role::Compositor => vec!["this build runs the compositor; the refusal is a bug".to_owned()],
        Role::Runtime => vec![
            "the Lua host of `gnoblin-lua`: the configuration loader, the event handlers, the \
             request handlers and the timers, which this build does not carry"
                .to_owned(),
            "a privileged IPC connection the guardian hands down as an inherited descriptor \
             (`design/rust-compositor.md`, \"Process model\"), which the guardian does not \
             create yet"
                .to_owned(),
        ],
        Role::Services => vec![
            "the D-Bus agents of `gnoblin-services`: polkit and GeoClue agents, `prompt.secret`, \
             the PipeWire privacy monitor, `org.freedesktop.ScreenSaver`, idle and inhibit"
                .to_owned(),
            "the same inherited-descriptor connection the runtime uses, so a request is offered \
             to exactly one privileged client"
                .to_owned(),
        ],
        Role::Greeter => vec![
            "the IPC extension namespace `bsm.greeter`, which design/rust-compositor.md's \"IPC \
             extensions\" describes and this build does not implement"
                .to_owned(),
            "a restricted connection from `bsm-helper` that grants that namespace and read-only \
             appearance, and nothing else on the IPC"
                .to_owned(),
        ],
    }
}

/// What the guardian needs that `gnoblin-session` does not carry.
///
/// The crate is the guardian's whole policy as a pure state machine — a plan of
/// children, events in, effects out — and the missing half is the process supervisor
/// that performs the effects. The list is built from a real plan, so it names the
/// children a session starts today rather than describing them in prose.
pub fn guardian_needs() -> Vec<String> {
    let mut needs = Vec::new();
    match Plan::standard(false, Vec::new()) {
        Ok(plan) => {
            let children: Vec<&str> = plan.children().iter().map(|child| child.name.as_str()).collect();
            needs.push(format!(
                "a supervisor loop driving `gnoblin_session::Guardian::apply` over the standard plan \
                 ({}), performing the effects it returns in order",
                children.join(", ")
            ));
        }
        Err(error) => needs.push(format!(
            "the standard session plan `gnoblin-session` describes: {error}"
        )),
    }
    needs.push(
        "a process spawner: `Effect::Spawn` names a child, a generation and the complete \
         environment, and the child must start in its own process group so a stop reaches its \
         helpers"
            .to_owned(),
    );
    needs.push(
        "readiness detection per child: the compositor's is its Wayland and IPC sockets, the \
         runtime's and services' is the connection they open, and each has a deadline \
         `gnoblin-session` reports as a missed `Ready`"
            .to_owned(),
    );
    needs.push(
        "signal plumbing: `Effect::Signal` and `Effect::Kill` name a child's group, and the \
         session manager's own SIGTERM has to become an `Event`"
            .to_owned(),
    );
    needs.push(
        "`Effect::ExportEnv` and `gnoblin_session::EnvironmentRules`: the compositor's published \
         display variables are exported before the next child is spawned and validated against \
         the rules"
            .to_owned(),
    );
    needs.push(
        "a clock and a log sink: every `Event` carries its own `gnoblin_session::Tick`, so the loop \
         needs a timer, and every `Effect::Log` line needs a destination; `Effect::SetRecovery` \
         needs the recovery state on disk"
            .to_owned(),
    );
    needs
}

#[cfg(test)]
mod tests {
    use super::{Role, command, from_matches, guardian_needs, not_implemented, version_line};

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
    fn the_guardian_names_the_plan_it_would_supervise() {
        let needs = guardian_needs();
        assert!(needs.len() > 1, "several pieces are missing, not one: {needs:#?}");
        assert!(
            needs[0].contains("compositor"),
            "the plan it would supervise is named: {}",
            needs[0]
        );
        assert!(
            needs[0].contains("services") && needs[0].contains("runtime"),
            "every child of the standard plan is named: {}",
            needs[0]
        );
    }

    #[test]
    fn every_unimplemented_role_says_what_it_would_run() {
        for role in [Role::Runtime, Role::Services, Role::Greeter] {
            let needs = not_implemented(role);
            assert!(!needs.is_empty(), "`{}` names what it needs", role.name());
            assert!(
                needs.iter().all(|need| !need.is_empty()),
                "`{}` has an empty entry",
                role.name()
            );
        }
    }
}
