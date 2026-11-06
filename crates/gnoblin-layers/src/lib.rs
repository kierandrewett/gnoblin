//! Layers: named, owned records that say where a provider's surface or painter
//! sits in the desktop stack.
//!
//! # The model
//!
//! A *layer* is a record a provider registers with the compositor. It has one
//! full name, a *band* (its coarse slot in the stack), a *scope* (the output or
//! window it is attached to), an *order* (its place inside the band), an *input*
//! policy and a `secure` flag (whether its pixels may be captured). The full
//! contract is in `design/layers.md`. This crate is the pure model of it and
//! nothing more.
//!
//! Each part is in its own module:
//!
//! - [`band`]: the closed set of eight bands, bottom to top. Declaration order is
//!   paint order.
//! - [`name`]: the names. A full name is `<provider>.<local>`. The registry builds
//!   the provider part, so a provider chooses only its local part.
//! - [`provider`]: who registers a layer, a Gnoblin built-in or an extension.
//! - [`scope`] and [`state`]: the enums a spec is built from, and the lifecycle.
//! - [`spec`]: the data a provider asks for, [`LayerSpec`].
//!
//! # Names
//!
//! Every layer's full name is `<provider>.<local>`. Gnoblin's built-ins share the
//! namespace `gnoblin`, and an extension `<id>` has the namespace `ext.<id>`. A
//! uniform prefix means two providers cannot collide by construction, so no
//! reserved-name list has to be kept in step with the built-ins. The origin of a
//! layer is also visible in `layers.list`, config keys (`layers.<full name>.secure`)
//! and logs.
//!
//! # Why a separate crate
//!
//! Stacking policy has to be testable without a compositor, a display or Smithay.
//! Keeping it in a foundation crate lets the registry's rules be checked on every
//! platform in milliseconds, and lets the compositor later call one function
//! instead of re-deriving the rules in each engine. This crate does no I/O and
//! links no mechanism crate. `xtask deps` enforces that.
//!
//! # Phase 1 scope
//!
//! This phase adds the enums, the names and the spec. Nothing in the compositor
//! depends on this crate yet, so adding it changes no behaviour.
//!
//! # Errors
//!
//! Every refusal is a typed error (see [`name::NameError`] for grammar failures).
//! Callers match on the variant rather than on text, so a protocol layer can map
//! each one to a stable error code.

pub mod band;
pub mod name;
pub mod provider;
pub mod scope;
pub mod spec;
pub mod state;

pub use band::Band;
pub use name::{LayerName, LocalName, NameError, ProviderId};
pub use provider::Provider;
pub use scope::{ContentKind, EffectsPolicy, Focus, InputPolicy, LockBehaviour, Order, Scope};
pub use spec::LayerSpec;
pub use state::{EndReason, LayerState};
