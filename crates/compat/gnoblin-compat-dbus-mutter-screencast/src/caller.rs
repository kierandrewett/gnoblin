//! Who is calling, as far as a session bus can tell.
//!
//! A bus name proves nothing about a caller: any same-user process can own one. The
//! sandbox a client runs in is the one property the session bus can answer through the
//! caller's process, so the adapter asks the bus for the sender's pid and reads that
//! process's cgroup. A Flatpak application runs in a cgroup whose name carries
//! `flatpak`, which is the same fact `flatpak ps` reports.
//!
//! `wp_security_context_v1` is a Wayland concept and has no D-Bus spelling; a client
//! behind a security context reaches this adapter as an ordinary process. The portal
//! path is what covers that client, and the documentation says so rather than this
//! module guessing from a name.
//!
//! # Failure modes first
//!
//! 1. **A privilege question is answered from a guess.** When the bus cannot name the
//!    caller's process, [`CallerContext::probe`] returns an error and the service
//!    refuses a capture or injection call: `an_unidentifiable_caller_cannot_capture`.
//! 2. **A read fails because the caller is unknown.** A read still answers: the probe's
//!    failure is not turned into a capture authorisation, and `Version` is not a
//!    privilege.

use gnoblin_compat_core::CallerContext;
use zbus::Connection;
use zbus::fdo;
use zbus::names::BusName;

/// The caller's sandbox, as the bus and `/proc` report it.
///
/// # Errors
///
/// A sentence naming the failed lookup. It is the service's to interpret: a privileged
/// call is refused, a read falls back to [`CallerContext::UNSANDBOXED`] because the
/// question it needed answered was not a privilege one.
pub async fn probe(connection: &Connection, owner: &str) -> Result<CallerContext, String> {
    let bus_name = BusName::try_from(owner).map_err(|error| format!("`{owner}` is not a bus name: {error}"))?;
    let bus = fdo::DBusProxy::new(connection)
        .await
        .map_err(|error| format!("the bus proxy is unavailable: {error}"))?;
    let pid = bus
        .get_connection_unix_process_id(bus_name)
        .await
        .map_err(|error| format!("the bus could not name the caller's process: {error}"))?;
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map_err(|error| format!("the caller's cgroup is unreadable: {error}"))?;
    Ok(sandbox_from_cgroup(&cgroup))
}

/// The sandbox a cgroup listing names.
///
/// Pulled out as a pure function so the classification is testable without a bus or a
/// special process. The match is case-insensitive because cgroup paths are not
/// required to keep Flatpak's spelling.
fn sandbox_from_cgroup(cgroup: &str) -> CallerContext {
    if cgroup.to_ascii_lowercase().contains("flatpak") {
        CallerContext::flatpak()
    } else {
        CallerContext::UNSANDBOXED
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_compat_core::Sandbox;

    #[test]
    fn a_flatpak_cgroup_is_recognised() {
        // The failure mode: a sandboxed caller is treated as an ordinary same-user
        // process because the marker is spelled in another case.
        let context = sandbox_from_cgroup("0::/user.slice/user-1000.slice/app-flatpak-org.gnome.Calculator-1.scope\n");
        assert_eq!(context.sandbox(), Sandbox::Flatpak);
    }

    #[test]
    fn an_ordinary_session_is_not_flatpak() {
        let context = sandbox_from_cgroup("0::/user.slice/user-1000.slice/session-2.scope\n");
        assert_eq!(context.sandbox(), Sandbox::None);
    }
}
