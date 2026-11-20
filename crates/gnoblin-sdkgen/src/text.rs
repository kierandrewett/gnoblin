//! Identifier casing and the small code writer the targets share.
//!
//! Every target spells the same schema and ABI names in its own language's convention. The conversions live here, once, so
//! two targets cannot disagree about what `app_id` or `portal_ui` becomes, and a reserved word is escaped the same way
//! wherever it appears.

use std::fmt::Write as _;

/// `portal_ui` and `portal-ui` as `PortalUi`.
pub fn pascal(name: &str) -> String {
    let mut out = String::new();
    for word in name.split(['_', '-', '.']).filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// `portal_ui` as `portalUi`.
pub fn camel(name: &str) -> String {
    let pascal = pascal(name);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `PortalUi` as `portal_ui`, and `portal-ui` as `portal_ui`.
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c == '-' || c == '.' {
            out.push('_');
            previous_lower = false;
        } else if c.is_ascii_uppercase() {
            if previous_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            previous_lower = false;
        } else {
            out.push(c);
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        }
    }
    out
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become",
    "box", "do", "final", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

/// A Rust identifier for `name`: a keyword gets a trailing underscore (`type` becomes `type_`).
pub fn rust_ident(name: &str) -> String {
    if RUST_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match", "case", "type",
];

/// A Python identifier for `name`: a keyword gets a trailing underscore (`from` becomes `from_`).
pub fn python_ident(name: &str) -> String {
    if PYTHON_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

const TS_RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    "await",
    "async",
];

/// A TypeScript identifier for `name`: a reserved word gets a trailing underscore.
pub fn ts_ident(name: &str) -> String {
    if TS_RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

/// An indenting line writer. A target builds a file with it and takes the text at the end.
pub struct CodeWriter {
    text: String,
    level: usize,
    unit: &'static str,
}

impl CodeWriter {
    /// A writer that indents by `unit` per level.
    pub fn new(unit: &'static str) -> CodeWriter {
        CodeWriter {
            text: String::new(),
            level: 0,
            unit,
        }
    }

    /// One line at the current level; an empty line is written bare, with no trailing spaces.
    pub fn line(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        if !line.is_empty() {
            for _ in 0..self.level {
                self.text.push_str(self.unit);
            }
            self.text.push_str(line);
        }
        self.text.push('\n');
    }

    /// An empty line, unless the text already ends with one (or is empty), so a generator can call it freely.
    pub fn blank(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with("\n\n") {
            self.text.push('\n');
        }
    }

    /// One level deeper.
    pub fn indent(&mut self) {
        self.level += 1;
    }

    /// One level back.
    pub fn dedent(&mut self) {
        self.level = self.level.saturating_sub(1);
    }

    /// Writes `open`, runs `body` one level deeper, then writes `close`.
    pub fn block(&mut self, open: impl AsRef<str>, body: impl FnOnce(&mut CodeWriter), close: impl AsRef<str>) {
        self.line(open);
        self.indent();
        body(self);
        self.dedent();
        self.line(close);
    }

    /// Doc comment lines with `prefix` (`///`, `#`, ` *`), one per line of `doc`.
    pub fn doc(&mut self, prefix: &str, doc: &[String]) {
        for line in doc {
            if line.is_empty() {
                self.line(prefix);
            } else {
                self.line(format!("{prefix} {line}"));
            }
        }
    }

    /// The finished text, with one newline at the end.
    pub fn finish(mut self) -> String {
        while self.text.ends_with("\n\n") {
            self.text.pop();
        }
        if !self.text.ends_with('\n') {
            let _ = writeln!(self.text);
        }
        self.text
    }
}

/// The lines of a doc string the ABI macros produce: one `///` literal per line, each starting with a space.
pub fn doc_lines(doc: &str) -> Vec<String> {
    let mut lines: Vec<String> = doc
        .lines()
        .map(|line| line.strip_prefix(' ').unwrap_or(line).trim_end().to_owned())
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casing_follows_each_language() {
        assert_eq!(pascal("portal_ui"), "PortalUi");
        assert_eq!(camel("gap_inner"), "gapInner");
        assert_eq!(pascal("gnoblin.session"), "GnoblinSession");
        assert_eq!(snake("PortalUi"), "portal_ui");
        assert_eq!(snake("GnoblinStdEvent"), "gnoblin_std_event");

        assert_eq!(rust_ident("type"), "type_");
        assert_eq!(python_ident("from"), "from_");
        assert_eq!(ts_ident("default"), "default_");
        assert_eq!(ts_ident("title"), "title");
    }

    #[test]
    fn doc_lines_drop_the_macro_padding() {
        assert_eq!(doc_lines(" One.\n Two.\n"), vec!["One.".to_owned(), "Two.".to_owned()]);
        assert!(doc_lines("").is_empty());
        assert!(doc_lines("\n").is_empty());
    }

    #[test]
    fn the_writer_indents_and_collapses_blank_lines() {
        let mut code = CodeWriter::new("    ");
        code.block("fn a() {", |code| code.line("b();"), "}");
        code.blank();
        code.blank();
        code.line("c");
        assert_eq!(code.finish(), "fn a() {\n    b();\n}\n\nc\n");
    }
}
