//! The JSON text the machine-readable sinks write: one object per line, every value a string.
//!
//! The console sink's JSON format and the file sink share these helpers, so both escape a value
//! the same way and a reader needs one set of rules.

use std::fmt::Write as _;

/// Writes `"key":"value"` with both sides escaped.
pub(crate) fn pair(out: &mut String, key: &str, value: &str) {
    string(out, key);
    out.push(':');
    string(out, value);
}

/// Writes `text` as a JSON string, escaping quotes, backslashes and control characters.
pub(crate) fn string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_backslashes_and_control_characters_are_escaped() {
        let mut out = String::new();
        string(&mut out, "a\"b\\c\nd\u{1}");
        assert_eq!(out, r#""a\"b\\c\nd\u0001""#);
    }
}
