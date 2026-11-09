//! The `extensions` section of host configuration, end to end.
//!
//! Each test builds a configuration document, validates it through
//! [`Config::from_value`] (the config load path), and reads its `extensions`
//! section with [`load_extensions`]. Each named failure mode refuses only the entry
//! that has it, with a reason that names the check.

mod support;

use std::path::{Path, PathBuf};

use gnoblin_config::extensions::{LoadedExtensions, load_extensions};
use gnoblin_config::{Config, Value};
use gnoblin_schema::{Catalog, Schema, parse_schema};

use support::{flag, list, table, text};

const EXAMPLE_NAMESPACE: &str = "example.ext";

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/extensions"))
}

/// The schema the extension package ships. It lives with the schema crate's fixtures.
fn example_schema_path() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/tests/fixtures/extensions/example-ext.toml"
    )
    .to_owned()
}

fn stock() -> Vec<Schema> {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../gnoblin-schema/schema"));
    Catalog::load_dir(dir)
        .expect("the stock catalog loads")
        .declared_schemas()
        .to_vec()
}

fn entry(namespace: &str, schema: &str, provider: &str, grants: Vec<Value>) -> Value {
    table([
        ("namespace", text(namespace)),
        ("schema", text(schema)),
        ("provider", text(provider)),
        ("grants", list(grants)),
    ])
}

fn document(entries: Vec<Value>) -> Value {
    table([("extensions", list(entries))])
}

/// Validate the document through the config load path, then read its extensions
/// against the given stock catalog.
fn load_with(base: &Path, stock: &[Schema], document: Value) -> LoadedExtensions {
    let config = Config::from_value(document).expect("the config accepts the extensions section");
    load_extensions(config.document(), base, stock)
}

fn load(document: Value) -> LoadedExtensions {
    load_with(&fixture_dir(), &stock(), document)
}

fn refusal_reason<'a>(loaded: &'a LoadedExtensions, namespace: &str) -> &'a str {
    &loaded
        .refused
        .iter()
        .find(|refusal| refusal.namespace == namespace)
        .unwrap_or_else(|| panic!("`{namespace}` is refused; refused: {:?}", loaded.refused))
        .reason
}

fn assert_refused_with(loaded: &LoadedExtensions, namespace: &str, needle: &str) {
    let reason = refusal_reason(loaded, namespace);
    assert!(
        reason.contains(needle),
        "`{namespace}` refused for `{reason}`, expected `{needle}`"
    );
}

#[test]
fn a_valid_entry_loads_with_its_schema_provider_and_grants() {
    let loaded = load(document(vec![entry(
        EXAMPLE_NAMESPACE,
        &example_schema_path(),
        "example",
        vec![text("panel"), text("launcher")],
    )]));
    assert!(loaded.refused.is_empty(), "nothing is refused: {:?}", loaded.refused);
    let [accepted] = loaded.accepted.as_slice() else {
        panic!("one extension loads, found {}", loaded.accepted.len());
    };
    assert_eq!(accepted.namespace, EXAMPLE_NAMESPACE);
    assert_eq!(accepted.provider, "example");
    assert_eq!(
        accepted.grants,
        ["panel", "launcher"],
        "grants keep configuration order"
    );
    assert_eq!(accepted.schema.namespace.as_str(), EXAMPLE_NAMESPACE);
    let objects: Vec<&str> = accepted
        .schema
        .objects
        .iter()
        .map(|object| object.name.as_str())
        .collect();
    assert_eq!(objects, ["probe", "counter"]);
}

#[test]
fn a_relative_schema_path_resolves_against_the_configuration_directory() {
    let base = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/tests/fixtures/extensions"
    ));
    let loaded = load_with(
        base,
        &stock(),
        document(vec![entry(
            EXAMPLE_NAMESPACE,
            "example-ext.toml",
            "example",
            vec![text("panel")],
        )]),
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.accepted.len(), 1);
    assert_eq!(loaded.accepted[0].schema_path, base.join("example-ext.toml"));
}

#[test]
fn an_empty_grant_list_is_accepted() {
    let loaded = load(document(vec![entry(
        EXAMPLE_NAMESPACE,
        &example_schema_path(),
        "example",
        vec![],
    )]));
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.accepted.len(), 1);
    assert!(
        loaded.accepted[0].grants.is_empty(),
        "no socket is granted the namespace"
    );
}

#[test]
fn a_document_without_an_extensions_section_loads_and_refuses_nothing() {
    let config = Config::from_value(table([])).expect("an empty document is valid");
    let loaded = load_extensions(config.document(), &fixture_dir(), &stock());
    assert!(loaded.accepted.is_empty());
    assert!(loaded.refused.is_empty());
}

#[test]
fn a_missing_schema_file_refuses_only_that_extension() {
    let loaded = load(document(vec![
        entry("example.missing", "does-not-exist.toml", "example", vec![]),
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("panel")],
        ),
    ]));
    assert_refused_with(&loaded, "example.missing", "cannot read the schema");
    assert_eq!(loaded.accepted.len(), 1, "the other extension still loads");
    assert_eq!(loaded.accepted[0].namespace, EXAMPLE_NAMESPACE);
}

#[test]
fn a_malformed_schema_file_is_refused_with_its_parse_error() {
    let loaded = load(document(vec![entry(
        "example.broken",
        "malformed.toml",
        "example",
        vec![],
    )]));
    assert_refused_with(&loaded, "example.broken", "the schema does not parse");
    assert!(loaded.accepted.is_empty());
}

#[test]
fn a_schema_declaring_another_namespace_is_refused() {
    let loaded = load(document(vec![entry(
        "example.other",
        &example_schema_path(),
        "example",
        vec![],
    )]));
    assert_refused_with(
        &loaded,
        "example.other",
        "declares namespace `example.ext`, not `example.other`",
    );
}

#[test]
fn a_namespace_listed_twice_is_refused_on_the_second_listing() {
    let loaded = load(document(vec![
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("panel")],
        ),
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("launcher")],
        ),
    ]));
    assert_eq!(loaded.accepted.len(), 1, "the first listing loads");
    assert_eq!(loaded.accepted[0].grants, ["panel"]);
    assert_refused_with(&loaded, EXAMPLE_NAMESPACE, "listed more than once");
}

#[test]
fn the_core_namespace_cannot_be_claimed() {
    let loaded = load(document(vec![entry(
        "gnoblin",
        &example_schema_path(),
        "example",
        vec![],
    )]));
    assert_refused_with(&loaded, "gnoblin", "is the core namespace");
    assert!(loaded.accepted.is_empty());
}

#[test]
fn a_namespace_the_stock_catalog_owns_cannot_be_claimed() {
    // The stock catalog is a parameter, so this case supplies a stock schema that
    // owns `example.ext`, the namespace the configuration then tries to claim.
    let mut catalog = stock();
    let owned = parse_schema(
        &example_schema_path(),
        &std::fs::read_to_string(example_schema_path()).expect("the fixture reads"),
    )
    .expect("the fixture parses");
    catalog.push(owned);
    let loaded = load_with(
        &fixture_dir(),
        &catalog,
        document(vec![entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![],
        )]),
    );
    assert_refused_with(&loaded, EXAMPLE_NAMESPACE, "belongs to the stock catalog");
    assert!(loaded.accepted.is_empty());
}

#[test]
fn an_empty_namespace_segment_is_refused() {
    let loaded = load(document(vec![entry(
        "example..ext",
        &example_schema_path(),
        "example",
        vec![],
    )]));
    assert_refused_with(&loaded, "example..ext", "empty segment");
}

#[test]
fn an_object_colliding_with_a_stock_object_is_refused() {
    let loaded = load(document(vec![
        entry("example.stock", "stock-object.toml", "example", vec![]),
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("panel")],
        ),
    ]));
    assert_refused_with(
        &loaded,
        "example.stock",
        "object `window` is already declared by namespace `gnoblin`",
    );
    assert_eq!(loaded.accepted.len(), 1, "the other extension still loads");
}

#[test]
fn an_object_colliding_with_an_accepted_extension_is_refused() {
    let loaded = load(document(vec![
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("panel")],
        ),
        entry("example.second", "second-probe.toml", "example", vec![]),
    ]));
    assert_eq!(loaded.accepted.len(), 1, "example.ext stays loaded");
    assert_refused_with(
        &loaded,
        "example.second",
        "object `probe` is already declared by namespace `example.ext`",
    );
}

#[test]
fn a_member_declared_twice_in_one_object_is_refused_by_the_schema_parser() {
    let loaded = load(document(vec![entry(
        "example.dup",
        "duplicate-member.toml",
        "example",
        vec![],
    )]));
    assert_refused_with(&loaded, "example.dup", "duplicate member name `ping`");
    assert!(loaded.accepted.is_empty());
}

#[test]
fn an_object_reference_parameter_refuses_the_extension_at_load() {
    let loaded = load(document(vec![entry(
        "example.objparam",
        "object-param.toml",
        "example",
        vec![text("panel")],
    )]));
    assert_refused_with(
        &loaded,
        "example.objparam",
        "unsupported-field-kind: probe.adopt.other is an object reference",
    );
    assert!(loaded.accepted.is_empty(), "the method is never bound");
}

#[test]
fn a_list_parameter_refuses_only_that_extension_at_load() {
    let loaded = load(document(vec![
        entry("example.listparam", "list-param.toml", "example", vec![]),
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("panel")],
        ),
    ]));
    assert_refused_with(
        &loaded,
        "example.listparam",
        "unsupported-field-kind: probe.tag.names is a list",
    );
    assert_eq!(loaded.accepted.len(), 1, "the other extension still loads");
    assert_eq!(loaded.accepted[0].namespace, EXAMPLE_NAMESPACE);
}

#[test]
fn a_non_table_entry_is_refused_by_its_index() {
    let loaded = load(document(vec![
        text("example.ext"),
        entry(
            EXAMPLE_NAMESPACE,
            &example_schema_path(),
            "example",
            vec![text("panel")],
        ),
    ]));
    assert_refused_with(&loaded, "<extensions[0]>", "expected a table, found a string");
    assert_eq!(loaded.accepted.len(), 1);
}

#[test]
fn a_missing_provider_is_refused() {
    let loaded = load(document(vec![table([
        ("namespace", text(EXAMPLE_NAMESPACE)),
        ("schema", text(&example_schema_path())),
        ("grants", list(vec![])),
    ])]));
    assert_refused_with(&loaded, EXAMPLE_NAMESPACE, "`provider` is missing");
}

#[test]
fn an_empty_provider_is_refused() {
    let loaded = load(document(vec![entry(
        EXAMPLE_NAMESPACE,
        &example_schema_path(),
        "",
        vec![],
    )]));
    assert_refused_with(&loaded, EXAMPLE_NAMESPACE, "`provider` is empty");
}

#[test]
fn a_grant_that_is_not_text_is_refused() {
    let loaded = load(document(vec![entry(
        EXAMPLE_NAMESPACE,
        &example_schema_path(),
        "example",
        vec![text("panel"), flag(true)],
    )]));
    assert_refused_with(
        &loaded,
        EXAMPLE_NAMESPACE,
        "`grants` expected role names, found a boolean",
    );
}

#[test]
fn a_role_granted_twice_is_refused() {
    let loaded = load(document(vec![entry(
        EXAMPLE_NAMESPACE,
        &example_schema_path(),
        "example",
        vec![text("panel"), text("panel")],
    )]));
    assert_refused_with(&loaded, EXAMPLE_NAMESPACE, "`grants` names `panel` twice");
}

#[test]
fn an_unknown_field_is_refused() {
    let loaded = load(document(vec![table([
        ("namespace", text(EXAMPLE_NAMESPACE)),
        ("schema", text(&example_schema_path())),
        ("provider", text("example")),
        ("grants", list(vec![])),
        ("isolation", text("process")),
    ])]));
    assert_refused_with(&loaded, EXAMPLE_NAMESPACE, "`isolation` is not an extension key");
    assert!(loaded.accepted.is_empty());
}

#[test]
fn a_section_that_is_not_a_list_is_refused_as_a_whole() {
    let config = Config::from_value(table([("extensions", table([("namespace", text(EXAMPLE_NAMESPACE))]))]))
        .expect("the section is carried, so the config accepts its shape");
    let loaded = load_extensions(config.document(), &fixture_dir(), &stock());
    assert!(loaded.accepted.is_empty());
    assert_refused_with(&loaded, "<extensions>", "expected a list of extensions, found a table");
}
