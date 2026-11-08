//! The object schema: object types, properties, methods, signals and error codes,
//! with the version that added each member.
//!
//! This crate is the single source of truth the IPC, the client libraries, the
//! script bindings, `gnoblinctl` and the API reference are generated from
//! (design/rust-compositor.md, "One IPC" and "Contracts are data"). It holds the
//! model and its rules and nothing else: it knows no wire format, no socket and
//! no compositor type, so generators and tests can depend on it freely.
//!
//! ```no_run
//! use gnoblin_schema::Catalog;
//!
//! let catalog = Catalog::load_dir(std::path::Path::new("crates/gnoblin-schema/schema"))?;
//! for namespace in catalog.namespaces() {
//!     println!("{namespace} {}", catalog.version(&namespace).expect("version"));
//!     for object in catalog.objects(&namespace) {
//!         println!("  object {}", object.name);
//!     }
//! }
//! # Ok::<(), gnoblin_schema::SchemaError>(())
//! ```
//!
//! A namespace is `gnoblin` for the compositor's own objects, or an extension
//! name such as `example.ext`. An extension ships its own files in the same
//! format, in its own namespace; Gnoblin's schema never contains extension
//! members. The same generator runs over both, which is why the format carries a
//! namespace rather than assuming one.
//!
//! [`proto`] renders the format as Protocol Buffers text, so the wire layer's
//! build script and `cargo xtask gen-proto` produce the same bytes from one
//! implementation.

pub mod derived;
mod error;
mod model;
mod parse;
pub mod proto;

pub use error::SchemaError;
pub use gnoblin_types::{InvalidVersion, Version};
pub use model::{
    Catalog, ErrorDecl, Field, Method, Namespace, ObjectType, Property, Schema, Signal, Type, is_error_code,
    is_lower_name,
};
pub use parse::parse_schema;
