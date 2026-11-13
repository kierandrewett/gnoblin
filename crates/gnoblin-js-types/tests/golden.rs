//! The generated declarations of the real schema, pinned to a golden file.
//!
//! A schema change that alters any declaration fails here; a change that is meant to
//! alter them regenerates the golden with
//! `UPDATE_GOLDEN=1 cargo test -p gnoblin-js-types --test golden`.

use std::path::{Path, PathBuf};

use gnoblin_js_types::{render_dts, stock_catalog};

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/gnoblin.d.ts")
}

#[test]
fn the_rendered_stock_schema_matches_the_golden() {
    let catalog = stock_catalog().expect("the embedded schema loads");
    let rendered = render_dts(&catalog);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(golden_path(), &rendered).expect("write the golden");
        return;
    }
    let golden = std::fs::read_to_string(golden_path()).expect("the golden is checked in");
    assert!(
        rendered == golden,
        "gnoblin.d.ts drifted from tests/golden/gnoblin.d.ts; if the schema change is meant, \
         regenerate with UPDATE_GOLDEN=1"
    );
}

#[test]
fn rendering_twice_is_the_same_text() {
    let first = render_dts(&stock_catalog().expect("the embedded schema loads"));
    let second = render_dts(&stock_catalog().expect("the embedded schema loads"));
    assert_eq!(first, second);
}

#[test]
fn the_declarations_are_ascii() {
    let rendered = render_dts(&stock_catalog().expect("the embedded schema loads"));
    assert!(rendered.is_ascii(), "the generated file must be ASCII");
}
