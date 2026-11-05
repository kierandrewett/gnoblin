//! The runtime role: what it refuses, and where the rest is covered.
//!
//! `gnoblin --role runtime` is the Lua host as its own process (design/rust-compositor.md, D1).
//! Its contract with the guardian is one inherited descriptor and one role word:
//!
//! 1. **The guardian did not pass a connection.** The role refuses at once and names the
//!    variable, so the guardian's log says what is wrong instead of the child hanging:
//!    `the_role_refuses_without_a_privileged_connection`.
//!
//! # Where the rest of the role is covered
//!
//! The path that matters — connect, subscribe, load and validate the configuration, commit it,
//! serve — is covered end to end against a real compositor by
//! `crates/gnoblin/tests/guardian.rs`: the guardian spawns this role from the built binary, the
//! compositor grants it `commit-config`, and the session logs `configuration revision 1
//! accepted`. A hand-written peer here was the wrong shape, because what such a test exercises
//! is the peer's copy of the protocol rather than the role.

use gnoblin_config::SearchPaths;
use gnoblin_lua::{Limits, Role};

#[test]
fn the_role_refuses_without_a_privileged_connection() {
    let role = Role {
        env: Box::new(|_| None),
        paths: SearchPaths {
            home: None,
            config_home: None,
            override_path: None,
        },
        limits: Limits::default(),
    };
    let message = role.run(|| true).expect_err("no connection").to_string();
    assert!(message.contains("GNOBLIN_IPC_FD"), "{message}");
}
