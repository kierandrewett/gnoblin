//! TypeScript to JavaScript, by stripping types with the oxc transformer.
//!
//! The runtime runs the JavaScript this produces. Enums and parameter properties are rewritten to
//! plain JavaScript by the same pass, so an author never writes a second dialect. The source map
//! is kept so a stack frame in the stripped code can be shown at the `.ts` line that wrote it
//! (`generation` does the remap). Syntax errors are reported against the original text, because
//! the parser never sees the stripped form.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_sourcemap::SourceMap;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

use crate::error::JsTsError;

/// The result of stripping one TypeScript file.
#[derive(Debug)]
pub struct JsStripped {
    /// The JavaScript the runtime evaluates.
    pub code: String,
    /// Maps positions in `code` back to the `.ts` source. `None` when the transformer produced no map.
    pub map: Option<SourceMap<'static>>,
}

/// Strips the types from `source`, a TypeScript module at `path`.
///
/// The first error from the parser, the semantic pass or the transformer is returned with its
/// location in `source`'s coordinates.
pub fn strip_types(path: &Path, source: &str) -> Result<JsStripped, JsTsError> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts().with_module(true)).parse();
    if let Some(first) = parsed.diagnostics.errors().next() {
        return Err(error_at(path, source, first));
    }
    let mut program = parsed.program;

    // `with_enum_eval` lets the transformer fold constant enum members; without it the transformer panics on any `enum`.
    let semantic = SemanticBuilder::new().with_enum_eval(true).build(&program);
    if let Some(first) = semantic.diagnostics.errors().next() {
        return Err(error_at(path, source, first));
    }
    let scoping = semantic.semantic.into_scoping();

    let transformed =
        Transformer::new(&allocator, path, &TransformOptions::default()).build_with_scoping(scoping, &mut program);
    if let Some(first) = transformed.diagnostics.errors().next() {
        return Err(error_at(path, source, first));
    }

    let options = CodegenOptions {
        source_map_path: Some(path.to_path_buf()),
        ..CodegenOptions::default()
    };
    let generated = Codegen::new()
        .with_options(options)
        .with_source_text(source)
        .build(&program);
    Ok(JsStripped {
        code: generated.code,
        map: generated.map.map(SourceMap::into_owned),
    })
}

/// Builds the error for `diagnostic`, located at its first label in `source`.
fn error_at(path: &Path, source: &str, diagnostic: &OxcDiagnostic) -> JsTsError {
    let offset = diagnostic.labels.first().map_or(0, |label| label.offset() as usize);
    let (line, column) = position(source, offset);
    JsTsError {
        path: path.to_path_buf(),
        line,
        column,
        message: diagnostic.message.to_string(),
    }
}

/// The 1-based line and column of byte `offset` in `source`. The column counts characters, the way
/// an editor shows it. An offset past the end, or inside a character, is clamped back to the last
/// character boundary before it.
pub(crate) fn position(source: &str, offset: usize) -> (u32, u32) {
    let mut end = offset.min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let before = &source[..end];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |text| text.chars().count()) + 1;
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{position, strip_types};

    fn strip(source: &str) -> String {
        strip_types(Path::new("/src/m.ts"), source)
            .expect("the source is valid TypeScript")
            .code
    }

    #[test]
    fn strips_annotations() {
        let code = strip("const n: number = 1;\nfunction f(a: string): boolean { return a.length > n; }\n");
        assert!(!code.contains(": number"), "{code}");
        assert!(!code.contains(": string"), "{code}");
        assert!(code.contains("const n = 1"), "{code}");
    }

    #[test]
    fn interface_is_removed() {
        let code = strip("interface Point { x: number; y: number }\nexport const p = { x: 1, y: 2 };\n");
        assert!(!code.contains("interface"), "{code}");
        assert!(code.contains("export const p"), "{code}");
    }

    #[test]
    fn enum_becomes_plain_javascript() {
        let code = strip("export enum Mode { Tile, Float }\n");
        assert!(!code.contains("enum "), "{code}");
        assert!(code.contains("Mode"), "{code}");
    }

    #[test]
    fn import_type_is_dropped() {
        let code =
            strip("import type { Shape } from './shape';\nimport { area } from './area';\nexport const a = area;\n");
        assert!(!code.contains("./shape"), "{code}");
        assert!(code.contains("./area"), "{code}");
    }

    #[test]
    fn parameter_property_is_assigned_in_constructor() {
        let code = strip("export class Box {\n  constructor(private readonly size: number) {}\n}\n");
        assert!(code.contains("this.size = size"), "{code}");
        assert!(!code.contains("private"), "{code}");
    }

    #[test]
    fn source_map_is_produced() {
        let stripped = strip_types(Path::new("/src/m.ts"), "const n: number = 1;\n").expect("valid TypeScript");
        assert!(stripped.map.is_some());
    }

    #[test]
    fn syntax_error_reports_path_line_and_column() {
        let source = "const ok = 1;\nconst broken = ;\n";
        let error = strip_types(Path::new("/src/bad.ts"), source).expect_err("the source does not parse");
        assert_eq!(error.path, Path::new("/src/bad.ts"));
        assert_eq!(error.line, 2, "{error}");
        assert_eq!(error.column, 16, "{error}");
    }

    #[test]
    fn position_counts_lines_and_characters() {
        assert_eq!(position("ab\ncd", 4), (2, 2));
        assert_eq!(position("", 0), (1, 1));
        assert_eq!(position("\u{e9}x", 2), (1, 2));
        assert_eq!(position("ab", 99), (1, 3));
    }
}
