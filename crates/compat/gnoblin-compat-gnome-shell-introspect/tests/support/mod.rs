//! Test support: the private bus, and the graph the fake IPC server serves.
//!
//! Both exist for one reason: a test must never reach the developer's session bus, and the
//! window list the adapter reports has to come from real objects the schema validates. Every
//! value here goes through `Graph::create`, so a property name this crate gets wrong is a test
//! failure rather than a silently empty dictionary.

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
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    Graph::new(catalog)
}

/// The properties of one `window` object.
pub fn window_properties(id: u64, title: &str, app_id: &str) -> Fields {
    let mut properties = Fields::new();
    properties.insert("id".to_owned(), Value::Uint(id));
    properties.insert("title".to_owned(), Value::Text(title.to_owned()));
    properties.insert("app_id".to_owned(), Value::Text(app_id.to_owned()));
    properties.insert("focused".to_owned(), Value::Bool(false));
    properties.insert("x".to_owned(), Value::Int(0));
    properties.insert("y".to_owned(), Value::Int(0));
    properties.insert("width".to_owned(), Value::Uint(800));
    properties.insert("height".to_owned(), Value::Uint(600));
    properties
}

/// The properties of one `output` object: a 1920x1080 panel at the origin.
pub fn output_properties(name: &str, width: u32, height: u32, enabled: bool) -> Fields {
    let mut properties = Fields::new();
    properties.insert("name".to_owned(), Value::Text(name.to_owned()));
    properties.insert("make".to_owned(), Value::Text("Gnoblin".to_owned()));
    properties.insert("model".to_owned(), Value::Text("Test Panel".to_owned()));
    properties.insert(
        "modes".to_owned(),
        Value::List(vec![Value::Text(format!("{width}x{height}@60.000"))]),
    );
    properties.insert("mode".to_owned(), Value::Text(format!("{width}x{height}@60.000")));
    properties.insert("scale".to_owned(), Value::Double(1.0));
    properties.insert("transform".to_owned(), Value::Uint(0));
    properties.insert("primary".to_owned(), Value::Bool(true));
    properties.insert("enabled".to_owned(), Value::Bool(enabled));
    properties.insert("x".to_owned(), Value::Int(0));
    properties.insert("y".to_owned(), Value::Int(0));
    properties.insert("width".to_owned(), Value::Uint(u64::from(width)));
    properties.insert("height".to_owned(), Value::Uint(u64::from(height)));
    properties
}

/// The properties of the `lock` object, unlocked.
pub fn unlocked() -> Fields {
    let mut properties = Fields::new();
    properties.insert("locked".to_owned(), Value::Bool(false));
    properties
}
