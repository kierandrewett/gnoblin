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
    /// This is the services role: polkit, the passphrase prompter and the location
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
}

impl Capability {
    /// Every capability, in the order the plan lists them.
    pub const ALL: [Capability; 3] = [
        Capability::OfferRequest,
        Capability::CommitConfig,
        Capability::InputPolicy,
    ];

    /// The name a log line, the handshake and a refusal use.
    pub const fn name(self) -> &'static str {
        match self {
            Capability::OfferRequest => "offer-request",
            Capability::CommitConfig => "commit-config",
            Capability::InputPolicy => "input-policy",
        }
    }

    /// The bit this capability occupies in a grant.
    pub const fn bit(self) -> u8 {
        match self {
            Capability::OfferRequest => 1 << 0,
            Capability::CommitConfig => 1 << 1,
            Capability::InputPolicy => 1 << 2,
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
pub struct Capabilities(u8);

impl Capabilities {
    /// No capability: what a connection on the ordinary socket has, always.
    pub const NONE: Capabilities = Capabilities(0);

    /// The set of the capabilities named.
    pub const fn of(capabilities: &[Capability]) -> Capabilities {
        let mut bits = 0u8;
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

    /// The set as the one byte the handshake carries.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The set a handshake byte names, with every bit this build does not know
    /// dropped.
    ///
    /// Dropping rather than rejecting is the deny-by-default rule: a compositor that
    /// grants a capability a later Gnoblin added still serves this build, and this
    /// build simply does not hold the capability it cannot name.
    pub const fn from_bits(bits: u8) -> Capabilities {
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
        let granted = Capabilities::from_bits(0xff);
        assert_eq!(granted, Capabilities::of(&Capability::ALL));
        for capability in Capability::ALL {
            assert!(granted.contains(capability), "the known ones survive");
        }
        assert_eq!(granted.names().count(), Capability::ALL.len());
        assert_eq!(Capabilities::from_bits(0x80), Capabilities::NONE);
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
