//! Measuring and cutting text the way a terminal draws it.
//!
//! Widths count terminal columns, not characters: CJK and most emoji take two
//! columns, combining marks take none. Escape sequences take none either.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Removes CSI escape sequences (colour and style) from `text`.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // A CSI sequence is ESC '[', parameter bytes, then a final byte in
        // 0x40..=0x7e.
        if chars.next() == Some('[') {
            for end in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&end) {
                    break;
                }
            }
        }
    }
    out
}

/// The number of terminal columns `text` occupies, ignoring escape sequences.
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(text).as_str())
}

fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Cuts plain `text` to at most `max` columns. When it is cut, the result ends
/// with `ellipsis`, and the ellipsis counts towards `max`.
pub fn truncate(text: &str, max: usize, ellipsis: &str) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let tail = width(ellipsis);
    let (budget, suffix) = if max >= tail { (max - tail, ellipsis) } else { (max, "") };
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > budget {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str(suffix);
    out
}

/// Pads plain `text` with spaces on the right to `columns` columns.
pub fn pad(text: &str, columns: usize) -> String {
    let mut out = text.to_string();
    out.push_str(&" ".repeat(columns.saturating_sub(width(text))));
    out
}

/// Breaks plain `text` into lines of at most `max` columns, at spaces. A word
/// wider than a line is split across lines. Existing line breaks are kept, so a
/// blank line stays blank.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_whitespace() {
            for piece in split_word(word, max) {
                let w = width(&piece);
                if used == 0 {
                    line = piece;
                    used = w;
                } else if used + 1 + w <= max {
                    line.push(' ');
                    line.push_str(&piece);
                    used += 1 + w;
                } else {
                    lines.push(std::mem::take(&mut line));
                    line = piece;
                    used = w;
                }
            }
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Splits one word into pieces no wider than `max`.
fn split_word(word: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for c in word.chars() {
        let w = char_width(c);
        if used > 0 && used + w > max {
            pieces.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(c);
        used += w;
    }
    if !current.is_empty() || pieces.is_empty() {
        pieces.push(current);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_colour_and_keeps_text() {
        assert_eq!(strip_ansi("\x1b[1;31mred\x1b[0m!"), "red!");
    }

    #[test]
    fn wide_characters_count_two_columns() {
        assert_eq!(width("東京"), 4);
        assert_eq!(width("\x1b[36m東\x1b[0m"), 2);
    }

    #[test]
    fn truncate_keeps_the_ellipsis_inside_the_limit() {
        assert_eq!(truncate("abcdef", 6, "..."), "abcdef");
        assert_eq!(truncate("abcdef", 5, "..."), "ab...");
        assert_eq!(truncate("abcdef", 2, "..."), "");
        assert_eq!(truncate("東京都", 4, "..."), "東...");
    }

    #[test]
    fn wrap_breaks_at_spaces_and_splits_long_words() {
        assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap("a\n\nb", 9), vec!["a", "", "b"]);
        assert_eq!(wrap("", 9), vec![""]);
    }
}
