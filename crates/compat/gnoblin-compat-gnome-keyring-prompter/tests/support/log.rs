//! A `tracing` subscriber that keeps every event's fields as text.
//!
//! The failing assertion is "this string is absent from every line the adapter logged",
//! which needs the events as text rather than as a formatted terminal output. Records
//! both `Debug` and `str` fields, because a log line may pass either.

use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// The captured lines, shared with the test that asserts on them.
#[derive(Clone, Default)]
pub struct Captured {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Captured {
    /// Every line logged since the subscriber was installed.
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("the capture lock is not poisoned").clone()
    }

    /// Whether any line contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.lines().iter().any(|line| line.contains(needle))
    }

    /// Install this subscriber as the thread's default, returning the guard that keeps it
    /// installed.
    ///
    /// Thread-local on purpose: `tests` runs every test in its own thread, so two tests
    /// capturing logs at once never see each other's events.
    pub fn install(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(Recorder { captured: self.clone() })
    }
}

struct Recorder {
    captured: Captured,
}

impl Subscriber for Recorder {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut line = Line::default();
        event.record(&mut line);
        let metadata = event.metadata();
        let text = format!("{} {}: {}", metadata.level(), metadata.target(), line.0);
        self.captured
            .lines
            .lock()
            .expect("the capture lock is not poisoned")
            .push(text);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value}");
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}
