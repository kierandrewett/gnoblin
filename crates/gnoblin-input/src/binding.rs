//! The binding table: which chord belongs to which declaration.
//!
//! The configuration declares keyboard and pointer bindings by name, and a later
//! declaration with the same name replaces the earlier one. Everything else must be
//! visible: two declarations that want the same chord are a [`Conflict`] the caller
//! reports, never a binding that silently shadows another and never a pair whose
//! winner depends on hash iteration order.
//!
//! Two rules make the table deterministic:
//!
//! - A chord has one identity, however it was written (see [`Chord`]), so alias
//!   spellings cannot become two rows.
//! - A chord belongs to one binding for both edges. A press-triggered and a
//!   release-triggered binding on the same chord would have to share one physical
//!   press, and a release trigger reserves its press outright, so the second
//!   declaration is a conflict rather than a second row.

use std::collections::BTreeMap;
use std::fmt;

use crate::chord::Chord;
use crate::name::Name;

/// Which edge of a chord runs a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trigger {
    /// The key or button press.
    Press,
    /// The key or button release, which reserves and consumes the press as well.
    Release,
}

impl Trigger {
    /// The name the configuration uses for this trigger.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Release => "release",
        }
    }
}

/// The identity a table gives a binding.
///
/// An id survives a reload that keeps the name, so a press owned by this binding
/// before the reload still names a binding afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingId(u32);

impl BindingId {
    /// The raw number, for logs and diagnostics.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for BindingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "binding:{}", self.0)
    }
}

/// One declared binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    id: BindingId,
    name: Name,
    chord: Chord,
    trigger: Trigger,
    allow_when_inhibited: bool,
}

impl Binding {
    /// The identity in the table.
    pub const fn id(&self) -> BindingId {
        self.id
    }

    /// The declared name.
    pub const fn name(&self) -> &Name {
        &self.name
    }

    /// The chord.
    pub const fn chord(&self) -> &Chord {
        &self.chord
    }

    /// The edge that runs it.
    pub const fn trigger(&self) -> Trigger {
        self.trigger
    }

    /// Whether the binding still runs while the focused client inhibits shortcuts.
    pub const fn allows_when_inhibited(&self) -> bool {
        self.allow_when_inhibited
    }

    /// Whether this binding runs on a press or a release edge.
    pub const fn fires_on(&self, edge: Trigger) -> bool {
        matches!(
            (self.trigger, edge),
            (Trigger::Press, Trigger::Press) | (Trigger::Release, Trigger::Release)
        )
    }
}

/// What an insertion did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insertion {
    /// A new name was added under this id.
    Added(BindingId),
    /// An existing name was replaced in place and kept this id.
    Replaced(BindingId),
}

impl Insertion {
    /// The id the name now has.
    pub const fn id(self) -> BindingId {
        match self {
            Self::Added(id) | Self::Replaced(id) => id,
        }
    }
}

/// A refused insertion: another binding already owns this chord.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The chord both declarations want.
    pub chord: Chord,
    /// The binding that already had it.
    pub existing: BindingId,
    /// The name of that binding.
    pub existing_name: Name,
    /// The name of the declaration that was refused.
    pub incoming_name: Name,
}

impl fmt::Display for Conflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "binding \"{}\" cannot use {}: \"{}\" ({}) already has it",
            self.incoming_name, self.chord, self.existing_name, self.existing
        )
    }
}

/// The stable reason a declaration was refused. A session reports it by name, so a log
/// or a control client never has to parse the human message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictCode {
    /// Another binding already owns the chord.
    DuplicateChord,
}

impl ConflictCode {
    /// The wire and log spelling of the code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateChord => "duplicate-chord",
        }
    }
}

impl Conflict {
    /// Why the declaration was refused.
    pub const fn code(&self) -> ConflictCode {
        ConflictCode::DuplicateChord
    }
}

impl std::error::Error for Conflict {}

/// One binding as a configuration declares it, the input to [`BindingTable::reconcile`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The declared name.
    pub name: Name,
    /// The chord the binding owns.
    pub chord: Chord,
    /// The edge that runs it.
    pub trigger: Trigger,
    /// Whether it still runs while the focused client inhibits shortcuts.
    pub allow_when_inhibited: bool,
}

/// The bindings a session has declared, keyed by chord and by name.
#[derive(Clone, Debug, Default)]
pub struct BindingTable {
    entries: BTreeMap<BindingId, Binding>,
    by_chord: BTreeMap<Chord, BindingId>,
    by_name: BTreeMap<String, BindingId>,
    next_id: u32,
}

impl BindingTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a binding, or replace the one with the same name.
    ///
    /// On conflict the table is left exactly as it was, so a failed reload keeps the
    /// working bindings instead of half-applying a new set.
    pub fn insert(
        &mut self,
        name: &Name,
        chord: Chord,
        trigger: Trigger,
        allow_when_inhibited: bool,
    ) -> Result<Insertion, Conflict> {
        let existing = self.by_name.get(name.as_str()).copied();
        // A chord already owned by a different binding is a conflict. Re-declaring the same
        // name with the same chord is not, because the owner is then that name's own row.
        if let Some(owner) = self.by_chord.get(&chord).copied()
            && Some(owner) != existing
        {
            let owner_binding = self.entries.get(&owner).expect("every chord has a binding");
            return Err(Conflict {
                chord,
                existing: owner,
                existing_name: owner_binding.name.clone(),
                incoming_name: name.clone(),
            });
        }
        match existing {
            Some(id) => {
                let replaced = self.entries.get_mut(&id).expect("a named binding exists");
                let previous_chord = replaced.chord.clone();
                replaced.chord = chord.clone();
                replaced.trigger = trigger;
                replaced.allow_when_inhibited = allow_when_inhibited;
                // Drop the old chord only if this binding still owns it: in a reload that
                // swaps two chords, the old chord may already belong to a sibling.
                if self.by_chord.get(&previous_chord) == Some(&id) {
                    self.by_chord.remove(&previous_chord);
                }
                self.by_chord.insert(chord, id);
                Ok(Insertion::Replaced(id))
            }
            None => {
                // Ids are never reused: `remove` does not free a number, so a press that
                // still names a removed binding cannot resolve to a later declaration.
                let id = BindingId(self.next_id);
                self.next_id += 1;
                self.entries.insert(
                    id,
                    Binding {
                        id,
                        name: name.clone(),
                        chord: chord.clone(),
                        trigger,
                        allow_when_inhibited,
                    },
                );
                self.by_chord.insert(chord, id);
                self.by_name.insert(name.as_str().to_owned(), id);
                Ok(Insertion::Added(id))
            }
        }
    }

    /// Remove a binding by id, freeing its chord.
    pub fn remove(&mut self, id: BindingId) -> Option<Binding> {
        let removed = self.entries.remove(&id)?;
        self.by_chord.remove(removed.chord());
        self.by_name.remove(removed.name().as_str());
        Some(removed)
    }

    /// Remove the binding declared under this name.
    pub fn remove_name(&mut self, name: &Name) -> Option<Binding> {
        let id = self.by_name.get(name.as_str()).copied()?;
        self.remove(id)
    }

    /// The table a configuration reload produces from the declared set.
    ///
    /// Names that are declared again keep their id, names that are no longer declared are
    /// removed, and every chord is freed before the set is inserted, so two names that swap
    /// chords do not conflict with each other. The result is all or nothing: a conflict
    /// returns the error and `self` is untouched, so a failed reload keeps the bindings
    /// that worked.
    pub fn reconcile(&self, declared: &[Declaration]) -> Result<Self, Conflict> {
        let mut next = self.clone();
        let wanted: std::collections::BTreeSet<&str> = declared.iter().map(|d| d.name.as_str()).collect();
        let stale: Vec<BindingId> = next
            .by_name
            .iter()
            .filter(|(name, _)| !wanted.contains(name.as_str()))
            .map(|(_, id)| *id)
            .collect();
        for id in stale {
            next.remove(id);
        }
        for name in &wanted {
            if let Some(id) = next.by_name.get(*name).copied() {
                let chord = next.entries[&id].chord.clone();
                next.by_chord.remove(&chord);
            }
        }
        for declaration in declared {
            next.insert(
                &declaration.name,
                declaration.chord.clone(),
                declaration.trigger,
                declaration.allow_when_inhibited,
            )?;
        }
        Ok(next)
    }

    /// The binding with this id.
    pub fn get(&self, id: BindingId) -> Option<&Binding> {
        self.entries.get(&id)
    }

    /// The binding that owns this chord.
    pub fn lookup(&self, chord: &Chord) -> Option<&Binding> {
        let id = self.by_chord.get(chord)?;
        self.entries.get(id)
    }

    /// The binding declared under this name.
    pub fn lookup_name(&self, name: &Name) -> Option<&Binding> {
        let id = self.by_name.get(name.as_str())?;
        self.entries.get(id)
    }

    /// Every binding in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &Binding> {
        self.entries.values()
    }

    /// How many bindings are declared.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is declared.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{BindingTable, Insertion, Trigger};
    use crate::chord::{Chord, ChordKind, Keysym, Keysyms, Symbol};
    use crate::name::Name;

    struct Table;

    impl Keysyms for Table {
        fn resolve(&self, name: &str) -> Option<Symbol> {
            let lowered = name.to_ascii_lowercase();
            let (symbol, canonical) = match lowered.as_str() {
                "q" => (0x71, "q"),
                "w" => (0x77, "w"),
                "e" => (0x65, "e"),
                _ => return None,
            };
            Some(Symbol::new(Keysym::new(symbol), canonical))
        }
    }

    fn chord(text: &str) -> Chord {
        Chord::parse(text, ChordKind::Key, &Table).expect(text)
    }

    fn name(text: &str) -> Name {
        Name::new(text).expect(text)
    }

    #[test]
    fn a_later_declaration_replaces_in_place() {
        let mut table = BindingTable::new();
        let added = table
            .insert(&name("a"), chord("<Super>q"), Trigger::Press, false)
            .expect("a");
        let replaced = table
            .insert(&name("a"), chord("<Super>w"), Trigger::Release, true)
            .expect("a");
        assert!(matches!(added, Insertion::Added(_)));
        assert_eq!(replaced, Insertion::Replaced(added.id()));
        assert_eq!(table.len(), 1);
        assert_eq!(table.lookup(&chord("<Super>q")), None);
        let binding = table.lookup(&chord("<Super>w")).expect("w");
        assert_eq!(binding.trigger(), Trigger::Release);
        assert!(binding.allows_when_inhibited());
    }

    #[test]
    fn a_chord_belongs_to_one_binding() {
        let mut table = BindingTable::new();
        table
            .insert(&name("first"), chord("<Super>q"), Trigger::Press, false)
            .expect("first");
        let conflict = table
            .insert(&name("second"), chord("<super>q"), Trigger::Press, false)
            .expect_err("the same chord written another way");
        assert_eq!(conflict.code(), super::ConflictCode::DuplicateChord);
        assert_eq!(conflict.code().as_str(), "duplicate-chord");
        assert_eq!(conflict.existing_name, name("first"));
        assert_eq!(conflict.incoming_name, name("second"));
        assert_eq!(table.len(), 1);
        let conflict = table
            .insert(&name("third"), chord("<Super>q"), Trigger::Release, false)
            .expect_err("one press cannot serve two edges");
        assert_eq!(conflict.existing_name, name("first"));
    }

    #[test]
    fn a_refused_insertion_changes_nothing() {
        let mut table = BindingTable::new();
        table.insert(&name("a"), chord("q"), Trigger::Press, false).expect("a");
        table.insert(&name("b"), chord("w"), Trigger::Press, false).expect("b");
        table
            .insert(&name("b"), chord("q"), Trigger::Press, false)
            .expect_err("conflict");
        assert_eq!(table.lookup(&chord("w")).expect("w").name(), &name("b"));
        assert_eq!(table.lookup(&chord("q")).expect("q").name(), &name("a"));
        assert_eq!(table.len(), 2);
    }

    fn declaration(name_text: &str, chord_text: &str) -> super::Declaration {
        super::Declaration {
            name: name(name_text),
            chord: chord(chord_text),
            trigger: Trigger::Press,
            allow_when_inhibited: false,
        }
    }

    #[test]
    fn a_reload_removes_names_it_no_longer_declares() {
        let mut table = BindingTable::new();
        let kept = table
            .insert(&name("a"), chord("q"), Trigger::Press, false)
            .expect("a")
            .id();
        table.insert(&name("b"), chord("w"), Trigger::Press, false).expect("b");
        let reloaded = table.reconcile(&[declaration("a", "e")]).expect("reload");
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.lookup(&chord("w")), None);
        assert_eq!(reloaded.lookup(&chord("e")).expect("e").id(), kept);
        assert_eq!(reloaded.lookup(&chord("q")), None);
    }

    #[test]
    fn names_may_swap_chords_in_one_reload() {
        let mut table = BindingTable::new();
        table.insert(&name("a"), chord("q"), Trigger::Press, false).expect("a");
        table.insert(&name("b"), chord("w"), Trigger::Press, false).expect("b");
        let reloaded = table
            .reconcile(&[declaration("a", "w"), declaration("b", "q")])
            .expect("a swap is not a conflict");
        assert_eq!(reloaded.lookup(&chord("w")).expect("w").name(), &name("a"));
        assert_eq!(reloaded.lookup(&chord("q")).expect("q").name(), &name("b"));
    }

    #[test]
    fn a_conflicting_reload_leaves_the_table_it_came_from() {
        let mut table = BindingTable::new();
        table.insert(&name("a"), chord("q"), Trigger::Press, false).expect("a");
        let conflict = table
            .reconcile(&[declaration("a", "e"), declaration("c", "e")])
            .expect_err("two names want e");
        assert_eq!(conflict.existing_name, name("a"));
        assert_eq!(conflict.incoming_name, name("c"));
        assert_eq!(table.lookup(&chord("q")).expect("q").name(), &name("a"));
        assert_eq!(table.lookup(&chord("e")), None);
    }
}
