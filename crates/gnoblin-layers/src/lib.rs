//! Layers: named, owned records that say where a provider's surface or painter
//! sits in the desktop stack.
//!
//! # The model
//!
//! A *layer* is a record a provider registers with the compositor. It has a
//! unique `name`, a *band* (its place in the stack), a *scope* (the output or
//! the window it is attached to), an *input policy* (whether it takes pointer
//! and keyboard input) and a `secure` flag (whether its pixels may be captured).
//! The full contract is in `design/layers.md`; this crate is the pure model of
//! it and nothing more.
//!
//! The model has three parts, each in its own module:
//!
//! - [`band`]: the closed set of eight bands, from `background` at the bottom to
//!   `cursor` at the top. Configuration cannot add a band. `Ord` on [`LayerBand`]
//!   follows declaration order, so sorting by band is sorting bottom to top.
//! - [`record`]: the data a provider registers. A [`LayerRecord`] is plain data;
//!   it has no behaviour until a registry accepts it.
//! - [`registry`]: the owner of every registered record. It enforces that a name
//!   belongs to its first registrant, that built-in names are reserved, and that
//!   only the owner may remove a layer.
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
//! This phase adds the record types and the registry only. It does not order
//! layers, apply follow rules or enforce limits; those arrive in later phases.
//! Nothing in the compositor depends on this crate yet, so adding it changes no
//! behaviour.
//!
//! # Errors
//!
//! Every rule the registry enforces is a variant of [`LayerError`]. Callers match
//! on the variant rather than parsing text, so a protocol layer can map each
//! variant to a stable error code.

pub mod band;
pub mod error;
pub mod record;
pub mod registry;

pub use band::LayerBand;
pub use error::LayerError;
pub use record::{LayerContent, LayerInput, LayerProvider, LayerProviderKind, LayerRecord, LayerScope};
pub use registry::{LayerRegistry, is_reserved_name};
