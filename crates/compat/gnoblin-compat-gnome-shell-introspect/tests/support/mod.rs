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
    gnoblin_testkit::window_fields(&[
        ("id", Value::Uint(id)),
        ("title", Value::Text(title.to_owned())),
        ("app_id", Value::Text(app_id.to_owned())),
        ("width", Value::Uint(800)),
        ("height", Value::Uint(600)),
    ])
}

/// The properties of one `output` object: a 1920x1080 panel at the origin.
pub fn output_properties(name: &str, width: u32, height: u32, enabled: bool) -> Fields {
    let mode = format!("{width}x{height}@60.000");
    gnoblin_testkit::object_fields(
        "output",
        &[
            ("name", Value::Text(name.to_owned())),
            ("make", Value::Text("Gnoblin".to_owned())),
            ("model", Value::Text("Test Panel".to_owned())),
            ("modes", Value::List(vec![Value::Text(mode.clone())])),
            ("mode", Value::Text(mode)),
            ("scale", Value::Double(1.0)),
            ("primary", Value::Bool(true)),
            ("enabled", Value::Bool(enabled)),
            ("width", Value::Uint(u64::from(width))),
            ("height", Value::Uint(u64::from(height))),
        ],
    )
}

/// The properties of the `lock` object, unlocked.
pub fn unlocked() -> Fields {
    let mut properties = Fields::new();
    properties.insert("locked".to_owned(), Value::Bool(false));
    properties
}
