//! The declaration file: the two globals a script has without an import.

/// The globals (`gnoblin`, `console`), declared in terms of the real library's types.
const GLOBALS_DTS: &str = include_str!("../static/globals.d.ts");

/// What the file says about itself, above the globals.
const HEADER: &str = "\
// gnoblin.d.ts: the globals of the Gnoblin script API, `gnoblin` and `console`.
//
// Written by `gnoblinctl types init`; the next call rewrites it, so do not edit it.
//
// The library, `import ... from \"gnoblin-core\"` and `\"gnoblin-std\"` (and their submodules), is not declared here: it
// is the real TypeScript source under `.gnoblin/lib/`, which the tsconfig maps those names to.

";

/// The text of `gnoblin.d.ts`: a header, then the globals.
#[must_use]
pub fn render_dts() -> String {
    format!("{HEADER}{GLOBALS_DTS}")
}
