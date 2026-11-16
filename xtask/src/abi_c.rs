//! Rust-to-C translation of the ABI's type strings, and the C header built from them.
//!
//! `gnoblin-core-abi` describes every type, constant and export as data (`reflect`), with each type kept as the string the
//! compiler saw (`stringify!`). The ABI's type grammar is deliberately tiny, so this module parses exactly that grammar:
//! primitives and named types, `*const T` and `*mut T`, `Option<fn>`, arrays, and `extern "C"` function pointers with named
//! parameters. A type outside the grammar is an error, so a new construct cannot slip into the ABI unrendered.
//!
//! Why not `cbindgen`: it is not installed on the build machines, and it would read source text where this reads the
//! layouts rustc computed. The same parse also gives the canonical type text `abi.lock` records, so a change of the
//! compiler's pretty-printing never reads as an ABI change.

use std::fmt::Write as _;

use gnoblin_core_abi::reflect::{ConstDef, ExportDef, Reflection, StructLayout};

/// A type in the ABI's grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// A primitive or a type the ABI declares (`u32`, `GnoblinCtx`).
    Named(String),
    /// `*const T` or `*mut T`.
    Ptr { mutable: bool, to: Box<Ty> },
    /// `Option<T>` where `T` is a function pointer: a nullable function pointer in C.
    Option(Box<Ty>),
    /// `[T; N]`.
    Array(Box<Ty>, usize),
    /// A function pointer.
    Fn(FnSig),
}

/// A function type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnSig {
    pub unsafe_fn: bool,
    pub extern_c: bool,
    pub params: Vec<(Option<String>, Ty)>,
    pub ret: Option<Box<Ty>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    Str(String),
    Number(usize),
    Punct(char),
    Arrow,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c.is_whitespace() {
            index += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut ident = String::new();
            loop {
                while index < chars.len() && (chars[index].is_ascii_alphanumeric() || chars[index] == '_') {
                    ident.push(chars[index]);
                    index += 1;
                }
                // A path such as `core::ffi::c_void` keeps its last segment.
                if chars.get(index) == Some(&':') && chars.get(index + 1) == Some(&':') {
                    index += 2;
                    ident.clear();
                } else {
                    break;
                }
            }
            tokens.push(Token::Ident(ident));
        } else if c.is_ascii_digit() {
            let mut value = 0usize;
            while index < chars.len() && chars[index].is_ascii_digit() {
                value = value * 10 + chars[index].to_digit(10).unwrap_or(0) as usize;
                index += 1;
            }
            tokens.push(Token::Number(value));
        } else if c == '"' {
            let end = chars[index + 1..]
                .iter()
                .position(|&ch| ch == '"')
                .ok_or_else(|| format!("unterminated string in type `{text}`"))?;
            tokens.push(Token::Str(chars[index + 1..index + 1 + end].iter().collect()));
            index += end + 2;
        } else if c == '-' && chars.get(index + 1) == Some(&'>') {
            tokens.push(Token::Arrow);
            index += 2;
        } else if "*<>(),:;[]".contains(c) {
            tokens.push(Token::Punct(c));
            index += 1;
        } else {
            return Err(format!("unexpected `{c}` in type `{text}`"));
        }
    }
    Ok(tokens)
}

struct TypeParser {
    tokens: Vec<Token>,
    at: usize,
    text: String,
}

impl TypeParser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn error(&self, message: &str) -> String {
        format!("{message} in type `{}`", self.text)
    }

    fn punct(&mut self, expected: char) -> Result<(), String> {
        match self.next() {
            Some(Token::Punct(c)) if c == expected => Ok(()),
            _ => Err(self.error(&format!("expected `{expected}`"))),
        }
    }

    fn ident(&mut self, expected: &str) -> Result<(), String> {
        match self.next() {
            Some(Token::Ident(name)) if name == expected => Ok(()),
            _ => Err(self.error(&format!("expected `{expected}`"))),
        }
    }

    fn eat_ident(&mut self, expected: &str) -> bool {
        if matches!(self.peek(), Some(Token::Ident(name)) if name == expected) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn ty(&mut self) -> Result<Ty, String> {
        match self.peek().cloned() {
            Some(Token::Punct('*')) => {
                self.at += 1;
                let mutable = match self.next() {
                    Some(Token::Ident(word)) if word == "mut" => true,
                    Some(Token::Ident(word)) if word == "const" => false,
                    _ => return Err(self.error("expected `const` or `mut` after `*`")),
                };
                Ok(Ty::Ptr {
                    mutable,
                    to: Box::new(self.ty()?),
                })
            }
            Some(Token::Punct('[')) => {
                self.at += 1;
                let inner = self.ty()?;
                self.punct(';')?;
                let Some(Token::Number(count)) = self.next() else {
                    return Err(self.error("expected an array length"));
                };
                self.punct(']')?;
                Ok(Ty::Array(Box::new(inner), count))
            }
            Some(Token::Ident(word)) if word == "Option" => {
                self.at += 1;
                self.punct('<')?;
                let inner = self.ty()?;
                self.punct('>')?;
                if !matches!(inner, Ty::Fn(_) | Ty::Named(_)) {
                    return Err(self.error("`Option` is only used for function pointers and their typedef names"));
                }
                Ok(Ty::Option(Box::new(inner)))
            }
            Some(Token::Ident(word)) if matches!(word.as_str(), "unsafe" | "extern" | "fn") => self.function(),
            Some(Token::Ident(name)) => {
                self.at += 1;
                Ok(Ty::Named(name))
            }
            _ => Err(self.error("expected a type")),
        }
    }

    fn function(&mut self) -> Result<Ty, String> {
        let unsafe_fn = self.eat_ident("unsafe");
        let mut extern_c = false;
        if self.eat_ident("extern") {
            match self.next() {
                Some(Token::Str(abi)) if abi == "C" => extern_c = true,
                _ => return Err(self.error("only `extern \"C\"` functions are part of the ABI")),
            }
        }
        self.ident("fn")?;
        self.punct('(')?;
        let mut params = Vec::new();
        loop {
            if matches!(self.peek(), Some(Token::Punct(')'))) {
                self.at += 1;
                break;
            }
            let name = match (self.peek().cloned(), self.tokens.get(self.at + 1)) {
                (Some(Token::Ident(name)), Some(Token::Punct(':'))) => {
                    self.at += 2;
                    Some(name)
                }
                _ => None,
            };
            params.push((name, self.ty()?));
            match self.peek() {
                Some(Token::Punct(',')) => self.at += 1,
                Some(Token::Punct(')')) => {}
                _ => return Err(self.error("expected `,` or `)`")),
            }
        }
        let ret = if matches!(self.peek(), Some(Token::Arrow)) {
            self.at += 1;
            Some(Box::new(self.ty()?))
        } else {
            None
        };
        Ok(Ty::Fn(FnSig {
            unsafe_fn,
            extern_c,
            params,
            ret,
        }))
    }
}

/// Parses one type of the ABI's grammar.
pub fn parse(text: &str) -> Result<Ty, String> {
    let mut parser = TypeParser {
        tokens: tokenize(text)?,
        at: 0,
        text: text.to_owned(),
    };
    let ty = parser.ty()?;
    if parser.at != parser.tokens.len() {
        return Err(parser.error("trailing tokens"));
    }
    Ok(ty)
}

/// The type as canonical Rust text, independent of how the compiler spaced it. This is what `abi.lock` records.
pub fn canonical(ty: &Ty) -> String {
    match ty {
        Ty::Named(name) => name.clone(),
        Ty::Ptr { mutable, to } => format!("*{} {}", if *mutable { "mut" } else { "const" }, canonical(to)),
        Ty::Option(inner) => format!("Option<{}>", canonical(inner)),
        Ty::Array(inner, count) => format!("[{}; {count}]", canonical(inner)),
        Ty::Fn(sig) => canonical_fn(sig),
    }
}

fn canonical_fn(sig: &FnSig) -> String {
    let mut text = String::new();
    if sig.unsafe_fn {
        text.push_str("unsafe ");
    }
    if sig.extern_c {
        text.push_str("extern \"C\" ");
    }
    text.push_str("fn(");
    let params: Vec<String> = sig
        .params
        .iter()
        .map(|(name, ty)| match name {
            Some(name) => format!("{name}: {}", canonical(ty)),
            None => canonical(ty),
        })
        .collect();
    text.push_str(&params.join(", "));
    text.push(')');
    if let Some(ret) = &sig.ret {
        let _ = write!(text, " -> {}", canonical(ret));
    }
    text
}

/// The C spelling of a named type.
fn c_name(name: &str) -> &str {
    match name {
        "u8" => "uint8_t",
        "u16" => "uint16_t",
        "u32" => "uint32_t",
        "u64" => "uint64_t",
        "i8" => "int8_t",
        "i16" => "int16_t",
        "i32" => "int32_t",
        "i64" => "int64_t",
        "usize" => "size_t",
        "isize" => "ptrdiff_t",
        "f32" => "float",
        "f64" => "double",
        "c_void" => "void",
        "c_char" => "char",
        "c_int" => "int",
        other => other,
    }
}

/// A C declaration of `name` with type `ty`, e.g. `const uint8_t *ptr` or `int32_t (*init)(const GnoblinApi *api)`.
pub fn declaration(ty: &Ty, name: &str) -> String {
    decl(ty, name.to_owned(), false)
}

fn decl(ty: &Ty, inner: String, is_const: bool) -> String {
    match ty {
        Ty::Named(n) => {
            let base = format!("{}{}", if is_const { "const " } else { "" }, c_name(n));
            if inner.is_empty() {
                base
            } else {
                format!("{base} {inner}")
            }
        }
        Ty::Ptr { mutable, to } => {
            let wrapped = if matches!(**to, Ty::Fn(_) | Ty::Array(..)) {
                format!("(*{inner})")
            } else {
                format!("*{inner}")
            };
            decl(to, wrapped, !*mutable)
        }
        Ty::Option(inner_ty) => decl(inner_ty, inner, is_const),
        Ty::Array(element, count) => decl(element, format!("{inner}[{count}]"), is_const),
        Ty::Fn(sig) => decl_fn(sig, format!("(*{inner})")),
    }
}

/// `R declarator(params)` for a function whose declarator is already formed (`name` for a prototype, `(*name)` for a pointer).
fn decl_fn(sig: &FnSig, declarator: String) -> String {
    let params = if sig.params.is_empty() {
        "void".to_owned()
    } else {
        sig.params
            .iter()
            .map(|(name, ty)| declaration(ty, name.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let call = format!("{declarator}({params})");
    match &sig.ret {
        Some(ret) => decl(ret, call, false),
        None => format!("void {call}"),
    }
}

/// A C function prototype for an export: `int32_t gnoblin_get_api(uint32_t want_major, ...)`.
pub fn prototype(ty: &Ty, symbol: &str) -> Result<String, String> {
    match ty {
        Ty::Fn(sig) => Ok(decl_fn(sig, symbol.to_owned())),
        _ => Err(format!("export `{symbol}` is not a function")),
    }
}

/// A doc comment as a C block comment; empty docs give nothing.
fn comment(doc: &str, indent: &str) -> String {
    let lines: Vec<String> = doc
        .lines()
        .map(|line| line.strip_prefix(' ').unwrap_or(line).trim_end().replace("*/", "* /"))
        .collect();
    let lines: Vec<&str> = {
        let start = lines.iter().position(|line| !line.is_empty()).unwrap_or(lines.len());
        let end = lines
            .iter()
            .rposition(|line| !line.is_empty())
            .map_or(start, |end| end + 1);
        lines[start..end].iter().map(String::as_str).collect()
    };
    match lines.as_slice() {
        [] => String::new(),
        [one] => format!("{indent}/* {one} */\n"),
        many => {
            let mut text = format!("{indent}/*\n");
            for line in many {
                if line.is_empty() {
                    let _ = writeln!(text, "{indent} *");
                } else {
                    let _ = writeln!(text, "{indent} * {line}");
                }
            }
            let _ = writeln!(text, "{indent} */");
            text
        }
    }
}

fn define(constant: &ConstDef) -> String {
    let value = if constant.value < 0 {
        format!("({})", constant.value)
    } else {
        constant.value.to_string()
    };
    // Unsigned constants get a `u` suffix so they keep their type in an expression.
    let suffix = if constant.ty.starts_with('u') { "u" } else { "" };
    format!("#define GNOBLIN_{} {value}{suffix}\n", constant.name)
}

fn render_struct(layout: &StructLayout) -> Result<String, String> {
    let mut text = comment(layout.doc, "");
    let _ = writeln!(text, "struct {} {{", layout.name);
    for field in layout.fields {
        text.push_str(&comment(field.doc, "    "));
        let ty = parse(field.ty).map_err(|error| format!("{}.{}: {error}", layout.name, field.name))?;
        let _ = writeln!(text, "    {};", declaration(&ty, field.name));
    }
    text.push_str("};\n");
    Ok(text)
}

/// The text of `include/gnoblin/core.h` for this build of the ABI crate.
pub fn render_header(reflection: &Reflection) -> Result<String, String> {
    let mut text = String::new();
    text.push_str("/* gnoblin/core.h: the C ABI of libgnoblin-core.\n");
    text.push_str(" *\n");
    text.push_str(
        " * Generated by `cargo xtask abi-header` from the declarations in crates/gnoblin-core-abi; do not edit.\n",
    );
    text.push_str(" * Link with -lgnoblin-core (pkg-config: gnoblin-core). A native shell exports one symbol,\n");
    text.push_str(" * `const GnoblinShellEntry *gnoshell_entry_v1(void)`, and reaches everything else through the\n");
    text.push_str(
        " * function table returned by gnoblin_get_api(). Strings are UTF-8 (pointer, length), not NUL terminated.\n",
    );
    text.push_str(" */\n");
    text.push_str("#ifndef GNOBLIN_CORE_H\n#define GNOBLIN_CORE_H\n\n");
    text.push_str("#include <stddef.h>\n#include <stdint.h>\n\n");
    text.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    let _ = writeln!(text, "#define GNOBLIN_ABI_MAJOR {}", reflection.major);
    let _ = writeln!(text, "#define GNOBLIN_ABI_MINOR {}", reflection.minor);
    let _ = writeln!(text, "#define GNOBLIN_ABI_PATCH {}", reflection.patch);
    let _ = writeln!(text, "#define GNOBLIN_SONAME \"{}\"", reflection.soname);
    let _ = writeln!(
        text,
        "#define GNOBLIN_SHELL_ENTRY_SYMBOL \"{}\"\n",
        gnoblin_core_abi::SHELL_ENTRY_SYMBOL
    );

    for constant in reflection.consts {
        text.push_str(&comment(constant.doc, ""));
        text.push_str(&define(constant));
    }
    text.push('\n');

    for alias in reflection.aliases {
        text.push_str(&comment(alias.doc, ""));
        let ty = parse(alias.ty).map_err(|error| format!("{}: {error}", alias.name))?;
        let _ = writeln!(text, "typedef {};", declaration(&ty, alias.name));
    }
    text.push('\n');

    for layout in reflection.structs {
        let _ = writeln!(text, "typedef struct {0} {0};", layout.name);
    }
    text.push('\n');

    for callback in reflection.callbacks {
        text.push_str(&comment(callback.doc, ""));
        let ty = parse(callback.ty).map_err(|error| format!("{}: {error}", callback.name))?;
        let _ = writeln!(text, "typedef {};", declaration(&ty, callback.name));
    }
    text.push('\n');

    for layout in reflection.structs {
        text.push_str(&render_struct(layout)?);
        text.push('\n');
    }

    for export in reflection.exports {
        text.push_str(&export_prototype(export)?);
    }
    text.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n#endif /* GNOBLIN_CORE_H */\n");
    Ok(text)
}

fn export_prototype(export: &ExportDef) -> Result<String, String> {
    let ty = parse(export.ty).map_err(|error| format!("{}: {error}", export.symbol))?;
    let mut text = comment(export.doc, "");
    let _ = writeln!(text, "{};", prototype(&ty, export.symbol)?);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_pointers_and_paths_parse() {
        assert_eq!(parse("u32").unwrap(), Ty::Named("u32".into()));
        assert_eq!(
            parse("*const u8").unwrap(),
            Ty::Ptr {
                mutable: false,
                to: Box::new(Ty::Named("u8".into()))
            }
        );
        assert_eq!(
            parse("* mut core::ffi::c_void").unwrap(),
            Ty::Ptr {
                mutable: true,
                to: Box::new(Ty::Named("c_void".into()))
            }
        );
        assert_eq!(
            parse("[u32; 4]").unwrap(),
            Ty::Array(Box::new(Ty::Named("u32".into())), 4)
        );
    }

    #[test]
    fn function_pointers_keep_parameter_names_and_return_types() {
        let ty = parse("unsafe extern \"C\" fn(ctx: GnoblinCtx, out : * mut GnoblinCtxInfo) -> i32").unwrap();
        assert_eq!(
            canonical(&ty),
            "unsafe extern \"C\" fn(ctx: GnoblinCtx, out: *mut GnoblinCtxInfo) -> i32"
        );
        let nullable = parse("Option<unsafe extern \"C\" fn(user: *mut c_void)>").unwrap();
        assert_eq!(
            canonical(&nullable),
            "Option<unsafe extern \"C\" fn(user: *mut c_void)>"
        );
    }

    #[test]
    fn canonical_text_ignores_the_compilers_spacing() {
        let tight = parse("unsafe extern \"C\" fn(a:u32,b:*const u8)->u64").unwrap();
        let loose = parse("unsafe   extern \"C\"  fn ( a : u32 , b : * const  u8 ) -> u64").unwrap();
        assert_eq!(canonical(&tight), canonical(&loose));
    }

    #[test]
    fn constructs_outside_the_grammar_are_refused() {
        for bad in [
            "Vec<u8>",
            "extern \"Rust\" fn()",
            "Option<*const u8>",
            "&u8",
            "*u8",
            "fn(",
            "u32 u32",
        ] {
            assert!(parse(bad).is_err(), "`{bad}` parsed");
        }
    }

    #[test]
    fn c_declarators_follow_the_inside_out_rule() {
        let cases = [
            ("*const u8", "ptr", "const uint8_t *ptr"),
            ("*mut u32", "out", "uint32_t *out"),
            ("*mut *const GnoblinApi", "out", "const GnoblinApi **out"),
            ("usize", "len", "size_t len"),
            ("*mut c_void", "user", "void *user"),
            (
                "Option<unsafe extern \"C\" fn(user: *mut c_void)>",
                "drop_user",
                "void (*drop_user)(void *user)",
            ),
            (
                "unsafe extern \"C\" fn(ctx: GnoblinCtx, out: *mut GnoblinCtxInfo) -> i32",
                "ctx_info",
                "int32_t (*ctx_info)(GnoblinCtx ctx, GnoblinCtxInfo *out)",
            ),
            (
                "unsafe extern \"C\" fn() -> u64",
                "uptime_ms",
                "uint64_t (*uptime_ms)(void)",
            ),
            (
                "unsafe extern \"C\" fn(ctx: GnoblinCtx)",
                "shutdown",
                "void (*shutdown)(GnoblinCtx ctx)",
            ),
            ("[u32; 4]", "kinds", "uint32_t kinds[4]"),
        ];
        for (rust, name, c) in cases {
            assert_eq!(declaration(&parse(rust).unwrap(), name), c, "{rust}");
        }
    }

    #[test]
    fn a_prototype_is_a_plain_function_declaration() {
        let ty = parse("unsafe extern \"C\" fn(want_major: u32, built_minor: u32, out: *mut *const GnoblinApi) -> i32")
            .unwrap();
        assert_eq!(
            prototype(&ty, "gnoblin_get_api").unwrap(),
            "int32_t gnoblin_get_api(uint32_t want_major, uint32_t built_minor, const GnoblinApi **out)"
        );
        let void = parse("unsafe extern \"C\" fn(major: *mut u32)").unwrap();
        assert_eq!(
            prototype(&void, "gnoblin_x").unwrap(),
            "void gnoblin_x(uint32_t *major)"
        );
    }

    #[test]
    fn doc_comments_become_c_comments_without_closing_the_comment_early() {
        assert_eq!(comment(" One line.\n", ""), "/* One line. */\n");
        assert_eq!(comment(" a\n b */ c\n", ""), "/*\n * a\n * b * / c\n */\n");
        assert_eq!(comment("", ""), "");
    }

    #[test]
    fn negative_and_unsigned_constants_are_rendered_as_c_would_have_them() {
        let negative = ConstDef {
            name: "E_PANIC",
            ty: "i32",
            value: -2,
            doc: "",
        };
        assert_eq!(define(&negative), "#define GNOBLIN_E_PANIC (-2)\n");
        let unsigned = ConstDef {
            name: "VALUE_BOOL",
            ty: "u32",
            value: 1,
            doc: "",
        };
        assert_eq!(define(&unsigned), "#define GNOBLIN_VALUE_BOOL 1u\n");
    }

    #[test]
    fn every_type_of_the_real_abi_parses() {
        let reflection = gnoblin_core_abi::reflect::reflection();
        for layout in reflection.structs {
            for field in layout.fields {
                parse(field.ty).unwrap_or_else(|error| panic!("{}.{}: {error}", layout.name, field.name));
            }
        }
        for callback in reflection.callbacks {
            parse(callback.ty).unwrap();
        }
        for export in reflection.exports {
            parse(export.ty).unwrap();
        }
        for alias in reflection.aliases {
            parse(alias.ty).unwrap();
        }
    }

    #[test]
    fn the_header_declares_every_struct_before_using_it_and_ends_its_guard() {
        let header = render_header(&gnoblin_core_abi::reflect::reflection()).unwrap();
        assert!(header.starts_with("/* gnoblin/core.h"));
        assert!(header.contains("typedef struct GnoblinApi GnoblinApi;"));
        assert!(header.contains("struct GnoblinApi {"));
        assert!(header.contains("typedef uint64_t GnoblinCtx;"));
        assert!(
            header.contains(
                "int32_t gnoblin_get_api(uint32_t want_major, uint32_t built_minor, const GnoblinApi **out);"
            )
        );
        assert!(header.contains("#define GNOBLIN_E_RETIRED (-7)"));
        assert!(header.trim_end().ends_with("#endif /* GNOBLIN_CORE_H */"));
        let typedef = header.find("typedef struct GnoblinApi GnoblinApi;").unwrap();
        let definition = header.find("struct GnoblinApi {").unwrap();
        assert!(typedef < definition);
    }
}
