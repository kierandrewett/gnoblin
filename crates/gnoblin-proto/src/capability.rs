//! What a connection may do beyond reading the object graph.
//!
//! Design/rust-compositor.md, "One IPC": the runtime and the services role connect
//! over an inherited descriptor "that carries extra capabilities: trusted input
//! contexts, input policy decisions, request handling and configuration commits". A
//! capability is therefore a property of the *connection*, not of the client: the
//! compositor decides it from which descriptor the connection arrived on, and a
//! client that connected to the ordinary socket cannot ask for one.
//!
//! # Deny by default
//!
//! [`Capabilities::NONE`] is what [`Handshake::hello`](crate::Handshake::hello) and
//! [`Handshake::accept`](crate::Handshake::accept) carry, so every connection starts
//! with nothing. Only the compositor widens a set, in the one frame it answers a
//! handshake with, and [`Capabilities::from_bits`] drops every bit this build does
//! not name: a newer compositor that grants a capability this build has never heard
//! of grants nothing here rather than something undefined.

use core::fmt;

/// One thing a connection is allowed to do on the IPC.
///
/// The names are the vocabulary design/rust-compositor.md's "One IPC" bullet uses,
/// and they are what a log line and the handshake's grant say, so a refused member
/// and a granted one name the same thing.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    /// Offer a request to the slot that owns it (`request.offer`).
    ///
    /// This is the services role: polkit, the keyring's unlock prompt and the location
    /// agent all ask a provider to decide something, and only the host that needs
    /// the decision may create it.
    OfferRequest,
    /// Commit a configuration document (`config.commit`).
    ///
    /// This is the runtime: it evaluates the user's files and is the only party that
    /// knows what a document that validated means.
    CommitConfig,
    /// Decide input policy for a device the session trusts.
    ///
    /// This is the runtime as well. No member serves it in this build; the name is
    /// here because a grant must be a closed set, and the set is the plan's.
    InputPolicy,
    /// Ask the built-in portal UI to decide a request (`portal.ui`).
    ///
    /// This is the portal relay's connection. The dialog it opens is the one
    /// thing on screen the user must trust, so only the relay may create one.
    PortalUi,
    /// Route keyboard input to the open portal dialog (`portal.key`).
    ///
    /// This is the input relay. While a dialog is open, keys go only to it.
    PortalInput,
    /// Register a layer above a window through the `layers` object (`layers.register`).
    ///
    /// This is a trusted built-in provider's connection. The layer it makes is drawn
    /// on screen above a window, so only a built-in may create one until extensions
    /// get their own Lua environments (design/layers.md, phase 10).
    RegisterLayer,
    /// Offer the internal `secret.derive` slot (`request.offer`) on behalf of an app.
    ///
    /// This is the portal relay's connection, and it may use this one slot only. The
    /// relay asks the services role for an app's secret after the user allowed it, so
    /// the relay needs no general `OfferRequest` and cannot ask a polkit or prompt
    /// question. The compositor still refuses the offer unless it granted the app.
    RetrieveSecret,
    /// Read the configuration (`config get`, CONFIG_CURRENT).
    ///
    /// Granted to the session owner's ordinary socket connection: reading the
    /// document the runtime already evaluates changes nothing, so the user's CLI may
    /// do it without the runtime's commit power.
    ReadConfig,
    /// Ask the runtime to reload the configuration (`config reload`, CONFIG_RELOAD).
    ///
    /// Granted to the session owner's ordinary socket connection. The runtime still
    /// does the reload: a connection with this capability commits nothing itself, and
    /// the reload is refused unless the runtime is connected.
    ReloadConfig,
    /// Change a window's state: close it, minimise or restore it, maximise it, or make it
    /// fullscreen (`window.close`, `window.set_minimized`, `window.set_maximized`,
    /// `window.set_fullscreen`).
    ///
    /// Granted to the session owner's ordinary socket connection and to the runtime, the
    /// two parties that shape windows on the user's behalf. Any other connection is refused
    /// `denied` before the graph sees the call.
    WindowControl,
    /// Take a screenshot of an output (`capture.screenshot`).
    ///
    /// This is the portal relay's connection, and the request returns the pixels of
    /// what is on screen, so only the relay may ask for them. The compositor still
    /// refuses while the session is locked, and masks secure windows.
    CaptureScreen,
}

impl Capability {
    /// Every capability, in the order the plan lists them.
    pub const ALL: [Capability; 11] = [
        Capability::OfferRequest,
        Capability::CommitConfig,
        Capability::InputPolicy,
        Capability::PortalUi,
        Capability::PortalInput,
        Capability::RegisterLayer,
        Capability::RetrieveSecret,
        Capability::ReadConfig,
        Capability::ReloadConfig,
        Capability::WindowControl,
        Capability::CaptureScreen,
    ];

    /// The name a log line, the handshake and a refusal use.
    pub const fn name(self) -> &'static str {
        match self {
            Capability::OfferRequest => "offer-request",
            Capability::CommitConfig => "commit-config",
            Capability::InputPolicy => "input-policy",
            Capability::PortalUi => "portal-ui",
            Capability::PortalInput => "portal-input",
            Capability::RegisterLayer => "register-layer",
            Capability::RetrieveSecret => "retrieve-secret",
            Capability::ReadConfig => "read-config",
            Capability::ReloadConfig => "reload-config",
            Capability::WindowControl => "window-control",
            Capability::CaptureScreen => "capture-screen",
        }
    }

    /// The bit this capability occupies in a grant.
    pub const fn bit(self) -> u16 {
        match self {
            Capability::OfferRequest => 1 << 0,
            Capability::CommitConfig => 1 << 1,
            Capability::InputPolicy => 1 << 2,
            Capability::PortalUi => 1 << 3,
            Capability::PortalInput => 1 << 4,
            Capability::RegisterLayer => 1 << 5,
            Capability::RetrieveSecret => 1 << 6,
            Capability::ReadConfig => 1 << 7,
            Capability::ReloadConfig => 1 << 8,
            Capability::WindowControl => 1 << 9,
            Capability::CaptureScreen => 1 << 10,
        }
    }

    /// The capability a name stands for, if this build has it.
    pub fn from_name(name: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|capability| capability.name() == name)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The set of capabilities one connection holds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities(u16);

impl Capabilities {
    /// No capability: what a connection on the ordinary socket has, always.
    pub const NONE: Capabilities = Capabilities(0);

    /// The set of the capabilities named.
    pub const fn of(capabilities: &[Capability]) -> Capabilities {
        let mut bits = 0u16;
        let mut index = 0;
        while index < capabilities.len() {
            bits |= capabilities[index].bit();
            index += 1;
        }
        Capabilities(bits)
    }

    /// Whether `capability` is granted.
    pub const fn contains(self, capability: Capability) -> bool {
        self.0 & capability.bit() != 0
    }

    /// Whether the set grants nothing at all.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The set as the two bytes the handshake carries, low byte first.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The set a handshake byte names, with every bit this build does not know
    /// dropped.
    ///
    /// Dropping rather than rejecting is the deny-by-default rule: a compositor that
    /// grants a capability a later Gnoblin added still serves this build, and this
    /// build simply does not hold the capability it cannot name.
    pub const fn from_bits(bits: u16) -> Capabilities {
        Capabilities(bits & Capabilities::of(&Capability::ALL).bits())
    }

    /// The names of the granted capabilities, in [`Capability::ALL`] order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Capability::ALL
            .into_iter()
            .filter(move |capability| self.contains(*capability))
            .map(Capability::name)
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return formatter.write_str("none");
        }
        let names: Vec<&str> = self.names().collect();
        formatter.write_str(&names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::{Capabilities, Capability};

    #[test]
    fn every_capability_round_trips_through_its_name_and_its_bit() {
        for capability in Capability::ALL {
            assert_eq!(Capability::from_name(capability.name()), Some(capability));
            let set = Capabilities::of(&[capability]);
            assert!(set.contains(capability));
            assert_eq!(Capabilities::from_bits(set.bits()), set);
        }
        assert_eq!(Capability::from_name("root"), None);
    }

    #[test]
    fn an_unknown_bit_is_not_a_capability() {
        // The failure this prevents: a newer compositor grants something this build
        // has no name for, and the connection ends up holding a bit that a later
        // `contains` could match by accident.
        let granted = Capabilities::from_bits(0xffff);
        assert_eq!(granted, Capabilities::of(&Capability::ALL));
        for capability in Capability::ALL {
            assert!(granted.contains(capability), "the known ones survive");
        }
        assert_eq!(granted.names().count(), Capability::ALL.len());
    }

    #[test]
    fn reading_and_reloading_the_configuration_are_not_committing_it() {
        // The failure this prevents: the user's CLI is granted the runtime's commit
        // power because it needs to read or reload, so a grant meant for reading lets a
        // same-owner shell rewrite the document the runtime evaluates.
        assert_eq!(Capability::ReadConfig.name(), "read-config");
        assert_eq!(Capability::ReloadConfig.name(), "reload-config");
        let owner = Capabilities::of(&[Capability::ReadConfig, Capability::ReloadConfig]);
        assert!(owner.contains(Capability::ReadConfig));
        assert!(owner.contains(Capability::ReloadConfig));
        assert!(!owner.contains(Capability::CommitConfig), "{owner}");
        assert_eq!(owner.to_string(), "read-config, reload-config");
        assert_eq!(Capability::from_name("read-config"), Some(Capability::ReadConfig));
        assert_eq!(Capability::from_name("reload-config"), Some(Capability::ReloadConfig));
    }

    #[test]
    fn capturing_the_screen_is_its_own_capability() {
        // The failure this prevents: a portal's screenshot request rides on a bit that
        // another grant also carries, so a connection that may move windows or answer
        // consent could read the pixels of every output.
        assert_eq!(Capability::CaptureScreen.name(), "capture-screen");
        assert_eq!(Capability::CaptureScreen.bit(), 1 << 10);
        assert_eq!(Capability::from_name("capture-screen"), Some(Capability::CaptureScreen));
        assert!(Capability::ALL.contains(&Capability::CaptureScreen));
        let only = Capabilities::of(&[Capability::CaptureScreen]);
        assert_eq!(only.to_string(), "capture-screen");
        assert!(!only.contains(Capability::PortalUi));
        assert!(!only.contains(Capability::WindowControl));
    }

    #[test]
    fn the_empty_set_names_itself_none_and_grants_nothing() {
        assert_eq!(Capabilities::NONE.to_string(), "none");
        assert!(Capabilities::NONE.is_empty());
        for capability in Capability::ALL {
            assert!(!Capabilities::NONE.contains(capability));
        }
        assert_eq!(Capabilities::default(), Capabilities::NONE);
    }

    #[test]
    fn a_set_names_every_capability_it_holds() {
        let set = Capabilities::of(&[Capability::CommitConfig, Capability::InputPolicy]);
        assert_eq!(set.to_string(), "commit-config, input-policy");
        assert!(!set.contains(Capability::OfferRequest));
        assert_eq!(set.bits(), 0b110);
    }
}
