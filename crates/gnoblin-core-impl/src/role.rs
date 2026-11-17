//! The compositor role, served from the library: what `gnoblin_main` runs.
//!
//! The `gnoblin` launcher is a thin program: it parses the command line, runs the guardian, the services and the script
//! runtime itself, and hands `--role compositor` to the core, which links the compositor, the compat adapters and the
//! shell host. This module is what used to be the launcher's compositor role: it reads the logging configuration and
//! installs the log of the compositor process (the log is a per-library singleton, so the copy that logs the compositor's
//! records must be installed by the library that holds them), builds the [`SessionConfig`] from the environment and
//! the documents, optionally adds the in-process native shell, and runs the session until it ends.
//!
//! The command line is the launcher's, forwarded as it was parsed: the program name, `--role compositor`, and at most one
//! positional argument, the path of a shell module. Nothing else is accepted, so the two parsers cannot disagree about a
//! flag.

use std::ffi::OsString;
use std::path::PathBuf;

use gnoblin_compositor::{
    Compositor, LOGOUT_EXIT_CODE, LogConsole, Logging, Privileged, SessionConfig, SessionEnd,
    guardian_from_environment, logging_from_environment, open_log_file,
};
use gnoblin_log::ProcessRole;
use gnoblin_log::init::{Console, LogOptions, LogSettings, init};

use crate::{log, native_shell};

/// The exit code of a command line the core does not serve, as the launcher's own usage errors.
pub const EXIT_USAGE: i32 = 2;

/// The exit code of a session that could not start or whose loop failed.
pub const EXIT_FAILED: i32 = 1;

/// What the launcher asked the core to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositorRequest {
    /// The shell module to run in the compositor process, from the command line; `None` for a session whose shell is the
    /// script runtime child (or none).
    pub shell: Option<PathBuf>,
}

/// Reads the command line the launcher forwards: `<program> --role compositor [PATH]`.
///
/// # Errors
///
/// The text to print: a role this core does not serve, a flag it does not know or a second positional argument.
pub fn parse(args: &[OsString]) -> Result<CompositorRequest, String> {
    let mut words = args.iter().skip(1);
    let mut role_seen = false;
    let mut shell = None;
    while let Some(word) = words.next() {
        let text = word.to_string_lossy();
        if text == "--role" {
            let name = words.next().map(|name| name.to_string_lossy().into_owned());
            if name.as_deref() != Some("compositor") {
                return Err(format!(
                    "the core serves `--role compositor`, not `--role {}`",
                    name.unwrap_or_default()
                ));
            }
            role_seen = true;
        } else if text.starts_with('-') && text.len() > 1 {
            return Err(format!("the core does not know the flag `{text}`"));
        } else if shell.replace(PathBuf::from(word)).is_some() {
            return Err("the core takes one shell path".to_owned());
        }
    }
    if !role_seen {
        return Err("the core serves `--role compositor`; no role was named".to_owned());
    }
    Ok(CompositorRequest { shell })
}

/// Serves the request and returns the process's exit code. Runs on the calling thread until the session ends.
pub fn run(args: &[OsString]) -> i32 {
    let request = match parse(args) {
        Ok(request) => request,
        Err(message) => {
            // The log is not installed yet, and a usage error is for the person at the terminal.
            eprintln!("gnoblin: {message}");
            return EXIT_USAGE;
        }
    };
    // The configuration is read before the log exists, so a problem with it is reported once the log is installed. The
    // defaults apply meanwhile.
    let (logging, problem) = match logging_from_environment() {
        Ok(logging) => (logging, None),
        Err(problem) => (Logging::default(), Some(problem)),
    };
    let options = log_options(&logging);
    let _log = init(ProcessRole::Compositor, &options);
    if let Some(problem) = problem {
        log::warn!(area = "logging", "{problem}; the default logging is used");
    }
    serve(request)
}

/// The log settings the `logging` section asks for. An unset field keeps the default the log uses with no configuration.
fn log_settings(logging: &Logging) -> LogSettings {
    let defaults = LogSettings::default();
    LogSettings {
        console: match logging.console {
            None | Some(LogConsole::Auto) => Console::Auto,
            Some(LogConsole::Pretty) => Console::Pretty,
            Some(LogConsole::Off) => Console::Off,
        },
        level: logging.level.clone().unwrap_or(defaults.level),
    }
}

/// The log options for this process: the environment over the `logging` section. When the section keeps the log, the log
/// file receives every record. A file that cannot open is a problem the log reports once it is installed, and console
/// logging still works.
fn log_options(logging: &Logging) -> LogOptions {
    let mut options = LogOptions::from_environment(&log_settings(logging));
    match open_log_file(logging) {
        Ok(Some(sink)) => options.sink = Some(gnoblin_log::record::SinkHandle::new(Box::new(sink))),
        Ok(None) => {}
        Err(problem) => options.problems.push(problem),
    }
    options
}

/// Runs the compositor in the foreground, with the shell the request names in process.
fn serve(request: CompositorRequest) -> i32 {
    // The guardian keeps one control socket open to this process and sends each privileged child's connection down it,
    // tagged with the role it was made for. A control socket that cannot be adopted is a refusal rather than a session
    // without privileged children: every child the guardian starts after it would talk into a socket nobody reads.
    let guardian = match guardian_from_environment() {
        Ok(guardian) => guardian,
        Err(error) => {
            log::error!("{error}");
            return EXIT_FAILED;
        }
    };
    let mut config = SessionConfig {
        privileged: Privileged::declared(),
        guardian,
        power: Some(gnoblin_services::power::system_power_backend()),
        ..SessionConfig::default()
    };
    // The privileged socket is the one door a user-declared tool uses to reach screencopy. It lives in the runtime
    // directory and is owner-only, so it is reachable by this user's own processes and nobody else. Sandboxed clients
    // never get it: they connect through their own listener, which carries no privileged role.
    config.wayland.privileged_socket_path = gnoblin_compositor::default_privileged_socket_path();
    // The compat adapters are served with the session's default mode until a configuration key selects one. A failure to
    // build them starts the session without them.
    if let Err(error) = gnoblin_compat_host::session::configure(&mut config, gnoblin_compat_host::Mode::default()) {
        log::warn!("compat adapters not loaded, starting without them: {error:?}");
    }
    gnoblin_compositor::configure_cursor_from_environment(&mut config);
    gnoblin_compositor::configure_bell_from_environment(&mut config);
    gnoblin_compositor::configure_accessibility_from_environment(&mut config);
    gnoblin_compositor::configure_window_management_from_environment(&mut config);
    gnoblin_compositor::configure_session_tables_from_environment(&mut config);
    gnoblin_compositor::configure_monitors_from_environment(&mut config);
    gnoblin_compositor::configure_xwayland_from_environment(&mut config);
    gnoblin_compositor::configure_renderer_from_environment(&mut config);
    // The backend is decided last: it reads the same document and the environment, and a refusal of it (a value no backend
    // has, a nested session with no host) ends the start with the reason rather than starting headless.
    if let Err(error) = gnoblin_compositor::configure_backend_from_environment(&mut config) {
        log::error!("{error}");
        return EXIT_FAILED;
    }
    if let Some(path) = request.shell {
        config.shell = Some(native_shell::spec(path));
    }
    match Compositor::run(config) {
        Ok(SessionEnd::Stopped) => 0,
        // The guardian reads this code as a logout and ends the session rather than restarting it.
        Ok(SessionEnd::LoggedOut) => i32::from(LOGOUT_EXIT_CODE),
        Err(error) => {
            log::error!("{error}");
            EXIT_FAILED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_compositor_role_with_no_shell_is_a_request_for_none() {
        assert_eq!(
            parse(&args(&["gnoblin", "--role", "compositor"])),
            Ok(CompositorRequest { shell: None })
        );
    }

    #[test]
    fn a_positional_argument_is_the_shell_path() {
        assert_eq!(
            parse(&args(&["gnoblin", "--role", "compositor", "/x/libshell.so"])),
            Ok(CompositorRequest {
                shell: Some(PathBuf::from("/x/libshell.so"))
            })
        );
    }

    #[test]
    fn another_role_a_flag_or_a_second_path_is_refused() {
        for bad in [
            &["gnoblin", "--role", "services"][..],
            &["gnoblin", "--role"][..],
            &["gnoblin", "--role", "compositor", "--isolate"][..],
            &["gnoblin", "--role", "compositor", "a.so", "b.so"][..],
            &["gnoblin", "a.so"][..],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
