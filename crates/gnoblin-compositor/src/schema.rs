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
//!
//! The `tree/` directory is the owner-socket vocabulary (design/node-core.md
//! section 2.3). [`catalog`] holds every file and serves the owner connection;
//! [`public_catalog`] leaves `tree/` out, the same exclusion the Lua and CLI builds
//! make with `load_dir_except`, so a non-owner listing cannot name a tree member.

use std::path::{Component, Path};

use gnoblin_schema::{Catalog, SchemaError, parse_schema};

include!(concat!(env!("OUT_DIR"), "/schema_files.rs"));

/// The directory whose schema files only the session owner's connection may see.
const OWNER_ONLY_DIR: &str = "tree";

/// The catalog every graph in this process validates against, owner vocabulary included.
///
/// # Errors
///
/// [`SchemaError`] when an embedded file does not parse or the files do not make
/// one consistent catalog. It cannot happen for a build this script accepted, and
/// it is reported rather than unwrapped so a hand-edited build output is a
/// message instead of a panic.
pub fn catalog() -> Result<Catalog, SchemaError> {
    load(|_| true)
}

/// The catalog a connection that is not the session owner's is shown: every
/// embedded file except the owner-only `tree/` directory.
///
/// # Errors
///
/// As [`catalog`].
pub fn public_catalog() -> Result<Catalog, SchemaError> {
    load(|name| !is_owner_only(name))
}

/// Whether an embedded file name sits in the owner-only directory.
fn is_owner_only(name: &str) -> bool {
    Path::new(name)
        .components()
        .next()
        .is_some_and(|first| first == Component::Normal(OWNER_ONLY_DIR.as_ref()))
}

fn load(keep: impl Fn(&str) -> bool) -> Result<Catalog, SchemaError> {
    let schemas = FILES
        .iter()
        .filter(|(name, _)| keep(name))
        .map(|(name, text)| parse_schema(name, text))
        .collect::<Result<Vec<_>, _>>()?;
    Catalog::new(schemas)
}

#[cfg(test)]
mod tests {
    use gnoblin_schema::{Catalog, Namespace};

    use super::{FILES, catalog, is_owner_only, public_catalog};

    fn object_names(catalog: &Catalog, namespace: &str) -> Vec<String> {
        let namespace = Namespace::parse(namespace).expect("a schema namespace");
        catalog.objects(&namespace).map(|object| object.name.clone()).collect()
    }

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
    fn a_public_listing_has_no_tree_members() {
        // The owner-only `tree/` vocabulary (namespace `tree`) is in the owner catalog and
        // absent from the public one, so a listing built from the public catalog cannot
        // name a tree member (design/node-core.md section 2.3, F28).
        assert!(
            FILES.iter().any(|(name, _)| is_owner_only(name)),
            "the build embeds the owner-only tree schema"
        );
        let owner = catalog().expect("the owner catalog loads");
        let public = public_catalog().expect("the public catalog loads");
        assert!(
            object_names(&owner, "tree").iter().any(|name| name == "node"),
            "the owner catalog declares the tree class `node`"
        );
        assert!(
            object_names(&public, "tree").is_empty(),
            "the public catalog declares no tree class"
        );
        assert!(
            object_names(&public, "gnoblin").iter().any(|name| name == "window"),
            "the public catalog keeps the ordinary objects"
        );
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
