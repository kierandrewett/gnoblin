//! Command line client for the Gnoblin IPC.
//!
//! `gnoblinctl` is the short-lived client of `design/rust-compositor.md` ("One
//! IPC"): it connects to `$XDG_RUNTIME_DIR/gnoblin/ipc.sock` (or
//! `$GNOBLIN_IPC_SOCKET`), exchanges the fixed-layout handshake, and speaks the
//! object model through [`gnoblin_client`]. It has no compositor code in it and no
//! opinion about policy: it subscribes, prints, calls and waits.
//!
//! # The command tree is generated
//!
//! The schema is the only description of what exists, so every subcommand comes
//! from it (`build.rs` renders `crates/gnoblin-schema/schema/*.toml` into
//! `OUT_DIR`, and `src/tree.rs` includes the result). One shape covers every
//! object the schema can ever declare:
//!
//! ```text
//! gnoblinctl [--json] <object> list
//! gnoblinctl [--json] <object> get <ID> [PROPERTY]
//! gnoblinctl [--json] <object> watch [ID] [--count N]
//! gnoblinctl [--json] <object> call <ID> <METHOD> [NAME=VALUE ...]
//! gnoblinctl ipc dump [FILE]
//! gnoblinctl completions <bash|zsh|fish> [--output PATH]
//! gnoblinctl log [--database PATH] [--level LEVEL] [--role ROLE] [--target TARGET] [--limit N]
//!     [--since TIME] [--until TIME] [--follow [--interval DURATION]]
//! ```
//!
//! A property, a method or an argument the schema does not declare is refused by
//! the parser with exit 2, and an argument is parsed and checked against the
//! member's declared type before anything is sent. Method arguments are
//! `NAME=VALUE` pairs rather than generated flags, because the tree is built at run
//! time and one token shape covers every method the schema can declare.
//!
//! JSON is an edge, not a dialect: `--json` prints the model's own members as JSON
//! (one document for `list`/`get`/`call`, one object per line for `watch`).
//!
//! # Exit codes
//!
//! | code | meaning                                                             |
//! | ---- | ------------------------------------------------------------------- |
//! | 0    | the command did what it was asked                                   |
//! | 1    | an error response, or the connection ended early                    |
//! | 2    | the command line was wrong                                          |
//! | 3    | the compositor could not be reached                                 |
//! | 4    | the compositor speaks an IPC version this build cannot use           |
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it. `tests/failure_modes.rs` drives the
//! real parse, the real command logic and the real exit codes over the in-process
//! loopback or a real socket; the unit tests live beside the code they check.
//!
//! 1. **The command tree drifts from the schema.** A member the schema declares has
//!    no subcommand, possible value or typed parse, or one it does not declare
//!    appears: `the_command_tree_matches_the_schema` walks the clap tree and the
//!    catalog and compares them member by member, and
//!    `the_wire_version_is_the_schema_version` checks the version the handshake
//!    advertises. The generator fails the build when two objects would share a
//!    command name, and every generated type is parsed by `gnoblin_schema::Type`
//!    rather than by a copy of its grammar.
//! 2. **A sensitive payload is printed.** `ipc dump` prints the routing fields of a
//!    `sensitive` frame and `[redacted]` in place of everything else:
//!    `a_sensitive_frame_is_never_printed`. An argument the schema marks secret is
//!    never echoed in an error either: `args::tests::a_secret_parameter_value_never_reaches_the_error`.
//!    A property the schema marks secret is skipped by `list`, `get` and `watch`
//!    (`output::secret`); the current schema declares no secret property, so that
//!    branch is the one place with no test.
//! 3. **A hang when the server is gone.** A socket that is not there is exit 3 with
//!    the path named, and a compositor that closes the connection ends `watch` with
//!    exit 1 instead of blocking forever: `a_missing_socket_is_a_connection_error`,
//!    `a_closed_connection_ends_a_watch`.
//! 4. **A wrong exit code.** Each code has one reason and one test:
//!    `a_successful_list_exits_0`, `an_error_response_exits_1`,
//!    `a_failed_operation_exits_1`, `watching_an_unknown_identity_is_an_error_response`,
//!    `bad_arguments_exit_2`,
//!    `a_missing_socket_is_a_connection_error` (3), `a_version_mismatch_exits_4`.
//! 5. **A recording this build cannot read is half-printed.** A stream that ends
//!    inside a frame, or declares more bytes than the frame limit, is reported
//!    instead of being shown short: `a_truncated_recording_is_reported`.
//!
//! # What is not here
//!
//! The socket. It lives in `gnoblin_client::unix` rather than here, because the
//! path resolution and the stream over it are what every IPC client shares; this
//! crate re-exports them and adds nothing of its own.

use std::io::{self, Write};
use std::path::PathBuf;

mod args;
mod cli;
mod commands;
mod connection;
mod dump;
mod logs;
mod output;
mod tree;
mod value;

pub use connection::{ConnectFailure, Connection, SocketConnection};
// Where the socket is and the transport over it are the client library's, because
// every client of the IPC has to find the same file and speak the same handshake.
pub use gnoblin_client::unix::{DEFAULT_SOCKET, RUNTIME_VARIABLE, SOCKET_VARIABLE, UnixTransport, socket_path};

/// The command tree, built from the generated schema table.
///
/// Public so the tests can compare it with the schema itself: the drift this
/// crate must not have is between these two, and the only way to check it is to
/// walk both.
pub fn command_tree() -> clap::Command {
    cli::command_tree()
}

/// Run one command line, without the program name.
///
/// The connection is passed in so a test drives the real parse, the real command
/// logic and the real exit codes over the in-process loopback or a socket, without
/// a process-global environment write.
pub fn run(
    args: impl IntoIterator<Item = std::ffi::OsString>,
    connection: impl Connection,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let invocation = match cli::parse(std::iter::once(std::ffi::OsString::from("gnoblinctl")).chain(args)) {
        Ok(invocation) => invocation,
        Err(usage) => {
            let stream: &mut dyn Write = if usage.help { out } else { err };
            let _ = write!(stream, "{}", usage.text);
            return usage.exit_code();
        }
    };
    match invocation {
        cli::Invocation::IpcDump { source } => dump::run(&source, out, err),
        cli::Invocation::Completions { shell, output } => completions(shell, output, out, err),
        cli::Invocation::Log { request } => logs::run(&request, out, err),
        cli::Invocation::Object { object, action, json } => match connection.connect() {
            Ok(mut client) => commands::execute(&mut client, object, &action, json, out, err),
            Err(failure) => {
                let _ = output::error(err, json, failure.code(), &failure.to_string());
                failure.exit_code()
            }
        },
    }
}

/// Write a completion script for `shell`.
fn completions(shell: cli::Shell, output: Option<PathBuf>, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut buffer = Vec::new();
    clap_complete::generate(shell.generator(), &mut command_tree(), "gnoblinctl", &mut buffer);
    let written = match &output {
        Some(path) => {
            std::fs::write(path, &buffer).map_err(|error| format!("cannot write {}: {error}", path.display()))
        }
        None => out
            .write_all(&buffer)
            .map_err(|error| format!("cannot write the completion script: {error}")),
    };
    match written {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "gnoblinctl: {message}");
            1
        }
    }
}

/// The binary's entry point: the process environment and the process streams.
pub fn main() -> u8 {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let stderr = io::stderr();
    let mut err = stderr.lock();
    run(args, SocketConnection::from_environment(), &mut out, &mut err)
}
