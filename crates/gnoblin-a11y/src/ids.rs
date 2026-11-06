//! The identifiers the machines key their tables by.
//!
//! [`OutputId`] and [`OwnerId`] are `gnoblin-types`' shared identities, and
//! [`KeyCode`] and [`Keysym`] are its shared keyboard values: those crate names
//! have no constants because they have no state, so all five live in
//! `gnoblin-types` and are re-exported here rather than declared again. A keycode
//! this crate matches against is therefore the same keycode `gnoblin-input`
//! produces, and a keysym a screen reader grabs is the same one the layout
//! resolved.
//!
//! Nothing here allocates an identifier or reads a clock: every keycode,
//! keysym, owner and output comes from the compositor, which is what makes
//! [`crate::A11y::apply`] replayable from an event log.

/// Output and owner identifiers plus key codes and keysyms, re-exported from `gnoblin-types`.
pub use gnoblin_types::{KeyCode, Keysym, OutputId, OwnerId};
