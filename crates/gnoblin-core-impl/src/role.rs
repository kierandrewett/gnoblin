//! The compositor and shell roles, served from the library: what `gnoblin_main` runs.
//!
//! The `gnoblin` launcher is a thin program: it parses the command line, resolves the shell, runs the guardian and the
//! services itself, and hands `--role compositor` or `--role shell` to the core, which links the compositor, the compat
//! adapters and the shell host. The compositor role reads the logging configuration and installs the log of the
//! compositor process (the log is a per-library singleton, so the copy that logs the compositor's records must be installed
//! by the library that holds them), builds the [`SessionConfig`] from the environment and the documents, optionally adds
//! the in-process shell, and runs the session until it ends. The shell role is the same shell host in a process of its own
//! (`--isolate`): it attaches to the compositor over its socket ([`crate::shell_role`]).
//!
//! The command line is the launcher's, forwarded as it was parsed: the program name, `--role compositor` or `--role shell`,
//! the flags `--watch` and `--language LANGUAGE`, and at most one positional argument, the path of a shell module. Nothing
//! else is accepted, so the two parsers cannot disagree about a flag.

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

/// Which role the core serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreRole {
    /// The compositor, with the shell in its process when it is given a module.
    Compositor,
    /// The shell host in a process of its own.
    Shell,
}

/// The module a shell runs and how it is loaded and watched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellModule {
    /// The module file.
    pub path: PathBuf,
    /// The language to load it as (`native`, `python`, `js`, `ts`, ...), when the launcher resolved one; `None` lets the loader
    /// detect it.
    pub language: Option<String>,
    /// Reload the shell when its module changes (`--watch`).
    pub watch: bool,
}

/// What the launcher asked the core to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreRequest {
    /// The role.
    pub role: CoreRole,
    /// The shell module: for the compositor, the shell to run in its process (`None` runs the compositor alone); for the
    /// shell role, always there.
    pub shell: Option<ShellModule>,
}

/// Reads the command line the launcher forwards:
/// `<program> --role compositor|shell [--watch] [--language LANGUAGE] [PATH]`.
///
/// # Errors
///
/// The text to print: a role this core does not serve, a flag it does not know, a second positional argument, flags with no
/// module to apply to, or a shell role with no module.
pub fn parse(args: &[OsString]) -> Result<CoreRequest, String> {
    let mut words = args.iter().skip(1);
    let mut role = None;
    let mut path = None;
    let mut watch = false;
    let mut language = None;
    while let Some(word) = words.next() {
        let text = word.to_string_lossy();
        if text == "--role" {
            let name = words.next().map(|name| name.to_string_lossy().into_owned());
            role = Some(match name.as_deref() {
                Some("compositor") => CoreRole::Compositor,
                Some("shell") => CoreRole::Shell,
                other => {
                    return Err(format!(
                        "the core serves `--role compositor` and `--role shell`, not `--role {}`",
                        other.unwrap_or_default()
                    ));
                }
            });
        } else if text == "--watch" {
            watch = true;
        } else if text == "--language" {
            let Some(name) = words.next().map(|name| name.to_string_lossy().into_owned()) else {
                return Err("`--language` needs a language".to_owned());
            };
            language = Some(name);
        } else if text.starts_with('-') && text.len() > 1 {
            return Err(format!("the core does not know the flag `{text}`"));
        } else if path.replace(PathBuf::from(word)).is_some() {
            return Err("the core takes one shell path".to_owned());
        }
    }
    let Some(role) = role else {
        return Err("the core serves `--role compositor` and `--role shell`; no role was named".to_owned());
    };
    let shell = match path {
        Some(path) => Some(ShellModule { path, language, watch }),
        None if role == CoreRole::Shell => return Err("`--role shell` needs the shell module's path".to_owned()),
        None if watch || language.is_some() => {
            return Err("`--watch` and `--language` apply to a shell module, and none was given".to_owned());
        }
        None => None,
    };
    Ok(CoreRequest { role, shell })
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
    match request.role {
        CoreRole::Compositor => serve(request.shell),
        CoreRole::Shell => crate::shell_role::serve_isolated(request.shell.expect("the shell role was given a module")),
    }
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
fn serve(shell: Option<ShellModule>) -> i32 {
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
    // The providers std registers (window management, portal dialogs, the visual settings) are put in the process's slots,
    // which the session takes when it serves.
    config.providers = crate::providers::slots();
    match shell {
        // A shell given on the command line decides for itself whether to load std (its `open` registers std's defaults), so
        // a shell that does not runs the core's own defaults.
        Some(module) => config.shell = Some(native_shell::spec(module)),
        // No shell in this process: the compositor opens std for the stock session.
        None => crate::std_host::install_stock(),
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

    fn module(path: &str) -> ShellModule {
        ShellModule {
            path: PathBuf::from(path),
            language: None,
            watch: false,
        }
    }

    #[test]
    fn the_compositor_role_with_no_shell_is_a_request_for_none() {
        assert_eq!(
            parse(&args(&["gnoblin", "--role", "compositor"])),
            Ok(CoreRequest {
                role: CoreRole::Compositor,
                shell: None
            })
        );
    }

    #[test]
    fn a_positional_argument_is_the_shell_path() {
        assert_eq!(
            parse(&args(&["gnoblin", "--role", "compositor", "/x/libshell.so"])),
            Ok(CoreRequest {
                role: CoreRole::Compositor,
                shell: Some(module("/x/libshell.so"))
            })
        );
    }

    #[test]
    fn the_flags_of_a_module_come_before_its_path() {
        assert_eq!(
            parse(&args(&[
                "gnoblin",
                "--role",
                "shell",
                "--watch",
                "--language",
                "python",
                "/x/shell.py"
            ])),
            Ok(CoreRequest {
                role: CoreRole::Shell,
                shell: Some(ShellModule {
                    path: PathBuf::from("/x/shell.py"),
                    language: Some("python".to_owned()),
                    watch: true,
                })
            })
        );
    }

    #[test]
    fn another_role_a_flag_a_second_path_or_a_flag_without_a_path_is_refused() {
        for bad in [
            &["gnoblin", "--role", "services"][..],
            &["gnoblin", "--role"][..],
            &["gnoblin", "--role", "compositor", "--isolate"][..],
            &["gnoblin", "--role", "compositor", "a.so", "b.so"][..],
            &["gnoblin", "a.so"][..],
            &["gnoblin", "--role", "shell"][..],
            &["gnoblin", "--role", "compositor", "--watch"][..],
            &["gnoblin", "--role", "compositor", "--language"][..],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
