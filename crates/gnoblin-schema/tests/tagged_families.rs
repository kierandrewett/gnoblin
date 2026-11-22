//! A record that is the family of a tagged value: its `tag`, its variants, and every rule the catalog holds them to.
//!
//! Each test names the item of the value-class failure list it guards (the list is in the commit body of the commit that added this
//! file): F1 a tag that collides with a field or is not an enum; F2 a type that does not match; F3 a field a variant omits; F4 a value
//! without a variant, a variant twice or for a value the enum lacks, a name that collides; F10 an optional property of a variant against
//! a field that is not; F14 a fingerprint that ignores variants.

use gnoblin_schema::{Catalog, SchemaError, parse_schema};

/// A local namespace with an enum of three kinds, a record `shader` and the family `filter`, whose variants `variants` declares.
fn family(variants: &str) -> String {
    format!(
        r#"namespace = "test"
version = "0.2.0"
local = true

[[enum]]
name = "kind"
since = "0.2.0"
values = ["blur", "grade", "shader"]

[[object]]
name = "shader"
since = "0.2.0"
kind = "record"

[[object.property]]
name = "source"
type = "string"
since = "0.2.0"

[[object.property]]
name = "params"
type = "list<double>"
since = "0.2.0"
optional = true

[[object]]
name = "filter"
since = "0.2.0"
kind = "record"
tag = "kind"

[[object.property]]
name = "kind"
type = "enum:kind"
since = "0.2.0"

[[object.property]]
name = "amount"
type = "double"
since = "0.2.0"
optional = true

[[object.property]]
name = "shader"
type = "object:shader"
since = "0.2.0"
optional = true
{variants}"#
    )
}

const GOOD: &str = r#"
[[object.variant]]
name = "blur_filter"
tag = "blur"
since = "0.2.0"

[[object.variant.property]]
name = "radius"
type = "double"
since = "0.2.0"
default = 12.0
field = "amount"

[[object.variant]]
name = "grade_filter"
tag = "grade"
since = "0.2.0"

[[object.variant.property]]
name = "amount"
type = "double"
since = "0.2.0"
default = 1.0
field = "amount"

[[object.variant]]
name = "shader_filter"
tag = "shader"
since = "0.2.0"

[[object.variant.property]]
name = "source"
type = "string"
since = "0.2.0"
field = "shader.source"

[[object.variant.property]]
name = "params"
type = "list<double>"
since = "0.2.0"
default = []
field = "shader.params"
"#;

fn problems_of(variants: &str) -> Vec<String> {
    let schema = parse_schema("test.toml", &family(variants));
    let schema = match schema {
        Ok(schema) => schema,
        Err(SchemaError::Invalid { problems, .. }) => return problems,
        Err(other) => panic!("{other}"),
    };
    match Catalog::new(vec![schema]) {
        Ok(_) => Vec::new(),
        Err(SchemaError::Invalid { problems, .. }) => problems,
        Err(other) => panic!("{other}"),
    }
}

#[track_caller]
fn refused(variants: &str, wanted: &str) {
    let problems = problems_of(variants);
    assert!(
        problems.iter().any(|problem| problem.contains(wanted)),
        "wanted a problem that says `{wanted}`, got {problems:#?}"
    );
}

/// F4: the good case; a variant is not a class of the namespace.
#[test]
fn a_family_with_a_variant_for_every_value_is_a_catalog() {
    assert_eq!(problems_of(GOOD), Vec::<String>::new());
    let schema = parse_schema("test.toml", &family(GOOD)).expect("parses");
    let catalog = Catalog::new(vec![schema]).expect("a catalog");
    let namespace = catalog.namespaces().remove(0);
    let filter = catalog
        .objects(&namespace)
        .find(|object| object.name == "filter")
        .expect("filter");
    assert_eq!(filter.tag.as_deref(), Some("kind"));
    let names: Vec<&str> = filter.variants.iter().map(|variant| variant.name.as_str()).collect();
    assert_eq!(names, ["blur_filter", "grade_filter", "shader_filter"]);
    let shader = &filter.variants[2];
    assert_eq!(shader.properties[0].field.as_deref(), Some("shader.source"));
    assert_eq!(
        catalog.objects(&namespace).count(),
        2,
        "a variant is not a class of the namespace"
    );
}

/// F14: a change to a variant moves the namespace fingerprint.
#[test]
fn the_namespace_fingerprint_moves_with_a_variant_field() {
    let a = Catalog::new(vec![parse_schema("test.toml", &family(GOOD)).expect("parses")]).expect("catalog");
    let moved = GOOD.replace("default = 12.0", "default = 13.0");
    let b = Catalog::new(vec![parse_schema("test.toml", &family(&moved)).expect("parses")]).expect("catalog");
    let namespace = a.namespaces().remove(0);
    assert_ne!(a.namespace_fingerprint(&namespace), b.namespace_fingerprint(&namespace));
}

/// F4: a value of the tag with no variant.
#[test]
fn a_value_of_the_tag_without_a_variant_is_refused() {
    let without_shader = GOOD
        .split("[[object.variant]]\nname = \"shader_filter\"")
        .next()
        .expect("a head");
    refused(without_shader, "no variant stands for `shader`");
}

/// F4: two variants for one value.
#[test]
fn two_variants_for_one_value_are_refused() {
    let twice = format!(
        "{GOOD}\n[[object.variant]]\nname = \"again\"\ntag = \"blur\"\nsince = \"0.2.0\"\n\n[[object.variant.property]]\nname = \"radius\"\ntype = \"double\"\nsince = \"0.2.0\"\nfield = \"amount\"\n"
    );
    refused(&twice, "both stand for `blur`");
}

/// F4: a variant for a value the enum lacks.
#[test]
fn a_variant_for_a_value_the_enum_lacks_is_refused() {
    refused(
        &GOOD.replace("tag = \"grade\"", "tag = \"sparkle\""),
        "`sparkle` is not a value of the enum",
    );
}

/// F1 and F3: a field that is not there, a path through a field that holds no record, and the tag as a carrier.
#[test]
fn a_property_carried_by_a_field_that_is_not_there_is_refused() {
    refused(
        &GOOD.replace(
            "field = \"amount\"\n\n[[object.variant]]\nname = \"grade_filter\"",
            "field = \"radius\"\n\n[[object.variant]]\nname = \"grade_filter\"",
        ),
        "names no field of the family",
    );
    refused(
        &GOOD.replace("field = \"shader.params\"", "field = \"shader.nothing\""),
        "names no field of the record `shader`",
    );
    refused(
        &GOOD.replace("field = \"shader.params\"", "field = \"amount.params\""),
        "holds no record",
    );
    refused(
        &GOOD.replace("field = \"shader.params\"", "field = \"kind\""),
        "is the `tag`",
    );
}

/// F2: a property of another type than the field that carries it.
#[test]
fn a_property_of_another_type_than_its_field_is_refused() {
    refused(
        &GOOD
            .replace(
                "name = \"radius\"\ntype = \"double\"",
                "name = \"radius\"\ntype = \"string\"",
            )
            .replace("default = 12.0", "default = \"x\""),
        "that carries it is `double`",
    );
}

/// F10: an optional property needs an optional field to say it is absent.
#[test]
fn an_optional_property_needs_an_optional_field() {
    refused(
        &GOOD.replace(
            "name = \"source\"\ntype = \"string\"",
            "name = \"source\"\ntype = \"string\"\noptional = true",
        ),
        "nothing could say it is absent",
    );
}

/// F3: a field the family always has is not carried.
#[test]
fn a_field_the_family_always_has_must_be_carried() {
    let family = family(GOOD).replace(
        "name = \"amount\"\ntype = \"double\"\nsince = \"0.2.0\"\noptional = true",
        "name = \"amount\"\ntype = \"double\"\nsince = \"0.2.0\"",
    );
    let schema = parse_schema("test.toml", &family).expect("parses");
    let Err(SchemaError::Invalid { problems, .. }) = Catalog::new(vec![schema]) else {
        panic!("a variant that leaves out a field the family always has is refused");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("the family field `amount` is always there")),
        "{problems:#?}"
    );
}

/// F3: a field the family always has is not carried, though it has a default. Each language's codec writes an uncarried field
/// as absent, so a default would not reach the wire.
#[test]
fn a_field_the_family_always_has_must_be_carried_even_with_a_default() {
    let family = family(GOOD).replace(
        "name = \"amount\"\ntype = \"double\"\nsince = \"0.2.0\"\noptional = true",
        "name = \"amount\"\ntype = \"double\"\nsince = \"0.2.0\"\ndefault = 1.0",
    );
    let schema = parse_schema("test.toml", &family).expect("parses");
    let Err(SchemaError::Invalid { problems, .. }) = Catalog::new(vec![schema]) else {
        panic!("a variant that leaves out a field the family always has is refused, default or not");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("the family field `amount` is always there")),
        "{problems:#?}"
    );
}

/// F3: part of a record carried, a field the record always has left out, though it has a default.
#[test]
fn part_of_a_record_must_carry_the_fields_that_record_always_has_even_with_a_default() {
    let family = family(&GOOD.replace(
        "\n[[object.variant.property]]\nname = \"params\"\ntype = \"list<double>\"\nsince = \"0.2.0\"\ndefault = []\nfield = \"shader.params\"\n",
        "\n",
    ))
    .replace(
        "name = \"params\"\ntype = \"list<double>\"\nsince = \"0.2.0\"\noptional = true",
        "name = \"params\"\ntype = \"list<double>\"\nsince = \"0.2.0\"\ndefault = []",
    );
    let schema = parse_schema("test.toml", &family).expect("parses");
    let Err(SchemaError::Invalid { problems, .. }) = Catalog::new(vec![schema]) else {
        panic!(
            "a variant that carries part of a record and leaves out a field it always has is refused, default or not"
        );
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("leaves out `shader.params`")),
        "{problems:#?}"
    );
}

/// F3: part of a record carried, a field the record always has left out.
#[test]
fn part_of_a_record_must_carry_the_fields_that_record_always_has() {
    refused(
        &GOOD
            .replace(
                "name = \"source\"\ntype = \"string\"\nsince = \"0.2.0\"\nfield = \"shader.source\"",
                "name = \"label\"\ntype = \"string\"\nsince = \"0.2.0\"\ndefault = \"\"\nfield = \"shader.params\"",
            )
            .replace(
                "name = \"params\"\ntype = \"list<double>\"\nsince = \"0.2.0\"\ndefault = []\nfield = \"shader.params\"",
                "name = \"extra\"\ntype = \"list<double>\"\nsince = \"0.2.0\"\ndefault = []\nfield = \"shader.params\"",
            ),
        "leaves out `shader.source`",
    );
}

/// F1: a tag without variants, variants without a tag.
#[test]
fn a_family_needs_its_tag_and_a_tag_needs_a_family() {
    let without_tag = family(GOOD).replace("tag = \"kind\"\n", "");
    let Err(SchemaError::Invalid { problems, .. }) = parse_schema("test.toml", &without_tag) else {
        panic!("variants without a `tag` are refused");
    };
    assert!(
        problems.iter().any(|problem| problem.contains("needs both its `tag`")),
        "{problems:#?}"
    );

    let without_variants = family("");
    let Err(SchemaError::Invalid { problems, .. }) = parse_schema("test.toml", &without_variants) else {
        panic!("a `tag` without variants is refused");
    };
    assert!(
        problems.iter().any(|problem| problem.contains("needs both its `tag`")),
        "{problems:#?}"
    );
}

/// F7: the compositor's schema (the wire) takes no variants.
#[test]
fn a_variant_is_refused_in_the_compositors_schema_and_in_a_property_that_is_not_one() {
    let compositor = family(GOOD).replace("local = true\n", "");
    let Err(SchemaError::Invalid { problems, .. }) = parse_schema("test.toml", &compositor) else {
        panic!("a namespace the library does not serve takes no variants");
    };
    assert!(
        problems.iter().any(|problem| problem.contains("`local = true`")),
        "{problems:#?}"
    );

    let stray = family("").replace(
        "name = \"amount\"\ntype = \"double\"",
        "name = \"amount\"\ntype = \"double\"\nfield = \"x\"",
    );
    let Err(SchemaError::Invalid { problems, .. }) = parse_schema("test.toml", &stray) else {
        panic!("`field` belongs to a variant");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("belongs to a property of a variant")),
        "{problems:#?}"
    );
}

/// F4: a variant name that is a class of the namespace.
#[test]
fn a_variant_name_may_not_be_a_class_of_the_namespace() {
    refused(
        &GOOD.replace("name = \"blur_filter\"", "name = \"shader\""),
        "the name is a class",
    );
}

/// F1: the tag must be an enum property that is always there.
#[test]
fn a_tag_that_is_not_an_enum_or_may_be_missing_is_refused() {
    let optional = family(GOOD).replace(
        "name = \"kind\"\ntype = \"enum:kind\"",
        "name = \"kind\"\ntype = \"enum:kind\"\noptional = true",
    );
    let Err(SchemaError::Invalid { problems, .. }) =
        Catalog::new(vec![parse_schema("test.toml", &optional).expect("parses")])
    else {
        panic!("an optional tag is refused");
    };
    assert!(
        problems.iter().any(|problem| problem.contains("is always there")),
        "{problems:#?}"
    );

    let text = family(GOOD).replace("tag = \"kind\"\n", "tag = \"amount\"\n");
    let Err(SchemaError::Invalid { problems, .. }) =
        Catalog::new(vec![parse_schema("test.toml", &text).expect("parses")])
    else {
        panic!("a tag that is a number is refused");
    };
    assert!(
        problems.iter().any(|problem| problem.contains("must be an enum")),
        "{problems:#?}"
    );
}
