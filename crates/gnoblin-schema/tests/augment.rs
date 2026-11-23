//! `[[augment]]`: members a library adds to a class of another library's catalog.
//!
//! The std catalog adds `workspaces`, `displays`, `scene`, `gestures` and `requestTransition` to the core's `gnoblin` object, and the core
//! catalog never reads the std files. The catalog refuses an augment whose target the core does not have, and one whose member the target
//! already has, each with the names in the message; it lowers the members of an augment into one service class of the augmenting
//! namespace, which is what its library implements; and it leaves the core namespace exactly as it was.

use gnoblin_schema::{Catalog, ObjectKind, SchemaError, parse_schema};

/// The core catalog of the tests: the root `gnoblin` with a property, a method and a signal, and a handle.
const CORE: &str = r#"namespace = "gnoblin.model"
version = "0.2.0"
local = true

[[object]]
name = "gnoblin"
since = "0.2.0"
kind = "service"

[[object.property]]
name = "clients"
type = "list<string>"
since = "0.2.0"
readonly = true

[[object.method]]
name = "version"
since = "0.2.0"
returns = "string"

[[object.signal]]
name = "client_created"
since = "0.2.0"

[[object]]
name = "output_collection"
since = "0.2.0"
kind = "handle"

[[object.property]]
name = "length"
type = "uint"
since = "0.2.0"
readonly = true

[[object]]
name = "rect"
since = "0.2.0"
kind = "record"

[[object.property]]
name = "x"
type = "double"
since = "0.2.0"
default = 0.0
"#;

/// A std catalog: a service `workspaces`, and the augments `augments` writes.
fn std_with(augments: &str) -> String {
    format!(
        r#"namespace = "gnoblin.std"
version = "0.2.0"
local = true

[[object]]
name = "workspaces"
since = "0.2.0"
kind = "service"

[[object.property]]
name = "length"
type = "uint"
since = "0.2.0"
readonly = true

[[object]]
name = "profiles"
since = "0.2.0"
kind = "service"

[[callback]]
name = "work"
since = "0.2.0"
{augments}"#
    )
}

const WORKSPACES: &str = r#"
[[augment]]
target = "core.gnoblin"
since = "0.2.0"

[[augment.property]]
name = "workspaces"
type = "object:workspaces"
since = "0.2.0"
readonly = true

[[augment.method]]
name = "request"
since = "0.2.0"
returns = "bool"

[[augment.method.param]]
name = "run"
type = "callback:work"
"#;

fn catalog(files: &[(&str, &str)]) -> Result<Catalog, SchemaError> {
    let mut schemas = Vec::new();
    for (origin, text) in files {
        schemas.push(parse_schema(origin, text)?);
    }
    Catalog::new(schemas)
}

/// The problems a set of files is refused with.
fn refused(files: &[(&str, &str)]) -> Vec<String> {
    match catalog(files) {
        Err(SchemaError::Invalid { problems, .. }) => problems,
        Err(other) => vec![other.to_string()],
        Ok(_) => panic!("the files should be refused"),
    }
}

fn namespace(catalog: &Catalog, name: &str) -> gnoblin_schema::Namespace {
    catalog
        .namespaces()
        .into_iter()
        .find(|candidate| candidate.as_str() == name)
        .unwrap_or_else(|| panic!("no namespace {name}"))
}

/// The members of an augment are one service class of the std namespace, named after its target, and the core's class has none of them.
#[test]
fn an_augment_is_a_service_class_of_the_augmenting_namespace() {
    let std = std_with(WORKSPACES);
    let catalog = catalog(&[("core.toml", CORE), ("std.toml", &std)]).expect("the catalog loads");
    let std_ns = namespace(&catalog, "gnoblin.std");
    let class = catalog
        .objects(&std_ns)
        .find(|object| object.name == "gnoblin_augment")
        .expect("the members of the augment are a class");
    assert_eq!(class.kind, ObjectKind::Service);
    assert_eq!(class.augment_of.as_deref(), Some("gnoblin"));
    assert_eq!(
        class
            .properties
            .iter()
            .map(|property| property.name.as_str())
            .collect::<Vec<_>>(),
        ["workspaces"]
    );
    assert_eq!(
        class
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>(),
        ["request"]
    );
    // The core's class is what its own file says.
    let core_ns = namespace(&catalog, "gnoblin.model");
    let root = catalog
        .objects(&core_ns)
        .find(|object| object.name == "gnoblin")
        .expect("the root");
    assert_eq!(root.properties.len(), 1);
    assert_eq!(root.methods.len(), 1);
    assert!(root.augment_of.is_none());
}

/// The core catalog is the same bytes of catalog with and without the std file: nothing of an augment is in the core's fingerprint, its
/// classes or its enums.
#[test]
fn the_core_namespace_does_not_see_the_augment() {
    let std = std_with(WORKSPACES);
    let with = catalog(&[("core.toml", CORE), ("std.toml", &std)]).expect("with std");
    let without = catalog(&[("core.toml", CORE)]).expect("without std");
    let core_ns = namespace(&with, "gnoblin.model");
    assert_eq!(
        with.namespace_fingerprint(&core_ns),
        without.namespace_fingerprint(&namespace(&without, "gnoblin.model")),
        "the core's fingerprint does not move"
    );
    let names = |catalog: &Catalog| {
        catalog
            .objects(&namespace(catalog, "gnoblin.model"))
            .map(|object| object.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&with), names(&without));
}

/// The std fingerprint says what the std library serves, so a member added to an augment moves it.
#[test]
fn the_std_fingerprint_follows_the_augment() {
    let one = std_with(WORKSPACES);
    let two = std_with(&format!(
        "{WORKSPACES}\n[[augment.method]]\nname = \"extra\"\nsince = \"0.2.0\"\n"
    ));
    let first = catalog(&[("core.toml", CORE), ("std.toml", &one)]).expect("one");
    let second = catalog(&[("core.toml", CORE), ("std.toml", &two)]).expect("two");
    assert_ne!(
        first.namespace_fingerprint(&namespace(&first, "gnoblin.std")),
        second.namespace_fingerprint(&namespace(&second, "gnoblin.std"))
    );
}

/// Failure mode: the target is not in the core catalog. The message names the target.
#[test]
fn a_target_the_core_lacks_is_refused_with_its_name() {
    let std = std_with(&WORKSPACES.replace("core.gnoblin", "core.no_such_class"));
    let problems = refused(&[("core.toml", CORE), ("std.toml", &std)]);
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("core.no_such_class") && problem.contains("std.toml")),
        "{problems:?}"
    );
}

/// A target that is not a class of `core.` at all, and one that is not a class that has members, are refused too.
#[test]
fn a_target_must_be_a_core_class_that_takes_members() {
    let elsewhere = std_with(&WORKSPACES.replace("core.gnoblin", "gnoblin.std.workspaces"));
    let problems = refused(&[("core.toml", CORE), ("std.toml", &elsewhere)]);
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("gnoblin.std.workspaces")),
        "{problems:?}"
    );
    let record = std_with(&WORKSPACES.replace("core.gnoblin", "core.rect"));
    let problems = refused(&[("core.toml", CORE), ("std.toml", &record)]);
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("core.rect") && problem.contains("record")),
        "{problems:?}"
    );
}

/// Failure mode: a member the target already has. The message names the augment's member and the core's.
#[test]
fn a_member_that_clashes_with_a_core_member_is_refused_with_both_names() {
    for (clash, kind) in [
        ("clients", "property"),
        ("version", "method"),
        ("client_created", "signal"),
    ] {
        let std = std_with(&WORKSPACES.replace("\"workspaces\"", &format!("\"{clash}\"")));
        let problems = refused(&[("core.toml", CORE), ("std.toml", &std)]);
        assert!(
            problems.iter().any(|problem| problem.contains(clash)
                && problem.contains("core.gnoblin")
                && problem.contains("std.toml")
                && problem.contains("core.toml")
                && problem.contains(kind)),
            "{kind} `{clash}`: {problems:?}"
        );
    }
}

/// The same name from two augments of one target is a clash too, naming both files.
#[test]
fn two_augments_may_not_declare_one_member() {
    let first = std_with(WORKSPACES);
    let second = r#"namespace = "gnoblin.std"
version = "0.2.0"
local = true

[[augment]]
target = "core.gnoblin"
since = "0.2.0"

[[augment.property]]
name = "workspaces"
type = "object:workspaces"
since = "0.2.0"
readonly = true
"#;
    let problems = refused(&[("core.toml", CORE), ("a.toml", &first), ("b.toml", second)]);
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("workspaces") && problem.contains("a.toml") && problem.contains("b.toml")),
        "{problems:?}"
    );
}

/// Augments of one target in several files are one class; of two targets, two classes.
#[test]
fn augments_of_one_target_join_in_one_class() {
    let first = std_with(WORKSPACES);
    let second = r#"namespace = "gnoblin.std"
version = "0.2.0"
local = true

[[augment]]
target = "core.gnoblin"
since = "0.2.0"

[[augment.property]]
name = "profiles"
type = "object:profiles"
since = "0.2.0"
readonly = true

[[augment]]
target = "core.output_collection"
since = "0.2.0"

[[augment.property]]
name = "profiles"
type = "object:profiles"
since = "0.2.0"
readonly = true
"#;
    let catalog = catalog(&[("core.toml", CORE), ("a.toml", &first), ("b.toml", second)]).expect("loads");
    let std_ns = namespace(&catalog, "gnoblin.std");
    let augments: Vec<_> = catalog
        .objects(&std_ns)
        .filter(|object| object.augment_of.is_some())
        .map(|object| {
            (
                object.name.clone(),
                object.properties.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        augments,
        [
            (
                "gnoblin_augment".to_owned(),
                vec!["workspaces".to_owned(), "profiles".to_owned()]
            ),
            ("output_collection_augment".to_owned(), vec!["profiles".to_owned()]),
        ]
    );
}

/// A member of an augment names types of the augmenting namespace that exist.
#[test]
fn an_augment_member_must_name_a_class_that_exists() {
    let std = std_with(&WORKSPACES.replace("object:workspaces", "object:no_such_class"));
    let problems = refused(&[("core.toml", CORE), ("std.toml", &std)]);
    assert!(
        problems.iter().any(|problem| problem.contains("no_such_class")),
        "{problems:?}"
    );
}

/// An augment is a key of a namespace a library serves; a file of the compositor's schema may not use it.
#[test]
fn only_a_local_namespace_augments() {
    let text = r#"namespace = "gnoblin"
version = "0.2.0"

[[augment]]
target = "core.gnoblin"
since = "0.2.0"
"#;
    match parse_schema("plain.toml", text) {
        Err(SchemaError::Invalid { problems, .. }) => assert!(
            problems.iter().any(|problem| problem.contains("local = true")),
            "{problems:?}"
        ),
        other => panic!("a plain namespace has no augment: {other:?}"),
    }
}

/// An augment is read-only: the members are served by the library, and nothing in a language writes them.
#[test]
fn an_augment_property_is_read_only_and_a_method_is_not_a_function() {
    let writable = std_with(&WORKSPACES.replace("readonly = true", "readonly = false"));
    let problems = refused(&[("core.toml", CORE), ("std.toml", &writable)]);
    assert!(
        problems.iter().any(|problem| problem.contains("read-only")),
        "{problems:?}"
    );
    let function = std_with(&WORKSPACES.replace(
        "name = \"request\"\nsince = \"0.2.0\"\n",
        "name = \"request\"\nsince = \"0.2.0\"\nfunction = true\n",
    ));
    let problems = refused(&[("core.toml", CORE), ("std.toml", &function)]);
    assert!(
        problems.iter().any(|problem| problem.contains("function")),
        "{problems:?}"
    );
}
