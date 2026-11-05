//! Output descriptors, a declarative layout engine, and transactional output configuration.
//!
//! This crate is the decision half of `gnoblin-output` (design/rust-compositor.md,
//! "Backends and outputs"). It holds no DRM, KMS, Smithay, Wayland or udev type: an
//! output arrives as an [`OutputDescriptor`], a configuration is a [`LayoutConfig`],
//! and the result is a [`Layout`] of integer logical monitor rectangles. The backend
//! performs the mode sets, gamma updates and power changes that the effects name.
//!
//! Geometry, the compositing scale and graph identities are the shared primitives of
//! `gnoblin-types`, not copies kept here, so a rectangle that leaves this crate is
//! the same value the window manager and the renderer already know.
//!
//! # Two identities
//!
//! An output has two names and they serve different readers. The graph token
//! ([`gnoblin_types::OutputId`]) is what window management, accessibility and the
//! locker key their tables by: a cheap compare-and-hash value the backend allocates.
//! The connector name (`"DP-1"`) is what a person, the public output record and a
//! saved configuration use, and it is the only one that survives a reboot. A
//! configuration therefore names connectors ([`OutputConfig::connector`]) and a
//! [`Layout`] carries tokens ([`LogicalMonitor::members`]) plus the monitor's
//! connector name ([`LogicalMonitor::monitor`]); this crate owns the table between
//! them because it owns discovery.
//!
//! # Three pieces
//!
//! - [`compute`] turns a declarative configuration into a layout. Placement is
//!   relative (`right of DP-1`, centred), absolute, at the origin, or inherited from
//!   the output it mirrors. Logical sizes come from the mode, the orientation and
//!   the scale, always as whole pixels (see `gnoblin_types::Scale::apply`). A layout
//!   with two overlapping monitors, or with a monitor that is not edge-adjacent to
//!   the rest, is refused unless the configuration declares gaps.
//! - [`Transaction`] is the apply protocol `wlr-output-management` and the display
//!   configuration interface need: propose, let the caller test the hardware, commit,
//!   then confirm or roll back. A confirmation timeout measured in caller ticks
//!   restores the previous layout, which is also what a failed test and an explicit
//!   rollback do. A proposal that enables no output is refused.
//! - [`OutputModel`] is the hotplug state machine that owns the connected set, the
//!   declared configuration and the transaction, so a connect or disconnect during a
//!   pending test cannot leave the desktop on a layout that names a gone output.
//!
//! [`Edid::parse`] reads the identity, physical size and preferred mode out of an
//! EDID base block for the vendor names the output record carries. It rejects
//! everything it cannot check instead of panicking on it, because the bytes come
//! from a monitor that may be broken or lying.
//!
//! # Failure modes first
//!
//! The plan's component card and the transaction in "Phase 4: feature parity" give
//! this list, extended with the ways the layout engine and the parser can go wrong.
//! Each entry names the test that covers it in `tests/failure_modes.rs` unless it
//! says otherwise.
//!
//! 1. **Rollback restores a different layout.** The transaction keeps the applied
//!    layout it was proposed against, and every rollback path (failed test, timeout,
//!    explicit request) emits that layout, not the proposal and not a recomputation:
//!    `rollback_restores_the_previous_layout_exactly` and
//!    `an_explicit_rollback_after_commit_restores_the_previous_layout`.
//! 2. **A configuration enables no output.** [`LayoutError::NoEnabledOutputs`] is
//!    returned when no connected output is enabled, so the transaction, the model's
//!    `Configured` event and the model test all see a refusal instead of a black
//!    desktop: `a_configuration_that_enables_no_output_is_rejected`.
//! 3. **A scale change overlaps two outputs.** Absolute placement fixes a pixel
//!    origin, so growing a monitor's logical size under it overlaps its neighbour and
//!    the layout is refused; relative placement recomputes from the new size and
//!    stays flush: `a_scale_change_that_would_overlap_is_rejected` and
//!    `a_scale_change_with_relative_placement_stays_flush`.
//! 4. **A confirmation arrives after the timeout.** Expiry rolls back and ends the
//!    transaction, so the late confirmation is rejected rather than silently
//!    accepted: `confirming_after_the_timeout_rolls_back`.
//! 5. **An output is hotplugged during a transaction.** The pending test is
//!    abandoned and the layout is re-derived from what is still connected, so no
//!    monitor names a gone output: `hotplug_mid_transaction_aborts_the_test` and
//!    `disconnecting_the_anchor_rearranges_the_remaining_outputs`.
//! 6. **A malformed EDID panics.** Every read is bounds-checked and every division
//!    is guarded, over every truncation and every single-byte mutation of a valid
//!    block: `a_malformed_edid_never_panics`.
//! 7. **A mode set fails half way.** A failed test is indistinguishable from any
//!    other failed proposal here: the caller reports it and gets the previous layout
//!    back before anything was committed (`a_failed_test_rolls_back_before_commit`).
//!    The half-applied hardware state itself belongs to the backend, which must not
//!    return `Passed` until every output accepted its mode.
//! 8. **A configuration names an output twice, or an unreachable anchor.** Duplicate
//!    entries, placements whose anchor is not an enabled output, and placement cycles
//!    are refused by name instead of resolved arbitrarily:
//!    `a_duplicate_or_unreachable_configuration_is_rejected`.
//! 9. **A layout has an undeclared gap.** A gap is refused unless the configuration
//!    declares it, so a stale absolute position shows up as an error rather than a
//!    monitor the pointer cannot cross:
//!    `an_undeclared_gap_is_rejected_and_a_declared_gap_is_allowed`.
//! 10. **Mirroring a mirror, or a disabled source.** A clone chain has no defined
//!     size and a disabled source has no mode, so both are refused:
//!     `mirroring_two_outputs_produces_one_monitor` covers the working case and both
//!     refusals.
//!
//! # Determinism
//!
//! Nothing here reads a clock, a random source or the environment; the confirmation
//! timeout advances only by the ticks the caller supplies. Outputs keep connection
//! order and logical monitors are emitted in a fixed order, so the same event
//! sequence produces the same effect stream. `tests/model.rs` replays every random
//! sequence twice and compares the streams.

mod edid;
mod effect;
mod engine;
mod layout;
mod mode;
mod model;
mod output;
mod physical;
mod transaction;

pub use edid::{Edid, EdidError};
pub use effect::{Effect, Rejection, RollbackReason};
pub use engine::{LayoutError, compute};
pub use layout::{Align, Edge, Layout, LayoutConfig, LogicalMonitor, OutputConfig, Placement};
pub use mode::Mode;
pub use model::{Broken, Event, ModelOptions, OutputModel};
pub use output::{OutputDescriptor, VrrSupport};
pub use physical::PhysicalSize;
pub use transaction::{Phase, Transaction, TransactionToken, Verdict};
