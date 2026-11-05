//! Identifiers for the children a plan declares.
//!
//! [`ChildId`] is the plan slot, not a process id: it survives a restart so an
//! effect still names the same child after one. It is a plain newtype with
//! `new` and `get` so that, when `gnoblin-types` owns every identity, it becomes
//! one of that crate's `u64` `identity!` types without touching a call site.

/// A planned child, stable for the life of the session.
///
/// The identifier is the plan's, not the operating system's: a restarted child
/// keeps its identifier and changes only its generation, so an effect that names
/// a child still names the same thing after a restart, and a late exit from the
/// old process can be recognised as stale.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChildId(u32);

impl ChildId {
    /// The child declared with this identifier.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The identifier as a plain number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
