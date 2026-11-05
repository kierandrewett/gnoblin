//! The slot registry: which provider owns each optional capability.
//!
//! `design/rust-compositor.md` ("Providers and slots") makes every optional
//! capability a named slot with one contract. The services role holds the default
//! provider for three of them and `prompt.secret` has none on purpose, so a prompt
//! that nobody can draw fails closed instead of waiting out its deadline.
//!
//! It lives beside [`RequestBroker`](crate::RequestBroker) because the two are the
//! same layer: a request is offered to a *slot*, and the registry is what says who
//! that is. Both ends of the IPC need the answer — the services role, which claims
//! its defaults and publishes them, and the compositor, which routes an offer to the
//! connection that provides the slot — and a second copy on either side would be a
//! second answer to "who draws this prompt".
//!
//! Two things live here and nothing else:
//!
//! - [`Slot`], the closed set of slot names from the plan's table, with the schema
//!   property each one is published as. The names are the schema's: adding a slot
//!   is a schema change and a property in `crates/gnoblin-schema/schema/session.toml`
//!   at the same time, which is what keeps the plan's table and the wire in step.
//! - [`Slots`], the provider state, the one claim rule and the fields a
//!   publication writes.
//!
//! # The claim rule
//!
//! A shell claims a slot and takes it over from the services role; the services
//! role never takes one back while a shell holds it. That is the plan's
//! "Taken over by" column, and it is the only ordering the table has. Two
//! different shells cannot hold one slot, because a second claim would make it
//! impossible to say which connection is drawing the prompt, and a claim is
//! refused with the holder's provider rather than silently won or lost.
//!
//! Releasing a claim restores the slot's default provider. The caller that owns the
//! claim is named in the release, so a connection cannot release a slot it does
//! not hold.
//!
//! # Failure modes first
//!
//! 1. **A second claimant takes a slot.** The first claim wins and the second is
//!    refused naming the holder: `a_second_claimant_is_refused`.
//! 2. **A request goes to a slot nobody provides.** [`Slots::provider`] is `None`
//!    and the caller refuses instead of offering, which is why the registry is the
//!    first thing a request consults:
//!    `no_provider_fails_closed_and_names_the_slot`.
//! 3. **A publication names a property the schema does not declare.** Every
//!    property name comes from [`Slot::property`], and the test compares the whole
//!    field set with the checked-in schema:
//!    `the_published_fields_are_the_schema_properties`.

use std::fmt;
use std::str::FromStr;

use gnoblin_types::OwnerId;

use crate::value::{Fields, Value};

/// How many slots the table has.
const COUNT: usize = 4;

/// One optional capability, named as the plan's slot table names it.
///
/// The declaration order is the publication order and the index order of
/// [`Slots`], so the registry needs no map and a publication is a fixed list.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Slot {
    /// The polkit agent: an authentication request answered as a `Request`,
    /// default provider the services role.
    AuthPolkit,
    /// A passphrase or confirmation prompt. No default provider: without a shell
    /// that claimed it, a prompt fails closed.
    PromptSecret,
    /// `org.freedesktop.GeoClue2.Agent`, default provider the services role.
    LocationAgent,
    /// Idle watches and the ScreenSaver names, default provider the services role.
    IdlePolicy,
}

impl Slot {
    /// Every slot, in publication order.
    pub const ALL: [Slot; COUNT] = [
        Slot::AuthPolkit,
        Slot::PromptSecret,
        Slot::LocationAgent,
        Slot::IdlePolicy,
    ];

    /// The slot name from the plan's table, which is also the configuration and
    /// log spelling.
    pub const fn name(self) -> &'static str {
        match self {
            Slot::AuthPolkit => "auth.polkit",
            Slot::PromptSecret => "prompt.secret",
            Slot::LocationAgent => "location.agent",
            Slot::IdlePolicy => "idle.policy",
        }
    }

    /// The `session` property this slot's provider is published as.
    ///
    /// A property name cannot contain a dot, so the slot name's dot becomes an
    /// underscore behind a `slot_` prefix. The mapping is total and const so a
    /// publication cannot invent a second spelling.
    pub const fn property(self) -> &'static str {
        match self {
            Slot::AuthPolkit => "slot_auth_polkit",
            Slot::PromptSecret => "slot_prompt_secret",
            Slot::LocationAgent => "slot_location_agent",
            Slot::IdlePolicy => "slot_idle_policy",
        }
    }

    /// The provider the plan's table gives this slot when nobody claimed it.
    pub const fn default_provider(self) -> Provider {
        match self {
            Slot::PromptSecret => Provider::None,
            Slot::AuthPolkit | Slot::LocationAgent | Slot::IdlePolicy => Provider::Services,
        }
    }

    /// The registry index of this slot.
    const fn index(self) -> usize {
        match self {
            Slot::AuthPolkit => 0,
            Slot::PromptSecret => 1,
            Slot::LocationAgent => 2,
            Slot::IdlePolicy => 3,
        }
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Why a slot name could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownSlot {
    /// What the caller wrote.
    pub name: String,
}

impl fmt::Display for UnknownSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` is not a slot; the slots are {}",
            self.name,
            Slot::ALL.map(Slot::name).join(", ")
        )
    }
}

impl std::error::Error for UnknownSlot {}

impl FromStr for Slot {
    type Err = UnknownSlot;

    fn from_str(name: &str) -> Result<Slot, UnknownSlot> {
        Slot::ALL
            .into_iter()
            .find(|slot| slot.name() == name)
            .ok_or_else(|| UnknownSlot { name: name.to_owned() })
    }
}

/// Who provides a slot.
///
/// `None` is a state, not an error: it is what the plan gives `prompt.secret` and
/// what a slot falls back to when a shell releases a claim on a slot whose default
/// is nobody.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Provider {
    /// Nobody provides the slot. A request for it fails closed.
    #[default]
    None,
    /// The services role's default provider.
    Services,
    /// A shell claimed the slot and draws the prompt itself.
    Shell,
}

impl Provider {
    /// The spelling published in the session property and read back from it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Provider::None => "none",
            Provider::Services => "services",
            Provider::Shell => "shell",
        }
    }

    /// Parse the published spelling.
    ///
    /// `None` is a value that is not one of the three, which the caller reports
    /// rather than guessing a provider for.
    pub fn parse(word: &str) -> Option<Provider> {
        match word {
            "none" => Some(Provider::None),
            "services" => Some(Provider::Services),
            "shell" => Some(Provider::Shell),
            _ => None,
        }
    }

    /// Whether a request for the slot can be offered at all.
    pub const fn is_present(self) -> bool {
        !matches!(self, Provider::None)
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a slot cannot be claimed or released.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClaimError {
    /// A shell already holds the slot, and a second claim would leave two
    /// connections drawing one prompt.
    Held {
        /// The slot that is held.
        slot: Slot,
        /// Who holds it, which is what the refused claimant is told.
        by: Provider,
    },
    /// The release named a connection that does not hold the slot.
    NotHolder {
        /// The slot the caller tried to release.
        slot: Slot,
    },
    /// The services role cannot take a slot back from a shell; only the claiming
    /// shell can release it.
    TakenOver {
        /// The slot a services claim was refused for.
        slot: Slot,
    },
}

impl fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClaimError::Held { slot, by } => {
                write!(formatter, "the `{slot}` slot is already claimed by the {by}")
            }
            ClaimError::NotHolder { slot } => {
                write!(formatter, "this connection does not hold the `{slot}` slot")
            }
            ClaimError::TakenOver { slot } => {
                write!(formatter, "a shell holds the `{slot}` slot; only it can release it")
            }
        }
    }
}

impl std::error::Error for ClaimError {}

/// The provider of every slot, and who claims the ones a shell took over.
///
/// Small enough to copy-by-value in a command and cheap enough that a request
/// consults it directly rather than caching an answer that could go stale.
#[derive(Clone, Debug)]
pub struct Slots {
    providers: [Provider; COUNT],
    claims: [Option<OwnerId>; COUNT],
}

impl Default for Slots {
    fn default() -> Slots {
        Slots::new()
    }
}

impl Slots {
    /// The registry at the plan's defaults, before any shell claimed anything.
    pub fn new() -> Slots {
        Slots {
            providers: Slot::ALL.map(Slot::default_provider),
            claims: [None; COUNT],
        }
    }

    /// Who provides the slot.
    pub fn provider(&self, slot: Slot) -> Provider {
        self.providers[slot.index()]
    }

    /// Who claimed the slot, when a shell holds it.
    pub fn claimant(&self, slot: Slot) -> Option<OwnerId> {
        self.claims[slot.index()]
    }

    /// Claim a slot for a shell.
    ///
    /// A shell takes the slot over from the services role, which is why the claim
    /// succeeds over `Services`; it fails when another shell already holds it,
    /// because two holders make "who is drawing this prompt" unanswerable. A
    /// services claim only fills a slot that nobody provides.
    ///
    /// Returns whether the provider changed, so the caller can cancel the requests
    /// the previous provider was holding.
    pub fn claim(&mut self, slot: Slot, provider: Provider, owner: Option<OwnerId>) -> Result<bool, ClaimError> {
        let current = self.provider(slot);
        match (provider, current) {
            (Provider::Shell, Provider::None | Provider::Services) => {
                self.providers[slot.index()] = Provider::Shell;
                self.claims[slot.index()] = owner;
                Ok(current != Provider::Shell)
            }
            (Provider::Shell, Provider::Shell) => Err(ClaimError::Held {
                slot,
                by: Provider::Shell,
            }),
            (Provider::Services, Provider::Shell) => Err(ClaimError::TakenOver { slot }),
            (Provider::Services, Provider::None | Provider::Services) => {
                self.providers[slot.index()] = Provider::Services;
                self.claims[slot.index()] = None;
                Ok(current != Provider::Services)
            }
            (Provider::None, _) => {
                self.providers[slot.index()] = Provider::None;
                self.claims[slot.index()] = None;
                Ok(current != Provider::None)
            }
        }
    }

    /// Release a shell's claim, restoring the slot's default provider.
    ///
    /// Returns whether the provider changed. A release from a connection that does
    /// not hold the slot is refused, so one shell cannot take a prompt away from
    /// another by releasing its claim.
    pub fn release(&mut self, slot: Slot, owner: OwnerId) -> Result<bool, ClaimError> {
        if self.provider(slot) != Provider::Shell || self.claims[slot.index()] != Some(owner) {
            return Err(ClaimError::NotHolder { slot });
        }
        let next = slot.default_provider();
        self.providers[slot.index()] = next;
        self.claims[slot.index()] = None;
        Ok(true)
    }

    /// The fields a publication of this registry writes.
    ///
    /// One property per slot, typed as the schema declares them, so the session
    /// object a client subscribes to carries exactly the state a shell, `gnoblinctl`
    /// and the recovery overlay read.
    pub fn fields(&self) -> Fields {
        Slot::ALL
            .into_iter()
            .map(|slot| {
                (
                    slot.property().to_owned(),
                    Value::Text(self.provider(slot).as_str().to_owned()),
                )
            })
            .collect()
    }
}
