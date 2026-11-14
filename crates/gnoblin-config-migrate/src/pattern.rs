//! Lua patterns to window-rule patterns.
//!
//! A Lua rule matched `app_id` and `title` with a Lua pattern searched anywhere in the text; a `defineConfig` rule
//! takes a glob (the whole text, `*` and `?`) or a RegExp (searched anywhere). This translates the subset of Lua
//! patterns that has an exact equivalent and refuses the rest with the reason. A pattern is never approximated: a
//! rule that matched too much would act on windows the author never named.
//!
//! The subset: the anchors `^` and `$`, `.`, a `%` escape of a punctuation character, the classes `%a %d %s %w %l %u
//! %x` and their upper-case complements (outside a set), the quantifiers `*`, `+`, `-` and `?` on a single item,
//! character sets with ranges, and capture parentheses, which a rule never reads and so are dropped. A pattern of
//! literal text only becomes a glob, which is what a rule author would write by hand.
//!
//! Lua patterns work on bytes and a RegExp on UTF-16 units, so `.` and a negated class match one byte there and one
//! character here. Rules match application ids and window titles; the difference shows only on non-ASCII text.

/// What a Lua pattern becomes in a window rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternTranslation {
    /// A glob string such as `"foot"` or `"*fire*"`: a glob matches the whole text, so the stars say "anywhere".
    Glob(String),
    /// A RegExp literal: its source and its flags.
    RegExp {
        /// What goes between the slashes, with `/` already escaped.
        source: String,
        /// The flags after the closing slash: `s` when the pattern has a `.`, so it matches a newline as Lua's does.
        flags: String,
    },
    /// No exact equivalent: why.
    Unmapped(String),
}

impl PatternTranslation {
    /// How the translation is written in a TypeScript rule, or `None` when it is unmapped.
    #[must_use]
    pub fn typescript(&self) -> Option<String> {
        match self {
            PatternTranslation::Glob(glob) => Some(crate::names::string(glob)),
            PatternTranslation::RegExp { source, flags } => Some(format!("/{source}/{flags}")),
            PatternTranslation::Unmapped(_) => None,
        }
    }
}

/// How often an item may repeat.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Repeat {
    One,
    ZeroOrMore,
    OneOrMore,
    /// Lua's `-`: as few as will do.
    Lazy,
    ZeroOrOne,
}

/// One member of a character set.
enum SetMember {
    Char(char),
    Range(char, char),
    Class(char),
}

/// What one item of a pattern matches.
enum Atom {
    Literal(char),
    Any,
    Class(char),
    Set { negated: bool, members: Vec<SetMember> },
}

/// One item and its repeat.
struct Item {
    atom: Atom,
    repeat: Repeat,
}

/// A pattern read to the end: the items and whether each end is anchored.
struct Parsed {
    start: bool,
    end: bool,
    items: Vec<Item>,
}

/// Translates a Lua pattern. See the module documentation for what is supported.
#[must_use]
pub fn translate_lua_pattern(pattern: &str) -> PatternTranslation {
    match parse(pattern) {
        Ok(parsed) => render(&parsed),
        Err(reason) => PatternTranslation::Unmapped(reason),
    }
}

fn parse(pattern: &str) -> Result<Parsed, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let start = chars.first() == Some(&'^');
    if start {
        i = 1;
    }
    let mut end = false;
    let mut items = Vec::new();
    let mut depth = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' && i + 1 == chars.len() {
            end = true;
            i += 1;
            continue;
        }
        if c == '(' {
            if chars.get(i + 1) == Some(&')') {
                return Err("a position capture `()` has no equivalent".to_owned());
            }
            depth += 1;
            i += 1;
            continue;
        }
        if c == ')' {
            if depth == 0 {
                return Err("an unbalanced `)`".to_owned());
            }
            depth -= 1;
            i += 1;
            continue;
        }
        let (atom, next) = read_atom(&chars, i)?;
        i = next;
        let repeat = match chars.get(i) {
            Some('*') => Repeat::ZeroOrMore,
            Some('+') => Repeat::OneOrMore,
            Some('-') => Repeat::Lazy,
            Some('?') => Repeat::ZeroOrOne,
            _ => Repeat::One,
        };
        if repeat != Repeat::One {
            if matches!(&atom, Atom::Literal(c) if !c.is_ascii()) {
                return Err("a quantifier after a non-ASCII character repeats only its last byte in Lua".to_owned());
            }
            i += 1;
        }
        items.push(Item { atom, repeat });
    }
    if depth != 0 {
        return Err("an unclosed `(`".to_owned());
    }
    Ok(Parsed { start, end, items })
}

/// The item that starts at `chars[i]` and the index after it.
fn read_atom(chars: &[char], i: usize) -> Result<(Atom, usize), String> {
    match chars[i] {
        '%' => {
            let Some(&next) = chars.get(i + 1) else {
                return Err("the pattern ends with a lone `%`".to_owned());
            };
            if next.is_ascii_alphanumeric() {
                if is_class(next) {
                    Ok((Atom::Class(next), i + 2))
                } else if next.is_ascii_digit() {
                    Err(format!("the back-reference `%{next}` has no equivalent"))
                } else {
                    Err(format!("`%{next}` has no exact equivalent"))
                }
            } else {
                Ok((Atom::Literal(next), i + 2))
            }
        }
        '[' => read_set(chars, i),
        '.' => Ok((Atom::Any, i + 1)),
        c => Ok((Atom::Literal(c), i + 1)),
    }
}

/// The classes with an exact equivalent: `a d s w l u x` and their complements.
fn is_class(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'd' | 's' | 'w' | 'l' | 'u' | 'x')
}

/// The set that opens at `chars[open]` and the index after its `]`.
fn read_set(chars: &[char], open: usize) -> Result<(Atom, usize), String> {
    let mut i = open + 1;
    let negated = chars.get(i) == Some(&'^');
    if negated {
        i += 1;
    }
    let mut members = Vec::new();
    let mut first = true;
    loop {
        let Some(&c) = chars.get(i) else {
            return Err("a character set that is never closed".to_owned());
        };
        if c == ']' && !first {
            i += 1;
            break;
        }
        first = false;
        let member = if c == '%' {
            let Some(&next) = chars.get(i + 1) else {
                return Err("a character set that is never closed".to_owned());
            };
            i += 2;
            if next.is_ascii_alphanumeric() {
                if is_class(next) && next.is_ascii_lowercase() {
                    SetMember::Class(next)
                } else {
                    return Err(format!("`%{next}` inside a set has no exact equivalent"));
                }
            } else {
                SetMember::Char(next)
            }
        } else if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|&end| end != ']') {
            let end = chars[i + 2];
            if end == '%' {
                return Err("a range that ends in an escape has no equivalent".to_owned());
            }
            if c > end {
                return Err(format!("the range `{c}-{end}` is empty in Lua and invalid in a RegExp"));
            }
            i += 3;
            SetMember::Range(c, end)
        } else {
            i += 1;
            SetMember::Char(c)
        };
        members.push(member);
    }
    Ok((Atom::Set { negated, members }, i))
}

fn render(parsed: &Parsed) -> PatternTranslation {
    if let Some(glob) = as_glob(parsed) {
        return PatternTranslation::Glob(glob);
    }
    let mut source = String::new();
    if parsed.start {
        source.push('^');
    }
    let mut dotted = false;
    for item in &parsed.items {
        match &item.atom {
            Atom::Literal(c) => push_literal(&mut source, *c),
            Atom::Any => {
                dotted = true;
                source.push('.');
            }
            Atom::Class(class) => source.push_str(class_outside_set(*class)),
            Atom::Set { negated, members } => {
                source.push('[');
                if *negated {
                    source.push('^');
                }
                for member in members {
                    match member {
                        SetMember::Char(c) => push_set_char(&mut source, *c),
                        SetMember::Range(from, to) => {
                            push_set_char(&mut source, *from);
                            source.push('-');
                            push_set_char(&mut source, *to);
                        }
                        SetMember::Class(class) => source.push_str(class_inside_set(*class)),
                    }
                }
                source.push(']');
            }
        }
        source.push_str(match item.repeat {
            Repeat::One => "",
            Repeat::ZeroOrMore => "*",
            Repeat::OneOrMore => "+",
            Repeat::Lazy => "*?",
            Repeat::ZeroOrOne => "?",
        });
    }
    if parsed.end {
        source.push('$');
    }
    PatternTranslation::RegExp {
        source,
        flags: if dotted { "s" } else { "" }.to_owned(),
    }
}

/// The glob a pattern of plain text is: literal characters with no repeat, none of them a glob wildcard.
fn as_glob(parsed: &Parsed) -> Option<String> {
    let mut text = String::new();
    for item in &parsed.items {
        match (&item.atom, item.repeat) {
            (Atom::Literal(c), Repeat::One) if *c != '*' && *c != '?' => text.push(*c),
            _ => return None,
        }
    }
    Some(match (parsed.start, parsed.end) {
        (true, true) => text,
        (true, false) => format!("{text}*"),
        (false, true) => format!("*{text}"),
        (false, false) if text.is_empty() => "*".to_owned(),
        (false, false) => format!("*{text}*"),
    })
}

/// A literal character in a RegExp source: escaped when it is syntax.
fn push_literal(out: &mut String, c: char) {
    match c {
        '\\' | '^' | '$' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/' | '-' => {
            out.push('\\');
            out.push(c);
        }
        c => push_plain(out, c),
    }
}

/// A character inside a set: escaped when it is syntax there.
fn push_set_char(out: &mut String, c: char) {
    match c {
        '\\' | ']' | '[' | '^' | '-' | '/' => {
            out.push('\\');
            out.push(c);
        }
        c => push_plain(out, c),
    }
}

/// A character that needs no backslash for its syntax; a control character is written as an escape.
fn push_plain(out: &mut String, c: char) {
    match c {
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\x{:02x}", c as u32)),
        c => out.push(c),
    }
}

/// A class as an item. The complement of a class is a negated set.
fn class_outside_set(class: char) -> &'static str {
    match class {
        'a' => "[A-Za-z]",
        'd' => "[0-9]",
        's' => "[ \\t\\n\\v\\f\\r]",
        'w' => "[A-Za-z0-9]",
        'l' => "[a-z]",
        'u' => "[A-Z]",
        'x' => "[0-9A-Fa-f]",
        'A' => "[^A-Za-z]",
        'D' => "[^0-9]",
        'S' => "[^ \\t\\n\\v\\f\\r]",
        'W' => "[^A-Za-z0-9]",
        'L' => "[^a-z]",
        'U' => "[^A-Z]",
        // `is_class` admits no other letter.
        _ => "[^0-9A-Fa-f]",
    }
}

/// A class as a member of a set: the ranges without the brackets.
fn class_inside_set(class: char) -> &'static str {
    match class {
        'a' => "A-Za-z",
        'd' => "0-9",
        's' => " \\t\\n\\v\\f\\r",
        'w' => "A-Za-z0-9",
        'l' => "a-z",
        'u' => "A-Z",
        // `read_set` admits no other letter.
        _ => "0-9A-Fa-f",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(text: &str) -> PatternTranslation {
        PatternTranslation::Glob(text.to_owned())
    }

    fn regexp(source: &str, flags: &str) -> PatternTranslation {
        PatternTranslation::RegExp {
            source: source.to_owned(),
            flags: flags.to_owned(),
        }
    }

    fn unmapped(pattern: &str) -> String {
        match translate_lua_pattern(pattern) {
            PatternTranslation::Unmapped(reason) => reason,
            other => panic!("{pattern} translated to {other:?}"),
        }
    }

    /// The pattern table: each Lua pattern and what a rule gets.
    #[test]
    fn the_pattern_table() {
        let table: &[(&str, PatternTranslation)] = &[
            // Plain text is a glob, and the anchors decide the stars.
            ("^foot$", glob("foot")),
            ("^org%.mozilla%.", glob("org.mozilla.*")),
            ("Notes$", glob("*Notes")),
            ("fire", glob("*fire*")),
            ("", glob("*")),
            ("^$", glob("")),
            // An escape of punctuation is that character.
            ("^org%.gnome%.Nautilus$", glob("org.gnome.Nautilus")),
            // A glob wildcard written as a literal needs a RegExp, which has no wildcard to mistake it for.
            ("^a%*b$", regexp("^a\\*b$", "")),
            ("^what%?$", regexp("^what\\?$", "")),
            // Classes.
            ("^org%.x%.(%w+)$", regexp("^org\\.x\\.[A-Za-z0-9]+$", "")),
            ("%d+ unread", regexp("[0-9]+ unread", "")),
            ("^%a%s*%d?$", regexp("^[A-Za-z][ \\t\\n\\v\\f\\r]*[0-9]?$", "")),
            ("%S+", regexp("[^ \\t\\n\\v\\f\\r]+", "")),
            ("^%u%l-%x$", regexp("^[A-Z][a-z]*?[0-9A-Fa-f]$", "")),
            // `.` matches a newline in Lua, so the RegExp says so.
            ("^foo.bar$", regexp("^foo.bar$", "s")),
            ("^.-$", regexp("^.*?$", "s")),
            // Sets.
            ("^[abc]+$", regexp("^[abc]+$", "")),
            ("^[a-z%d_]+$", regexp("^[a-z0-9_]+$", "")),
            ("^[^%s]+$", regexp("^[^ \\t\\n\\v\\f\\r]+$", "")),
            ("[]]", regexp("[\\]]", "")),
            ("[^]x]", regexp("[^\\]x]", "")),
            ("[a%-z]", regexp("[a\\-z]", "")),
            ("[%.%]]", regexp("[.\\]]", "")),
            // Characters that are syntax in a RegExp are escaped, and `/` too so the literal stays one token.
            ("a/b+", regexp("a\\/b+", "")),
            ("1+1", regexp("1+1", "")),
            ("^a|b$", glob("a|b")),
            ("^[{}]$", regexp("^[{}]$", "")),
            // An anchor that is not at an end of the pattern is a character.
            ("a^b", glob("*a^b*")),
            ("a$b", glob("*a$b*")),
            // Captures are dropped: a rule reads no capture. A quantifier after `)` is a literal in Lua.
            ("^(a)(b)$", glob("ab")),
            ("^(%d)+$", regexp("^[0-9]\\+$", "")),
        ];
        for (pattern, expected) in table {
            assert_eq!(&translate_lua_pattern(pattern), expected, "{pattern}");
        }
    }

    #[test]
    fn what_has_no_exact_equivalent_is_reported_and_never_guessed() {
        assert!(unmapped("%bxy").contains("%b"));
        assert!(unmapped("%f[%w]x").contains("%f"));
        assert!(unmapped("(a)%1").contains("back-reference"));
        assert!(unmapped("%p").contains("%p"));
        assert!(unmapped("%g+").contains("%g"));
        assert!(unmapped("%c").contains("%c"));
        assert!(unmapped("()a").contains("position capture"));
        assert!(unmapped("[%A]").contains("inside a set"));
        assert!(unmapped("[%p]").contains("inside a set"));
        assert!(unmapped("[z-a]").contains("empty"));
        assert!(unmapped("[abc").contains("never closed"));
        assert!(unmapped("abc%").contains("lone"));
        assert!(unmapped("(a").contains("unclosed"));
        assert!(unmapped("a)").contains("unbalanced"));
        assert!(unmapped("\u{e9}+").contains("non-ASCII"));
    }

    #[test]
    fn text_with_non_ascii_characters_and_no_quantifier_is_a_glob() {
        assert_eq!(translate_lua_pattern("^caf\u{e9}$"), glob("caf\u{e9}"));
    }

    #[test]
    fn the_translation_is_written_as_typescript() {
        assert_eq!(
            translate_lua_pattern("^foot$").typescript().as_deref(),
            Some("\"foot\"")
        );
        assert_eq!(
            translate_lua_pattern("^org%.x%.(%w+)$").typescript().as_deref(),
            Some("/^org\\.x\\.[A-Za-z0-9]+$/")
        );
        assert_eq!(translate_lua_pattern("^a.b$").typescript().as_deref(), Some("/^a.b$/s"));
        assert_eq!(translate_lua_pattern("%p").typescript(), None);
    }
}
