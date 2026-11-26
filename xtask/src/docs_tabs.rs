//! The language tabs of a documentation page: `::: tabs` holding one fenced example for each SDK language.
//!
//! ````text
//! ::: tabs
//! ```typescript
//! gnoblin.idle.watch(60_000, { onIdle: () => gnoblin.session.lock() });
//! ```
//! ```python
//! from gnoblin.objects.model import Gnoblin
//! ...
//! ```
//! ```rust skip="needs a running session"
//! ...
//! ```
//! :::
//! ````
//!
//! The website renders the container as tabs (`infra/site/src/lib/server/markdown.ts` reads the same grammar and refuses the same
//! mistakes), and `cargo xtask docs-examples` checks each example in its own language. This module is the one reader of the grammar on
//! the Rust side; the rules:
//!
//! - the container holds fenced blocks and blank lines, nothing else, and starts and ends in the first column;
//! - it holds exactly one block each of `typescript`, `python` and `rust`, in that order;
//! - a fence may carry `skip="reason"`, which keeps the example on the page and out of the check; the reason is required.

use std::fmt;

/// The languages of a tab group, in the order the tabs are written.
pub use gnoblin_sdkgen::DocLanguage as Language;

/// The language whose fence opens with `word`.
fn language_of(word: &str) -> Option<Language> {
    Language::ALL.into_iter().find(|language| language.slug() == word)
}

/// One fenced example of a tab group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    /// The page, relative to `docs/` (`shell-api/idle.md`).
    pub page: String,
    /// The line of the opening fence, counted from 1. A line `n` of the code is line `fence_line + n` of the page.
    pub fence_line: usize,
    pub language: Language,
    pub code: String,
    /// Why the example is not checked, when it is not.
    pub skip: Option<String>,
}

/// A problem with the markup of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupProblem {
    pub page: String,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for MarkupProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "docs/{}:{}: {}", self.page, self.line, self.message)
    }
}

/// What the info string of a fence says after the language.
fn parse_meta(meta: &str) -> Result<Option<String>, String> {
    let meta = meta.trim();
    if meta.is_empty() {
        return Ok(None);
    }
    let Some(reason) = meta.strip_prefix("skip=\"").and_then(|rest| rest.strip_suffix('"')) else {
        return Err(format!(
            "the fence says `{meta}`, and the only option is skip=\"reason\""
        ));
    };
    if reason.trim().is_empty() || reason.contains('"') {
        return Err("skip=\"reason\" needs a reason that says why the example is not checked".to_owned());
    }
    Ok(Some(reason.to_owned()))
}

/// Where a fence opens: its marker and the text after it, or `None` when the line is not a fence.
fn fence_open(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let width = trimmed.chars().take_while(|c| *c == '`' || *c == '~').count();
    let marker = &trimmed[..width];
    let valid = width >= 3 && marker.chars().all(|c| c == marker.chars().next().unwrap_or('`'));
    valid.then(|| (marker, trimmed[width..].trim()))
}

/// The examples of every tab group in `text`, and the mistakes in the markup of the groups.
pub fn examples(page: &str, text: &str) -> (Vec<Example>, Vec<MarkupProblem>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let problem = |line: usize, message: String| MarkupProblem {
        page: page.to_owned(),
        line,
        message,
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    let mut outer_fence: Option<String> = None;
    while index < lines.len() {
        let line = lines[index];
        if let Some((marker, _)) = fence_open(line) {
            match &outer_fence {
                None => outer_fence = Some(marker.to_owned()),
                Some(open) if line.trim_start().starts_with(open.as_str()) && line.trim() == open.as_str() => {
                    outer_fence = None
                }
                Some(_) => {}
            }
            index += 1;
            continue;
        }
        if outer_fence.is_some() || line.trim() != "::: tabs" {
            index += 1;
            continue;
        }
        let open_line = index + 1;
        if line != "::: tabs" {
            problems.push(problem(open_line, "`::: tabs` starts in the first column".to_owned()));
        }
        index += 1;
        let mut group: Vec<Example> = Vec::new();
        let mut closed = false;
        while index < lines.len() {
            let inner = lines[index];
            if inner == ":::" {
                closed = true;
                index += 1;
                break;
            }
            if inner.trim().is_empty() {
                index += 1;
                continue;
            }
            let Some((marker, info)) = fence_open(inner).filter(|_| !inner.starts_with(char::is_whitespace)) else {
                problems.push(problem(
                    index + 1,
                    "a `::: tabs` group holds fenced code blocks and nothing else, starting in the first column"
                        .to_owned(),
                ));
                index += 1;
                continue;
            };
            let fence_line = index + 1;
            let (word, meta) = info.split_once(char::is_whitespace).unwrap_or((info, ""));
            let mut code = String::new();
            index += 1;
            let mut ended = false;
            while index < lines.len() {
                if lines[index].trim_end() == marker {
                    ended = true;
                    index += 1;
                    break;
                }
                code.push_str(lines[index]);
                code.push('\n');
                index += 1;
            }
            if !ended {
                problems.push(problem(fence_line, "the code block is not closed".to_owned()));
                continue;
            }
            let Some(language) = language_of(word) else {
                problems.push(problem(
                    fence_line,
                    format!("`{word}` is not a language of a tab group: use typescript, python or rust"),
                ));
                continue;
            };
            let skip = match parse_meta(meta) {
                Ok(skip) => skip,
                Err(message) => {
                    problems.push(problem(fence_line, message));
                    None
                }
            };
            group.push(Example {
                page: page.to_owned(),
                fence_line,
                language,
                code,
                skip,
            });
        }
        if !closed {
            problems.push(problem(
                open_line,
                "the `::: tabs` group is not closed with `:::`".to_owned(),
            ));
        }
        let order: Vec<Language> = group.iter().map(|example| example.language).collect();
        if order != Language::ALL {
            let wanted = Language::ALL.map(Language::slug).join(", ");
            problems.push(problem(
                open_line,
                format!("a `::: tabs` group holds one block each of {wanted}, in that order"),
            ));
        }
        found.extend(group);
    }
    (found, problems)
}

/// `text` with every example of the languages other than TypeScript blanked, keeping the line numbers. The documentation parity gate reads
/// the names a page documents from TypeScript; the Python and Rust spellings of the same example are not names of that surface.
pub fn blank_other_languages(page: &str, text: &str) -> String {
    let (found, _) = examples(page, text);
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    for example in found.iter().filter(|example| example.language != Language::TypeScript) {
        let count = example.code.lines().count();
        for line in lines.iter_mut().skip(example.fence_line).take(count) {
            line.clear();
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# Title\n\n::: tabs\n```typescript\nlet a = 1;\n```\n```python\na = 1\n```\n```rust skip=\"needs a session\"\nlet a = 1;\n```\n:::\n\nAfter.\n";

    #[test]
    fn a_group_gives_one_example_for_each_language_with_the_line_of_its_fence() {
        let (found, problems) = examples("a.md", GOOD);
        assert!(problems.is_empty(), "{problems:?}");
        let lines: Vec<(Language, usize)> = found.iter().map(|e| (e.language, e.fence_line)).collect();
        assert_eq!(
            lines,
            [(Language::TypeScript, 4), (Language::Python, 7), (Language::Rust, 10)]
        );
        assert_eq!(found[0].code, "let a = 1;\n");
        assert_eq!(found[2].skip.as_deref(), Some("needs a session"));
        assert_eq!(found[1].skip, None);
    }

    #[test]
    fn a_group_that_misses_a_language_or_has_them_out_of_order_is_a_problem() {
        let missing = "::: tabs\n```typescript\nx\n```\n```python\ny\n```\n:::\n";
        let (_, problems) = examples("a.md", missing);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("one block each"), "{problems:?}");
        let swapped = "::: tabs\n```python\ny\n```\n```typescript\nx\n```\n```rust\nz\n```\n:::\n";
        assert_eq!(examples("a.md", swapped).1.len(), 1);
    }

    #[test]
    fn prose_in_a_group_an_unknown_option_and_an_empty_reason_are_problems() {
        let prose = "::: tabs\nText.\n```typescript\nx\n```\n```python\ny\n```\n```rust\nz\n```\n:::\n";
        assert_eq!(examples("a.md", prose).1.len(), 1);
        let option = "::: tabs\n```typescript title=\"x\"\nx\n```\n```python\ny\n```\n```rust\nz\n```\n:::\n";
        assert!(examples("a.md", option).1[0].message.contains("only option"));
        let empty = "::: tabs\n```typescript skip=\"\"\nx\n```\n```python\ny\n```\n```rust\nz\n```\n:::\n";
        assert!(examples("a.md", empty).1[0].message.contains("needs a reason"));
    }

    #[test]
    fn a_fence_outside_a_group_and_a_tabs_marker_inside_a_fence_are_ignored() {
        let text = "```text\n::: tabs\n```\n\n```typescript\nlet a = 1;\n```\n";
        let (found, problems) = examples("a.md", text);
        assert!(found.is_empty() && problems.is_empty(), "{found:?} {problems:?}");
    }

    #[test]
    fn an_unclosed_group_is_a_problem() {
        let (_, problems) = examples("a.md", "::: tabs\n```typescript\nx\n```\n");
        assert!(
            problems.iter().any(|p| p.message.contains("not closed")),
            "{problems:?}"
        );
    }

    #[test]
    fn blanking_keeps_typescript_and_the_line_numbers() {
        let blanked = blank_other_languages("a.md", GOOD);
        assert_eq!(blanked.lines().count(), GOOD.lines().count());
        assert!(blanked.contains("let a = 1;"));
        assert!(!blanked.contains("a = 1\n```"), "{blanked}");
    }
}
