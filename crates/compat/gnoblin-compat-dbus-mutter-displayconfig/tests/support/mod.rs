//! Test support: the private bus, and the graph the fake IPC server serves.
//!
//! Both exist for one reason: a test must never reach the developer's session bus, and
//! the state the adapter reports has to come from real objects the schema validates, not
//! from a struct a test filled in. Every value here goes through `Graph::create`, so a
//! property name this crate gets wrong is a test failure rather than a silently empty
//! state.

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

/// The properties of one `output` object: a 1920x1080 panel with a 1280x720 mode too.
pub fn output_properties(name: &str, scale: f64, primary: bool) -> Fields {
    gnoblin_testkit::object_fields(
        "output",
        &[
            ("name", Value::Text(name.to_owned())),
            ("make", Value::Text("Gnoblin".to_owned())),
            ("model", Value::Text("Test Panel".to_owned())),
            (
                "modes",
                Value::List(vec![
                    Value::Text("1920x1080@60.000".to_owned()),
                    Value::Text("1280x720@60.000".to_owned()),
                ]),
            ),
            ("mode", Value::Text("1920x1080@60.000".to_owned())),
            ("scale", Value::Double(scale)),
            ("primary", Value::Bool(primary)),
            ("enabled", Value::Bool(true)),
            ("width", Value::Uint(1920)),
            ("height", Value::Uint(1080)),
        ],
    )
}

/// The properties of the `lock` object: the session is not locked.
pub fn unlocked() -> Fields {
    let mut properties = Fields::new();
    properties.insert("locked".to_owned(), Value::Bool(false));
    properties
}
