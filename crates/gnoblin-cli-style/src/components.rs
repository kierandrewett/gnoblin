//! The building blocks of human output. Every function takes plain text, uses
//! the `OutputTheme` for colour and glyphs, and returns a string without a trailing
//! newline. None of them reads the environment, so tests compare exact output.

use crate::text::{pad, truncate, width, wrap};
use crate::theme::{ColourToken, Glyph, OutputTheme};

/// Gap between table columns and between a label and its value.
const GAP: usize = 2;
/// Width of the label column in an error block (`error:` plus a space).
const LABEL_COLUMN: usize = 7;

/// The outcome shown by a status line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineOutcome {
    /// Finished as wanted.
    Ok,
    /// Finished, but something needs a look.
    Warn,
    /// Did not finish.
    Fail,
    /// Neutral information.
    Info,
}

impl LineOutcome {
    fn word(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Info => "info",
        }
    }

    fn glyph(self) -> Glyph {
        match self {
            Self::Ok => Glyph::Ok,
            Self::Warn => Glyph::Warn,
            Self::Fail => Glyph::Fail,
            Self::Info => Glyph::Info,
        }
    }

    fn token(self) -> ColourToken {
        match self {
            Self::Ok => ColourToken::Success,
            Self::Warn => ColourToken::Warning,
            Self::Fail => ColourToken::Danger,
            Self::Info => ColourToken::Info,
        }
    }
}

/// What went wrong, why it probably happened, and what to do next.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrorReport {
    /// What happened, in one sentence.
    pub summary: String,
    /// The likely cause, when it is known.
    pub why: Option<String>,
    /// Things to try, in order.
    pub hints: Vec<String>,
}

/// Joins lines with newlines.
fn join(lines: &[String]) -> String {
    lines.join("\n")
}

/// Wraps `text` into lines of at most `max` columns. The first line starts
/// after `first` and the rest after `rest`; both are plain.
fn indented(first: &str, rest: &str, text: &str, max: usize) -> Vec<String> {
    let body = max.saturating_sub(width(first).max(width(rest))).max(1);
    wrap(text, body)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let lead = if index == 0 { first } else { rest };
            format!("{lead}{line}")
        })
        .collect()
}

/// A section heading: the title in strong text with a rule under it.
pub fn section(theme: &OutputTheme, title: &str) -> String {
    let rule = theme.glyph(Glyph::Rule).repeat(width(title).max(1));
    format!(
        "{}\n{}",
        theme.paint(ColourToken::Strong, title),
        theme.paint(ColourToken::Muted, &rule)
    )
}

/// A table with a header row. Columns are sized to their widest cell. When the
/// table is wider than the theme allows, the widest column shrinks and its
/// cells end with an ellipsis, so every row stays aligned.
pub fn table(theme: &OutputTheme, headers: &[&str], rows: &[Vec<String>]) -> String {
    let columns = headers.len();
    if columns == 0 {
        return String::new();
    }
    let mut widths: Vec<usize> = headers.iter().map(|header| width(header)).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate().take(columns) {
            widths[index] = widths[index].max(width(cell));
        }
    }
    let gaps = GAP * (columns - 1);
    while widths.iter().sum::<usize>() + gaps > theme.width() {
        let Some((index, &widest)) = widths.iter().enumerate().max_by_key(|&(_, w)| *w) else {
            break;
        };
        if widest <= 1 {
            break;
        }
        widths[index] -= 1;
    }
    let ellipsis = theme.glyph(Glyph::Ellipsis);
    let render = |cells: Vec<String>| -> String {
        let mut line = String::new();
        for (index, cell) in cells.iter().enumerate() {
            if index > 0 {
                line.push_str(&" ".repeat(GAP));
            }
            let last = index + 1 == columns;
            if last {
                line.push_str(cell);
            } else {
                line.push_str(&pad(cell, widths[index]));
            }
        }
        line
    };
    let header_cells: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| truncate(header, widths[index], ellipsis))
        .collect();
    let rule_cells: Vec<String> = widths.iter().map(|&w| theme.glyph(Glyph::Rule).repeat(w)).collect();
    let mut lines = vec![
        theme.paint(ColourToken::Strong, render(header_cells).trim_end()),
        theme.paint(ColourToken::Muted, render(rule_cells).trim_end()),
    ];
    for row in rows {
        let cells: Vec<String> = (0..columns)
            .map(|index| {
                let cell = row.get(index).map(String::as_str).unwrap_or("");
                truncate(cell, widths[index], ellipsis)
            })
            .collect();
        lines.push(render(cells).trim_end().to_string());
    }
    join(&lines)
}

/// Key-value pairs, with the keys aligned in one column. Long values wrap under
/// the value column.
pub fn kv_list(theme: &OutputTheme, pairs: &[(&str, &str)]) -> String {
    let key_width = pairs.iter().map(|(key, _)| width(key)).max().unwrap_or(0);
    let mut lines = Vec::new();
    for (key, value) in pairs {
        let lead = " ".repeat(key_width + GAP);
        let first = format!(
            "{}{}",
            theme.paint(ColourToken::Accent, key),
            " ".repeat(key_width - width(key) + GAP)
        );
        if value.is_empty() {
            lines.push(first.trim_end().to_string());
            continue;
        }
        let body = indented("", &lead, value, theme.width());
        let mut body = body.into_iter();
        let head = body.next().unwrap_or_default();
        lines.push(format!("{first}{head}"));
        lines.extend(body);
    }
    join(&lines)
}

/// Terms, each followed by its description indented beneath it. Used for
/// `--help` option and command lists.
pub fn definition_list(theme: &OutputTheme, items: &[(&str, &str)]) -> String {
    let mut lines = Vec::new();
    for (term, description) in items {
        lines.push(theme.paint(ColourToken::Accent, term));
        if !description.is_empty() {
            lines.extend(indented("    ", "    ", description, theme.width()));
        }
    }
    join(&lines)
}

/// One line that says how something went: a glyph and a word, then the detail.
/// In ASCII the word alone leads the line.
pub fn status_line(theme: &OutputTheme, status: LineOutcome, text: &str) -> String {
    let lead = if theme.unicode() {
        format!("{} {}", theme.glyph(status.glyph()), status.word())
    } else {
        status.word().to_string()
    };
    let lead_width = width(&lead) + GAP;
    let first = format!("{}{}", theme.paint(status.token(), &lead), " ".repeat(GAP));
    let rest = " ".repeat(lead_width);
    let body = indented("", &rest, text, theme.width());
    let mut body = body.into_iter();
    let head = body.next().unwrap_or_default();
    let mut lines = vec![format!("{first}{head}")];
    lines.extend(body);
    join(&lines)
}

/// An error: what happened, why, and what to try next, each with a label.
pub fn error_block(theme: &OutputTheme, report: &ErrorReport) -> String {
    let mut lines = Vec::new();
    let label = |text: &str, token: ColourToken| {
        format!("{}{}", theme.paint(token, text), " ".repeat(LABEL_COLUMN - width(text)))
    };
    let blank = " ".repeat(LABEL_COLUMN);
    let mut push = |text: &str, label_text: &str, token: ColourToken| {
        let first = label(label_text, token);
        let body = indented(&first, &blank, text, theme.width());
        lines.extend(body);
    };
    push(&report.summary, "error:", ColourToken::Danger);
    if let Some(why) = &report.why {
        push(why, "why:", ColourToken::Muted);
    }
    for hint in &report.hints {
        push(hint, "hint:", ColourToken::Accent);
    }
    join(&lines)
}

/// One line of a `watch` stream: a timestamp, the changed subject and, when
/// known, the old and new values. Values are cut to fit the width.
pub fn event_line(theme: &OutputTheme, timestamp: &str, subject: &str, change: Option<(&str, &str)>) -> String {
    let head = format!(
        "{}{}{}",
        theme.paint(ColourToken::Muted, timestamp),
        " ".repeat(GAP),
        theme.paint(ColourToken::Accent, subject)
    );
    let Some((old, new)) = change else {
        return head;
    };
    let arrow = theme.glyph(Glyph::Arrow);
    let used = width(timestamp) + GAP + width(subject);
    let arrow_cost = GAP + 2 + width(arrow);
    let budget = theme.width().saturating_sub(used + arrow_cost);
    let each = (budget / 2).max(1);
    let ellipsis = theme.glyph(Glyph::Ellipsis);
    format!(
        "{head}{}{} {arrow} {}",
        " ".repeat(GAP),
        truncate(old, each, ellipsis),
        truncate(new, each, ellipsis)
    )
}

/// The line shown while an operation waits. On a terminal it is a spinner
/// frame; elsewhere it is a plain `text...` line that is never rewritten.
pub fn waiting_line(theme: &OutputTheme, tick: usize, text: &str) -> String {
    if theme.animate() {
        format!("{} {text}", theme.paint(ColourToken::Accent, theme.spinner(tick)))
    } else {
        format!("{text}...")
    }
}

/// The escape sequence that erases a waiting line before the result is
/// printed. Empty when the output is not animated.
pub fn clear_waiting(theme: &OutputTheme) -> &'static str {
    if theme.animate() { "\r\x1b[2K" } else { "" }
}

/// A short label in a token colour, such as `[running]`.
pub fn badge(theme: &OutputTheme, token: ColourToken, text: &str) -> String {
    theme.paint(token, &format!("[{text}]"))
}

/// A name that the user may copy: a window id, an object or a property name.
pub fn identifier(theme: &OutputTheme, text: &str) -> String {
    theme.paint(ColourToken::Accent, text)
}
