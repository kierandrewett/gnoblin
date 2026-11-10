//! Test support: the private bus, and the graph the fake IPC server serves.
//!
//! Both exist for one reason: a test must never reach the developer's session bus, and the
//! counter the adapter reports has to come from a real object the schema validates. Every
//! value here goes through `Graph::create`, so a property name this crate gets wrong is a
//! test failure rather than a silently empty state.

#![allow(dead_code)]

pub mod bus;

use std::path::Path;

use gnoblin_model::{Fields, Graph, Value};
use gnoblin_proto::Version;
use gnoblin_schema::Catalog;

/// The IPC version the tests negotiate.
pub const VERSION: Version = Version::new(0, 2, 0);

/// A graph over the checked-in schema.
pub fn graph() -> Graph {
    let catalog = Catalog::load_dir_except(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../gnoblin-schema/schema")),
        &["tree"],
    )
    .expect("the checked-in schema loads");
    Graph::new(catalog)
}

/// The properties of the `idle` object.
pub fn idle_properties(idle_ms: u64) -> Fields {
    let mut properties = Fields::new();
    properties.insert("idle_ms".to_owned(), Value::Uint(idle_ms));
    properties.insert("inhibitors".to_owned(), Value::Uint(0));
    properties
}

/// The properties of the `lock` object, unlocked.
pub fn unlocked() -> Fields {
    let mut properties = Fields::new();
    properties.insert("locked".to_owned(), Value::Bool(false));
    properties
}
