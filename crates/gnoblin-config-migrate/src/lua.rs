//! Lua source to a tree the converter can read.
//!
//! The parser (`full_moon`) gives a syntax tree and each node's position; nothing is run. This module turns an
//! expression into an [`Lv`]: a value with the file, line and text it came from, or a [`LuaKind::Failed`] that says why it
//! has no static value. A table literal, a string, a number, a boolean, `nil`, the arithmetic of numbers, the
//! concatenation of two strings and a name bound earlier to such a value are static. A call, a function, an index, a
//! comparison and every other operator are not.

use std::collections::HashMap;
use std::rc::Rc;

use full_moon::LuaVersion;
use full_moon::ast::{Call, Expression, Field as AstField, FunctionArgs, FunctionCall, Index, Prefix, Suffix, Var};
use full_moon::node::Node;
use full_moon::tokenizer::{StringLiteralQuoteType, Symbol, TokenReference, TokenType};

/// Where a piece of source is: the file, the line and the text, for a message and for a comment in the output.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Span {
    pub file: Rc<str>,
    pub line: usize,
    pub text: String,
}

/// A value of the source, with where it was written.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lv {
    pub span: Span,
    pub kind: LuaKind,
}

/// What a value is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LuaKind {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Nil,
    /// A table written with positional elements only.
    List(Vec<Lv>),
    /// A table written with keys only; `{}` is a table with no field.
    Table(Vec<LuaField>),
    /// Not a static value: why.
    Failed(String),
}

/// One `key = value` of a table.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LuaField {
    /// The key, or empty when the key is not a literal string.
    pub key: String,
    /// The whole field as written.
    pub span: Span,
    pub value: Lv,
}

impl Lv {
    pub(crate) fn failed(span: Span, reason: impl Into<String>) -> Lv {
        Lv {
            span,
            kind: LuaKind::Failed(reason.into()),
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match &self.kind {
            LuaKind::Str(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self.kind {
            LuaKind::Bool(value) => Some(value),
            _ => None,
        }
    }

    /// The elements of a list, and of the empty table that is also an empty list.
    pub(crate) fn elements(&self) -> Option<&[Lv]> {
        match &self.kind {
            LuaKind::List(items) => Some(items),
            LuaKind::Table(fields) if fields.is_empty() => Some(&[]),
            _ => None,
        }
    }

    /// The fields of a table, and of the empty list that is also an empty table.
    pub(crate) fn fields(&self) -> Option<&[LuaField]> {
        match &self.kind {
            LuaKind::Table(fields) => Some(fields),
            LuaKind::List(items) if items.is_empty() => Some(&[]),
            _ => None,
        }
    }

    /// What kind of thing this is, for a message.
    pub(crate) fn describe(&self) -> &'static str {
        match self.kind {
            LuaKind::Str(_) => "a string",
            LuaKind::Int(_) | LuaKind::Float(_) => "a number",
            LuaKind::Bool(_) => "a boolean",
            LuaKind::Nil => "nil",
            LuaKind::List(_) => "a list",
            LuaKind::Table(_) => "a table",
            LuaKind::Failed(_) => "a value computed at run time",
        }
    }
}

/// One file being converted: its name, as the user sees it, and its text.
pub(crate) struct SourceFile<'s> {
    pub name: Rc<str>,
    pub source: &'s str,
}

impl SourceFile<'_> {
    /// The span of a node: the line it starts on and the text it covers, without the comments around it.
    pub(crate) fn span(&self, node: &impl Node) -> Span {
        let start = node.start_position();
        let end = node.end_position();
        let text = match (start, end) {
            (Some(from), Some(to)) => self.source.get(from.bytes()..to.bytes()).map(str::to_owned),
            _ => None,
        };
        Span {
            file: Rc::clone(&self.name),
            line: start.map_or(1, |position| position.line()),
            text: text.unwrap_or_default(),
        }
    }
}

/// The names bound so far: a file's locals and the globals every file shares.
pub(crate) struct LuaScope<'a> {
    pub locals: &'a HashMap<String, Lv>,
    pub globals: &'a HashMap<String, Lv>,
}

impl LuaScope<'_> {
    fn get(&self, name: &str) -> Option<&Lv> {
        self.locals.get(name).or_else(|| self.globals.get(name))
    }
}

/// Parses `source` as Lua 5.4. On failure: the line and the parser's message.
pub(crate) fn parse(source: &str) -> Result<full_moon::ast::Ast, (usize, String)> {
    full_moon::parse_fallible(source, LuaVersion::lua54())
        .into_result()
        .map_err(|errors| {
            let first = errors.first();
            let line = first.map_or(1, |error| error.range().0.line());
            let message = errors
                .iter()
                .map(|error| error.error_message().into_owned())
                .collect::<Vec<_>>()
                .join("; ");
            (line, message)
        })
}

/// The static value of `expr`, or a [`LuaKind::Failed`] saying why it has none.
pub(crate) fn lower(file: &SourceFile<'_>, scope: &LuaScope<'_>, expr: &Expression) -> Lv {
    let span = file.span(expr);
    let kind = match expr {
        Expression::Number(token) => number(&token.token().to_string()),
        Expression::String(token) => string(token),
        Expression::Symbol(token) => match token.token().token_type() {
            TokenType::Symbol { symbol: Symbol::True } => LuaKind::Bool(true),
            TokenType::Symbol { symbol: Symbol::False } => LuaKind::Bool(false),
            TokenType::Symbol { symbol: Symbol::Nil } => LuaKind::Nil,
            _ => LuaKind::Failed("a vararg is only known at run time".to_owned()),
        },
        Expression::Parentheses { expression, .. } => return lower(file, scope, expression),
        Expression::UnaryOperator { unop, expression } => {
            let operand = lower(file, scope, expression);
            unary(&unop.token().token().to_string(), operand.kind)
        }
        Expression::BinaryOperator { lhs, binop, rhs } => {
            let left = lower(file, scope, lhs);
            let right = lower(file, scope, rhs);
            binary(&binop.token().token().to_string(), left.kind, right.kind)
        }
        Expression::TableConstructor(table) => {
            return lower_table(file, scope, table.fields().iter(), span);
        }
        Expression::Var(Var::Name(token)) => {
            let name = token.token().to_string();
            match scope.get(&name) {
                Some(bound) => return bound.clone(),
                None => LuaKind::Failed(format!("`{name}` is not a value the converter knows")),
            }
        }
        Expression::Var(_) => LuaKind::Failed("an index of a value is evaluated at run time".to_owned()),
        Expression::FunctionCall(_) => LuaKind::Failed("a function call is evaluated at run time".to_owned()),
        Expression::Function(_) => LuaKind::Failed("a function".to_owned()),
        _ => LuaKind::Failed("an expression the converter does not evaluate".to_owned()),
    };
    Lv { span, kind }
}

fn lower_table<'a>(
    file: &SourceFile<'_>,
    scope: &LuaScope<'_>,
    fields: impl Iterator<Item = &'a AstField>,
    span: Span,
) -> Lv {
    let mut positional: Vec<Lv> = Vec::new();
    let mut keyed: Vec<LuaField> = Vec::new();
    for field in fields {
        let field_span = file.span(field);
        match field {
            AstField::NameKey { key, value, .. } => {
                let key = key.token().to_string();
                add_field(&mut keyed, key, field_span, lower(file, scope, value));
            }
            AstField::ExpressionKey { key, value, .. } => {
                let key = lower(file, scope, key);
                let value = lower(file, scope, value);
                match key.kind {
                    LuaKind::Str(text) if !text.is_empty() => add_field(&mut keyed, text, field_span, value),
                    _ => {
                        let failed = Lv::failed(field_span.clone(), "a key that is not a literal string");
                        keyed.push(LuaField {
                            key: String::new(),
                            span: field_span,
                            value: failed,
                        });
                    }
                }
            }
            AstField::NoKey(expr) => positional.push(lower(file, scope, expr)),
            _ => {
                let failed = Lv::failed(field_span.clone(), "a table field the converter does not read");
                keyed.push(LuaField {
                    key: String::new(),
                    span: field_span,
                    value: failed,
                });
            }
        }
    }
    let kind = if positional.is_empty() {
        LuaKind::Table(keyed)
    } else if keyed.is_empty() {
        LuaKind::List(positional)
    } else {
        LuaKind::Failed("a table that mixes a list and keys".to_owned())
    };
    Lv { span, kind }
}

/// Adds a field; a key written twice keeps the later value in the earlier place, as the table would.
fn add_field(fields: &mut Vec<LuaField>, key: String, span: Span, value: Lv) {
    match fields.iter_mut().find(|field| field.key == key) {
        Some(existing) => {
            existing.span = span;
            existing.value = value;
        }
        None => fields.push(LuaField { key, span, value }),
    }
}

fn number(text: &str) -> LuaKind {
    let lower = text.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return match i64::from_str_radix(hex, 16) {
            Ok(value) => LuaKind::Int(value),
            Err(_) => LuaKind::Failed("a hexadecimal number the converter does not read".to_owned()),
        };
    }
    if let Ok(value) = lower.parse::<i64>() {
        return LuaKind::Int(value);
    }
    match lower.parse::<f64>() {
        Ok(value) if value.is_finite() => LuaKind::Float(value),
        _ => LuaKind::Failed("a number the converter does not read".to_owned()),
    }
}

fn string(token: &TokenReference) -> LuaKind {
    let TokenType::StringLiteral {
        literal, quote_type, ..
    } = token.token().token_type()
    else {
        return LuaKind::Failed("a string the converter does not read".to_owned());
    };
    let raw = literal.to_string();
    if *quote_type == StringLiteralQuoteType::Brackets {
        // A long string drops a newline right after its opening bracket.
        let body = raw
            .strip_prefix("\r\n")
            .or_else(|| raw.strip_prefix('\n'))
            .unwrap_or(&raw);
        return LuaKind::Str(body.to_owned());
    }
    match unescape(&raw) {
        Ok(text) => LuaKind::Str(text),
        Err(reason) => LuaKind::Failed(reason),
    }
}

/// The text a quoted Lua string stands for. An escape that is a byte and not a character is refused.
fn unescape(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(escape) = chars.next() else {
            return Err("a string that ends in a backslash".to_owned());
        };
        match escape {
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\u{b}'),
            '\\' | '"' | '\'' | '\n' => out.push(escape),
            'z' => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            'x' => {
                let digits: String = chars.by_ref().take(2).collect();
                push_byte(&mut out, u32::from_str_radix(&digits, 16).ok())?;
            }
            'u' => {
                if chars.next() != Some('{') {
                    return Err("a malformed \\u escape".to_owned());
                }
                let digits: String = chars.by_ref().take_while(|c| *c != '}').collect();
                let code = u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32);
                match code {
                    Some(character) => out.push(character),
                    None => return Err("a \\u escape that is not a character".to_owned()),
                }
            }
            digit if digit.is_ascii_digit() => {
                let mut digits = String::from(digit);
                while digits.len() < 3 && chars.peek().is_some_and(char::is_ascii_digit) {
                    digits.extend(chars.next());
                }
                push_byte(&mut out, digits.parse::<u32>().ok())?;
            }
            _ => return Err("an escape the converter does not read".to_owned()),
        }
    }
    Ok(out)
}

/// A byte escape is a character only below 0x80; above it the byte is not text on its own.
fn push_byte(out: &mut String, byte: Option<u32>) -> Result<(), String> {
    match byte.and_then(|value| u8::try_from(value).ok()) {
        Some(value) if value.is_ascii() => {
            out.push(char::from(value));
            Ok(())
        }
        _ => Err("a byte escape above 0x7f is not a character".to_owned()),
    }
}

fn unary(operator: &str, operand: LuaKind) -> LuaKind {
    match (operator, operand) {
        ("-", LuaKind::Int(value)) => value
            .checked_neg()
            .map_or_else(|| LuaKind::Failed("a number out of range".to_owned()), LuaKind::Int),
        ("-", LuaKind::Float(value)) => LuaKind::Float(-value),
        ("not", LuaKind::Bool(value)) => LuaKind::Bool(!value),
        _ => LuaKind::Failed("an operator over something that is not a static number or boolean".to_owned()),
    }
}

fn binary(operator: &str, left: LuaKind, right: LuaKind) -> LuaKind {
    let unsupported = || LuaKind::Failed("an operator over something that is not static".to_owned());
    match (operator, left, right) {
        ("..", LuaKind::Str(a), LuaKind::Str(b)) => LuaKind::Str(a + &b),
        ("+" | "-" | "*" | "/", LuaKind::Int(a), LuaKind::Int(b)) => {
            let exact = match operator {
                "+" => a.checked_add(b),
                "-" => a.checked_sub(b),
                "*" => a.checked_mul(b),
                _ => None,
            };
            match exact {
                Some(value) => LuaKind::Int(value),
                // `/` is a float in Lua whatever its operands, and an overflow becomes one too.
                None => float_arithmetic(operator, a as f64, b as f64),
            }
        }
        (
            "+" | "-" | "*" | "/",
            a @ (LuaKind::Int(_) | LuaKind::Float(_)),
            b @ (LuaKind::Int(_) | LuaKind::Float(_)),
        ) => float_arithmetic(operator, as_float(&a), as_float(&b)),
        _ => unsupported(),
    }
}

fn as_float(kind: &LuaKind) -> f64 {
    match kind {
        LuaKind::Int(value) => *value as f64,
        LuaKind::Float(value) => *value,
        _ => 0.0,
    }
}

fn float_arithmetic(operator: &str, a: f64, b: f64) -> LuaKind {
    let value = match operator {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        _ => a / b,
    };
    if value.is_finite() {
        LuaKind::Float(value)
    } else {
        LuaKind::Failed("a number out of range".to_owned())
    }
}

/// The dotted name a call or an assignment names: `gnoblin.configure` is `["gnoblin", "configure"]`. `None` when
/// the prefix is not a plain name or an index is not a name or a literal string.
pub(crate) fn dotted_path<'a>(
    prefix: &Prefix,
    suffixes: impl Iterator<Item = &'a Suffix>,
    file: &SourceFile<'_>,
) -> Option<Vec<String>> {
    let Prefix::Name(name) = prefix else {
        return None;
    };
    let mut path = vec![name.token().to_string()];
    let empty = HashMap::new();
    let scope = LuaScope {
        locals: &empty,
        globals: &empty,
    };
    for suffix in suffixes {
        match suffix {
            Suffix::Index(Index::Dot { name, .. }) => path.push(name.token().to_string()),
            Suffix::Index(Index::Brackets { expression, .. }) => {
                let key = lower(file, &scope, expression);
                path.push(key.as_str()?.to_owned());
            }
            _ => return None,
        }
    }
    Some(path)
}

/// The path of a call and its arguments, when the call is `a.b.c(...)` with plain names.
pub(crate) fn call_target<'a>(
    call: &'a FunctionCall,
    file: &SourceFile<'_>,
) -> Option<(Vec<String>, &'a FunctionArgs)> {
    let suffixes: Vec<&Suffix> = call.suffixes().collect();
    let (last, names) = suffixes.split_last()?;
    let path = dotted_path(call.prefix(), names.iter().copied(), file)?;
    match last {
        Suffix::Call(Call::AnonymousCall(args)) => Some((path, &**args)),
        _ => None,
    }
}

/// The arguments of a call, each lowered: `f(a, b)` has two, `f "text"` one string, `f { ... }` one table.
pub(crate) fn lower_args(file: &SourceFile<'_>, scope: &LuaScope<'_>, args: &FunctionArgs) -> Vec<Lv> {
    match args {
        FunctionArgs::Parentheses { arguments, .. } => arguments.iter().map(|expr| lower(file, scope, expr)).collect(),
        FunctionArgs::String(token) => vec![Lv {
            span: file.span(token),
            kind: string(token),
        }],
        FunctionArgs::TableConstructor(table) => {
            vec![lower_table(file, scope, table.fields().iter(), file.span(&**table))]
        }
        _ => vec![Lv::failed(
            file.span(args),
            "an argument list the converter does not read",
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use full_moon::ast::Stmt;

    /// The value of the right-hand side of `x = <source>`, lowered with no names bound.
    fn value(source: &str) -> Lv {
        let text = format!("x = {source}");
        let ast = parse(&text).expect("the fixture parses");
        let Some(Stmt::Assignment(assignment)) = ast.nodes().stmts().next() else {
            panic!("not an assignment: {text}");
        };
        let file = SourceFile {
            name: Rc::from("t.lua"),
            source: &text,
        };
        let empty = HashMap::new();
        let scope = LuaScope {
            locals: &empty,
            globals: &empty,
        };
        let expr = assignment.expressions().iter().next().expect("one expression");
        lower(&file, &scope, expr)
    }

    fn kind(source: &str) -> LuaKind {
        value(source).kind
    }

    #[test]
    fn literals_are_static() {
        assert_eq!(kind("\"click\""), LuaKind::Str("click".to_owned()));
        assert_eq!(kind("'a b'"), LuaKind::Str("a b".to_owned()));
        assert_eq!(kind("5000"), LuaKind::Int(5000));
        assert_eq!(kind("0x1F"), LuaKind::Int(31));
        assert_eq!(kind("0.5"), LuaKind::Float(0.5));
        assert_eq!(kind("1e3"), LuaKind::Float(1000.0));
        assert_eq!(kind("true"), LuaKind::Bool(true));
        assert_eq!(kind("nil"), LuaKind::Nil);
        assert_eq!(kind("(7)"), LuaKind::Int(7));
    }

    #[test]
    fn strings_decode_their_escapes_and_long_strings_stay_raw() {
        assert_eq!(kind(r#""a\nb\t\"q\"\\""#), LuaKind::Str("a\nb\t\"q\"\\".to_owned()));
        assert_eq!(kind(r#""\x41\65\u{e9}""#), LuaKind::Str("AA\u{e9}".to_owned()));
        assert_eq!(kind("[[^org%.x$]]"), LuaKind::Str("^org%.x$".to_owned()));
        assert_eq!(kind("[==[a]]b]==]"), LuaKind::Str("a]]b".to_owned()));
        assert_eq!(kind("[[\nfirst]]"), LuaKind::Str("first".to_owned()));
        assert!(matches!(kind(r#""\xff""#), LuaKind::Failed(_)));
    }

    #[test]
    fn arithmetic_of_numbers_and_joined_strings_are_static() {
        assert_eq!(kind("5 * 1000"), LuaKind::Int(5000));
        assert_eq!(kind("-3"), LuaKind::Int(-3));
        assert_eq!(kind("-0.5"), LuaKind::Float(-0.5));
        assert_eq!(kind("1 / 4"), LuaKind::Float(0.25));
        assert_eq!(kind("2 + 0.5"), LuaKind::Float(2.5));
        assert_eq!(kind("\"a\" .. \"b\""), LuaKind::Str("ab".to_owned()));
        assert_eq!(kind("not false"), LuaKind::Bool(true));
    }

    #[test]
    fn a_call_a_function_an_index_and_a_comparison_are_not_static() {
        for source in [
            "os.getenv(\"HOME\")",
            "function() end",
            "a.b",
            "1 < 2",
            "\"a\" .. name",
            "#list",
            "unknown",
        ] {
            assert!(matches!(kind(source), LuaKind::Failed(_)), "{source}");
        }
    }

    #[test]
    fn a_table_is_a_list_a_table_of_fields_or_a_refusal() {
        let list = value("{ 1, 2, 3 }");
        assert_eq!(list.elements().map(<[Lv]>::len), Some(3));
        let table = value("{ a = 1, [\"b-c\"] = { d = true }, a = 2 }");
        let fields = table.fields().expect("a table");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, "a");
        assert_eq!(fields[0].value.kind, LuaKind::Int(2), "the later `a` wins");
        assert_eq!(fields[1].key, "b-c");
        assert!(matches!(kind("{ 1, a = 2 }"), LuaKind::Failed(_)));
        assert!(matches!(
            value("{ [f()] = 1 }").fields().expect("fields")[0].value.kind,
            LuaKind::Failed(_)
        ));
        assert!(value("{}").elements().is_some() && value("{}").fields().is_some());
    }

    #[test]
    fn a_field_carries_its_line_and_its_text() {
        let table = value("{\n    a = 1,\n    b = f(),\n}");
        let fields = table.fields().expect("a table");
        assert_eq!(fields[1].span.line, 3);
        assert_eq!(fields[1].span.text, "b = f()");
        assert!(matches!(fields[1].value.kind, LuaKind::Failed(_)));
        assert_eq!(fields[1].value.span.text, "f()");
    }

    #[test]
    fn a_syntax_error_names_its_line() {
        let (line, message) = parse("x = 1\ny = = 2\n").expect_err("a syntax error");
        assert_eq!(line, 2);
        assert!(!message.is_empty());
    }

    #[test]
    fn a_bound_name_is_its_value() {
        let text = "x = limit";
        let ast = parse(text).expect("parses");
        let Some(Stmt::Assignment(assignment)) = ast.nodes().stmts().next() else {
            panic!("an assignment");
        };
        let file = SourceFile {
            name: Rc::from("t.lua"),
            source: text,
        };
        let mut locals = HashMap::new();
        let empty = HashMap::new();
        let bound = value("42");
        locals.insert("limit".to_owned(), bound.clone());
        let scope = LuaScope {
            locals: &locals,
            globals: &empty,
        };
        let expr = assignment.expressions().iter().next().expect("an expression");
        assert_eq!(lower(&file, &scope, expr).kind, bound.kind);
    }
}
