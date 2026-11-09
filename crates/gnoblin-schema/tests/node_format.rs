//! The node-core additions to the schema format: `base` on an object, and `default`,
//! `readonly` and `animated` on a property (design/node-core.md, section 2.2).

use gnoblin_schema::derived::DerivedEvent;
use gnoblin_schema::{Catalog, DefaultValue, ObjectType, SchemaError, Version, parse_schema};

fn catalog(files: &[(&str, &str)]) -> Result<Catalog, SchemaError> {
    let schemas = files
        .iter()
        .map(|(name, text)| parse_schema(name, text))
        .collect::<Result<Vec<_>, _>>()?;
    Catalog::new(schemas)
}

fn problems(error: &SchemaError) -> String {
    match error {
        SchemaError::Invalid { problems, .. } => problems.join("\n"),
        other => other.to_string(),
    }
}

/// The resolved object of that name, from whichever file declares it.
fn object<'a>(catalog: &'a Catalog, name: &str) -> &'a ObjectType {
    catalog
        .schemas()
        .iter()
        .flat_map(|schema| schema.objects.iter())
        .find(|object| object.name == name)
        .unwrap_or_else(|| panic!("no object `{name}`"))
}

const TREE: &str = r#"
namespace = "tree"
version = "0.2.0"

[[object]]
name = "node"
since = "0.2.0"

[[object.property]]
name = "id"
type = "uint"
since = "0.2.0"
default = 0
readonly = true

[[object.property]]
name = "title"
type = "string"
since = "0.2.0"
default = ""

[[object.signal]]
name = "changed"
since = "0.2.0"

[[object.signal.field]]
name = "value"
type = "uint"
since = "0.2.0"

[[object]]
name = "window"
since = "0.2.0"
base = "node"

[[object.property]]
name = "output"
type = "string"
since = "0.2.0"

[[object.signal]]
name = "closed"
since = "0.2.0"
"#;

#[test]
fn base_parses_on_an_object() {
    let schema = parse_schema("tree.toml", TREE).expect("a base parses");
    let window = schema
        .objects
        .iter()
        .find(|object| object.name == "window")
        .expect("window");
    assert_eq!(window.base.as_deref(), Some("node"));
    let node = schema
        .objects
        .iter()
        .find(|object| object.name == "node")
        .expect("node");
    assert_eq!(node.base, None);
}

#[test]
fn default_of_the_wrong_type_is_an_error_naming_the_property() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "count"
type = "uint"
since = "0.2.0"
default = "three"
"#;
    let error = parse_schema("tree.toml", text).expect_err("a string default for a uint is refused");
    let message = problems(&error);
    assert!(message.contains("`count`"), "names the property: {message}");
    assert!(message.contains("default"), "names the key: {message}");
}

#[test]
fn default_outside_the_range_of_its_type_is_an_error() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "count"
type = "uint"
since = "0.2.0"
default = -1
"#;
    let error = parse_schema("tree.toml", text).expect_err("a negative default for a uint is refused");
    assert!(problems(&error).contains("`count`"));
}

#[test]
fn defaults_of_each_type_parse_into_the_model() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "flag"
type = "bool"
since = "0.2.0"
default = true
[[object.property]]
name = "delta"
type = "int"
since = "0.2.0"
default = -4
[[object.property]]
name = "count"
type = "uint"
since = "0.2.0"
default = 3
[[object.property]]
name = "scale"
type = "double"
since = "0.2.0"
default = 1.5
[[object.property]]
name = "label"
type = "string"
since = "0.2.0"
default = "x"
[[object.property]]
name = "steps"
type = "list<uint>"
since = "0.2.0"
default = [1, 2]
"#;
    let schema = parse_schema("tree.toml", text).expect("defaults parse");
    let properties = &schema.objects[0].properties;
    let default_of = |name: &str| {
        properties
            .iter()
            .find(|property| property.name == name)
            .and_then(|property| property.default.clone())
    };
    assert_eq!(default_of("flag"), Some(DefaultValue::Bool(true)));
    assert_eq!(default_of("delta"), Some(DefaultValue::Int(-4)));
    assert_eq!(default_of("count"), Some(DefaultValue::Uint(3)));
    assert_eq!(default_of("scale"), Some(DefaultValue::Double(1.5)));
    assert_eq!(default_of("label"), Some(DefaultValue::Text("x".to_owned())));
    assert_eq!(
        default_of("steps"),
        Some(DefaultValue::List(vec![DefaultValue::Uint(1), DefaultValue::Uint(2)]))
    );
}

#[test]
fn base_naming_an_unknown_class_is_an_error_naming_both() {
    let text = TREE.replace("base = \"node\"", "base = \"missing\"");
    let error = catalog(&[("tree.toml", &text)]).expect_err("an unknown base is refused");
    let message = problems(&error);
    assert!(message.contains("`window`"), "names the derived class: {message}");
    assert!(message.contains("`missing`"), "names the base: {message}");
}

#[test]
fn base_must_name_a_class_in_the_same_namespace() {
    let other = r#"
namespace = "other"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
"#;
    let text = TREE.replace("name = \"node\"", "name = \"thing\"");
    let error = catalog(&[("tree.toml", &text), ("other.toml", other)]).expect_err("a cross-namespace base is refused");
    let message = problems(&error);
    assert!(message.contains("`node`"), "names the base: {message}");
}

#[test]
fn base_cycle_is_an_error_naming_the_classes() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "alpha"
since = "0.2.0"
base = "beta"
[[object]]
name = "beta"
since = "0.2.0"
base = "alpha"
"#;
    let error = catalog(&[("tree.toml", text)]).expect_err("a base cycle is refused");
    let message = problems(&error);
    assert!(message.contains("cycle"), "says cycle: {message}");
    assert!(
        message.contains("alpha") && message.contains("beta"),
        "names both: {message}"
    );
}

#[test]
fn a_class_may_not_be_its_own_base() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "alpha"
since = "0.2.0"
base = "alpha"
"#;
    let error = catalog(&[("tree.toml", text)]).expect_err("a self base is refused");
    assert!(problems(&error).contains("cycle"));
}

#[test]
fn readonly_and_animated_parse_as_booleans_and_default_to_false() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "x"
type = "double"
since = "0.2.0"
readonly = true
animated = true
[[object.property]]
name = "y"
type = "double"
since = "0.2.0"
"#;
    let schema = parse_schema("tree.toml", text).expect("the flags parse");
    let x = &schema.objects[0].properties[0];
    let y = &schema.objects[0].properties[1];
    assert!(x.readonly && x.animated);
    assert!(!y.readonly && !y.animated);
}

#[test]
fn readonly_must_be_a_boolean() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "x"
type = "double"
since = "0.2.0"
readonly = "yes"
"#;
    let error = parse_schema("tree.toml", text).expect_err("a string readonly is refused");
    assert!(matches!(error, SchemaError::Toml { .. }));
}

#[test]
fn an_unknown_key_is_still_refused_on_a_property_and_an_object() {
    let property = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "x"
type = "double"
since = "0.2.0"
animatable = true
"#;
    let object = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
extends = "thing"
"#;
    for text in [property, object] {
        let error = parse_schema("tree.toml", text).expect_err("an unknown key is refused");
        assert!(matches!(error, SchemaError::Toml { .. }), "{error}");
    }
}

#[test]
fn a_derived_class_inherits_base_properties_and_signals_base_first() {
    let catalog = catalog(&[("tree.toml", TREE)]).expect("the tree catalog resolves");
    let window = object(&catalog, "window");
    let names: Vec<&str> = window
        .properties
        .iter()
        .map(|property| property.name.as_str())
        .collect();
    assert_eq!(names, ["id", "title", "output"]);
    let signals: Vec<&str> = window.signals.iter().map(|signal| signal.name.as_str()).collect();
    assert_eq!(signals, ["changed", "closed"]);

    let inherited = &window.properties[0];
    assert!(inherited.readonly, "inherited flags travel with the property");
    assert_eq!(inherited.default, Some(DefaultValue::Uint(0)));

    let node = object(&catalog, "node");
    assert_eq!(node.properties.len(), 2, "the base is unchanged");
}

#[test]
fn inheritance_runs_through_an_intermediate_class_in_another_file() {
    let grand = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "id"
type = "uint"
since = "0.2.0"
"#;
    let middle = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "box"
since = "0.2.0"
base = "node"
[[object.property]]
name = "size"
type = "double"
since = "0.2.0"
"#;
    let leaf = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "leaf"
since = "0.2.0"
base = "box"
[[object.property]]
name = "label"
type = "string"
since = "0.2.0"
"#;
    let catalog = catalog(&[("a.toml", grand), ("b.toml", middle), ("c.toml", leaf)]).expect("resolves");
    let names: Vec<&str> = object(&catalog, "leaf")
        .properties
        .iter()
        .map(|property| property.name.as_str())
        .collect();
    assert_eq!(names, ["id", "size", "label"]);
}

#[test]
fn a_derived_property_may_redeclare_a_base_property_of_the_same_type_in_place() {
    let text = r#"
namespace = "tree"
version = "0.2.0"
[[object]]
name = "node"
since = "0.2.0"
[[object.property]]
name = "a"
type = "bool"
since = "0.2.0"
[[object.property]]
name = "b"
type = "uint"
since = "0.2.0"
default = 0
[[object]]
name = "window"
since = "0.2.0"
base = "node"
[[object.property]]
name = "b"
type = "uint"
since = "0.2.0"
default = 3
animated = true
"#;
    let catalog = catalog(&[("tree.toml", text)]).expect("the override resolves");
    let window = object(&catalog, "window");
    let names: Vec<&str> = window
        .properties
        .iter()
        .map(|property| property.name.as_str())
        .collect();
    assert_eq!(names, ["a", "b"], "the redeclared property keeps the base's position");
    assert_eq!(window.properties[1].default, Some(DefaultValue::Uint(3)));
    assert!(window.properties[1].animated);
}

#[test]
fn a_derived_property_may_not_redeclare_a_base_property_with_a_different_type() {
    let text = TREE.replace(
        "[[object.property]]\nname = \"output\"",
        "[[object.property]]\nname = \"id\"\ntype = \"string\"\nsince = \"0.2.0\"\n\n[[object.property]]\nname = \"output\"",
    );
    let error = catalog(&[("tree.toml", &text)]).expect_err("a retyped inherited property is refused");
    let message = problems(&error);
    assert!(message.contains("`id`"), "names the property: {message}");
    assert!(
        message.contains("uint") && message.contains("string"),
        "names both types: {message}"
    );
    assert!(message.contains("`node`"), "names the base: {message}");
}

#[test]
fn a_derived_signal_may_not_reuse_an_inherited_signal_name() {
    let text = TREE.replace(
        "[[object.signal]]\nname = \"closed\"",
        "[[object.signal]]\nname = \"changed\"",
    );
    let error = catalog(&[("tree.toml", &text)]).expect_err("a repeated signal name is refused");
    assert!(problems(&error).contains("`changed`"));
}

#[test]
fn a_derived_method_may_not_reuse_an_inherited_property_name() {
    let text = format!("{TREE}\n[[object.method]]\nname = \"title\"\nsince = \"0.2.0\"\n");
    let error = catalog(&[("tree.toml", &text)]).expect_err("a method shadowing an inherited property is refused");
    assert!(problems(&error).contains("`title`"));
}

#[test]
fn a_derived_event_on_a_class_with_a_base_builds_its_members_once() {
    let text = format!(
        "{}\n[[object.property]]\nname = \"focused\"\ntype = \"bool\"\nsince = \"0.2.0\"\n",
        TREE.replace("namespace = \"tree\"", "namespace = \"gnoblin\""),
    );
    let base = catalog(&[("gnoblin.toml", &text)]).expect("catalog");
    let event = DerivedEvent {
        object: "window".to_owned(),
        signal: "focused".to_owned(),
        property: "focused".to_owned(),
        when: true,
        since: Version::parse("0.2.0").expect("a version"),
        fields: Vec::new(),
    };
    let derived = base.with_derived(&[event]).expect("the derived event builds");
    let window = object(&derived, "window");
    assert_eq!(
        window
            .properties
            .iter()
            .filter(|property| property.name == "title")
            .count(),
        1
    );
    assert!(window.signals.iter().any(|signal| signal.name == "focused"));
    assert!(window.signals.iter().any(|signal| signal.name == "changed"));
}

#[test]
fn rebuilding_a_catalog_with_no_derived_events_changes_nothing() {
    let base = catalog(&[("tree.toml", TREE)]).expect("catalog");
    let rebuilt = base.with_derived(&[]).expect("the rebuild builds");
    assert_eq!(format!("{:?}", base.schemas()), format!("{:?}", rebuilt.schemas()));
}
