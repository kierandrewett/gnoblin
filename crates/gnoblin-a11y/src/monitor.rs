//! The keyboard-monitor watch and grab table.
//!
//! `org.freedesktop.a11y.KeyboardMonitor` lets an assistive technology watch
//! every key, grab every key, or grab a list of modifiers and keystrokes. The
//! table is keyed by owner and contains no entries without something set, so
//! there is no way to hold a grab without an owner: an owner leaves the table
//! through [`Monitors::drop_owner`] alone, which is what ends its grabs.
//!
//! The decision a key event gets is the whole point of the table:
//!
//! - a grab takes the key away from the focused client
//!   ([`KeyDecision::Consumed`]);
//! - a watch without a grab lets both see it ([`KeyDecision::Both`]);
//! - no interest at all passes it through ([`KeyDecision::PassThrough`]).

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{Keysym, OwnerId};
use crate::{Key, KeyDecision, Keystroke, Modifiers};

/// What one consumer asked for.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Consumer {
    /// Whether it asked for every key event with `WatchKeyboard`.
    watch_all: bool,
    /// Whether it asked to take every key with `GrabKeyboard`.
    grab_all: bool,
    /// The modifier keysyms it grabbed.
    modifiers: Vec<Keysym>,
    /// The keystrokes it grabbed.
    keystrokes: Vec<Keystroke>,
}

impl Consumer {
    /// Whether this consumer has asked for anything, so an entry that has asked
    /// for nothing can leave the table.
    fn is_empty(&self) -> bool {
        !self.watch_all && !self.grab_all && self.modifiers.is_empty() && self.keystrokes.is_empty()
    }

    /// Whether this consumer grabs this key.
    ///
    /// The mask is the one the client will see, so a chord typed with sticky
    /// keys matches a grabbed keystroke that asks for the same modifiers: sticky
    /// keys rewrites the event before the table sees it.
    fn grabs(&self, key: &Key, modifiers: Modifiers, held: &BTreeSet<Keysym>) -> bool {
        if self.grab_all {
            return true;
        }
        if self.modifiers.contains(&key.keysym) {
            return true;
        }
        // A key pressed while one of this consumer's grabbed modifiers is down
        // belongs to the grab: the consumer asked for the modifier, so the chord
        // typed with it is its too.
        if self.modifiers.iter().any(|keysym| held.contains(keysym)) {
            return true;
        }
        self.keystrokes
            .iter()
            .any(|keystroke| keystroke.keysym == key.keysym && keystroke.modifiers == modifiers)
    }
}

/// The consumers watching and grabbing keys.
#[derive(Debug, Default)]
pub struct Monitors {
    consumers: BTreeMap<OwnerId, Consumer>,
    /// The grabbed modifier keys that are down now. Only grabbed modifiers are
    /// tracked, because only they change a decision.
    held: BTreeSet<Keysym>,
}

impl Monitors {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A consumer started or stopped watching every key.
    pub fn watch(&mut self, owner: OwnerId, watching: bool) {
        self.entry(owner).watch_all = watching;
        self.prune(owner);
    }

    /// A consumer started or stopped grabbing every key.
    pub fn grab(&mut self, owner: OwnerId, grabbing: bool) {
        self.entry(owner).grab_all = grabbing;
        self.prune(owner);
    }

    /// A consumer replaced its grab list.
    pub fn set_grabs(&mut self, owner: OwnerId, modifiers: Vec<Keysym>, keystrokes: Vec<Keystroke>) {
        let consumer = self.entry(owner);
        consumer.modifiers = modifiers;
        consumer.keystrokes = keystrokes;
        self.prune(owner);
        self.sync_held();
    }

    /// An owner's connection went away: everything it asked for ends here.
    ///
    /// This is the only way a grab ends, which is why the table cannot hold a
    /// grab without a live owner.
    pub fn drop_owner(&mut self, owner: OwnerId) {
        self.consumers.remove(&owner);
        self.sync_held();
    }

    /// Records a key event so a held grabbed modifier can grab the keys pressed
    /// with it, and a released one stops doing so.
    pub fn note(&mut self, key: &Key, released: bool) {
        let grabbed = self
            .consumers
            .values()
            .any(|consumer| consumer.modifiers.contains(&key.keysym));
        if !grabbed {
            return;
        }
        if released {
            self.held.remove(&key.keysym);
        } else {
            self.held.insert(key.keysym);
        }
    }

    /// Who sees this key event.
    #[must_use]
    pub fn decide(&self, key: &Key, modifiers: Modifiers) -> (KeyDecision, Vec<OwnerId>) {
        let mut grabbed = BTreeSet::new();
        let mut watching = BTreeSet::new();
        for (owner, consumer) in &self.consumers {
            if consumer.grabs(key, modifiers, &self.held) {
                grabbed.insert(*owner);
            }
            if consumer.watch_all {
                watching.insert(*owner);
            }
        }
        if !grabbed.is_empty() {
            let monitors = grabbed.union(&watching).copied().collect();
            return (KeyDecision::Consumed, monitors);
        }
        if !watching.is_empty() {
            return (KeyDecision::Both, watching.into_iter().collect());
        }
        (KeyDecision::PassThrough, Vec::new())
    }

    /// The live owners, in owner order.
    pub fn owners(&self) -> impl Iterator<Item = OwnerId> + '_ {
        self.consumers.keys().copied()
    }

    /// Forgets a held grabbed modifier, for the cases where the key that held it
    /// down will never be released here: a session lock and a feature toggle.
    pub fn clear_held(&mut self) {
        self.held.clear();
    }

    fn entry(&mut self, owner: OwnerId) -> &mut Consumer {
        self.consumers.entry(owner).or_default()
    }

    fn prune(&mut self, owner: OwnerId) {
        if self.consumers.get(&owner).is_some_and(Consumer::is_empty) {
            self.consumers.remove(&owner);
        }
    }

    /// Drops held modifiers nobody grabs any more.
    fn sync_held(&mut self) {
        let grabbed: BTreeSet<Keysym> = self
            .consumers
            .values()
            .flat_map(|consumer| consumer.modifiers.iter().copied())
            .collect();
        self.held.retain(|keysym| grabbed.contains(keysym));
    }
}
