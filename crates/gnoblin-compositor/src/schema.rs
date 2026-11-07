//! The object schema this process serves, compiled in.
//!
//! The schema is the graph's vocabulary and the IPC version's source, and a
//! shipped binary has no `crates/gnoblin-schema/schema` beside it, so `build.rs`
//! embeds the checked-in files. Loading them here rather than keeping a second
//! copy of the members is what makes an object the schema does not declare
//! impossible to publish: [`gnoblin_model::Graph`] validates every write against
//! this catalog, and the catalog is the generated one.
//!
//! `build.rs` loads the same directory with the same loader, so a schema the
//! graph would refuse fails the build instead of the first session start.

use gnoblin_schema::{Catalog, SchemaError, parse_schema};

include!(concat!(env!("OUT_DIR"), "/schema_files.rs"));

/// The catalog every graph in this process validates against.
///
/// # Errors
///
/// [`SchemaError`] when an embedded file does not parse or the files do not make
/// one consistent catalog. It cannot happen for a build this script accepted, and
/// it is reported rather than unwrapped so a hand-edited build output is a
/// message instead of a panic.
pub fn catalog() -> Result<Catalog, SchemaError> {
    let schemas = FILES
        .iter()
        .map(|(name, text)| parse_schema(name, text))
        .collect::<Result<Vec<_>, _>>()?;
    Catalog::new(schemas)
}

#[cfg(test)]
mod tests {
    use gnoblin_schema::Namespace;

    use super::{FILES, catalog};

    #[test]
    fn the_embedded_schema_is_the_one_the_build_validated() {
        let catalog = catalog().expect("the embedded schema loads");
        let namespace = Namespace::parse("gnoblin").expect("the core namespace");
        let version = catalog
            .version(&namespace)
            .expect("the schema declares the IPC version");
        assert_eq!(
            (version.major, version.minor, version.patch),
            gnoblin_proto::IPC_VERSION,
            "the version the wire advertises is the catalog's"
        );
    }

    #[test]
    fn every_embedded_file_is_named_and_not_empty() {
        assert!(!FILES.is_empty(), "the build script embeds at least one file");
        for (name, text) in FILES {
            assert!(name.ends_with(".toml"), "{name} is a schema file");
            assert!(!text.is_empty(), "{name} is not empty");
        }
    }

    #[test]
    fn the_window_object_declares_the_members_this_build_publishes() {
        // The names the object store writes are the schema's. A rename would make every
        // `set` fail at run time, so it is a test failure here instead.
        let catalog = catalog().expect("the embedded schema loads");
        let namespace = Namespace::parse("gnoblin").expect("the core namespace");
        let window = catalog
            .objects(&namespace)
            .find(|object| object.name == "window")
            .expect("the schema declares window");
        for name in [
            "id", "title", "app_id", "pid", "focused", "x", "y", "width", "height", "icon", "tag", "modal",
        ] {
            assert!(
                window.properties.iter().any(|property| property.name == name),
                "`window` declares no property `{name}`"
            );
        }
        assert!(
            window.methods.iter().any(|method| method.name == "focus"),
            "`window` declares no method `focus`"
        );
    }
}
