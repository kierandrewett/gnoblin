//! A throw inside the stock library reports the `.ts` file and line that wrote it.
//!
//! The library is embedded as TypeScript and stripped when something imports it. A stack frame in the stripped text
//! names a module, not a file, and a line of generated JavaScript; the loader keeps each stock module's source map so
//! the frame is shown at the stock source's own line, the way a user's `.ts` file is.

use std::fs;
use std::path::Path;

use gnoblin_js::{JsConfig, JsErrorKind, JsGeneration, stock_sources_in};

/// A generation whose config root holds `shell.ts` with `body`, over the stock library embedded as TypeScript.
fn generation(body: &str) -> (tempfile::TempDir, JsGeneration) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), body).expect("shell.ts is written");
    let mut config = JsConfig::new(&root);
    for (name, module) in stock_sources_in(None) {
        config = config.embed_typescript(name, module);
    }
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    (dir, generation)
}

/// The 1-based line of the first line of `source` that contains `needle`.
fn line_of(source: &str, needle: &str) -> u32 {
    let at = source
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is not in the source"));
    u32::try_from(at + 1).expect("the line fits")
}

#[test]
fn a_throw_in_a_stock_layout_reports_the_stock_ts_line() {
    let (_dir, generation) = generation(
        "import { FloatingLayout } from \"gnoblin-std\";\n\
         const layout = new FloatingLayout();\n\
         (layout as unknown as { cascadeStep: number }).cascadeStep = 0;\n\
         layout.place(10, 10, { x: 0, y: 0, width: 100, height: 100 }, []);\n",
    );
    let error = generation
        .eval_module(Path::new("shell.ts"))
        .expect_err("a cascade step of 0 is refused");
    assert_eq!(error.kind, JsErrorKind::Exception, "{error}");
    assert!(error.message.contains("cascadeStep"), "{error}");

    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js/gnoblin-std/layouts/floating.ts"))
            .expect("the stock layout source");
    let want = line_of(&source, "throw new RangeError(\"FloatingLayout cascadeStep");

    let at = error.location.as_ref().expect("a location");
    assert!(
        at.file.ends_with("layouts/floating.ts"),
        "the location names the stock source file, not the module: {error:?}"
    );
    assert_eq!(
        at.line, want,
        "the line is the stock source's, not the stripped text's: {error:?}"
    );

    let stack = error.stack.as_deref().expect("a stack");
    assert!(
        stack.contains(&format!("layouts/floating.ts:{want}:")),
        "the stack frame is remapped: {stack}"
    );
    assert!(
        stack.contains("shell.ts:4:"),
        "the caller frame is the user's line: {stack}"
    );
}
