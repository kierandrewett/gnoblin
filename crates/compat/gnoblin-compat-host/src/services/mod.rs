//! The services role's wiring: the compat agents the role serves on its connection, and the
//! GeoClue agent and prompter it always or optionally links.
//!
//! Each compat agent lives in its own module; this module decides which of them the role starts for
//! a given mode. An `off` mode serves no compat adapter, so the role links only the agents that are
//! not compat adapters.

use gnoblin_compat_dbus_gnome_keyring_prompter::agent as gcr;
use gnoblin_compat_dbus_gnome_shell_introspect::agent as introspect;
use gnoblin_compat_dbus_mutter_displayconfig::agent as displayconfig;
use gnoblin_compat_dbus_mutter_idlemonitor::agent as idlemonitor;
use gnoblin_compat_file_gtk_dconf_settings::agent as dconf;

use gnoblin_compat_core::Mode;
use gnoblin_services::link::LinkedAgent;

/// The agents the services role links in for `mode`: the prompt adapter when `prompts` is on, the
/// GeoClue agent, which takes the `location` table of the configuration as its policy, and the compat
/// adapters (dconf layer, DisplayConfig, IdleMonitor and introspection) when the mode is on. An `off`
/// mode serves no compat adapter; the GeoClue agent is not a compat adapter and always links.
#[must_use]
pub fn linked_agents(mode: Mode, prompts: bool) -> Vec<Box<dyn LinkedAgent>> {
    let location = gnoblin_services::location_agent();
    let mut agents: Vec<Box<dyn LinkedAgent>> = Vec::new();
    if prompts {
        agents.push(Box::new(gcr::GcrPrompter::new()));
    }
    agents.push(location);
    if mode != Mode::Off {
        agents.push(Box::new(dconf::DconfLayer::new(mode)));
        agents.push(Box::new(displayconfig::MutterDisplayConfig::new(mode)));
        agents.push(Box::new(idlemonitor::MutterIdleMonitor::new(mode)));
        agents.push(Box::new(introspect::Introspection::new(mode)));
    }
    agents
}

/// The agents the services role links in for the environment's settings: the prompt switch and
/// the compat mode, both read here so the binary names no adapter.
#[must_use]
pub fn linked_agents_from_environment() -> Vec<Box<dyn LinkedAgent>> {
    let prompts = gnoblin_services::prompts::enabled_from_environment();
    linked_agents(
        gnoblin_compat_dbus_gnome_shell_introspect::mode_from_environment(),
        prompts,
    )
}

#[cfg(test)]
mod tests {
    use super::linked_agents;
    use gnoblin_compat_core::Mode;

    #[test]
    fn the_services_role_is_given_the_location_agent() {
        // The role claims the location.agent slot as the services provider, so the GeoClue agent
        // has to be in the list it starts, next to the prompt adapter and the compat adapters
        // that are on in the default mode.
        assert_eq!(linked_agents(Mode::default(), true).len(), 6);
    }

    #[test]
    fn the_prompter_is_linked_only_when_prompts_are_enabled() {
        // `prompts.enabled` is false by default, and the names stay unowned while it is: the
        // services role links the GeoClue agent and the compat adapters, but not the prompter.
        assert_eq!(linked_agents(Mode::default(), false).len(), 5);
        assert_eq!(linked_agents(Mode::default(), true).len(), 6);
    }

    #[test]
    fn the_compat_adapters_are_served_when_the_mode_is_on() {
        // Portal is the default and Unmediated is the other mode that serves reads: both add the
        // dconf layer, DisplayConfig, IdleMonitor and introspection to the prompter and GeoClue agent.
        assert_eq!(linked_agents(Mode::Portal, true).len(), 6);
        assert_eq!(linked_agents(Mode::Unmediated, true).len(), 6);
    }

    #[test]
    fn the_compat_adapters_are_not_served_when_the_mode_is_off() {
        assert_eq!(linked_agents(Mode::Off, true).len(), 2);
    }
}
