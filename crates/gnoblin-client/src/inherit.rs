//! Adopting the descriptor the guardian passed to this process.
//!
//! Design/rust-compositor.md, "Process model": the runtime and the services role reach
//! the compositor over "an inherited file descriptor that carries extra capabilities".
//! The guardian creates one socketpair per role child, keeps one end for the compositor
//! and hands the other to the child by clearing `FD_CLOEXEC` for exactly that
//! descriptor and exactly that spawn, with the number in [`FD_VARIABLE`]. This module is
//! the child's half: it reads the number and turns it back into a socket.
//!
//! # The one unsafe block in the client
//!
//! No safe Rust API turns an integer into a descriptor: `OwnedFd` can be built from a
//! `File`, a `TcpStream` or another owner, and every one of those conversions starts
//! from an owner, not a number. std and `rustix` both take `unsafe` to cross that line,
//! and the exception is the one the workspace's `Cargo.toml` describes: a crate that
//! must cross it drops the inherited lint table, restates the same `clippy` levels, sets
//! `unsafe_code = "deny"` and confines the block to a named module with a `SAFETY`
//! comment. That is this module, and [`descriptor`] is its only block.
//!
//! The alternative was to pass the connection as the child's standard input, which needs
//! no `unsafe` but makes a control socket the process's stdin for the whole login: every
//! helper the child spawns would inherit it, and a debugger or a shell attached to the
//! role would have no stdin left. An inherited descriptor with a number in the
//! environment is what the plan describes and what `sd_listen_fds` does, so the shim is
//! the smaller cost.
//!
//! # Failure modes first
//!
//! 1. **The variable names no descriptor.** The caller gets [`Inherited::Absent`], which
//!    is the ordinary state of a process the guardian did not start, rather than an
//!    error: `a_process_without_the_variable_has_no_inherited_connection`.
//! 2. **The variable names something that is not a descriptor number.** It is refused
//!    with the value quoted, because adopting the wrong number would take a descriptor
//!    away from whoever owns it: `a_value_that_is_not_a_number_is_refused`.
//! 3. **The variable names a descriptor of the wrong kind.** A pipe or a file where the
//!    socket belongs is refused after the adoption attempt rather than used as a socket:
//!    `a_descriptor_that_is_not_a_socket_is_refused`.

use std::fmt;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

use gnoblin_proto::privilege;

/// Why no inherited connection could be adopted.
#[derive(Debug)]
pub enum Inherited {
    /// The environment names no descriptor: the guardian did not start this process.
    Absent,
    /// The variable's value is not a descriptor number.
    NotANumber {
        /// The name of the variable that was read.
        variable: String,
        /// What it held.
        value: String,
    },
    /// The descriptor exists but is not a socket, or cannot be adopted.
    NotASocket {
        /// The number that was named.
        number: i32,
        /// What the kernel said.
        reason: String,
    },
}

impl fmt::Display for Inherited {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inherited::Absent => write!(
                formatter,
                "no inherited IPC connection: {} is not set",
                privilege::FD_VARIABLE
            ),
            Inherited::NotANumber { variable, value } => {
                write!(formatter, "{variable}={value} is not a descriptor number")
            }
            Inherited::NotASocket { number, reason } => {
                write!(formatter, "descriptor {number} is not a usable socket: {reason}")
            }
        }
    }
}

impl std::error::Error for Inherited {}

/// The socket `variable` names in `env`, if the variable names one.
///
/// `env` is passed in so a caller can read the environment once and so the failures
/// can be tested without a process-global read, which the tests share with every other
/// test in the binary. The variable is a parameter because there are two spellings of
/// the same mechanism: a role child reads [`privilege::FD_VARIABLE`], and the
/// compositor reads one [`privilege::owner_variable`] per child.
///
/// # Errors
///
/// [`Inherited::NotANumber`] when the variable holds something that is not a
/// descriptor number, and [`Inherited::NotASocket`] when the number names a descriptor
/// that is not a connected socket.
pub fn stream(variable: &str, env: impl Fn(&str) -> Option<String>) -> Result<Option<UnixStream>, Inherited> {
    let Some(value) = env(variable) else {
        return Ok(None);
    };
    let Some(number) = privilege::descriptor(&value) else {
        return Err(Inherited::NotANumber {
            variable: variable.to_owned(),
            value,
        });
    };
    adopt(number).map(Some)
}

/// The file `variable` names in `env`, if the variable names one.
///
/// The report descriptor a role child writes its readiness on is a pipe, not a socket, so
/// this is the same adoption as [`stream`] without the socket check. It is a separate
/// function rather than a flag because the two check different things: a socket that is
/// not a socket is a bug, and a pipe that is not a socket is the contract.
///
/// # Errors
///
/// [`Inherited::NotANumber`] when the variable holds something that is not a descriptor
/// number.
pub fn file(variable: &str, env: impl Fn(&str) -> Option<String>) -> Result<Option<std::fs::File>, Inherited> {
    let Some(value) = env(variable) else {
        return Ok(None);
    };
    let Some(number) = privilege::descriptor(&value) else {
        return Err(Inherited::NotANumber {
            variable: variable.to_owned(),
            value,
        });
    };
    Ok(Some(std::fs::File::from(descriptor(number))))
}

/// Take ownership of the descriptor `number` and check that it is a socket.
///
/// The check is `getsockname`, which a descriptor of any other kind answers with
/// `ENOTSOCK`: adopting a number is the part that needs the shim, and using a pipe as a
/// socket is the failure that would follow it silently.
fn adopt(number: i32) -> Result<UnixStream, Inherited> {
    let fd = descriptor(number);
    // `local_addr` is the probe: a `UnixStream` wrapped around a descriptor of the
    // wrong kind fails it rather than reading nonsense from it later.
    let stream = UnixStream::from(fd);
    match stream.local_addr() {
        Ok(_) => Ok(stream),
        Err(error) => Err(Inherited::NotASocket {
            number,
            reason: error.to_string(),
        }),
    }
}

/// The one block in this crate that turns a number into an owner.
///
/// # Safety
///
/// The descriptor came from this process's parent, which created it for this child,
/// cleared `FD_CLOEXEC` on it for exactly this `exec` and closed its own copy, so
/// nothing else owns it. `OwnedFd` takes that ownership and closes the descriptor
/// once, on drop.
#[allow(unsafe_code)]
fn descriptor(number: i32) -> OwnedFd {
    // SAFETY: see above. Ownership is transferred exactly once, to the returned value.
    unsafe { OwnedFd::from_raw_fd(number) }
}

#[cfg(test)]
mod tests {
    use std::os::fd::IntoRawFd;
    use std::os::unix::net::UnixStream;

    use super::{Inherited, stream};

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn a_process_without_the_variable_has_no_inherited_connection() {
        assert!(matches!(stream("GNOBLIN_IPC_FD", environment(&[])), Ok(None)));
    }

    #[test]
    fn a_value_that_is_not_a_number_is_refused() {
        let error = stream("GNOBLIN_IPC_FD", environment(&[("GNOBLIN_IPC_FD", "/tmp/ipc.sock")])).expect_err("refused");
        assert!(
            matches!(&error, Inherited::NotANumber { value, .. } if value == "/tmp/ipc.sock"),
            "{error}"
        );
        assert!(error.to_string().contains("GNOBLIN_IPC_FD"), "{error}");
    }

    #[test]
    fn a_descriptor_that_is_not_a_socket_is_refused() {
        // The failure this prevents: the variable names a descriptor the process owns
        // for something else — a pipe, a file, its own stdout — and a role reads frames
        // from it or, worse, closes it when it ends.
        let (read, write) = std::io::pipe().expect("a pipe");
        // The number is given up, not borrowed: the adoption takes the descriptor and
        // closes it on drop, and a second owner would close it twice.
        let number = read.into_raw_fd();
        let error = stream(
            "GNOBLIN_IPC_FD",
            environment(&[("GNOBLIN_IPC_FD", &number.to_string())]),
        )
        .expect_err("refused");
        assert!(
            matches!(&error, Inherited::NotASocket { number: named, .. } if *named == number),
            "{error}"
        );
        // The adopted descriptor closes the read end here; the write end stays ours.
        drop(write);
    }

    #[test]
    fn a_connected_socket_is_adopted_and_usable() {
        let (near, mut far) = UnixStream::pair().expect("a socket pair");
        // The number is handed over the way the guardian hands it over: the process
        // that made the pair keeps no copy, so the adoption is the only owner.
        let number = near.into_raw_fd();
        let mut adopted = stream(
            "GNOBLIN_IPC_FD",
            environment(&[("GNOBLIN_IPC_FD", &number.to_string())]),
        )
        .expect("a number")
        .expect("a connection");
        // The adopted descriptor is the same description as `near`, so the far end
        // answers: this is what proves the adoption, not a copy of the number.
        std::io::Write::write_all(&mut adopted, b"hello").expect("a write");
        let mut buffer = [0u8; 5];
        std::io::Read::read_exact(&mut far, &mut buffer).expect("the far end reads it");
        assert_eq!(&buffer, b"hello");
    }
}
