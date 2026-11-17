//! Session entry point: the role this process plays.
//!
//! Design/rust-compositor.md, "Process model": one executable, four roles, and a
//! guardian that starts and supervises the others. `gnoblin` is that executable. It
//! parses the command line, dispatches to exactly one role, and owns no policy of its own: a
//! role that needs work done names the crate that does it, which is why this file is
//! dispatch and nothing else.
//!
//! | role                  | what this build does                                              |
//! | --------------------- | ----------------------------------------------------------------- |
//! | none (the guardian)   | supervises the session: starts, restarts and stops every child      |
//! | `--role compositor`   | runs the compositor in the foreground, until it is stopped         |
//! | `--role runtime`      | runs the JavaScript role: loads the configuration and serves it   |
//! | `--role services`     | runs the D-Bus agents: polkit, the keyring and slots               |
//!
//! The compositor role is not in this executable. It is `libgnoblin-core.so.1`'s `gnoblin_main`
//! (`design/modular-framework.md`, M2.10): the launcher links the library through a stub, forwards
//! the command line and exits with the library's code. An optional positional `PATH` names a native
//! shell module (`gnoblin ./libshell.so`); the compositor role loads it in its own process, and the
//! guardian, which starts no runtime child for it, passes it to the compositor it starts.
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
//! 2. **A flag nobody documented appears in `--help`.** The only flags are `--role`,
//!    `--version` and the positional shell path, and the test that walks the help checks that:
//!    `tests/roles.rs::the_help_documents_only_the_flags_this_build_has`.
//! 3. **`--version` prints a partial version.** It prints the full triple the IPC
//!    serves and the build it came from:
//!    `tests/roles.rs::the_version_is_the_full_gnoblin_version`.
//! 4. **A role name is accepted and silently ignored.** An unknown role is a command
//!    line error (exit 2) naming the roles this build has:
//!    `tests/roles.rs::an_unknown_role_is_a_command_line_error`.

mod core_link;
mod role;

use std::process::ExitCode;

gnoblin_log::logger!(log = "gnoblin");

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
    let shell = shell_from_matches(&matches, role)?;
    // The compositor installs no log here: it is the core library's, the log is one copy per library, and the records of the
    // compositor are the library's to write.
    let _log = log_role(&role).map(install_log);
    dispatch(role, shell)
}

/// Installs the log of this process for `process` from the `logging` section and the environment.
fn install_log(process: gnoblin_log::ProcessRole) -> gnoblin_log::init::LogGuard {
    // The configuration is read before the log exists, so a problem with it is reported once
    // the log is installed. The defaults apply meanwhile.
    let (logging, problem) = match gnoblin_config::logging_from_environment() {
        Ok(logging) => (logging, None),
        Err(problem) => (gnoblin_config::Logging::default(), Some(problem)),
    };
    let options = log_options(&logging);
    let guard = gnoblin_log::init::init(process, &options);
    if let Some(problem) = problem {
        log::warn!(area = "logging", "{problem}; the default logging is used");
    }
    guard
}

/// The shell module the command line names, made absolute, or `None` for none.
///
/// Only the guardian and the compositor serve one; the guardian passes it on to the compositor it starts. A path that is not
/// a file ends the start here, with the reason, rather than as a compositor child that exits and a session that never comes up.
fn shell_from_matches(matches: &clap::ArgMatches, role: role::Role) -> Result<Option<std::path::PathBuf>, Exit> {
    let Some(path) = role::shell_from_matches(matches) else {
        return Ok(None);
    };
    if !matches!(role, role::Role::Guardian | role::Role::Compositor) {
        eprintln!(
            "gnoblin: a shell path applies to the guardian and the compositor, not to `--role {}`",
            role.name()
        );
        return Err(Exit(2));
    }
    match std::path::absolute(&path) {
        Ok(absolute) if absolute.is_file() => Ok(Some(absolute)),
        _ => {
            eprintln!("gnoblin: cannot find the shell module `{}`", path.display());
            Err(Exit(1))
        }
    }
}

/// The command line the core's `gnoblin_main` serves: the program, the role and the shell path.
fn core_arguments(shell: Option<&std::path::Path>) -> Vec<std::ffi::OsString> {
    let mut arguments = vec!["gnoblin".into(), "--role".into(), "compositor".into()];
    arguments.extend(shell.map(|path| path.as_os_str().to_owned()));
    arguments
}

/// The exit a code the core returned is.
fn core_exit(code: i32) -> Result<(), Exit> {
    match code {
        0 => Ok(()),
        code => Err(Exit(u8::try_from(code).unwrap_or(1))),
    }
}

/// Runs one role.
fn dispatch(role: role::Role, shell: Option<std::path::PathBuf>) -> Result<(), Exit> {
    match role {
        role::Role::Compositor => core_exit(core_link::main(&core_arguments(shell.as_deref()))),
        role::Role::Guardian => guardian(shell),
        role::Role::Runtime => runtime(),
        role::Role::Services => services(),
    }
}

/// The process role this executable's own log records for `role`, or `None` for the compositor, which the core library logs.
fn log_role(role: &role::Role) -> Option<gnoblin_log::ProcessRole> {
    match role {
        role::Role::Compositor => None,
        role::Role::Guardian => Some(gnoblin_log::ProcessRole::Guardian),
        role::Role::Runtime => Some(gnoblin_log::ProcessRole::Runtime),
        role::Role::Services => Some(gnoblin_log::ProcessRole::Services),
    }
}

/// The log settings the `logging` section asks for. An unset field keeps the default the log
/// uses with no configuration.
fn log_settings(logging: &gnoblin_config::Logging) -> gnoblin_log::init::LogSettings {
    let defaults = gnoblin_log::init::LogSettings::default();
    gnoblin_log::init::LogSettings {
        console: match logging.console {
            None | Some(gnoblin_config::LogConsole::Auto) => gnoblin_log::init::Console::Auto,
            Some(gnoblin_config::LogConsole::Pretty) => gnoblin_log::init::Console::Pretty,
            Some(gnoblin_config::LogConsole::Off) => gnoblin_log::init::Console::Off,
        },
        level: logging.level.clone().unwrap_or(defaults.level),
    }
}

/// The log options for this process: the environment over the `logging` section. When the
/// section keeps the log, the log file receives every record. A file that cannot open is a
/// problem the log reports once it is installed, and console logging still works.
fn log_options(logging: &gnoblin_config::Logging) -> gnoblin_log::init::LogOptions {
    let mut options = gnoblin_log::init::LogOptions::from_environment(&log_settings(logging));
    if logging.file.unwrap_or(true) {
        let opened = gnoblin_log::file::open_from_settings(logging.dir.as_deref(), logging.max_size_mb, |key| {
            std::env::var(key).ok()
        });
        match opened {
            Ok(sink) => options.sink = Some(gnoblin_log::record::SinkHandle::new(Box::new(sink))),
            Err(problem) => options.problems.push(problem),
        }
    }
    options
}

/// Runs the guardian: the session's supervisor, until the session ends.
fn guardian(shell: Option<std::path::PathBuf>) -> Result<(), Exit> {
    let program = std::env::current_exe().map_err(|error| {
        log::error!("cannot find this executable to start its children: {error}");
        Exit(1)
    })?;
    let mut config = gnoblin_session::SupervisorConfig::from_environment(program, shell).map_err(|error| {
        log::error!("{error}");
        Exit(1)
    })?;
    // The settings targets are exported into the session every child inherits.
    gnoblin_compat_host::session::export_guardian_environment(&mut config.rules.environment.session);
    match gnoblin_session::run_guardian(config) {
        Ok(0) => Ok(()),
        Ok(status) => Err(Exit(status)),
        Err(error) => {
            log::error!("{error}");
            Err(Exit(1))
        }
    }
}

/// Runs the runtime, the JavaScript role, until the session ends.
///
/// The role locates and evaluates `shell.ts` (or `shell.js`) itself, commits the configuration document and serves
/// `config.reload`. Nothing selects it: there is no other runtime role.
///
/// No signal handler: the guardian's `SIGTERM` ends this process, and a process that has
/// gone is exactly what the compositor needs to see to release its connection.
fn runtime() -> Result<(), Exit> {
    log::info!("runtime role: JavaScript");
    match gnoblin_js::JsRoleRunner::from_environment().run(never_stopped) {
        Ok(()) => Ok(()),
        Err(error) => {
            log::error!("{error}");
            Err(Exit(1))
        }
    }
}

/// Runs the services role: the D-Bus agents, until the session ends.
fn services() -> Result<(), Exit> {
    let linked = gnoblin_compat_host::services::linked_agents_from_environment();
    match gnoblin_services::Role::from_environment().run(never_stopped, linked) {
        Ok(()) => Ok(()),
        Err(error) => {
            log::error!("{error}");
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
    use super::{Exit, run};

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
