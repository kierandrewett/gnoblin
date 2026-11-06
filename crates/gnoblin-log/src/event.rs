//! The shape of one logged event, read once and shared by every sink.
//!
//! The console, the database and the journal all read an event through [`Logged::from_event`],
//! so a value that a type redacts through its `Debug` impl is redacted in every sink, and each
//! sink agrees on which field names the component and the message.

use std::fmt::{Debug, Write as _};

use tracing::Event;
use tracing::field::{Field, Visit};

/// The field that carries an event's component.
///
/// The level macros set it, so a sink reads the component from here and not from the target.
pub const COMPONENT_FIELD: &str = "component";

/// The field that carries an event's message.
pub const MESSAGE_FIELD: &str = "message";

/// The component of an event that no gnoblin crate logged: one from a dependency, or from
/// `tracing` itself. Such an event keeps its target as a field, so its origin is not lost.
pub const FOREIGN_COMPONENT: &str = "ext";

/// The field that keeps the target of a foreign event.
pub const TARGET_FIELD: &str = "target";

/// The parts of one logged event that every sink reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    /// The component that logged the event, or [`FOREIGN_COMPONENT`].
    pub component: String,
    /// The event's `message`, or empty when it has none.
    pub message: String,
    /// The module path that logged the event, as `tracing` recorded it.
    pub target: String,
    /// The event's other fields, in the order they were recorded, with their values
    /// rendered by `Debug`. The component is not among them; a foreign event's target is.
    pub fields: Vec<(String, String)>,
}

impl Logged {
    /// Reads `event` into its parts.
    pub fn from_event(event: &Event<'_>) -> Self {
        let mut collector = Collector::default();
        event.record(&mut collector);
        let target = event.metadata().target().to_owned();
        let component = match collector.component {
            Some(component) => component,
            None => {
                collector.fields.push((TARGET_FIELD.to_owned(), target.clone()));
                FOREIGN_COMPONENT.to_owned()
            }
        };
        Self {
            component,
            message: collector.message,
            target,
            fields: collector.fields,
        }
    }

    /// The fields as `name=value` pairs separated by spaces, for a line of text.
    pub fn fields_line(&self) -> String {
        let mut line = String::new();
        for (index, (name, value)) in self.fields.iter().enumerate() {
            if index > 0 {
                line.push(' ');
            }
            let _ = write!(line, "{name}={value}");
        }
        line
    }
}

/// Collects an event's component, message and other fields.
#[derive(Default)]
struct Collector {
    component: Option<String>,
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for Collector {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            COMPONENT_FIELD => self.component = Some(value.to_owned()),
            MESSAGE_FIELD => self.message.push_str(value),
            name => self.fields.push((name.to_owned(), format!("{value:?}"))),
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        match field.name() {
            MESSAGE_FIELD => {
                let _ = write!(self.message, "{value:?}");
            }
            name => self.fields.push((name.to_owned(), format!("{value:?}"))),
        }
    }
}

/// Captures the events a closure logs, for the facade's own tests and the sinks' tests.
#[cfg(test)]
pub(crate) mod capture {
    use std::sync::{Arc, Mutex};

    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    use super::Logged;

    /// Keeps the [`Logged`] parts of every event it is given.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Logged>>>);

    impl Subscriber for Recorder {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            self.0.lock().expect("the capture lock").push(Logged::from_event(event));
        }

        fn enter(&self, _: &Id) {}

        fn exit(&self, _: &Id) {}
    }

    /// The parts of the events `run` logs, in order.
    pub(crate) fn logged(run: impl FnOnce()) -> Vec<Logged> {
        let recorder = Recorder::default();
        tracing::subscriber::with_default(recorder.clone(), run);
        let events = recorder.0.lock().expect("the capture lock");
        events.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::capture::logged;
    use super::*;

    #[test]
    fn the_component_field_is_the_component_and_is_not_repeated_as_a_field() {
        let events = logged(|| {
            tracing::info!(component = "wayland", count = 3, "mapped {}", "window");
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].component, "wayland");
        assert_eq!(events[0].message, "mapped window");
        assert_eq!(events[0].fields, [("count".to_owned(), "3".to_owned())]);
    }

    #[test]
    fn a_string_field_is_quoted_so_its_spaces_read_as_one_value() {
        let events = logged(|| tracing::info!(component = "x", role = "two words", "m"));
        assert_eq!(events[0].fields_line(), r#"role="two words""#);
    }

    #[test]
    fn an_event_without_a_component_is_foreign_and_keeps_its_target() {
        let events = logged(|| tracing::warn!(retries = 2, "dependency retried"));
        assert_eq!(events[0].component, FOREIGN_COMPONENT);
        assert_eq!(events[0].target, module_path!());
        assert_eq!(
            events[0].fields,
            [
                ("retries".to_owned(), "2".to_owned()),
                (TARGET_FIELD.to_owned(), module_path!().to_owned()),
            ]
        );
    }

    #[test]
    fn fields_line_joins_pairs_with_single_spaces() {
        let logged = Logged {
            component: "c".to_owned(),
            message: String::new(),
            target: String::new(),
            fields: vec![("a".to_owned(), "1".to_owned()), ("b".to_owned(), "x".to_owned())],
        };
        assert_eq!(logged.fields_line(), "a=1 b=x");
    }
}
