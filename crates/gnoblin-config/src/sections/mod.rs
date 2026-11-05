//! The typed configuration sections.
//!
//! One module per part of the reference: what the compositor does
//! ([`general`]), how windows and workspaces behave ([`window`]), pointer,
//! keyboard and tablet settings ([`input`]) and the protocol and input-source
//! lists ([`protocols`]).
//!
//! A section parser reads a table it already knows is a table, records problems
//! in a [`Parser`](crate::fields::Parser) and returns the typed values it could
//! read. It never fails early: the caller decides whether one problem rejects the
//! document or only the section.

pub(crate) mod general;
pub(crate) mod input;
pub(crate) mod protocols;
pub(crate) mod window;

/// A setting that accepts a value or the word `"inherit"`.
///
/// `"inherit"` is not a value the compositor applies: it clears an override so
/// the system preference shows through again. Keeping it in the type stops a
/// caller from treating "not set" and "clear the override" as the same thing,
/// which is the difference a reload of layered files makes visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inherit<T> {
    /// An explicit value.
    Value(T),
    /// Clear the override and follow the system setting.
    Inherit,
}

impl<T> Inherit<T> {
    /// The explicit value, or `None` when this clears the override.
    pub fn value(&self) -> Option<&T> {
        match self {
            Inherit::Value(value) => Some(value),
            Inherit::Inherit => None,
        }
    }
}
