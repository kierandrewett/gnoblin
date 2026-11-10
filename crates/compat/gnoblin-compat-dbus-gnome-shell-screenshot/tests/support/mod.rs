//! Test support: the private bus, and the graph the fake IPC server serves.
//!
//! Both exist for one reason: a test must never reach the developer's session bus, and the
//! embargo the adapter enforces has to come from a real object the schema validates.

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

/// The properties of the `lock` object.
pub fn lock_properties(locked: bool) -> Fields {
    let mut properties = Fields::new();
    properties.insert("locked".to_owned(), Value::Bool(locked));
    properties
}
