//! The declarations a target writes, kept by the member they declare.
//!
//! A target that writes a declaration of a member (a TypeScript `.d.ts` line, a Python stub `def`, a Rust `fn`) tells the sink what it wrote
//! as it writes it, so the reference model (`reference.json`) shows the line the SDK file has and not a second spelling of it. A target
//! rendered without a sink pays nothing.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// Where a member's declarations are kept: the schema's class (or enum) name, and the member's (or value's) name.
type Key = (String, String);

/// The declarations one target wrote, in the order it wrote them.
#[derive(Debug, Default)]
pub(crate) struct Declarations {
    written: RefCell<BTreeMap<Key, Vec<String>>>,
}

impl Declarations {
    /// Notes that the target declared the member `member` of `class` as `text`: one line, without its indentation or its comment.
    pub(crate) fn record(&self, class: &str, member: &str, text: impl AsRef<str>) {
        self.written
            .borrow_mut()
            .entry((class.to_owned(), member.to_owned()))
            .or_default()
            .push(text.as_ref().trim().to_owned());
    }

    /// What the target declared for the member, in order; empty when it declared nothing.
    pub(crate) fn get(&self, class: &str, member: &str) -> Vec<String> {
        self.written
            .borrow()
            .get(&(class.to_owned(), member.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
}

/// Records through an optional sink.
pub(crate) fn note(sink: Option<&Declarations>, class: &str, member: &str, text: impl AsRef<str>) {
    if let Some(sink) = sink {
        sink.record(class, member, text);
    }
}
