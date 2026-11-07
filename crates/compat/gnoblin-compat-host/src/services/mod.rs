//! The services role's wiring: the compat agents the role serves on its connection, and the
//! GeoClue agent and built-in keyring it links.
//!
//! Each compat agent lives in its own module; this module decides which of them the role starts for
//! a given mode. An `off` mode serves no compat adapter, so the role links only the agents that are
//! not compat adapters.

use gnoblin_compat_dbus_gnome_shell_introspect::agent as introspect;
use gnoblin_compat_dbus_mutter_displayconfig::agent as displayconfig;
use gnoblin_compat_dbus_mutter_idlemonitor::agent as idlemonitor;
use gnoblin_compat_gtk_dconf_settings::agent as dconf;

use gnoblin_compat_core::Mode;
use gnoblin_services::link::LinkedAgent;

/// The agents the services role links in for `mode`: the built-in Secret Service agent when
/// `keyring` is given, the GeoClue agent, which takes the `location` table of the configuration as
/// its policy, and the compat adapters (dconf layer, DisplayConfig, IdleMonitor and introspection)
/// when the mode is on. An `off` mode serves no compat adapter; the GeoClue agent is not a compat
/// adapter and always links.
#[must_use]
pub fn linked_agents(mode: Mode, keyring: Option<Box<dyn LinkedAgent>>) -> Vec<Box<dyn LinkedAgent>> {
    let location = gnoblin_services::location_agent();
    let mut agents: Vec<Box<dyn LinkedAgent>> = Vec::new();
    agents.extend(keyring);
    agents.push(location);
    if mode != Mode::Off {
        agents.push(Box::new(dconf::DconfLayer::new(mode)));
        agents.push(Box::new(displayconfig::MutterDisplayConfig::new(mode)));
        agents.push(Box::new(idlemonitor::MutterIdleMonitor::new(mode)));
        agents.push(Box::new(introspect::Introspection::new(mode)));
    }
    agents
}

/// The agents the services role links in for the environment's settings: the keyring switch and
/// the compat mode, all read here so the binary names no adapter.
#[must_use]
pub fn linked_agents_from_environment() -> Vec<Box<dyn LinkedAgent>> {
    linked_agents(
        gnoblin_compat_dbus_gnome_shell_introspect::mode_from_environment(),
        gnoblin_services::keyring_agent(),
    )
}

#[cfg(test)]
mod tests {
    use super::linked_agents;
    use gnoblin_compat_core::Mode;
    use gnoblin_services::keyring::KeyringAgent;
    use gnoblin_services::link::LinkedAgent;
    use std::path::PathBuf;

    fn keyring() -> Option<Box<dyn LinkedAgent>> {
        Some(Box::new(KeyringAgent::new(
            PathBuf::from("/nonexistent/keyring.db"),
            PathBuf::from("/nonexistent/keyring-unlock.sock"),
        )))
    }

    #[test]
    fn the_services_role_is_given_the_location_agent() {
        // The role claims the location.agent slot as the services provider, so the GeoClue agent
        // has to be in the list it starts, next to the compat adapters that are on in the default
        // mode.
        assert_eq!(linked_agents(Mode::default(), None).len(), 5);
    }

    #[test]
    fn the_secret_service_is_linked_only_when_the_built_in_keyring_is_enabled() {
        // `keyring.enabled` is true by default. When it is off the role links no Secret Service
        // agent at all, and the name stays with the other program.
        assert_eq!(linked_agents(Mode::default(), keyring()).len(), 6);
        assert_eq!(linked_agents(Mode::default(), None).len(), 5);
    }

    #[test]
    fn the_compat_adapters_are_served_when_the_mode_is_on() {
        // Portal is the default and Unmediated is the other mode that serves reads: both add the
        // dconf layer, DisplayConfig, IdleMonitor and introspection to the GeoClue agent.
        assert_eq!(linked_agents(Mode::Portal, None).len(), 5);
        assert_eq!(linked_agents(Mode::Unmediated, None).len(), 5);
    }

    #[test]
    fn the_compat_adapters_are_not_served_when_the_mode_is_off() {
        assert_eq!(linked_agents(Mode::Off, None).len(), 1);
    }
}
