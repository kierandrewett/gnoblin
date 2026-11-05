//! Renders `gnoblinctl`'s command tree from the object schema.
//!
//! The tree is data, not code: one entry per object the schema declares, with its
//! command name and its typed members. `src/tree.rs` includes the rendered file and
//! gives it behaviour, so the same table feeds the clap command tree, the typed
//! argument parsing and the completion scripts without any of them reading the
//! schema at run time (design/rust-compositor.md, "Generated from the schema: ...
//! `gnoblinctl` subcommands and completion").
//!
//! Nothing generated is committed: `build.rs` renders it into `OUT_DIR`, so a
//! clean checkout builds with plain `cargo build` and a schema edit is a rebuild
//! rather than a regeneration somebody has to remember.
//!
//! The generator is a module of the crate rather than a private module of the
//! build script for one reason: `build.rs` includes this file, so the code the
//! build runs and the code the tests run cannot drift apart. The same arrangement
//! `gnoblin-config-codegen` uses, without a separate crate, because nothing else
//! consumes this tree.
//!
//! The generator refuses three things, because a command line that silently loses
//! a member is worse than a build that fails:
//!
//! - an object whose command name collides with another object's, even after the
//!   namespace qualifier ([`command_name`]);
//! - a schema whose `gnoblin` version is not the version this crate is built as,
//!   so `gnoblinctl` never advertises an IPC version the schema does not serve;
//! - a catalog with no `gnoblin` namespace at all, which would mean the tree has
//!   no version to speak.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use gnoblin_schema::{Catalog, Namespace, ObjectType, Version};

/// The namespace whose version is the IPC version.
const CORE_NAMESPACE: &str = "gnoblin";

/// One object in the tree, as `src/tree.rs` reads it.
struct Entry<'a> {
    command: String,
    /// Owned, because [`Catalog::namespaces`] hands out values that live for one
    /// loop rather than for the catalog.
    namespace: String,
    object: &'a ObjectType,
}

/// Render the whole tree module for a catalog built as `package_version`.
///
/// Every problem found is reported, not just the first, so one build teaches the
/// whole list.
pub fn render(catalog: &Catalog, package_version: &str) -> Result<String, Vec<String>> {
    let mut problems = Vec::new();
    let version = match core_version(catalog) {
        Ok(version) => Some(version),
        Err(problem) => {
            problems.push(problem);
            None
        }
    };
    if let Some(version) = version {
        match package_triple(package_version) {
            Ok(built) if built != version => problems.push(format!(
                "the schema's `{CORE_NAMESPACE}` namespace is IPC {version} but this crate is built as {package_version} ({built}); \
                 the IPC version is exactly the Gnoblin version"
            )),
            Ok(_) => {}
            Err(problem) => problems.push(problem),
        }
    }

    let entries = match entries(catalog) {
        Ok(entries) => entries,
        Err(found) => {
            problems.extend(found);
            Vec::new()
        }
    };
    if !problems.is_empty() {
        return Err(problems);
    }
    let version = version.expect("checked above");
    Ok(module(version, &entries))
}

/// The IPC version, from the schema's `gnoblin` namespace.
fn core_version(catalog: &Catalog) -> Result<Version, String> {
    let namespace = Namespace::parse(CORE_NAMESPACE).expect("the core namespace is a valid name");
    catalog.version(&namespace).ok_or_else(|| {
        format!("the schema declares no `{CORE_NAMESPACE}` namespace, so there is no IPC version to speak")
    })
}

/// The `MAJOR.MINOR.PATCH` this crate is built as, with any pre-release suffix
/// dropped.
///
/// The wire carries three numbers only (`gnoblin_schema::Version`), and a candidate
/// `0.2.0-alpha.N` advertises the triple of the release it becomes, so the suffix
/// is not a mismatch.
fn package_triple(package_version: &str) -> Result<Version, String> {
    let core = package_version
        .split(['-', '+'])
        .next()
        .expect("splitting always yields at least one part");
    Version::parse(core)
        .map_err(|error| format!("this crate's version `{package_version}` is not a release triple: {error}"))
}

/// Every object in the catalog with the command name it is reached by.
fn entries<'a>(catalog: &'a Catalog) -> Result<Vec<Entry<'a>>, Vec<String>> {
    // The files in load order, which `Catalog::load_dir` fixes as file-name order.
    // `Catalog::objects` would tie the namespace reference to the catalog's own
    // lifetime, which the entries below do not need: only the object is borrowed.
    let mut flat: Vec<(String, &'a ObjectType)> = Vec::new();
    for schema in catalog.schemas() {
        for object in &schema.objects {
            flat.push((schema.namespace.to_string(), object));
        }
    }

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, object) in &flat {
        *counts.entry(object.name.as_str()).or_default() += 1;
    }

    let mut problems = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut entries = Vec::with_capacity(flat.len());
    for (namespace, object) in flat {
        let command = command_name(&namespace, &object.name, &counts);
        let qualified = format!("{namespace}.{}", object.name);
        if let Some(other) = seen.insert(command.clone(), qualified.clone()) {
            problems.push(format!(
                "objects `{other}` and `{qualified}` would both be reached as `gnoblinctl {command}`"
            ));
            continue;
        }
        entries.push(Entry {
            command,
            namespace: namespace.to_owned(),
            object,
        });
    }
    if problems.is_empty() {
        Ok(entries)
    } else {
        Err(problems)
    }
}

/// The subcommand name for one object.
///
/// An object name that is unique across the whole catalog is used as it stands, so
/// `gnoblinctl window list` reads like the object it addresses. When two namespaces
/// declare the same object name, both are qualified with the last segment of their
/// namespace (`bsm.greeter` and `foo.greeter` become `greeter-greeter` and
/// `foo-greeter`), which keeps the name derivable from the schema instead of
/// depending on declaration order.
fn command_name(namespace: &str, object: &str, counts: &BTreeMap<&str, usize>) -> String {
    if counts.get(object).copied().unwrap_or(0) <= 1 {
        return object.to_owned();
    }
    let qualifier = namespace.rsplit('.').next().unwrap_or(namespace);
    format!("{qualifier}-{object}")
}

/// The rendered module.
fn module(version: Version, entries: &[Entry<'_>]) -> String {
    let mut text = String::new();
    text.push_str(
        "// Generated by crates/gnoblinctl/build.rs from crates/gnoblin-schema/schema/*.toml.\n\
         // Do not edit: change the schema and rebuild.\n\
         //\n\
         // The tree is data. `src/tree.rs` includes this file inside its own module, so the\n\
         // `CliObject` and friends named here are the types that module declares.\n\n",
    );
    let _ = writeln!(
        text,
        "/// The IPC version this build speaks, from the schema's `{CORE_NAMESPACE}` namespace.\n\
         pub const IPC_VERSION: (u32, u32, u32) = ({}, {}, {});\n",
        version.major, version.minor, version.patch
    );
    text.push_str("/// Every object the schema declares, in namespace then declaration order.\npub const OBJECTS: &[CliObject] = &[\n");
    for entry in entries {
        text.push_str("    CliObject {\n");
        let _ = writeln!(text, "        command: {:?},", entry.command);
        let _ = writeln!(text, "        namespace: {:?},", entry.namespace);
        let _ = writeln!(text, "        name: {:?},", entry.object.name);
        if entry.object.properties.is_empty() {
            text.push_str("        properties: &[],\n");
        } else {
            text.push_str("        properties: &[\n");
            for property in &entry.object.properties {
                text.push_str("            CliProperty {\n");
                let _ = writeln!(text, "                name: {:?},", property.name);
                let _ = writeln!(text, "                ty: {:?},", property.ty.to_string());
                let _ = writeln!(text, "                secret: {},", property.secret);
                text.push_str("            },\n");
            }
            text.push_str("        ],\n");
        }
        if entry.object.methods.is_empty() {
            text.push_str("        methods: &[],\n");
        } else {
            text.push_str("        methods: &[\n");
            for method in &entry.object.methods {
                text.push_str("            CliMethod {\n");
                let _ = writeln!(text, "                name: {:?},", method.name);
                if method.params.is_empty() {
                    text.push_str("                params: &[],\n");
                } else {
                    text.push_str("                params: &[\n");
                    for parameter in &method.params {
                        let _ = writeln!(
                            text,
                            "                    CliParam {{ name: {:?}, ty: {:?}, secret: {} }},",
                            parameter.name,
                            parameter.ty.to_string(),
                            parameter.secret
                        );
                    }
                    text.push_str("                ],\n");
                }
                text.push_str("            },\n");
            }
            text.push_str("        ],\n");
        }
        if entry.object.signals.is_empty() {
            text.push_str("        signals: &[],\n");
        } else {
            text.push_str("        signals: &[\n");
            for signal in &entry.object.signals {
                let _ = writeln!(text, "            CliSignal {{ name: {:?} }},", signal.name);
            }
            text.push_str("        ],\n");
        }
        text.push_str("    },\n");
    }
    text.push_str("];\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../gnoblin-schema/schema");
        Catalog::load_dir(&dir).expect("the checked-in schema loads")
    }

    #[test]
    fn the_checked_in_schema_renders_for_the_built_version() {
        let rendered = render(&catalog(), "0.2.0-alpha.0").expect("the schema and the version agree");
        assert!(rendered.contains("pub const IPC_VERSION: (u32, u32, u32) = (0, 2, 0);"));
        assert!(rendered.contains("command: \"window\","));
    }

    #[test]
    fn a_pre_release_suffix_is_the_release_triple() {
        assert_eq!(package_triple("0.2.0-alpha.0").unwrap(), Version::new(0, 2, 0));
        assert_eq!(package_triple("1.4.2").unwrap(), Version::new(1, 4, 2));
        assert!(package_triple("0.2").is_err());
    }

    #[test]
    fn a_version_the_schema_does_not_serve_is_refused() {
        let problems = render(&catalog(), "0.3.0").expect_err("the schema is 0.2.0");
        assert!(
            problems[0].contains("is IPC 0.2.0 but this crate is built as 0.3.0"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_shared_object_name_is_qualified_by_its_namespace() {
        let mut counts = BTreeMap::new();
        counts.insert("greeter", 2);
        assert_eq!(command_name("bsm.greeter", "greeter", &counts), "greeter-greeter");
        assert_eq!(command_name("foo.greeter", "greeter", &counts), "foo-greeter");
        let mut unique = BTreeMap::new();
        unique.insert("window", 1);
        assert_eq!(command_name("gnoblin", "window", &unique), "window");
    }
}
