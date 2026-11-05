//! The words that cross the guest ABI, generated from the schema.
//!
//! A module learns about the graph through one callback, `on-event`, whose
//! arguments are words rather than types: an event kind, an object identity, a
//! member index and an argument. The member index is this table's, and the
//! constants beside it are the dictionary a module author writes code against
//! (`WINDOW_TITLE` is the index of `window.title`).
//!
//! Two things are deliberate here:
//!
//! - The table is generated, so an index never has to be kept in step by hand.
//!   `tests/generated_wit.rs` compares it with the WIT package and with the schema
//!   member by member.
//! - Properties and signals share one index space. An event's kind already says
//!   which of the two it names, so one space keeps the callback to a single word
//!   and lets a module switch on one number.

/// What a member index names in the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    /// A property, whose change arrives as a `changed` event.
    Property,
    /// A signal, which arrives as a `signalled` event.
    Signal,
}

/// One member an event can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventMember {
    /// The object type that declares it, as the schema names it.
    pub object: &'static str,
    /// The member's schema name.
    pub name: &'static str,
    /// The word an event carries for it.
    pub index: u32,
    /// Whether it is a property or a signal.
    pub kind: MemberKind,
}

include!(concat!(env!("OUT_DIR"), "/members.rs"));

/// The event kind words `on-event` carries.
///
/// Kind `0` to `3` are the object model's four observations; `4` is a method's
/// completion, which carries the operation identity in the callback's argument
/// because an operation handle only exists for as long as the module holds it.
pub mod kind {
    /// An object came into existence with its properties.
    pub const OPENED: u32 = 0;
    /// One property changed.
    pub const CHANGED: u32 = 1;
    /// The compositor raised a signal on the object.
    pub const SIGNALLED: u32 = 2;
    /// The object's life ended.
    pub const CLOSED: u32 = 3;
    /// A method the module called completed once.
    pub const OPERATION_FINISHED: u32 = 4;
}

/// The index of `member` on `object`, or `None` when the schema declares no such
/// member.
///
/// A member of the wrong kind (a method, or a signal asked for as a property) has
/// no index: the table holds only what an event can name.
pub fn member_index(object: &str, member: &str) -> Option<u32> {
    MEMBERS
        .iter()
        .find(|row| row.object == object && row.name == member)
        .map(|row| row.index)
}

#[cfg(test)]
mod tests {
    use super::member_index;

    #[test]
    fn indexes_the_members_events_name() {
        assert!(member_index("window", "title").is_some());
        assert!(
            member_index("window", "focus").is_none(),
            "a method is not an event member"
        );
        assert!(member_index("window", "no_such_property").is_none());
    }
}
