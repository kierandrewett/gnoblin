//! Serving on a bus connection: the wait until zbus's object server answers.
//!
//! zbus starts the dispatcher of a connection's object server as a task of the connection's
//! runtime when [`Connection::object_server`] is first asked for, and that task subscribes to the
//! connection's method calls only when it first runs. The task that reads the socket hands each
//! message to the subscriptions there are at the moment it reads it, so a call it reads before the
//! dispatcher's first turn is dropped without a reply, and zbus gives its caller no deadline: the
//! caller waits for ever. A reader that is already mid-poll when an interface is served, reading
//! the `NameAcquired` signal the bus sends after `Hello`, goes on to read a call that arrives a
//! moment later, and on a busy machine that is enough: the SessionIdle service tests lost a
//! `GetActivity` that way (bead gnoblin-j5rg), as the portal's settings test lost a `Read`
//! (gnoblin-uplg).
//!
//! [`dispatching`] is the barrier: it returns once a call to the connection's own name has been
//! answered, which only the dispatcher does.

use std::time::Duration;

use zbus::Connection;

use crate::log;

/// How long [`dispatching`] waits for the answer to a `Ping` before it sends another. The bus
/// answers in milliseconds; a `Ping` that is merely slow costs one more.
const PING_PATIENCE: Duration = Duration::from_millis(250);

/// How many `Ping`s [`dispatching`] sends before it gives up: with [`PING_PATIENCE`], five
/// seconds, which is a bus that is gone, not slow.
const PING_TRIES: u32 = 20;

/// Wait until `connection`'s object server takes the method calls addressed to it.
///
/// Call it after the interfaces are served and before anything may call them: before the bus name
/// is requested, or before a caller is handed the connection's unique name. A `Ping` that was read
/// before the dispatcher listened is dropped like any other call, so one that is not answered
/// within [`PING_PATIENCE`] is sent again.
///
/// A connection with no unique name is not on a bus and cannot address itself: nothing is waited
/// for.
///
/// # Errors
///
/// The bus's refusal of the `Ping`, or [`zbus::Error::InputOutput`] when none of the [`PING_TRIES`]
/// was answered.
pub async fn dispatching(connection: &Connection) -> zbus::Result<()> {
    let Some(name) = connection.unique_name() else {
        return Ok(());
    };
    // Let the tasks zbus spawned have their first turn, so that the first `Ping` is read by a
    // dispatcher that is there to hear it.
    futures_lite::future::yield_now().await;
    let destination = name.as_str();
    let ping = move || connection.call_method(Some(destination), "/", Some("org.freedesktop.DBus.Peer"), "Ping", &());
    match answered(PING_TRIES, PING_PATIENCE, ping).await {
        Some(reply) => reply.map(|_| ()),
        None => Err(zbus::Error::InputOutput(std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("the object server on {name} did not answer {PING_TRIES} Pings sent to it"),
        )))),
    }
}

/// The answer to `ask`, asked up to `tries` times and given `patience` each time.
///
/// A try that is not answered in time is dropped, which withdraws it, and the next one is made.
/// `None` is every try unanswered, after `tries * patience`.
async fn answered<F, Fut, T>(tries: u32, patience: Duration, mut ask: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
{
    for _ in 0..tries {
        let reply = futures_lite::future::or(async { Some(ask().await) }, async {
            async_io::Timer::after(patience).await;
            None
        })
        .await;
        if reply.is_some() {
            return reply;
        }
        log::debug!("a call was not answered in time; asking again");
    }
    None
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};

    use super::*;

    #[test]
    fn a_dropped_try_is_asked_again_and_the_first_answer_is_the_result() {
        let mut asked = 0;
        let reply = futures_lite::future::block_on(answered(5, Duration::from_millis(20), || {
            asked += 1;
            let this = asked;
            async move {
                if this < 3 {
                    pending::<()>().await;
                }
                this
            }
        }));
        assert_eq!(reply, Some(3), "the third try is the first that answers");
        assert_eq!(asked, 3, "no try is made after the answer");
    }

    #[test]
    fn tries_that_are_never_answered_end_after_all_of_them() {
        let started = std::time::Instant::now();
        let mut asked = 0;
        let reply = futures_lite::future::block_on(answered(3, Duration::from_millis(30), || {
            asked += 1;
            pending::<u32>()
        }));
        assert_eq!(reply, None, "nothing answered");
        assert_eq!(asked, 3, "every try was made");
        assert!(
            started.elapsed() >= Duration::from_millis(90),
            "each try was given its patience, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn an_answer_to_the_first_try_asks_no_more() {
        let mut asked = 0;
        let reply = futures_lite::future::block_on(answered(3, Duration::from_secs(60), || {
            asked += 1;
            ready(9)
        }));
        assert_eq!((reply, asked), (Some(9), 1));
    }
}
