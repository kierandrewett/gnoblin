//! Every error code a schema file lists under `raises` is a real code of the script API (`gnoblin_object::ErrorCode`), so a reference that
//! names it names something a script can catch.

use std::fs;
use std::path::Path;

use gnoblin_object::ErrorCode;
use gnoblin_schema::{Notes, parse_schema};

/// The `raises` entries of a schema file's text that are not an `ErrorCode`, as `owner: code`.
fn unknown_codes(origin: &str, text: &str) -> Vec<String> {
    let schema = parse_schema(origin, text).unwrap_or_else(|error| panic!("{origin} does not parse: {error}"));
    let mut unknown = Vec::new();
    let mut check = |owner: String, notes: &Notes| {
        for code in &notes.raises {
            if ErrorCode::parse(code).is_none() {
                unknown.push(format!("{origin}: {owner} raises `{code}`"));
            }
        }
    };
    for object in &schema.objects {
        for property in &object.properties {
            check(format!("{}.{}", object.name, property.name), &property.notes);
        }
        for method in &object.methods {
            check(format!("{}.{}", object.name, method.name), &method.notes);
        }
    }
    for augment in &schema.augments {
        for property in &augment.properties {
            check(format!("augment {}.{}", augment.class, property.name), &property.notes);
        }
        for method in &augment.methods {
            check(format!("augment {}.{}", augment.class, method.name), &method.notes);
        }
    }
    unknown
}

#[test]
fn a_name_that_is_not_an_error_code_is_found() {
    let text = "namespace = \"gnoblin.model\"\nversion = \"0.2.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.2.0\"\nkind = \"handle\"\n\n\
                [[object.method]]\nname = \"poke\"\nsince = \"0.2.0\"\nraises = [\"invalid_argument\", \"no_such_code\"]\n";
    assert_eq!(
        unknown_codes("fixture.toml", text),
        ["fixture.toml: thing.poke raises `no_such_code`"]
    );
}

#[test]
fn every_raises_of_the_schema_files_is_an_error_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut unknown = Vec::new();
    let mut seen = 0;
    for dir in ["gnoblin-core-abi/schema", "gnoblin-std-abi/schema"] {
        for entry in fs::read_dir(root.join(dir)).expect("a schema directory") {
            let path = entry.expect("an entry").path();
            if path.extension().is_some_and(|extension| extension == "toml") {
                let text = fs::read_to_string(&path).expect("a schema file");
                seen += text.matches("raises = [").count();
                unknown.extend(unknown_codes(&path.display().to_string(), &text));
            }
        }
    }
    assert!(seen > 0, "the schema files name error codes");
    assert!(unknown.is_empty(), "not error codes:\n{}", unknown.join("\n"));
}
