//! Session entry point: the role this process plays.
//!
//! Design/rust-compositor.md, "Process model": one executable, five roles, and a
//! guardian that starts and supervises the others. `gnoblin` is that executable. It
//! parses one flag, dispatches to exactly one role, and owns no policy of its own: a
//! role that needs work done names the crate that does it, which is why this file is
//! dispatch and nothing else.
//!
//! | role                  | what this build does                                              |
//! | --------------------- | ----------------------------------------------------------------- |
//! | `--role compositor`   | runs the compositor in the foreground, until it is stopped         |
//! | `--role runtime`      | refused: exit 78, with what it would run                           |
//! | `--role services`     | refused: exit 78, with what it would run                           |
//! | `--role greeter`      | refused: exit 78, with what it would run                           |
//! | none (the guardian)   | refused: exit 78, with what `gnoblin-session` still needs          |
//!
//! Exit 78 is `EX_CONFIG` from `sysexits.h`: the invocation is well formed and the
//! process cannot do what it was asked on this build, which a supervisor must
//! distinguish from a crash (a signal), a bug (1) and a bad command line (2). A
//! supervisor that treats 78 as a crash loops forever on a build that will never
//! serve the role.
//!
//! # Why the guardian is not here yet
//!
//! `gnoblin-session` holds the guardian's whole policy as a pure state machine: a
//! [`gnoblin_session::Plan`] of children, [`gnoblin_session::Event`]s in, and
//! [`gnoblin_session::Effect`]s out. Performing those effects is the missing half, and
//! it is a process supervisor rather than a line of dispatch — see
//! [`role::guardian_needs`] for the list this binary prints.
//!
//! # Failure modes first
//!
//! 1. **A build answers a role it cannot serve and the supervisor believes it.** Every
//!    unimplemented role exits 78 with the reason on standard error, and the role it
//!    would have run named: `tests/roles.rs::an_unimplemented_role_exits_78`.
//! 2. **The compositor's failure is reported as success, or as a crash.** A refused
//!    start is exit 1 with the reason; the compositor's own loop failure is exit 1
//!    too, because the session is over either way and the message says which:
//!    `tests/roles.rs::the_compositor_reports_a_refused_start`.
//! 3. **A flag nobody documented appears in `--help`.** The only flags are `--role`
//!    and `--version`, and the test that walks the help checks that:
//!    `tests/roles.rs::the_help_documents_only_the_flags_this_build_has`.
//! 4. **`--version` prints a partial version.** It prints the full triple the IPC
//!    serves and the build it came from:
//!    `tests/roles.rs::the_version_is_the_full_gnoblin_version`.
//! 5. **A role name is accepted and silently ignored.** An unknown role is a command
//!    line error (exit 2) naming the roles this build has:
//!    `tests/roles.rs::an_unknown_role_is_a_command_line_error`.

mod role;

use std::process::ExitCode;

/// What a refusal exits with: `EX_CONFIG`, "the invocation is fine, this build
/// cannot serve it".
const NOT_IMPLEMENTED: u8 = 78;

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
/// [`Exit`] with 2 for a command line clap refuses, with [`NOT_IMPLEMENTED`] for a
/// role this build does not serve, and with 1 when the compositor cannot start or
/// stops with an error.
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
    dispatch(role)
}

/// Runs one role.
fn dispatch(role: role::Role) -> Result<(), Exit> {
    match role {
        role::Role::Compositor => compositor(),
        role::Role::Guardian => Err(refuse("guardian", &role::guardian_needs())),
        role::Role::Runtime | role::Role::Services | role::Role::Greeter => {
            Err(refuse(role.name(), &role::not_implemented(role)))
        }
    }
}

/// Runs the compositor in the foreground.
fn compositor() -> Result<(), Exit> {
    match gnoblin_compositor::Compositor::run(gnoblin_compositor::Config::default()) {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("gnoblin: compositor: {error}");
            Err(Exit(1))
        }
    }
}

/// Reports a role this build does not serve.
///
/// The message names the role, the pieces it needs and the exit code, in that order:
/// a person reads it once, and a supervisor's log keeps the reason next to the code.
fn refuse(role: &str, needs: &[String]) -> Exit {
    eprintln!("gnoblin: the {role} role is not implemented in this build ({NOT_IMPLEMENTED})");
    for need in needs {
        eprintln!("  needs: {need}");
    }
    Exit(NOT_IMPLEMENTED)
}

#[cfg(test)]
mod tests {
    use super::{Exit, NOT_IMPLEMENTED, run};

    #[test]
    fn an_unimplemented_role_is_refused_with_ex_config() {
        for role in ["runtime", "services", "greeter", "guardian"] {
            let exit = run(["--role", role].map(std::ffi::OsString::from)).expect_err("the role is not implemented");
            assert_eq!(exit, Exit(NOT_IMPLEMENTED), "`{role}` reports EX_CONFIG");
        }
    }

    #[test]
    fn the_guardian_is_the_default_role() {
        // No flag means the guardian, which is refused for the same documented reason
        // rather than doing something else quietly.
        assert_eq!(
            run(std::iter::empty()).expect_err("not implemented"),
            Exit(NOT_IMPLEMENTED)
        );
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
