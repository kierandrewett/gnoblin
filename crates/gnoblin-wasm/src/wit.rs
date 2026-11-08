//! Render the object schema as a WIT package, and the member table its events use.
//!
//! This file is compiled twice: into `build.rs`, which writes the rendered text,
//! and into nothing else. The generated WIT is the host API a module links
//! against, so it must come from the schema and never from a second hand-written
//! copy (design/rust-compositor.md, D6: "The host API is generated from
//! `gnoblin-schema` as a WIT interface, so modules see the same objects as Lua and
//! shells"). `tests/generated_wit.rs` then parses the generated text and compares
//! it with the schema member by member, which is the check that catches a
//! generator that quietly stops emitting something.
//!
//! # Shape
//!
//! One interface per schema namespace, named after the namespace with its dots
//! turned into dashes (`gnoblin` stays `gnoblin`, `example.ext` becomes
//! `example-ext`). One interface per namespace and not one interface in total,
//! because two namespaces may declare the same object name and putting both in
//! one interface would make the WIT ill-formed; the schema catalog permits the
//! duplicate, so the generator has to as well.
//!
//! Each object type becomes a resource: its properties become getters, its methods
//! return `result<operation, error-code>` (an operation handle, matching the
//! object model's "a method returns an `Operation`"), and `watch` is the
//! subscription that turns its changes and signals into events. Each namespace's
//! interface also carries `<object>-by-id` for every object type, which is how a
//! module turns an identity it saw in an event into a handle it can call.
//!
//! # The two worlds
//!
//! A module imports every namespace interface and exports two interfaces:
//! `lifecycle` (`start`, `stop`) and `callbacks` (`on-event`). The values a module
//! is told about travel as words, not types: `on-event` carries an event kind, an
//! object identity, a member index and an argument. The indices are the table
//! below, which is why the same generator emits it as Rust constants; an exported
//! interface's types have to be declared by the module itself, so a typed event
//! would force every module, in every language, to re-declare the schema's enums
//! and keep them in step. The words are stable because the table is generated from
//! the schema and checked by the same test.
//!
//! # Failures
//!
//! The generator refuses rather than emits a package it knows is wrong: a member
//! whose kebab-case form is not a WIT identifier, or is a WIT keyword, is reported
//! with the schema file that declared it (`render` returns [`GeneratorError`]). Emitting
//! it would produce a WIT file whose parse error names no schema member.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use gnoblin_schema::{Catalog, Field, Method, Namespace, ObjectType, Property, Type};

/// The package name every generated interface lives under.
pub const PACKAGE: &str = "gnoblin:ipc";

/// Error codes the host adds to every namespace, on top of the codes the schema
/// declares. They describe a refusal at the seam, not a fact about the model:
/// `denied` is a capability the module's manifest did not grant, `unknown-object`
/// an identity the module's mirror does not have, and `refused` a refusal the
/// compositor reported with a code this build does not know.
pub const HOST_ERRORS: &[&str] = &["denied", "refused", "unknown-object", "unknown-member"];

/// Words the generator must not turn into a WIT identifier unescaped.
///
/// The list is the WIT grammar's reserved words. A schema member that lands on one
/// of them is reported instead of renamed, because the generated API's names are
/// the schema's names and a silent rename would make the WIT disagree with the
/// schema it was generated from.
const KEYWORDS: &[&str] = &[
    "as",
    "async",
    "bool",
    "borrow",
    "char",
    "constructor",
    "enum",
    "export",
    "f32",
    "f64",
    "flags",
    "from",
    "func",
    "future",
    "import",
    "include",
    "interface",
    "list",
    "option",
    "own",
    "package",
    "record",
    "resource",
    "result",
    "s16",
    "s32",
    "s64",
    "s8",
    "static",
    "stream",
    "string",
    "tuple",
    "type",
    "u16",
    "u32",
    "u64",
    "u8",
    "use",
    "variant",
    "with",
    "world",
];

/// What one generation produced.
pub struct Rendered {
    /// The WIT package text.
    pub wit: String,
    /// The Rust member table, one row per property and signal in the schema.
    pub members: String,
}

/// One problem that stopped generation, worded for the build log.
pub struct GeneratorError {
    /// The schema file or member the problem is about.
    pub origin: String,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for GeneratorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.origin, self.message)
    }
}

/// Render the WIT package at `version` and the member table for the same catalog.
pub fn render(catalog: &Catalog, version: &str) -> Result<Rendered, GeneratorError> {
    check_version(version)?;
    let mut wit = String::new();
    let _ = writeln!(wit, "/// The IPC object model as a component-model interface.");
    let _ = writeln!(wit, "///");
    let _ = writeln!(wit, "/// Generated from the object schema. Do not edit.");
    let _ = writeln!(wit, "package {PACKAGE}@{version};");

    let namespaces = catalog.namespaces();
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for namespace in &namespaces {
        for object in catalog.objects(namespace) {
            if let Some(other) = seen.insert(object.name.as_str(), namespace.as_str())
                && other != namespace.as_str()
            {
                return Err(GeneratorError {
                    origin: format!("schema namespace `{namespace}`"),
                    message: format!(
                        "object type `{}` is already declared by namespace `{other}`; a module's \
                         capabilities and events name an object type without its namespace, so two \
                         namespaces cannot declare the same one",
                        object.name
                    ),
                });
            }
        }
        interface(&mut wit, catalog, namespace)?;
    }
    worlds(&mut wit, &namespaces);
    Ok(Rendered {
        wit,
        members: member_table(catalog),
    })
}

/// One namespace's interface: its error codes, the operation handle, one resource
/// per object type, and the lookups that turn an identity into a handle.
fn interface(wit: &mut String, catalog: &Catalog, namespace: &Namespace) -> Result<(), GeneratorError> {
    let name = interface_name(namespace);
    let _ = writeln!(wit);
    let _ = writeln!(wit, "/// The object types of the `{namespace}` namespace.");
    let _ = writeln!(wit, "interface {name} {{");

    let mut errors: Vec<String> = HOST_ERRORS.iter().map(|code| (*code).to_owned()).collect();
    for error in catalog.errors(namespace) {
        errors.push(error.name.clone());
    }
    errors.sort();
    errors.dedup();
    let _ = writeln!(wit, "    /// Why a call was refused.");
    let _ = writeln!(wit, "    enum error-code {{");
    for code in &errors {
        let case = identifier(code, "error code", namespace.as_str())?;
        let _ = writeln!(wit, "        {case},");
    }
    let _ = writeln!(wit, "    }}");

    let objects: Vec<&ObjectType> = catalog.objects(namespace).collect();
    for object in &objects {
        resource(wit, object, namespace.as_str())?;
    }

    for object in &objects {
        let resource = identifier(&object.name, "object type", namespace.as_str())?;
        let _ = writeln!(wit);
        let _ = writeln!(
            wit,
            "    /// The `{}` object with that identity, if this instance has it.",
            object.name
        );
        let _ = writeln!(
            wit,
            "    {resource}-by-id: func(id: u64) -> result<{resource}, error-code>;"
        );
    }

    let _ = writeln!(wit, "}}");
    Ok(())
}

/// One object type as a resource.
fn resource(wit: &mut String, object: &ObjectType, namespace: &str) -> Result<(), GeneratorError> {
    let resource = identifier(&object.name, "object type", namespace)?;
    let _ = writeln!(wit);
    let _ = writeln!(wit, "    /// The `{}` object.", object.name);
    let _ = writeln!(wit, "    resource {resource} {{");
    let _ = writeln!(wit, "        /// The object's identity in the compositor's graph.");
    let _ = writeln!(wit, "        object-id: func() -> u64;");
    for declared in &object.properties {
        property(wit, declared, namespace)?;
    }
    for method in &object.methods {
        method_signature(wit, method, namespace)?;
    }
    let _ = writeln!(wit, "        /// Deliver this object's changes, signals and end to");
    let _ = writeln!(
        wit,
        "        /// `on-event` until the host stops the module, answering whether"
    );
    let _ = writeln!(wit, "        /// this module now watches it.");
    let _ = writeln!(wit, "        watch: func() -> result<bool, error-code>;");
    let _ = writeln!(wit, "    }}");
    Ok(())
}

/// One property as a getter.
fn property(wit: &mut String, declared: &Property, namespace: &str) -> Result<(), GeneratorError> {
    let getter = identifier(&declared.name, "property", namespace)?;
    let ty = wit_type(&declared.ty);
    let _ = writeln!(
        wit,
        "        /// The `{}` property, as of the last event.",
        declared.name
    );
    let _ = writeln!(wit, "        {getter}: func() -> {ty};");
    Ok(())
}

/// One method: its declared parameters and the operation handle it returns.
fn method_signature(wit: &mut String, method: &Method, namespace: &str) -> Result<(), GeneratorError> {
    let name = identifier(&method.name, "method", namespace)?;
    let mut parameters = String::new();
    for (index, parameter) in method.params.iter().enumerate() {
        if index > 0 {
            parameters.push_str(", ");
        }
        let field = field(parameter, namespace)?;
        parameters.push_str(&field);
    }
    let _ = writeln!(
        wit,
        "        /// Calls `{}`, returning the operation its completion arrives for.",
        method.name
    );
    if parameters.is_empty() {
        let _ = writeln!(wit, "        {name}: func() -> result<operation, error-code>;");
    } else {
        let _ = writeln!(
            wit,
            "        {name}: func({parameters}) -> result<operation, error-code>;"
        );
    }
    Ok(())
}

/// A method parameter or a signal field.
fn field(field: &Field, namespace: &str) -> Result<String, GeneratorError> {
    let name = identifier(&field.name, "field", namespace)?;
    Ok(format!("{name}: {}", wit_type(&field.ty)))
}

/// The interfaces a module exports, and the world that ties the two directions
/// together.
fn worlds(wit: &mut String, namespaces: &[Namespace]) {
    let _ = writeln!(wit);
    let _ = writeln!(wit, "/// What the host calls, and when.");
    let _ = writeln!(wit, "interface lifecycle {{");
    let _ = writeln!(wit, "    /// Called once when the host starts the module.");
    let _ = writeln!(wit, "    start: func();");
    let _ = writeln!(wit, "    /// Called once when the host stops it, before it is dropped.");
    let _ = writeln!(wit, "    stop: func();");
    let _ = writeln!(wit, "}}");

    let _ = writeln!(wit);
    let _ = writeln!(wit, "/// What the host tells the module.");
    let _ = writeln!(wit, "interface callbacks {{");
    let _ = writeln!(wit, "    /// One observation the module asked for with `watch`.");
    let _ = writeln!(wit, "    ///");
    let _ = writeln!(wit, "    /// `kind` is an event kind word (see the generated");
    let _ = writeln!(wit, "    /// `MEMBERS` table: opened, changed, signalled, closed,");
    let _ = writeln!(
        wit,
        "    /// operation-finished), `object` the identity the event is about,"
    );
    let _ = writeln!(
        wit,
        "    /// `member` the index of the property or signal the event names (0 for"
    );
    let _ = writeln!(
        wit,
        "    /// the kinds that name none) and `argument` the operation identity for"
    );
    let _ = writeln!(wit, "    /// `operation-finished` (0 otherwise).");
    let _ = writeln!(
        wit,
        "    on-event: func(kind: u32, object: u64, member: u32, argument: u64);"
    );
    let _ = writeln!(wit, "}}");

    let _ = writeln!(wit);
    let _ = writeln!(wit, "/// The module the host runs.");
    let _ = writeln!(wit, "world module {{");
    for namespace in namespaces {
        let _ = writeln!(wit, "    import {};", interface_name(namespace));
    }
    let _ = writeln!(wit, "    export lifecycle;");
    let _ = writeln!(wit, "    export callbacks;");
    let _ = writeln!(wit, "}}");
}

/// The member table: every property and signal, with the index an event carries.
///
/// Properties and signals share one index space per schema, because an event's
/// kind is what says which of the two it is and one space keeps the callback to a
/// single word. The order is the catalog's (namespace, object, member as declared),
/// so an index never moves when a member is added.
fn member_table(catalog: &Catalog) -> String {
    let mut rows = String::new();
    let mut constants = String::new();
    let mut index = 0u32;
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            let mut members: Vec<(&str, &str)> = Vec::new();
            for property in &object.properties {
                members.push((&property.name, "Property"));
            }
            for signal in &object.signals {
                members.push((&signal.name, "Signal"));
            }
            for (name, kind) in members {
                let _ = writeln!(
                    rows,
                    "    EventMember {{ object: \"{}\", name: \"{name}\", index: {index}, kind: MemberKind::{kind} }},",
                    object.name
                );
                let _ = writeln!(
                    constants,
                    "/// The `{name}` {} of the `{}` object.",
                    if kind == "Property" { "property" } else { "signal" },
                    object.name
                );
                let _ = writeln!(
                    constants,
                    "pub const {}_{}: u32 = {index};",
                    constant_name(&object.name),
                    constant_name(name)
                );
                index += 1;
            }
        }
    }
    let mut members = constants;
    let _ = writeln!(
        members,
        "/// One member an event can name, indexed as the WIT package does."
    );
    let _ = writeln!(members, "pub const MEMBERS: &[EventMember] = &[");
    members.push_str(&rows);
    let _ = writeln!(members, "];");
    let _ = writeln!(members);
    let _ = writeln!(members, "/// Every object type the generated package exposes.");
    let _ = writeln!(members, "pub const OBJECT_TYPES: &[&str] = &[");
    for object in object_types(catalog) {
        let _ = writeln!(members, "    \"{object}\",");
    }
    let _ = writeln!(members, "];");
    let _ = writeln!(members);
    let _ = writeln!(
        members,
        "/// Every method the generated package exposes, by object type."
    );
    let _ = writeln!(members, "pub const CALLABLE: &[(&str, &str)] = &[");
    for (object, method) in callable(catalog) {
        let _ = writeln!(members, "    (\"{object}\", \"{method}\"),");
    }
    let _ = writeln!(members, "];");
    members
}

/// Every object type name the package declares, sorted.
fn object_types(catalog: &Catalog) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            names.push(object.name.clone());
        }
    }
    names.sort();
    names
}

/// Every (object type, method) pair the package declares, sorted.
fn callable(catalog: &Catalog) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            for method in &object.methods {
                pairs.push((object.name.clone(), method.name.clone()));
            }
        }
    }
    pairs.sort();
    pairs
}

/// Refuse a version WIT would not parse.
///
/// The package line is `package gnoblin:ipc@<version>;`, and WIT wants semver
/// there. Checking here means a bad version is a build error naming the version
/// rather than a parse error in a file nobody wrote by hand.
fn check_version(version: &str) -> Result<(), GeneratorError> {
    let (release, suffix) = match version.split_once(['-', '+']) {
        Some((release, suffix)) => (release, Some(suffix)),
        None => (version, None),
    };
    let numbers: Vec<&str> = release.split('.').collect();
    let numbers_ok = numbers.len() == 3
        && numbers
            .iter()
            .all(|part| !part.is_empty() && part.len() < 10 && part.bytes().all(|byte| byte.is_ascii_digit()));
    let suffix_ok = suffix.is_none_or(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    });
    if numbers_ok && suffix_ok {
        Ok(())
    } else {
        Err(GeneratorError {
            origin: "crate version".to_owned(),
            message: format!("`{version}` is not a semver version WIT accepts"),
        })
    }
}

/// The interface name a namespace becomes: dots turned into dashes.
pub fn interface_name(namespace: &Namespace) -> String {
    namespace.as_str().replace('.', "-")
}

/// The uppercase constant name a schema member becomes.
fn constant_name(name: &str) -> String {
    name.split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_uppercase())
        .collect::<Vec<String>>()
        .join("_")
}

/// A schema name as a WIT identifier: `app_id` is `app-id`.
fn identifier(name: &str, kind: &str, namespace: &str) -> Result<String, GeneratorError> {
    let kebab: String = name.replace('_', "-");
    let valid = name.starts_with(|first: char| first.is_ascii_lowercase())
        && kebab
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-')
        && !kebab.ends_with('-')
        && !kebab.contains("--");
    if !valid {
        return Err(GeneratorError {
            origin: format!("schema namespace `{namespace}`"),
            message: format!("{kind} `{name}` is not a WIT identifier once kebab-cased as `{kebab}`"),
        });
    }
    if KEYWORDS.contains(&kebab.as_str()) {
        return Err(GeneratorError {
            origin: format!("schema namespace `{namespace}`"),
            message: format!("{kind} `{name}` is the WIT keyword `{kebab}`; rename the schema member"),
        });
    }
    Ok(kebab)
}

/// A schema type as a WIT type.
///
/// An object reference becomes the referenced object's identity, not a resource
/// handle: a property that names another object has to be readable without handing
/// the module ownership of it, and a handle would make a read of one object change
/// another's lifetime. The module resolves the identity with `<object>-by-id` when
/// it has the capability to.
fn wit_type(ty: &Type) -> String {
    match ty {
        Type::Bool => "bool".to_owned(),
        Type::Int => "s64".to_owned(),
        Type::Uint => "u64".to_owned(),
        Type::Double => "f64".to_owned(),
        Type::Text => "string".to_owned(),
        Type::Bytes => "list<u8>".to_owned(),
        Type::ObjectRef(_) => "u64".to_owned(),
        Type::List(inner) => format!("list<{}>", wit_type(inner)),
    }
}

#[cfg(test)]
mod tests {
    use super::{check_version, constant_name, identifier, interface_name, wit_type};
    use gnoblin_schema::{Namespace, Type};

    #[test]
    fn kebab_cases_schema_names() {
        assert_eq!(
            identifier("app_id", "property", "gnoblin").expect("identifier"),
            "app-id"
        );
        assert_eq!(identifier("id", "property", "gnoblin").expect("identifier"), "id");
        assert!(identifier("App", "property", "gnoblin").is_err());
        assert!(identifier("type", "property", "gnoblin").is_err());
        assert!(identifier("app__id", "property", "gnoblin").is_err());
    }

    #[test]
    fn names_interfaces_after_namespaces() {
        assert_eq!(
            interface_name(&Namespace::parse("gnoblin").expect("namespace")),
            "gnoblin"
        );
        assert_eq!(
            interface_name(&Namespace::parse("example.ext").expect("namespace")),
            "example-ext"
        );
    }

    #[test]
    fn maps_schema_types() {
        assert_eq!(wit_type(&Type::Bytes), "list<u8>");
        assert_eq!(wit_type(&Type::List(Box::new(Type::Text))), "list<string>");
        assert_eq!(wit_type(&Type::ObjectRef("window".to_owned())), "u64");
    }

    #[test]
    fn checks_versions() {
        for version in ["0.2.0", "0.2.0-alpha.0", "1.0.0-rc.1"] {
            assert!(check_version(version).is_ok(), "{version}");
        }
        for version in ["0.2", "0.2.0.0", "v0.2.0", "0.2.0-", "0.2.x"] {
            assert!(check_version(version).is_err(), "{version}");
        }
    }

    #[test]
    fn names_constants() {
        assert_eq!(constant_name("app_id"), "APP_ID");
        assert_eq!(constant_name("focused"), "FOCUSED");
    }
}
