//! Where the process model's inherited descriptors are named.
//!
//! Design/rust-compositor.md, "Process model": the runtime and the services role
//! connect to the compositor "over an inherited file descriptor that carries extra
//! capabilities". The guardian is what creates the descriptor (it is the parent of
//! both processes), and a descriptor can only cross an `exec` as a number in the
//! environment, so the name of that number is part of the IPC's contract rather than
//! of one process.
//!
//! Two variables, and both are built from the constants here so the spelling exists
//! once:
//!
//! - [`FD_VARIABLE`] carries the descriptor a privileged child is given. The child
//!   reads it, adopts the connection, and the handshake tells it what the compositor
//!   granted ([`crate::Capability`]).
//! - [`OWNER_PREFIX`] plus a role's name carries the *other* end of a pair into the
//!   compositor, one variable per privileged child, because the compositor must know
//!   which descriptor belongs to which role before it can grant anything: a grant is
//!   a consequence of which child the descriptor was made for.
//!
//! A descriptor in the environment is only accessible when `FD_CLOEXEC` is clear, so
//! the guardian clears it for exactly the descriptor a child is given and for exactly
//! the one spawn it is given in. Nothing else in the environment is privileged: a
//! client that connects to the socket by path holds
//! [`Capabilities::NONE`](crate::Capabilities::NONE) however it was started.

/// The variable that carries this process's privileged IPC descriptor.
pub const FD_VARIABLE: &str = "GNOBLIN_IPC_FD";

/// The prefix of the variables that carry a role's end of a pair into the compositor.
pub const OWNER_PREFIX: &str = "GNOBLIN_IPC_FD_";

/// The variable that carries `role`'s end of a pair into the compositor.
///
/// The role name is upper-cased because the environment's vocabulary is upper case;
/// [`owner_role`] is the exact inverse, so a compositor reads back the role the
/// guardian wrote.
pub fn owner_variable(role: &str) -> String {
    format!("{OWNER_PREFIX}{}", role.to_uppercase())
}

/// The role a variable name belongs to, if it is one of the owner variables.
pub fn owner_role(variable: &str) -> Option<String> {
    variable
        .strip_prefix(OWNER_PREFIX)
        .filter(|role| !role.is_empty())
        .map(str::to_lowercase)
}

/// The descriptor number a variable's value names.
///
/// `None` for a value that is not a plain non-negative integer, which is what a
/// caller reports rather than guessing a descriptor: adopting the wrong number would
/// take the connection away from the process that owns it.
pub fn descriptor(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok().filter(|number| *number >= 0)
}

#[cfg(test)]
mod tests {
    use super::{FD_VARIABLE, OWNER_PREFIX, descriptor, owner_role, owner_variable};

    #[test]
    fn a_role_round_trips_through_its_variable_name() {
        assert_eq!(owner_variable("runtime"), "GNOBLIN_IPC_FD_RUNTIME");
        assert_eq!(owner_role("GNOBLIN_IPC_FD_RUNTIME"), Some("runtime".to_owned()));
        assert_eq!(owner_role("GNOBLIN_IPC_FD_SERVICES"), Some("services".to_owned()));
        // The child's own variable is not an owner variable: the two are different
        // ends of the pair and a scanner must not confuse them.
        assert_eq!(owner_role(FD_VARIABLE), None);
        assert_eq!(owner_role(OWNER_PREFIX), None);
        assert_eq!(owner_role("GNOBLIN_IPC_SOCKET"), None);
    }

    #[test]
    fn only_a_plain_descriptor_number_is_accepted() {
        assert_eq!(descriptor("7"), Some(7));
        assert_eq!(descriptor(" 12 "), Some(12));
        assert_eq!(descriptor("-3"), None);
        assert_eq!(descriptor(""), None);
        assert_eq!(descriptor("/tmp/socket"), None);
    }
}
