//! The caller-lifetime rule every D-Bus adapter shares.
//!
//! A bus gives no notice to a process that a peer has gone away except
//! `org.freedesktop.DBus`'s `NameOwnerChanged`. [`CallerWatcher`] reads that signal and
//! answers the unique names that vanished, so an adapter can end whatever the caller
//! owned: a watch, a session, a stream.

use futures_lite::StreamExt as _;
use zbus::{Connection, fdo};

/// The caller-lifetime rule: a watch or session ends with the bus connection that asked for it.
///
/// `org.freedesktop.DBus`'s `NameOwnerChanged` is the only notice a session bus gives when a
/// peer goes away, so this reads it and answers the unique names that vanished. It is a
/// value rather than a spawned task because the crate has no executor of its own: a host
/// runs an adapter's `watch_callers` beside the connection's service loop, and a test drives
/// one event at a time.
#[derive(Debug)]
pub struct CallerWatcher {
    stream: fdo::NameOwnerChangedStream,
}

impl CallerWatcher {
    /// Subscribe to the bus's own ownership signal.
    ///
    /// Subscribe before the loop that consumes the stream starts: a caller that leaves before
    /// the match rule is installed produces no signal.
    ///
    /// # Errors
    ///
    /// The bus error from adding the match rule.
    pub async fn new(connection: &Connection) -> zbus::Result<CallerWatcher> {
        let bus = fdo::DBusProxy::new(connection).await?;
        let stream = bus.receive_name_owner_changed().await?;
        Ok(CallerWatcher { stream })
    }

    /// The next caller that left the bus, or `None` when the stream ends.
    ///
    /// A name that changed *owner* is not a departure and is skipped: a well-known name
    /// moving between two processes is not the caller's unique name ending.
    pub async fn next(&mut self) -> Option<String> {
        while let Some(signal) = self.stream.next().await {
            let Ok(args) = signal.args() else {
                continue;
            };
            // The owner arguments are strings on the wire and the bus spells "nobody" as the
            // empty one, which zbus models as an absent value.
            if args.new_owner().as_ref().is_none() {
                return Some(args.name().to_string());
            }
        }
        None
    }
}
