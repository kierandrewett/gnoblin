//! Hands this library's events to a host that owns the log.
//!
//! The log is a per-library singleton: `tracing` keeps its global subscriber in the library that holds the `tracing` crate, so a
//! library that is loaded into a process that already logs (a language runtime loaded into the core) has a subscriber of its own
//! to set, and without one every event it logs is dropped. [`install`] sets one that gives each event to a function, which a
//! runtime points at the host's logging call, so the runtime's lines reach the same log, filtered and recorded as the host's.
//! Spans are not used by the gnoblin macros and are accepted and ignored.

use std::sync::atomic::{AtomicU64, Ordering};

use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

use crate::event::Logged;

/// The subscriber [`install`] sets.
struct Forwarder<F> {
    sink: F,
    spans: AtomicU64,
}

impl<F> Subscriber for Forwarder<F>
where
    F: Fn(Level, &Logged) + Send + Sync + 'static,
{
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _attributes: &Attributes<'_>) -> Id {
        Id::from_u64(self.spans.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        (self.sink)(*event.metadata().level(), &Logged::from_event(event));
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// Sets `sink` as this library's global subscriber: it receives every event the library's call sites emit, with the level and the
/// parts every sink reads. Returns `false` and changes nothing when a subscriber is set already.
///
/// The call sites decide what is emitted from the filter specification in force (`filter::set_spec`); install the specification
/// the host uses first, so a call site the host would drop formats nothing.
pub fn install<F>(sink: F) -> bool
where
    F: Fn(Level, &Logged) + Send + Sync + 'static,
{
    tracing::subscriber::set_global_default(Forwarder {
        sink,
        spans: AtomicU64::new(0),
    })
    .is_ok()
}
