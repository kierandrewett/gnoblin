//! What a callback may return.
//!
//! Every `on_*`, `bind*`, `after` and `every` closure may return `()` or a `Result`, so a callback written with `?` or ending in a
//! fallible call needs no `.ok()`:
//!
//! ```ignore
//! g.bind("Super+Return", || g.spawn("foot"))?;
//! g.on_client_created(|window| workspace.add(window))?;
//! ```
//!
//! An `Err` is not a panic and does not stop the shell: the SDK writes it to the core's log as a warning that names the
//! subscription, and the next event is delivered as usual.

use std::fmt::Display;

use crate::gnoblin::{Host, LogLevel};

/// The value a callback returns: `()`, or a `Result` whose error is logged.
pub trait CallbackResult {
    /// Handles the outcome on behalf of the callback named `what` of the generation `g` belongs to.
    fn settle(self, g: &Host, what: &str);
}

impl CallbackResult for () {
    fn settle(self, _g: &Host, _what: &str) {}
}

impl<T, E: Display> CallbackResult for Result<T, E> {
    fn settle(self, g: &Host, what: &str) {
        if let Err(error) = self {
            g.log_at(LogLevel::Warn, &format!("the {what} callback failed: {error}"));
        }
    }
}
