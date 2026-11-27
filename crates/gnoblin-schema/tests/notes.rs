//! The keys a table carries for the reference only: `title`, `range`, `unit`, `raises`, `example` and a parameter's `default`. They are read and
//! checked, kept in the catalog, and in no hash: a file with them and the same file without them have one fingerprint.

use gnoblin_schema::{Catalog, DefaultValue, Namespace, SchemaError, parse_schema};

const PLAIN: &str = r#"namespace = "gnoblin.model"
version = "0.2.0"
local = true

[[object]]
name = "mouse"
since = "0.2.0"
kind = "handle"

[[object.property]]
name = "sensitivity"
type = "double"
since = "0.2.0"
optional = true

[[object.signal]]
name = "moved"
since = "0.2.0"

[[object.signal.field]]
name = "delta"
type = "int"

[[object.method]]
name = "reset"
since = "0.2.0"

[[object.method.param]]
name = "name"
type = "string"

[[object.method.param]]
name = "step"
type = "uint"
optional = true
"#;

const NOTED: &str = r#"title = "Input and shortcuts"
namespace = "gnoblin.model"
version = "0.2.0"
local = true

[[object]]
name = "mouse"
since = "0.2.0"
kind = "handle"
example = { typescript = "const m = 1;", python = "m = 1" }

[[object.property]]
name = "sensitivity"
type = "double"
since = "0.2.0"
optional = true
range = [-1, 1]
unit = "ratio"
raises = ["denied", "invalid_argument"]
example = { rust = "let m = 1;" }

[[object.signal]]
name = "moved"
since = "0.2.0"
example = { python = "pass" }

[[object.signal.field]]
name = "delta"
type = "int"
range = [-100, 100]
unit = "px"

[[object.method]]
name = "reset"
since = "0.2.0"
raises = ["invalid_argument"]

[[object.method.param]]
name = "name"
type = "string"

[[object.method.param]]
name = "step"
type = "uint"
optional = true
default = 1
range = [1, 10]
"#;

fn catalog(text: &str) -> Catalog {
    Catalog::new(vec![parse_schema("mouse.toml", text).expect("the file parses")]).expect("the catalog builds")
}

fn problems(text: &str) -> String {
    match parse_schema("bad.toml", text) {
        Err(SchemaError::Invalid { problems, .. }) => problems.join("\n"),
        Err(other) => other.to_string(),
        Ok(_) => String::new(),
    }
}

fn with(extra_property: &str) -> String {
    PLAIN.replace(
        "optional = true\n\n[[object.signal]]",
        &format!("optional = true\n{extra_property}\n[[object.signal]]"),
    )
}

#[test]
fn the_keys_are_read_into_the_catalog() {
    let noted = catalog(NOTED);
    let namespace = Namespace::parse("gnoblin.model").expect("namespace");
    assert_eq!(noted.schemas()[0].title.as_deref(), Some("Input and shortcuts"));
    let mouse = noted.objects(&namespace).next().expect("the class");
    assert_eq!(
        mouse.notes.example.as_ref().and_then(|e| e.typescript.as_deref()),
        Some("const m = 1;")
    );
    let property = &mouse.properties[0];
    let range = property.notes.range.expect("a range");
    assert_eq!((range.min, range.max), (-1.0, 1.0));
    assert_eq!(property.notes.unit.as_deref(), Some("ratio"));
    assert_eq!(property.notes.raises, ["denied", "invalid_argument"]);
    assert_eq!(mouse.signals[0].fields[0].notes.unit.as_deref(), Some("px"));
    let step = &mouse.methods[0].params[1];
    assert_eq!(step.default, Some(DefaultValue::Uint(1)));
    assert_eq!(step.notes.range.map(|range| range.max), Some(10.0));
}

#[test]
fn no_hash_reads_them() {
    let plain = catalog(PLAIN);
    let noted = catalog(NOTED);
    let namespace = Namespace::parse("gnoblin.model").expect("namespace");
    assert_eq!(plain.fingerprint(), noted.fingerprint());
    assert_eq!(
        plain.namespace_fingerprint(&namespace),
        noted.namespace_fingerprint(&namespace)
    );
}

#[test]
fn a_key_of_the_reference_is_refused_in_the_compositors_catalog() {
    let text = "namespace = \"gnoblin\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"mouse\"\nsince = \"0.2.0\"\n\n\
                [[object.property]]\nname = \"speed\"\ntype = \"double\"\nsince = \"0.2.0\"\nunit = \"px\"\n";
    let found = problems(text);
    assert!(
        found.contains("`unit` is a key of a namespace a library serves"),
        "{found}"
    );
}

#[test]
fn a_bad_value_is_named() {
    let cases = [
        ("range = [1, 0]", "`range` is `[min, max]`"),
        ("range = [1]", "`range` is `[min, max]`"),
        ("unit = \"\"", "`unit` is one short word"),
        ("raises = [\"Denied\"]", "`raises` names error codes"),
        ("raises = [\"denied\", \"denied\"]", "twice"),
        ("example = {}", "needs a `typescript`, `python` or `rust` program"),
        ("example = { rust = \" \" }", "program is empty"),
    ];
    for (key, message) in cases {
        let found = problems(&with(key));
        assert!(found.contains(message), "{key}: {found}");
    }
    let found = problems(&with("readonly = true\nraises = [\"denied\"]"));
    assert!(found.contains("a read-only property is never written"), "{found}");
    let found = problems(&PLAIN.replace("type = \"string\"\n", "type = \"string\"\nrange = [0, 1]\n"));
    assert!(found.contains("`range` belongs to a number"), "{found}");
    let found = problems(&PLAIN.replace("type = \"string\"\n", "type = \"string\"\ndefault = \"a\"\n"));
    assert!(
        found.contains("a `default` belongs to an optional parameter"),
        "{found}"
    );
}
