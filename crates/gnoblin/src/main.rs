//! Session entry point: the role this process plays.
//!
//! Design/rust-compositor.md, "Process model": one executable, four roles, and a
//! guardian that starts and supervises the others. `gnoblin` is that executable. It
//! parses one flag, dispatches to exactly one role, and owns no policy of its own: a
//! role that needs work done names the crate that does it, which is why this file is
//! dispatch and nothing else.
//!
//! | role                  | what this build does                                              |
//! | --------------------- | ----------------------------------------------------------------- |
//! | none (the guardian)   | supervises the session: starts, restarts and stops every child      |
//! | `--role compositor`   | runs the compositor in the foreground, until it is stopped         |
//! | `--role runtime`      | runs the Lua host: loads the configuration and serves its handlers |
//! | `--role services`     | runs the D-Bus agents: polkit, the passphrase prompter and slots   |
//!
//! The guardian is the default because that is the process a session manager starts: it is
//! the one that lives for the login, and every other role is a child of it.
//!
//! # Why the guardian's supervisor is in `gnoblin-session`
//!
//! `gnoblin-session` holds the guardian's policy as a pure state machine — a plan of
//! children, events in, effects out — and its `supervisor` module performs those effects.
//! The split is deliberate: the policy is replayable and tested without a process, and the
//! one module that spawns, signals and reads descriptors is named as such. This binary
//! calls it, because the process that starts the other processes is the one that names
//! them.
//!
//! # Failure modes first
//!
//! 1. **The compositor's failure is reported as success, or as a crash.** A refused
//!    start is exit 1 with the reason; the compositor's own loop failure is exit 1
//!    too, because the session is over either way and the message says which:
//!    `tests/roles.rs::the_compositor_reports_a_refused_start`.
//! 2. **A flag nobody documented appears in `--help`.** The only flags are `--role`
//!    and `--version`, and the test that walks the help checks that:
//!    `tests/roles.rs::the_help_documents_only_the_flags_this_build_has`.
//! 3. **`--version` prints a partial version.** It prints the full triple the IPC
//!    serves and the build it came from:
//!    `tests/roles.rs::the_version_is_the_full_gnoblin_version`.
//! 4. **A role name is accepted and silently ignored.** An unknown role is a command
//!    line error (exit 2) naming the roles this build has:
//!    `tests/roles.rs::an_unknown_role_is_a_command_line_error`.

gnoblin_log::component!("gnoblin");

mod compat;
mod dconf;
mod displayconfig;
mod gcr;
mod idlemonitor;
mod introspect;
mod role;
mod settings;

use gnoblin_compat_core::Mode;

use std::process::ExitCode;

fn main() -> ExitCode {
    // The program name is `gnoblin` and run() adds it back, so it is skipped here.
    match run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Exit(code)) => ExitCode::from(code),
    }
}

/// The exit a run ends with, when it is not success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit(pub u8);

/// Parses one command line and runs its role.
///
/// It takes the arguments and returns the exit code instead of calling
/// `std::process::exit`, so the tests exercise the shipped parse and the shipped
/// dispatch rather than a copy of either.
///
/// # Errors
///
/// [`Exit`] with 2 for a command line clap refuses, and with 1 when the compositor
/// cannot start or stops with an error.
pub fn run(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<(), Exit> {
    let matches = match role::command()
        .try_get_matches_from(std::iter::once(std::ffi::OsString::from("gnoblin")).chain(arguments))
    {
        Ok(matches) => matches,
        Err(error) => {
            // clap writes help and version to standard output and a usage error to
            // standard error, and its own exit code says which.
            let _ = error.print();
            return Err(Exit(if error.use_stderr() { 2 } else { 0 }));
        }
    };
    let role = role::from_matches(&matches).map_err(|_| Exit(2))?;
    // The configuration is read before the log exists, so a problem with it is reported once
    // the log is installed. The defaults apply meanwhile.
    let (logging, problem) = match gnoblin_compositor::logging_from_environment() {
        Ok(logging) => (logging, None),
        Err(problem) => (gnoblin_compositor::Logging::default(), Some(problem)),
    };
    let options = gnoblin_log_init::LogOptions::from_environment(&log_settings(&logging));
    let _log = gnoblin_log_init::init(log_role(&role), &options);
    if let Some(problem) = problem {
        gnoblin_log::warn!("[logging] {problem}; the default logging is used");
    }
    dispatch(role)
}

/// Runs one role.
fn dispatch(role: role::Role) -> Result<(), Exit> {
    match role {
        role::Role::Compositor => compositor(),
        role::Role::Guardian => guardian(),
        role::Role::Runtime => runtime(),
        role::Role::Services => services(),
    }
}

/// The process role the log records for this role.
fn log_role(role: &role::Role) -> gnoblin_log::ProcessRole {
    match role {
        role::Role::Compositor => gnoblin_log::ProcessRole::Compositor,
        role::Role::Guardian => gnoblin_log::ProcessRole::Guardian,
        role::Role::Runtime => gnoblin_log::ProcessRole::Runtime,
        role::Role::Services => gnoblin_log::ProcessRole::Services,
    }
}

/// The log settings the `logging` section asks for. An unset field keeps the default the log
/// uses with no configuration.
fn log_settings(logging: &gnoblin_compositor::Logging) -> gnoblin_log_init::LogSettings {
    let defaults = gnoblin_log_init::LogSettings::default();
    gnoblin_log_init::LogSettings {
        console: !matches!(logging.console, Some(gnoblin_compositor::LogConsole::Off)),
        level: logging.level.clone().unwrap_or(defaults.level),
        store: logging.store.unwrap_or(defaults.store),
        store_path: logging
            .store_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from),
        retention: gnoblin_log_init::Retention {
            max_age_days: logging
                .store_max_age_days
                .and_then(|days| u32::try_from(days).ok())
                .unwrap_or(0),
            max_bytes: logging
                .store_max_size_mb
                .and_then(|megabytes| u64::try_from(megabytes).ok())
                .filter(|&megabytes| megabytes > 0)
                .map(|megabytes| megabytes * 1024 * 1024),
        },
    }
}

/// Runs the compositor in the foreground.
fn compositor() -> Result<(), Exit> {
    // The guardian hands this process one end of a pair per privileged child. A pair that
    // cannot be adopted is a refusal rather than a session without it: the child at the
    // other end would talk into a socket nobody reads.
    let privileged = match gnoblin_compositor::Privileged::from_environment() {
        Ok(privileged) => privileged,
        Err(error) => {
            gnoblin_log::error!("{error}");
            return Err(Exit(1));
        }
    };
    let mut config = gnoblin_compositor::SessionConfig {
        privileged,
        ..gnoblin_compositor::SessionConfig::default()
    };
    // A refused extension is logged and left out; the session starts either way.
    gnoblin_compositor::configure_from_environment(&mut config);
    // The compat adapters are served with the session's default mode until a configuration
    // key selects one. A failure to build them starts the session without them.
    if let Err(error) = compat::configure(&mut config, gnoblin_compat_core::Mode::default()) {
        gnoblin_log::warn!("compat adapters not loaded, starting without them: {error:?}");
    }
    gnoblin_compositor::configure_layouts_from_environment(&mut config);
    gnoblin_compositor::configure_cursor_from_environment(&mut config);
    gnoblin_compositor::configure_window_management_from_environment(&mut config);
    gnoblin_compositor::configure_session_tables_from_environment(&mut config);
    gnoblin_compositor::configure_monitors_from_environment(&mut config);
    gnoblin_compositor::configure_xwayland_from_environment(&mut config);
    gnoblin_compositor::configure_renderer_from_environment(&mut config);
    match gnoblin_compositor::Compositor::run(config) {
        Ok(()) => Ok(()),
        Err(error) => {
            gnoblin_log::error!("{error}");
            Err(Exit(1))
        }
    }
}

/// Runs the guardian: the session's supervisor, until the session ends.
fn guardian() -> Result<(), Exit> {
    let program = std::env::current_exe().map_err(|error| {
        gnoblin_log::error!("cannot find this executable to start its children: {error}");
        Exit(1)
    })?;
    let mut config = gnoblin_session::SupervisorConfig::from_environment(program).map_err(|error| {
        gnoblin_log::error!("{error}");
        Exit(1)
    })?;
    // The settings targets are exported into the session every child inherits; a switched-off mode exports nothing.
    let mode = gnoblin_compat_gnome_shell_introspect::mode_from_environment();
    settings::export_qt_settings(&mut config.rules.environment.session, mode);
    if let Some(directory) = dconf::profile_directory() {
        settings::export_dconf_profile(&mut config.rules.environment.session, mode, &directory);
    }
    match gnoblin_session::run_guardian(config) {
        Ok(0) => Ok(()),
        Ok(status) => Err(Exit(status)),
        Err(error) => {
            gnoblin_log::error!("{error}");
            Err(Exit(1))
        }
    }
}

/// Runs the runtime: the Lua host, until the session ends.
///
/// No signal handler: the guardian's `SIGTERM` ends this process, and a process that has
/// gone is exactly what the compositor needs to see to release its connection.
fn runtime() -> Result<(), Exit> {
    let catalog = gnoblin_compositor::catalog().map_err(|error| {
        gnoblin_log::error!("the stock schemas do not load: {error}");
        Exit(1)
    })?;
    match gnoblin_lua::RoleRunner::from_environment(catalog.schemas().to_vec()).run(never_stopped) {
        Ok(()) => Ok(()),
        Err(error) => {
            gnoblin_log::error!("{error}");
            Err(Exit(1))
        }
    }
}

/// The agents the services role links in for `mode`: the prompt adapter when `prompts` is on, the
/// GeoClue agent, which takes the `location` table of the configuration as its policy, and the compat
/// adapters (dconf layer, DisplayConfig, IdleMonitor and introspection) when the mode is on. An `off`
/// mode serves no compat adapter; the GeoClue agent is not a compat adapter and always links.
fn linked_agents_for(mode: Mode, prompts: bool) -> Vec<Box<dyn gnoblin_services::link::LinkedAgent>> {
    let location = gnoblin_services::location_agent();
    let mut agents: Vec<Box<dyn gnoblin_services::link::LinkedAgent>> = Vec::new();
    if prompts {
        agents.push(Box::new(gcr::GcrPrompter::new()));
    }
    agents.push(location);
    if mode != Mode::Off {
        agents.push(Box::new(dconf::DconfLayer::new(mode)));
        agents.push(Box::new(displayconfig::MutterDisplayConfig::new(mode)));
        agents.push(Box::new(idlemonitor::MutterIdleMonitor::new(mode)));
        agents.push(Box::new(introspect::Introspection::new(mode)));
    }
    agents
}

/// Runs the services role: the D-Bus agents, until the session ends.
fn services() -> Result<(), Exit> {
    let prompts = gnoblin_services::prompts::enabled_from_environment();
    let linked = linked_agents_for(gnoblin_compat_gnome_shell_introspect::mode_from_environment(), prompts);
    match gnoblin_services::Role::from_environment().run(never_stopped, linked) {
        Ok(()) => Ok(()),
        Err(error) => {
            gnoblin_log::error!("{error}");
            Err(Exit(1))
        }
    }
}

/// The stop condition a role works under when nobody can ask it to stop.
///
/// The guardian stops a role by signalling its process group, so a role does not install a
/// handler: the kernel's default action ends it, and the connection it held closes with it.
fn never_stopped() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{Exit, linked_agents_for, run};
    use gnoblin_compat_core::Mode;

    #[test]
    fn the_services_role_is_given_the_location_agent() {
        // The role claims the location.agent slot as the services provider, so the GeoClue agent
        // has to be in the list it starts, next to the prompt adapter and the compat adapters
        // that are on in the default mode.
        assert_eq!(linked_agents_for(Mode::default(), true).len(), 6);
    }

    #[test]
    fn the_prompter_is_linked_only_when_prompts_are_enabled() {
        // `prompts.enabled` is false by default, and the names stay unowned while it is: the
        // services role links the GeoClue agent and the compat adapters, but not the prompter.
        assert_eq!(linked_agents_for(Mode::default(), false).len(), 5);
        assert_eq!(linked_agents_for(Mode::default(), true).len(), 6);
    }

    #[test]
    fn the_compat_adapters_are_served_when_the_mode_is_on() {
        // Portal is the default and Unmediated is the other mode that serves reads: both add the
        // dconf layer, DisplayConfig, IdleMonitor and introspection to the prompter and GeoClue agent.
        assert_eq!(linked_agents_for(Mode::Portal, true).len(), 6);
        assert_eq!(linked_agents_for(Mode::Unmediated, true).len(), 6);
    }

    #[test]
    fn the_compat_adapters_are_not_served_when_the_mode_is_off() {
        assert_eq!(linked_agents_for(Mode::Off, true).len(), 2);
    }

    #[test]
    fn the_guardian_is_the_default_role() {
        // No flag means the guardian: that is the process a session manager starts, so the
        // default is the session rather than a usage error. The dispatch is checked through
        // the role the command line names, because running the guardian would run a session.
        let matches = super::role::command()
            .try_get_matches_from(["gnoblin"])
            .expect("no flag is fine");
        assert_eq!(super::role::from_matches(&matches), Ok(super::role::Role::Guardian));
    }

    #[test]
    fn an_unknown_role_is_a_command_line_error() {
        assert_eq!(
            run(["--role", "compositorr"].map(std::ffi::OsString::from)).expect_err("unknown"),
            Exit(2)
        );
    }

    #[test]
    fn the_help_is_not_an_error() {
        assert_eq!(
            run(["--help"].map(std::ffi::OsString::from)).expect_err("help exits without running a role"),
            Exit(0)
        );
    }

    #[test]
    fn the_version_is_printed_and_not_run_as_a_role() {
        assert_eq!(
            run(["--version"].map(std::ffi::OsString::from)).expect_err("version exits"),
            Exit(0)
        );
    }

    #[test]
    fn a_flag_this_build_does_not_have_is_refused() {
        assert_eq!(
            run(["--daemon"].map(std::ffi::OsString::from)).expect_err("no such flag"),
            Exit(2)
        );
    }
}
