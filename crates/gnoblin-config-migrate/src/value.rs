//! The tree the converter builds and the TypeScript it prints.
//!
//! A [`Val`] is a Lua value the converter could evaluate (a string, a number, a boolean, a table of either kind), a
//! piece of TypeScript it wrote itself (a `spawn(...)` call, a RegExp literal), or a comment: the marker the converter
//! leaves where something could not be converted. Tables merge the way the config document's layers do (tables merge
//! by key, lists and scalars are replaced), so several `gnoblin.configure` calls and several files become one value.

use crate::names;

/// The key of a map entry that is a comment and not a setting.
pub(crate) const COMMENT: &str = "";

/// One value of the tree.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A list, in the order written.
    List(Vec<Val>),
    /// A table of settings in the order written. An entry whose key is [`COMMENT`] is a comment.
    Map(Vec<(String, Val)>),
    /// TypeScript the converter wrote, printed as it stands.
    Raw(String),
    /// Comment lines, printed where the value stands. In a list or a map it stands for something not converted.
    Note(Vec<String>),
}

impl Val {
    /// The entry of a map by key.
    pub(crate) fn get(&self, key: &str) -> Option<&Val> {
        match self {
            Val::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn is_note(&self) -> bool {
        matches!(self, Val::Note(_))
    }

    /// How many elements a list holds, not counting comments.
    pub(crate) fn element_count(&self) -> usize {
        match self {
            Val::List(items) => items.iter().filter(|item| !item.is_note()).count(),
            _ => 0,
        }
    }
}

/// `higher` laid over `lower`: tables merge by key, everything else is replaced. A comment in `higher` is added to a
/// table and never replaces a setting, so a value that could not be converted leaves the one before it standing.
pub(crate) fn merge(lower: Val, higher: Val) -> Val {
    match (lower, higher) {
        (Val::Map(mut entries), Val::Map(over)) => {
            for (key, value) in over {
                if key == COMMENT || value.is_note() {
                    entries.push((COMMENT.to_owned(), value));
                    continue;
                }
                match entries.iter().position(|(k, _)| *k == key) {
                    Some(at) => {
                        let existing = std::mem::replace(&mut entries[at].1, Val::Bool(false));
                        entries[at].1 = merge(existing, value);
                    }
                    None => entries.push((key, value)),
                }
            }
            Val::Map(entries)
        }
        (_, higher) => higher,
    }
}

/// How many settings a value holds: every scalar and every list counts one; a table is the sum of its entries; a
/// comment counts none.
pub(crate) fn count_settings(value: &Val) -> usize {
    match value {
        Val::Map(entries) => entries.iter().map(|(_, v)| count_settings(v)).sum(),
        Val::Note(_) => 0,
        _ => 1,
    }
}

/// The widest an inline list may be before it is broken over lines.
const INLINE_LIMIT: usize = 72;

/// Prints `value` as TypeScript, the lines after the first indented for a value that stands at `depth`.
#[cfg(test)]
pub(crate) fn print(value: &Val, depth: usize) -> String {
    let mut out = String::new();
    write_value(&mut out, value, depth);
    out
}

fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// A table of scalars on one line, `{ id: "1", name: "One" }`, when it fits. A table inside a list is one item of a
/// list of them, and one line each reads better than five.
fn inline_table(value: &Val) -> Option<String> {
    let Val::Map(entries) = value else {
        return None;
    };
    if entries.is_empty() {
        return None;
    }
    let mut parts = Vec::with_capacity(entries.len());
    for (key, entry) in entries {
        if key == COMMENT {
            return None;
        }
        parts.push(format!("{}: {}", names::property(key), scalar(entry)?));
    }
    let line = format!("{{ {} }}", parts.join(", "));
    (line.len() <= INLINE_LIMIT).then_some(line)
}

fn scalar(value: &Val) -> Option<String> {
    match value {
        Val::Str(text) => Some(names::string(text)),
        Val::Int(number) => Some(number.to_string()),
        Val::Float(number) => Some(number.to_string()),
        Val::Bool(flag) => Some(flag.to_string()),
        Val::Raw(code) if !code.contains('\n') => Some(code.clone()),
        _ => None,
    }
}

fn write_value(out: &mut String, value: &Val, depth: usize) {
    if let Some(text) = scalar(value) {
        out.push_str(&text);
        return;
    }
    match value {
        Val::Raw(code) => out.push_str(code),
        Val::Map(entries) if entries.is_empty() => out.push_str("{}"),
        Val::Map(entries) => {
            out.push_str("{\n");
            for (key, entry) in entries {
                write_entry(out, key, entry, depth + 1);
            }
            out.push_str(&indent(depth));
            out.push('}');
        }
        Val::List(items) if items.is_empty() => out.push_str("[]"),
        Val::List(items) => {
            let inline: Option<Vec<String>> = items.iter().map(scalar).collect();
            if let Some(parts) = inline {
                let line = format!("[{}]", parts.join(", "));
                if line.len() <= INLINE_LIMIT {
                    out.push_str(&line);
                    return;
                }
            }
            out.push_str("[\n");
            for item in items {
                if let Val::Note(lines) = item {
                    write_comment(out, lines, depth + 1);
                } else {
                    out.push_str(&indent(depth + 1));
                    match inline_table(item) {
                        Some(inline) => out.push_str(&inline),
                        None => write_value(out, item, depth + 1),
                    }
                    out.push_str(",\n");
                }
            }
            out.push_str(&indent(depth));
            out.push(']');
        }
        // A comment that is a whole value has nothing to print in expression position.
        Val::Note(_) => out.push_str("undefined"),
        // The scalars were printed above.
        Val::Str(_) | Val::Int(_) | Val::Float(_) | Val::Bool(_) => {}
    }
}

/// One entry of a table, or a comment standing in a table.
pub(crate) fn write_entry(out: &mut String, key: &str, value: &Val, depth: usize) {
    if let Val::Note(lines) = value {
        write_comment(out, lines, depth);
        return;
    }
    out.push_str(&indent(depth));
    out.push_str(&names::property(key));
    out.push_str(": ");
    write_value(out, value, depth);
    out.push_str(",\n");
}

/// Comment lines at `depth`.
pub(crate) fn write_comment(out: &mut String, lines: &[String], depth: usize) {
    for line in lines {
        out.push_str(&indent(depth));
        if line.is_empty() {
            out.push_str("//\n");
        } else {
            out.push_str("// ");
            out.push_str(line);
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: Vec<(&str, Val)>) -> Val {
        Val::Map(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    fn text(value: &str) -> Val {
        Val::Str(value.to_owned())
    }

    #[test]
    fn tables_merge_by_key_and_lists_and_scalars_are_replaced() {
        let lower = map(vec![
            ("a", Val::Int(1)),
            ("list", Val::List(vec![Val::Int(1), Val::Int(2)])),
            ("inner", map(vec![("x", Val::Int(1)), ("y", Val::Int(2))])),
        ]);
        let higher = map(vec![
            ("a", Val::Int(9)),
            ("list", Val::List(vec![Val::Int(3)])),
            ("inner", map(vec![("y", Val::Int(5)), ("z", Val::Int(6))])),
            ("b", Val::Bool(true)),
        ]);
        let merged = merge(lower, higher);
        assert_eq!(merged.get("a"), Some(&Val::Int(9)));
        assert_eq!(merged.get("list"), Some(&Val::List(vec![Val::Int(3)])));
        assert_eq!(
            merged.get("inner"),
            Some(&map(vec![("x", Val::Int(1)), ("y", Val::Int(5)), ("z", Val::Int(6))]))
        );
        assert_eq!(merged.get("b"), Some(&Val::Bool(true)));
    }

    #[test]
    fn a_comment_never_replaces_a_setting() {
        let lower = map(vec![("a", Val::Int(1))]);
        let note = Val::Note(vec!["UNMAPPED".to_owned()]);
        let merged = merge(lower, map(vec![("a", note.clone())]));
        assert_eq!(merged.get("a"), Some(&Val::Int(1)));
        assert_eq!(merged, map(vec![("a", Val::Int(1)), (COMMENT, note)]));
    }

    #[test]
    fn a_table_prints_one_entry_a_line_with_trailing_commas() {
        let value = map(vec![
            ("focusMode", text("click")),
            ("autoRaiseDelay", Val::Int(500)),
            ("nested", map(vec![("on", Val::Bool(true))])),
            ("empty", map(vec![])),
            ("kebab-ish key", Val::Float(0.5)),
        ]);
        assert_eq!(
            print(&value, 0),
            "{\n    focusMode: \"click\",\n    autoRaiseDelay: 500,\n    nested: {\n        on: true,\n    },\n    empty: {},\n    \"kebab-ish key\": 0.5,\n}"
        );
    }

    #[test]
    fn a_short_list_of_scalars_is_one_line_and_anything_else_is_one_element_a_line() {
        assert_eq!(
            print(&Val::List(vec![text("foot"), text("--server")]), 0),
            "[\"foot\", \"--server\"]"
        );
        assert_eq!(print(&Val::List(vec![]), 0), "[]");
        let value = Val::List(vec![
            map(vec![("id", text("1"))]),
            Val::Note(vec!["UNMAPPED (init.lua:3): f()".to_owned()]),
        ]);
        assert_eq!(
            print(&value, 1),
            "[\n        { id: \"1\" },\n        // UNMAPPED (init.lua:3): f()\n    ]"
        );
        let long: Vec<Val> = (0..20).map(|n| text(&format!("argument-{n}"))).collect();
        assert!(print(&Val::List(long), 0).starts_with("[\n    \"argument-0\",\n"));
    }

    #[test]
    fn a_comment_in_a_table_is_printed_where_it_stands() {
        let value = map(vec![
            ("a", Val::Int(1)),
            (
                COMMENT,
                Val::Note(vec!["first".to_owned(), String::new(), "second".to_owned()]),
            ),
        ]);
        assert_eq!(print(&value, 0), "{\n    a: 1,\n    // first\n    //\n    // second\n}");
    }

    #[test]
    fn settings_are_counted_without_comments() {
        let value = map(vec![
            ("a", Val::Int(1)),
            (
                "b",
                map(vec![
                    ("c", Val::Bool(true)),
                    ("d", Val::List(vec![Val::Int(1), Val::Int(2)])),
                ]),
            ),
            ("e", map(vec![])),
            (COMMENT, Val::Note(vec!["x".to_owned()])),
        ]);
        assert_eq!(count_settings(&value), 3);
    }
}
