//! The generated files, pinned to golden files: `gnoblin.d.ts` and the two `tsconfig.json` files `types init` writes.
//!
//! A change that alters any of them fails here; a change that is meant to alter them regenerates the goldens with
//! `UPDATE_GOLDEN=1 cargo test -p gnoblin-js-types --test golden`.

use std::path::{Path, PathBuf};

use gnoblin_js_types::{render_dts, render_generated_tsconfig, render_user_tsconfig};

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn matches_golden(name: &str, rendered: &str) {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(golden_path(name), rendered).expect("write the golden");
        return;
    }
    let golden = std::fs::read_to_string(golden_path(name)).expect("the golden is checked in");
    assert!(
        rendered == golden,
        "{name} drifted from tests/golden/{name}; if the change is meant, regenerate with UPDATE_GOLDEN=1"
    );
}

#[test]
fn the_declarations_match_the_golden() {
    matches_golden("gnoblin.d.ts", &render_dts());
}

#[test]
fn the_generated_tsconfig_matches_the_golden() {
    matches_golden("tsconfig.library.json", &render_generated_tsconfig());
}

#[test]
fn the_user_tsconfig_matches_the_golden() {
    matches_golden("tsconfig.user.json", &render_user_tsconfig());
}

#[test]
fn the_declarations_are_ascii_and_the_same_every_time() {
    let rendered = render_dts();
    assert!(rendered.is_ascii(), "the generated file must be ASCII");
    assert_eq!(rendered, render_dts());
}

#[test]
fn the_declarations_declare_only_the_two_globals() {
    let rendered = render_dts();
    let declared: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with("declare "))
        .map(|line| line.split(':').next().unwrap_or(line))
        .collect();
    assert_eq!(declared, ["declare const gnoblin", "declare const console"]);
}
