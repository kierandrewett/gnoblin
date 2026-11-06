//! The watch registry: who asked to be told, and when each one fires.
//!
//! Two kinds of watch, exactly as the interface defines them:
//!
//! - an **idle watch** fires every time the idle time reaches its interval, so a policy
//!   that dims the screen at five minutes and suspends at thirty uses two of them;
//! - a **user-active watch** fires once, when the user becomes active again after having
//!   been idle, and is then gone.
//!
//! Ownership is part of the record rather than a separate map: a watch belongs to the
//! caller's bus identity, [`Watches::remove`] refuses one that is not the caller's, and
//! [`Watches::forget_owner`] drops everything a caller left behind when its connection
//! goes away. That is the failure mode this module exists for — a watch outliving its
//! caller would fire into a bus name nobody holds any more, and the next caller to be
//! given that identity would inherit a signal it never asked for.
//!
//! The registry holds no clock. [`Watches::advance`] takes the idle time the compositor
//! published and answers which watches fired, so a test drives the whole thing with two
//! numbers and no timer.

use std::collections::BTreeMap;

use crate::error::IdleError;

/// What a watch was added for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatchKind {
    /// Fires every time the idle time reaches the interval.
    Idle,
    /// Fires once when the user becomes active again.
    UserActive,
}

/// One watch a caller added.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Watch {
    /// The id the caller removes it by.
    pub id: u32,
    /// The caller's bus identity.
    pub owner: String,
    /// What it fires on.
    pub kind: WatchKind,
    /// The interval in milliseconds; zero for a user-active watch.
    pub interval: u64,
    /// Whether the watch was added while the session was already past its interval, so
    /// it fires on the next advance rather than waiting for the next crossing.
    due: bool,
}

/// Every watch this adapter handed out.
#[derive(Clone, Debug, Default)]
pub struct Watches {
    next: u32,
    idle_ms: u64,
    watches: BTreeMap<u32, Watch>,
}

impl Watches {
    /// An empty registry whose first watch id is 1, because upstream guarantees ids are
    /// greater than zero.
    pub fn new() -> Watches {
        Watches {
            next: 1,
            idle_ms: 0,
            watches: BTreeMap::new(),
        }
    }

    /// The idle time the registry last saw.
    pub const fn idle_ms(&self) -> u64 {
        self.idle_ms
    }

    /// How many watches are live.
    pub fn len(&self) -> usize {
        self.watches.len()
    }

    /// Whether no caller is watching.
    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    /// Add an idle watch and answer its id.
    pub fn add_idle(&mut self, owner: &str, interval: u64) -> u32 {
        let due = self.idle_ms >= interval;
        self.insert(owner, WatchKind::Idle, interval, due)
    }

    /// Add a user-active watch and answer its id.
    pub fn add_active(&mut self, owner: &str) -> u32 {
        self.insert(owner, WatchKind::UserActive, 0, false)
    }

    /// Remove a watch the caller owns.
    ///
    /// # Errors
    ///
    /// [`IdleError::NoWatch`] when no watch has that id, and [`IdleError::ForeignWatch`]
    /// when another caller added it.
    pub fn remove(&mut self, owner: &str, id: u32) -> Result<(), IdleError> {
        match self.watches.get(&id) {
            None => Err(IdleError::NoWatch { id }),
            Some(watch) if watch.owner != owner => Err(IdleError::ForeignWatch { id }),
            Some(_) => {
                self.watches.remove(&id);
                Ok(())
            }
        }
    }

    /// Drop every watch a caller owned, answering how many went.
    ///
    /// This is what a vanished bus identity means: nothing can answer the signal any
    /// more, so the watch ends with the connection that asked for it.
    pub fn forget_owner(&mut self, owner: &str) -> usize {
        let before = self.watches.len();
        self.watches.retain(|_, watch| watch.owner != owner);
        before - self.watches.len()
    }

    /// Tell the registry how long the session has been idle, answering the watches that
    /// fired, in id order.
    ///
    /// A fired user-active watch is removed here: upstream fires it once and it is gone.
    pub fn advance(&mut self, idle_ms: u64) -> Vec<u32> {
        let previous = self.idle_ms;
        self.idle_ms = idle_ms;
        let mut fired = Vec::new();
        let mut spent = Vec::new();
        for watch in self.watches.values_mut() {
            match watch.kind {
                WatchKind::Idle => {
                    let crossed = previous < watch.interval && idle_ms >= watch.interval;
                    if crossed || (watch.due && idle_ms >= watch.interval) {
                        watch.due = false;
                        fired.push(watch.id);
                    }
                }
                WatchKind::UserActive => {
                    // The user was away and came back: the idle time dropped to zero.
                    if previous > 0 && idle_ms == 0 {
                        fired.push(watch.id);
                        spent.push(watch.id);
                    }
                }
            }
        }
        for id in spent {
            self.watches.remove(&id);
        }
        fired
    }

    /// Every live watch, for a test that wants to see the registry.
    pub fn iter(&self) -> impl Iterator<Item = &Watch> {
        self.watches.values()
    }

    fn insert(&mut self, owner: &str, kind: WatchKind, interval: u64, due: bool) -> u32 {
        let id = self.next;
        self.next += 1;
        self.watches.insert(
            id,
            Watch {
                id,
                owner: owner.to_owned(),
                kind,
                interval,
                due,
            },
        );
        id
    }
}
