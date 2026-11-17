//! Listener registrations: [`Subscription`], the guard that ends one, and the handle of a whole listener object.

#![allow(unsafe_code)]

use gnoblin_std_abi::GnoblinStdReg;

use crate::api::Std;
use crate::error::Result;

/// A listener the library holds for this generation. It lasts until the generation retires, or until
/// [`unsubscribe`](Subscription::unsubscribe).
///
/// Dropping a `Subscription` does **not** unsubscribe: `clients.on_added(...)` as a statement is the ordinary way to listen for the
/// life of the shell, and a guard that undid it silently would be a trap, as it is for the core's subscriptions.
/// [`scoped`](Subscription::scoped) is the guard that does unsubscribe on drop.
#[derive(Clone)]
pub struct Subscription {
    std: Std,
    reg: GnoblinStdReg,
}

impl Subscription {
    pub(crate) fn new(std: Std, reg: GnoblinStdReg) -> Subscription {
        Subscription { std, reg }
    }

    /// Removes the listener. Its closure is dropped now, or when the delivery that is running returns. Removing it twice, or after
    /// the generation retired, is an error.
    ///
    /// # Errors
    ///
    /// The listener was removed already, or the generation retired.
    pub fn unsubscribe(self) -> Result<()> {
        // SAFETY: `remove` takes only the registration handle.
        self.std.check(unsafe { (self.std.api().remove)(self.reg) }).map(|_| ())
    }

    /// A guard that unsubscribes when it is dropped.
    pub fn scoped(self) -> ScopedSubscription {
        ScopedSubscription(Some(self))
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription").field("reg", &self.reg).finish()
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
