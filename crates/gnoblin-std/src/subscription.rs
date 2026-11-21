//! Listener registrations: [`Subscription`], the guard that ends one, and the handle of a whole listener object.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_std_abi::GnoblinStdReg;

use crate::api::Std;
use crate::error::{Error, ErrorKind, Result};

/// What ends a registration made through the object protocol: it runs once.
type Undo = Rc<RefCell<Option<Box<dyn FnOnce() -> Result<()>>>>>;

thread_local! {
    /// The registrations of the object protocol, held for the life of the thread: a listener of the protocol ends when it is dropped,
    /// and a [`Subscription`] that is dropped must not end it.
    static HELD: RefCell<Vec<Undo>> = const { RefCell::new(Vec::new()) };
}

/// The registration behind a [`Subscription`].
#[derive(Clone)]
enum Reg {
    /// A listener of the std library's function table.
    Table(Std, GnoblinStdReg),
    /// A listener of the object protocol.
    Object(Undo),
}

/// A listener the library holds for this generation. It lasts until the generation retires, or until
/// [`unsubscribe`](Subscription::unsubscribe).
///
/// Dropping a `Subscription` does **not** unsubscribe: `clients.on_added(...)` as a statement is the ordinary way to listen for the
/// life of the shell, and a guard that undid it silently would be a trap, as it is for the core's subscriptions.
/// [`scoped`](Subscription::scoped) is the guard that does unsubscribe on drop.
#[derive(Clone)]
pub struct Subscription {
    reg: Reg,
}

impl Subscription {
    pub(crate) fn new(std: Std, reg: GnoblinStdReg) -> Subscription {
        Subscription {
            reg: Reg::Table(std, reg),
        }
    }

    /// A registration of the object protocol, which `undo` ends. The closure owns what has to stay alive for the registration to
    /// last: the listeners, which end when they are dropped.
    pub(crate) fn from_undo(undo: impl FnOnce() -> Result<()> + 'static) -> Subscription {
        let undo: Undo = Rc::new(RefCell::new(Some(Box::new(undo))));
        HELD.with(|held| held.borrow_mut().push(Rc::clone(&undo)));
        Subscription { reg: Reg::Object(undo) }
    }

    /// Removes the listener. Its closure is dropped now, or when the delivery that is running returns. Removing it twice, or after
    /// the generation retired, is an error.
    ///
    /// # Errors
    ///
    /// The listener was removed already, or the generation retired.
    pub fn unsubscribe(self) -> Result<()> {
        match self.reg {
            // SAFETY: `remove` takes only the registration handle.
            Reg::Table(std, reg) => std.check(unsafe { (std.api().remove)(reg) }).map(|_| ()),
            Reg::Object(undo) => {
                let end = undo.borrow_mut().take();
                HELD.with(|held| held.borrow_mut().retain(|kept| !Rc::ptr_eq(kept, &undo)));
                match end {
                    Some(end) => end(),
                    None => Err(Error::new(ErrorKind::Stale, "the listener was removed already")),
                }
            }
        }
    }

    /// A guard that unsubscribes when it is dropped.
    pub fn scoped(self) -> ScopedSubscription {
        ScopedSubscription(Some(self))
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reg {
            Reg::Table(_, reg) => f.debug_struct("Subscription").field("reg", reg).finish(),
            Reg::Object(_) => f.debug_struct("Subscription").finish_non_exhaustive(),
        }
    }
}

/// A [`Subscription`] that unsubscribes when dropped. Made by [`Subscription::scoped`].
#[must_use = "a scoped subscription unsubscribes when it is dropped"]
pub struct ScopedSubscription(Option<Subscription>);

impl ScopedSubscription {
    /// Gives up the guard and keeps the subscription for the life of the generation.
    pub fn keep(mut self) -> Subscription {
        self.0
            .take()
            .expect("a scoped subscription holds its subscription until it is kept or dropped")
    }
}

impl Drop for ScopedSubscription {
    fn drop(&mut self) {
        if let Some(subscription) = self.0.take() {
            // The generation may have retired already, in which case the library removed it and there is nothing to undo.
            let _ = subscription.unsubscribe();
        }
    }
}

/// The subscriptions of one listener object (`add_listener`): one for each event it hears.
#[derive(Clone, Debug)]
pub struct ListenerHandle {
    subscriptions: Vec<Subscription>,
}

impl ListenerHandle {
    pub(crate) fn new(subscriptions: Vec<Subscription>) -> ListenerHandle {
        ListenerHandle { subscriptions }
    }

    /// Removes the listener from the collection: it hears nothing more.
    ///
    /// # Errors
    ///
    /// The first failure; the rest are still removed.
    pub fn remove(self) -> Result<()> {
        let mut first = Ok(());
        for subscription in self.subscriptions {
            if let Err(error) = subscription.unsubscribe() {
                first = first.and(Err(error));
            }
        }
        first
    }

    /// A guard that removes the listener when it is dropped.
    pub fn scoped(self) -> ScopedListener {
        ScopedListener(Some(self))
    }
}

/// A [`ListenerHandle`] that removes the listener when dropped.
#[must_use = "a scoped listener is removed when it is dropped"]
pub struct ScopedListener(Option<ListenerHandle>);

impl ScopedListener {
    /// Gives up the guard and keeps the listener for the life of the generation.
    pub fn keep(mut self) -> ListenerHandle {
        self.0
            .take()
            .expect("a scoped listener holds its handle until it is kept or dropped")
    }
}

impl Drop for ScopedListener {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = handle.remove();
        }
    }
}
