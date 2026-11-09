//! The generator-visible surface of the stock schema, pinned to a golden file.
//!
//! Every generator (the protobuf build, the Lua bindings, the IPC and CLI
//! generators) reads the catalog through `Catalog::objects` and `proto_files`. This
//! test renders exactly the members those readers see today and compares the text
//! with `tests/golden/catalog.txt`. A schema-format change that alters any existing
//! schema's output fails here; a change that is meant to alter it regenerates the
//! golden with `UPDATE_GOLDEN=1 cargo test -p gnoblin-schema --test golden`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gnoblin_schema::{Catalog, Namespace, ObjectType, proto};

const GOLDEN: &str = "tests/golden/catalog.txt";

fn stock_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema")
}

fn dump_object(text: &mut String, object: &ObjectType) {
    writeln!(
        text,
        "object {} since {} secret {}",
        object.name, object.since, object.secret
    )
    .unwrap();
    for property in &object.properties {
        writeln!(
            text,
            "  property {} type {} since {} secret {}",
            property.name, property.ty, property.since, property.secret
        )
        .unwrap();
    }
    for method in &object.methods {
        writeln!(
            text,
            "  method {} since {} secret {} function {}",
            method.name, method.since, method.secret, method.function
        )
        .unwrap();
        for param in &method.params {
            writeln!(
                text,
                "    param {} type {} since {} secret {}",
                param.name, param.ty, param.since, param.secret
            )
            .unwrap();
        }
    }
    for signal in &object.signals {
        writeln!(
            text,
            "  signal {} since {} secret {}",
            signal.name, signal.since, signal.secret
        )
        .unwrap();
        for field in &signal.fields {
            writeln!(
                text,
                "    field {} type {} since {} secret {}",
                field.name, field.ty, field.since, field.secret
            )
            .unwrap();
        }
    }
}

fn render(catalog: &Catalog) -> String {
    let mut text = String::new();
    for namespace in catalog.namespaces() {
        let version = catalog.version(&namespace).expect("every namespace has a version");
        writeln!(text, "namespace {namespace} version {version}").unwrap();
        for error in catalog.errors(&namespace) {
            writeln!(text, "error {} since {}", error.name, error.since).unwrap();
        }
        for object in catalog.objects(&namespace) {
            dump_object(&mut text, object);
        }
    }
    text.push_str("== proto ==\n");
    for (name, body) in proto::proto_files(catalog) {
        writeln!(text, "-- {name}").unwrap();
        text.push_str(&body);
    }
    text
}

#[test]
fn stock_schema_surface_matches_golden() {
    let catalog = Catalog::load_dir(&stock_dir()).expect("the stock schema loads");
    let actual = render(&catalog);
    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_path.parent().expect("golden has a parent")).expect("golden dir");
        std::fs::write(&golden_path, &actual).expect("write golden");
        return;
    }

    let expected = std::fs::read_to_string(&golden_path).unwrap_or_else(|error| {
        panic!(
            "read {}: {error}; regenerate with UPDATE_GOLDEN=1",
            golden_path.display()
        )
    });
    assert!(
        actual == expected,
        "the generator-visible surface of the stock schema changed; if intended, regenerate with UPDATE_GOLDEN=1"
    );
}

#[test]
fn golden_covers_every_namespace_in_the_stock_schema() {
    let catalog = Catalog::load_dir(&stock_dir()).expect("the stock schema loads");
    let namespaces: Vec<Namespace> = catalog.namespaces();
    assert!(namespaces.iter().any(|namespace| namespace.as_str() == "gnoblin"));
    let text = render(&catalog);
    for namespace in &namespaces {
        assert!(text.contains(&format!("namespace {namespace} version")));
    }
}
