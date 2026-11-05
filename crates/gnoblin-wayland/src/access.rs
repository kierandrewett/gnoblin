//! Per-client access to the privileged globals.
//!
//! The plan's safety table states the rule this module performs: "Access is decided
//! per client, not only per global. Sandboxed clients (`wp_security_context_v1`) do
//! not see screencopy, data-control, layer-shell or foreign-toplevel management
//! unless policy allows it" (`design/rust-compositor.md`, "Safety model").
//!
//! ## Why a roster rather than a flag
//!
//! A global in Wayland is advertised by the registry, once, to everybody, unless the
//! compositor answers the registry question per connection. Smithay's protocol states
//! that need this take a filter closure (`new_with_filter`), and the compositor's own
//! globals implement [`AccessPolicy`] through `GlobalDispatch2::can_view`, so the
//! decision has exactly one shape: given a connection and a global, may that
//! connection bind it.
//!
//! A connection's role is recorded when the connection is accepted, not inferred from
//! a request later, because every request arrives *after* the registry answer and
//! could therefore only be refused, never hidden. The two ways a connection earns a
//! role are the ones the plan names: it arrived on the privileged socket, or the
//! compositor marked it trusted. A connection the compositor accepted for an
//! application sandbox is marked sandboxed, and a sandboxed role outranks every other:
//! an application inside a sandbox never sees a privileged global, whatever else the
//! session knows about it.
//!
//! ## Failure modes first
//!
//! 1. **A privileged global leaks to a sandboxed client.** Sandboxing is a denial no
//!    other role can override: `a_sandboxed_client_sees_no_privileged_global` in
//!    `tests/access.rs`.
//! 2. **A global is advertised to a client that never earned it.** An unknown
//!    connection is unprivileged, which is the safe default rather than a privileged
//!    one: `an_unprivileged_client_sees_no_privileged_global` and the `decide` unit
//!    tests at the bottom of this module.
//! 3. **A disabled global is still visible.** Protocol gating is an earlier answer than
//!    the roster's: [`Global::is_advertised`] says whether the configuration kept the
//!    global at all, so a session with data-control turned off advertises it to
//!    nobody, privileged or not.
//! 4. **A trusted mark outlives the connection it was given to.** The record is kept
//!    alive by the connection's own client data ([`Registration`]), so it is dropped
//!    with the connection: a recycled identity cannot inherit a role.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_server::backend::ClientId as ClientKey;
use tracing::debug;

/// The privileged globals this crate serves.
///
/// Each name is one registry global whose clients act on other clients' surfaces,
/// their content or their keyboard input, which is what makes per-client access a
/// requirement rather than a nicety.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Global {
    /// `zwlr_layer_shell_v1`: a bar or a dock places itself over every application.
    LayerShell,
    /// `zwlr_foreign_toplevel_manager_v1`: a taskbar lists and controls every window.
    ForeignToplevelManagement,
    /// `ext_foreign_toplevel_list_v1`: the read-only half of the same list.
    ForeignToplevelList,
    /// `ext_data_control_manager_v1`: a clipboard manager reads every selection.
    DataControl,
    /// `zwlr_data_control_manager_v1`: the wlroots spelling of the same capability.
    WlrDataControl,
    /// `zwlr_screencopy_manager_v1`: a client reads every other client's pixels.
    Screencopy,
    /// `ext_image_copy_capture_manager_v1` and the two capture-source managers that name
    /// what it copies. They are one capability, so they are one global here.
    ImageCopyCapture,
}

impl Global {
    /// Whether this global needs a privileged connection to see it.
    ///
    /// The default answer is yes, which is the safe direction: a global a lane adds
    /// without thinking about it is hidden from applications rather than leaked to
    /// them. An unprivileged global — one every application legitimately needs, such
    /// as an idle-notify or an input-method manager — says so here and
    /// [`decide`] then admits every role that is not sandboxed.
    #[must_use]
    pub const fn is_privileged(self) -> bool {
        match self {
            Self::LayerShell
            | Self::ForeignToplevelManagement
            | Self::ForeignToplevelList
            | Self::DataControl
            | Self::WlrDataControl
            | Self::Screencopy
            | Self::ImageCopyCapture => true,
        }
    }

    /// The name a log line and a test use, which is the protocol's own.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LayerShell => "zwlr_layer_shell_v1",
            Self::ForeignToplevelManagement => "zwlr_foreign_toplevel_manager_v1",
            Self::ForeignToplevelList => "ext_foreign_toplevel_list_v1",
            Self::DataControl => "ext_data_control_manager_v1",
            Self::WlrDataControl => "zwlr_data_control_manager_v1",
            Self::Screencopy => "zwlr_screencopy_manager_v1",
            Self::ImageCopyCapture => "ext_image_copy_capture_manager_v1",
        }
    }

    /// Whether the session's configuration kept this global at all.
    ///
    /// The configuration is the whole session's answer, so a disabled global is
    /// advertised to nobody and this check comes before any per-client one.
    #[must_use]
    pub fn is_advertised(self, protocols: &gnoblin_config::Protocols) -> bool {
        let protocol = match self {
            Self::LayerShell => gnoblin_config::Protocol::WlrLayerShell,
            Self::ForeignToplevelManagement => gnoblin_config::Protocol::WlrForeignToplevelManagement,
            Self::ForeignToplevelList => gnoblin_config::Protocol::ExtForeignToplevelList,
            // The schema has one data-control name and it covers the family: a session
            // that turns data-control off means both spellings.
            Self::DataControl | Self::WlrDataControl => gnoblin_config::Protocol::ExtDataControl,
            Self::Screencopy => gnoblin_config::Protocol::WlrScreencopy,
            Self::ImageCopyCapture => gnoblin_config::Protocol::ExtImageCopyCapture,
        };
        protocols.toggles.get(&protocol).copied().unwrap_or(true)
    }
}

/// What a connection has earned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClientRole {
    /// Connected on the session's own socket with no other qualification.
    #[default]
    Unprivileged,
    /// Marked by the compositor as one of its own clients: the runtime, a service role
    /// or a shell it started.
    Trusted,
    /// Connected on the privileged socket, which is how the session's own processes
    /// reach the controls an application must not have.
    Privileged,
    /// Accepted for an application sandbox through `wp_security_context_v1`.
    Sandboxed,
}

impl ClientRole {
    /// The name a log line and a test use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unprivileged => "unprivileged",
            Self::Trusted => "trusted",
            Self::Privileged => "privileged",
            Self::Sandboxed => "sandboxed",
        }
    }
}

/// The decision for one connection and one global.
///
/// A pure function so the rule can be read and tested without a session: a sandbox
/// earns nothing, the two earned roles earn everything, and an unprivileged application
/// earns exactly the globals [`Global::is_privileged`] does not claim.
#[must_use]
pub const fn decide(role: ClientRole, global: Global) -> bool {
    match role {
        ClientRole::Sandboxed => false,
        ClientRole::Privileged | ClientRole::Trusted => true,
        ClientRole::Unprivileged => !global.is_privileged(),
    }
}

/// A decision procedure for which globals a connection may see.
///
/// The compositor's own globals answer through this, which is what lets a caller
/// replace the roster with a policy of its own without a second code path.
///
/// **A filter runs inside that client's own dispatch.** `GlobalDispatch2::can_view` is
/// called while the backend holds the client's state locked, so a policy that answers by
/// asking the backend about the client — `Client::get_credentials`, a resource lookup —
/// re-enters that lock and deadlocks the loop for every client on it. The roster is a
/// lock over a map for exactly this reason: a decision must be answerable from what the
/// compositor already recorded, never from a call back into the connection.
pub trait AccessPolicy: Send + Sync + 'static {
    /// Whether the connection `client` may bind `global`.
    fn allows(&self, client: &ClientKey, global: Global) -> bool;
}

/// What the roster holds for one connection.
#[derive(Clone, Copy, Debug)]
struct Membership {
    /// What the connection is.
    role: ClientRole,
    /// The connection's number in this session, handed out in accept order.
    number: u32,
}

/// The session's roster of connections and what each of them is.
///
/// Shared through an [`std::sync::Arc`] between the state, which records roles, and the
/// global filters, which are created before the state exists and are asked when a
/// client reads the registry.
#[derive(Debug, Default)]
pub struct Access {
    connections: Mutex<HashMap<ClientKey, Membership>>,
}

impl Access {
    /// An empty roster: every connection is unprivileged until it is recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records what a connection is and gives it a number.
    ///
    /// A sandboxed connection stays sandboxed: the compositor decides a connection's
    /// role when it accepts the connection, so the only thing a later mark can do is
    /// refine an unprivileged one.
    pub fn record(&self, client: &ClientKey, role: ClientRole) {
        let mut connections = self.connections();
        if connections
            .get(client)
            .is_some_and(|record| record.role == ClientRole::Sandboxed)
            && role != ClientRole::Sandboxed
        {
            debug!(
                client = ?client,
                refused = role.as_str(),
                "[access] a sandboxed connection keeps its sandbox"
            );
            return;
        }
        let number = connections.get(client).map_or_else(
            || {
                // Numbers start at one so an unknown connection's zero is not a
                // plausible number any real connection holds.
                u32::try_from(connections.len())
                    .unwrap_or(u32::MAX - 1)
                    .saturating_add(1)
            },
            |record| record.number,
        );
        debug!(client = ?client, role = role.as_str(), number, "[access] recorded");
        connections.insert(client.clone(), Membership { role, number });
    }

    /// Forgets a connection that disconnected.
    pub fn forget(&self, client: &ClientKey) {
        if self.connections().remove(client).is_some() {
            debug!(client = ?client, "[access] forgotten");
        }
    }

    /// What a connection is.
    #[must_use]
    pub fn role(&self, client: &ClientKey) -> ClientRole {
        self.connections()
            .get(client)
            .map_or(ClientRole::Unprivileged, |record| record.role)
    }

    /// The connection's session number, or zero when it is not recorded.
    ///
    /// Zero is never a real number, so a caller can tell "unknown connection" from
    /// "the first connection of the session".
    #[must_use]
    pub fn number(&self, client: &ClientKey) -> u32 {
        self.connections().get(client).map_or(0, |record| record.number)
    }

    /// Whether a connection may bind a global.
    #[must_use]
    pub fn allows(&self, client: &ClientKey, global: Global) -> bool {
        decide(self.role(client), global)
    }

    /// The roster, or an empty one if a panic poisoned it.
    ///
    /// A poisoned roster is a bug in this crate, not a reason to refuse every client
    /// for ever: the worst case of reading an empty roster is that a privileged client
    /// is treated as unprivileged, which hides a global rather than leaking one.
    fn connections(&self) -> std::sync::MutexGuard<'_, HashMap<ClientKey, Membership>> {
        self.connections.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl AccessPolicy for Access {
    fn allows(&self, client: &ClientKey, global: Global) -> bool {
        Self::allows(self, client, global)
    }
}

/// A connection's own claim on its roster entry.
///
/// Kept inside the client's data, which `wayland-server` drops when the connection
/// goes away: that is the one callback a client's disappearance reliably produces, so
/// this is what makes a role end exactly when the connection does.
#[derive(Debug)]
pub struct Registration {
    /// The roster that holds the entry.
    access: Arc<Access>,
    /// The connection's identity, recorded after the connection exists.
    client: Mutex<Option<ClientKey>>,
}

impl Registration {
    /// A registration for a connection that is about to be inserted.
    #[must_use]
    pub fn new(access: Arc<Access>) -> Self {
        Self {
            access,
            client: Mutex::new(None),
        }
    }

    /// Tells the registration which connection it belongs to.
    pub fn attach(&self, client: &ClientKey) {
        *self.client.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(client.clone());
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let guard = self.client.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(client) = guard.as_ref() {
            self.access.forget(client);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientRole, Global, decide};

    /// Every global this crate serves, so a new one cannot skip the decision tests.
    const ALL: &[Global] = &[
        Global::LayerShell,
        Global::ForeignToplevelManagement,
        Global::ForeignToplevelList,
        Global::DataControl,
        Global::WlrDataControl,
        Global::Screencopy,
        Global::ImageCopyCapture,
    ];

    #[test]
    fn a_sandboxed_client_is_refused_every_privileged_global() {
        for global in ALL {
            assert!(
                !decide(ClientRole::Sandboxed, *global),
                "sandboxing must deny {}",
                global.as_str()
            );
        }
    }

    #[test]
    fn an_unprivileged_client_sees_no_privileged_global() {
        for global in ALL {
            assert!(
                !decide(ClientRole::Unprivileged, *global),
                "an application must not see {}",
                global.as_str()
            );
        }
    }

    #[test]
    fn the_two_earned_roles_see_every_privileged_global() {
        for role in [ClientRole::Privileged, ClientRole::Trusted] {
            for global in ALL {
                assert!(decide(role, *global), "{} must see {}", role.as_str(), global.as_str());
            }
        }
    }

    #[test]
    fn an_unprivileged_global_is_admitted_to_applications_but_not_to_a_sandbox() {
        // A global a lane marks unprivileged must reach ordinary applications through
        // the same `decide` the privileged ones use, and must still be denied inside a
        // sandbox: this is the shape `idle_notify`, input methods and virtual
        // keyboards take.
        for global in ALL {
            if global.is_privileged() {
                assert!(!decide(ClientRole::Unprivileged, *global));
                continue;
            }
            assert!(
                decide(ClientRole::Unprivileged, *global),
                "an application needs {}",
                global.as_str()
            );
            assert!(
                !decide(ClientRole::Sandboxed, *global),
                "a sandbox is denied {}",
                global.as_str()
            );
        }
    }

    #[test]
    fn a_new_global_is_privileged_until_it_says_otherwise() {
        // The safe default, stated as a test: nothing in the enum is unprivileged today,
        // and a lane that adds a global has to opt out explicitly.
        for global in ALL {
            assert!(
                global.is_privileged(),
                "{} must opt out of privilege explicitly",
                global.as_str()
            );
        }
    }

    #[test]
    fn the_default_role_is_the_one_that_grants_nothing() {
        assert_eq!(ClientRole::default(), ClientRole::Unprivileged);
    }

    #[test]
    fn the_data_control_family_shares_one_configuration_name() {
        // The schema has one data-control name; both spellings must answer to it, or a
        // session that disabled it would still advertise the other one.
        let mut protocols = gnoblin_config::Protocols::default();
        protocols
            .toggles
            .insert(gnoblin_config::Protocol::ExtDataControl, false);
        assert!(!Global::DataControl.is_advertised(&protocols));
        assert!(!Global::WlrDataControl.is_advertised(&protocols));
    }

    #[test]
    fn an_unset_configuration_keeps_every_global() {
        let protocols = gnoblin_config::Protocols::default();
        for global in ALL {
            assert!(global.is_advertised(&protocols), "{} defaults to on", global.as_str());
        }
    }

    #[test]
    fn a_disabled_protocol_is_not_advertised() {
        let mut protocols = gnoblin_config::Protocols::default();
        protocols.toggles.insert(gnoblin_config::Protocol::WlrLayerShell, false);
        assert!(!Global::LayerShell.is_advertised(&protocols));
        assert!(Global::ForeignToplevelList.is_advertised(&protocols));
    }
}
