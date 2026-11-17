//! [`Emitter`]: typed events a shell makes up and its own code listens to.
//!
//! The library's collections raise the compositor's events (a window opened, a title changed). A shell has events of its own (the
//! overview opened, a mode changed), and `Emitter<E>` is the type for them: any number of listeners, called in the order they were
//! added with a reference to the event, each removable by the guard its `on` returned.
//!
//! An emitter is a value of the shell's, made of ordinary Rust and calling nothing in the library, so it works in any shell, with or
//! without the library loaded. It is `!Send`, as everything on the shell thread is.
//!
//! ```no_run
//! use gnoblin_std::Emitter;
//!
//! let opened = Emitter::<String>::new();
//! let guard = opened.on(|name| println!("opened {name}"));
//! opened.emit(&"files".to_owned());
//! drop(guard); // no listener is left
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Callback<E> = Box<dyn FnMut(&E)>;

struct Slot<E> {
    id: u64,
    /// Taken out while the callback runs, so a listener may add or remove listeners (and emit again) from inside itself.
    callback: Option<Callback<E>>,
    removed: bool,
}

struct Shared<E> {
    slots: RefCell<Vec<Slot<E>>>,
    next: Cell<u64>,
}

/// An event source. Cheap to clone: every clone is the same emitter.
pub struct Emitter<E> {
    shared: Rc<Shared<E>>,
}

impl<E> Clone for Emitter<E> {
    fn clone(&self) -> Emitter<E> {
        Emitter {
            shared: Rc::clone(&self.shared),
        }
    }
}

impl<E> Default for Emitter<E> {
    fn default() -> Emitter<E> {
        Emitter::new()
    }
}

impl<E> std::fmt::Debug for Emitter<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emitter")
            .field(
                "listeners",
                &self.shared.slots.borrow().iter().filter(|slot| !slot.removed).count(),
            )
            .finish()
    }
}

/// A listener of an [`Emitter`]. Dropping it removes the listener; [`keep`](Listener::keep) gives the listener the life of the emitter
/// instead.
#[must_use = "the listener is removed when this guard is dropped; call `.keep()` to leave it for the life of the emitter"]
pub struct Listener<E> {
    shared: Option<Rc<Shared<E>>>,
    id: u64,
}

impl<E> Emitter<E> {
    /// An emitter with no listeners.
    pub fn new() -> Emitter<E> {
        Emitter {
            shared: Rc::new(Shared {
                slots: RefCell::new(Vec::new()),
                next: Cell::new(0),
            }),
        }
    }

    /// Calls `f` for every event emitted from now on, until the returned guard is dropped.
    pub fn on<F>(&self, f: F) -> Listener<E>
    where
        F: FnMut(&E) + 'static,
    {
        let id = self.shared.next.get() + 1;
        self.shared.next.set(id);
        self.shared.slots.borrow_mut().push(Slot {
            id,
            callback: Some(Box::new(f)),
            removed: false,
        });
        Listener {
            shared: Some(Rc::clone(&self.shared)),
            id,
        }
    }

    /// Calls every listener with `event`, in the order they were added. A listener added while this runs hears the next event, not
    /// this one; a listener removed while it runs hears nothing more. A listener that emits an event of its own runs the other
    /// listeners for it, and is skipped for it (it is already running).
    pub fn emit(&self, event: &E) {
        let ids: Vec<u64> = self.shared.slots.borrow().iter().map(|slot| slot.id).collect();
        for id in ids {
            let callback = {
                let mut slots = self.shared.slots.borrow_mut();
                slots
                    .iter_mut()
                    .find(|slot| slot.id == id && !slot.removed)
                    .and_then(|slot| slot.callback.take())
            };
            let Some(mut callback) = callback else {
                continue;
            };
            callback(event);
            let mut slots = self.shared.slots.borrow_mut();
            if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
                slot.callback = Some(callback);
            }
        }
        self.shared.slots.borrow_mut().retain(|slot| !slot.removed);
    }

    /// How many listeners there are.
    pub fn len(&self) -> usize {
        self.shared.slots.borrow().iter().filter(|slot| !slot.removed).count()
    }

    /// Whether there are no listeners.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<E> Listener<E> {
    /// Gives up the guard and leaves the listener in place for the life of the emitter.
    pub fn keep(mut self) {
        self.shared = None;
    }
}

impl<E> Drop for Listener<E> {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            let mut slots = shared.slots.borrow_mut();
            if let Some(slot) = slots.iter_mut().find(|slot| slot.id == self.id) {
                // A listener that is running holds its callback outside the list: mark it, and the emit that runs it drops it.
                slot.removed = true;
            }
            // A removed slot that is not running holds its callback and can go now; a running one is dropped when its emit ends.
            slots.retain(|slot| !(slot.removed && slot.callback.is_some()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listeners_hear_events_in_the_order_they_were_added() {
        let emitter = Emitter::<u32>::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let first = Rc::clone(&seen);
        emitter.on(move |n| first.borrow_mut().push(("a", *n))).keep();
        let second = Rc::clone(&seen);
        emitter.on(move |n| second.borrow_mut().push(("b", *n))).keep();
        emitter.emit(&1);
        emitter.emit(&2);
        assert_eq!(*seen.borrow(), [("a", 1), ("b", 1), ("a", 2), ("b", 2)]);
    }

    #[test]
    fn dropping_the_guard_removes_the_listener() {
        let emitter = Emitter::<u32>::new();
        let count = Rc::new(Cell::new(0));
        let counted = Rc::clone(&count);
        let guard = emitter.on(move |_| counted.set(counted.get() + 1));
        emitter.emit(&0);
        assert_eq!(emitter.len(), 1);
        drop(guard);
        emitter.emit(&0);
        assert_eq!((count.get(), emitter.len()), (1, 0));
    }

    #[test]
    fn a_listener_may_remove_itself_while_it_runs() {
        let emitter = Emitter::<u32>::new();
        let count = Rc::new(Cell::new(0));
        let guard = Rc::new(RefCell::new(None));
        let (counted, slot) = (Rc::clone(&count), Rc::clone(&guard));
        *guard.borrow_mut() = Some(emitter.on(move |_| {
            counted.set(counted.get() + 1);
            slot.borrow_mut().take();
        }));
        emitter.emit(&0);
        emitter.emit(&0);
        assert_eq!(count.get(), 1);
        assert!(emitter.is_empty());
    }

    #[test]
    fn a_listener_added_while_emitting_hears_the_next_event() {
        let emitter = Emitter::<u32>::new();
        let late = Rc::new(Cell::new(0));
        let (inner, counted) = (emitter.clone(), Rc::clone(&late));
        let added = Cell::new(false);
        emitter
            .on(move |_| {
                if !added.replace(true) {
                    let counted = Rc::clone(&counted);
                    inner.on(move |_| counted.set(counted.get() + 1)).keep();
                }
            })
            .keep();
        emitter.emit(&0);
        assert_eq!(late.get(), 0);
        emitter.emit(&0);
        assert_eq!(late.get(), 1);
    }

    #[test]
    fn a_kept_listener_outlives_its_guard() {
        let emitter = Emitter::<&'static str>::new();
        let count = Rc::new(Cell::new(0));
        let counted = Rc::clone(&count);
        emitter.on(move |_| counted.set(counted.get() + 1)).keep();
        emitter.emit(&"x");
        assert_eq!(count.get(), 1);
        assert_eq!(emitter.len(), 1);
    }
}
