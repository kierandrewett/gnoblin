//! Glob patterns, and the directory walk that matches them.
//!
//! A configuration may split itself into files and load them with one pattern
//! (`gnoblin.load("conf.d/**/*.lua")`), so the host needs a matcher. It writes the
//! matcher here rather than taking a dependency because the pattern language is
//! the one the reference documents: `*`, `?`, `[abc]` and `**`, with no escapes
//! and no brace expansion. A dependency would bring a larger language and a
//! lifecycle, for forty lines of matching.
//!
//! Two rules keep a pattern from escaping the configuration directory:
//!
//! - A pattern is validated before it is used: no `..` component, no absolute
//!   path, no empty component, and every character class closed. A bad pattern is
//!   an error, not a pattern that silently matches nothing.
//! - The walk never follows a symbolic link to a directory, so `**` cannot descend
//!   out of the tree through one. A matched file is still canonicalised by the
//!   loader, which is what catches a file that links outside.
//!
//! A wildcard does not match a leading dot: hidden files need an explicit dot in
//! the pattern, the rule the reference documents state.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Whether `pattern` contains a wildcard and so is a glob rather than a path.
pub(super) fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

/// Refuse a pattern that is malformed or that could name something outside the
/// configuration directory.
pub(super) fn validate(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() {
        return Err("the pattern is empty".to_owned());
    }
    if Path::new(pattern).is_absolute() {
        return Err(format!(
            "`{pattern}` is absolute; a pattern resolves inside the configuration directory"
        ));
    }
    for component in pattern.split('/') {
        if component.is_empty() {
            return Err(format!("`{pattern}` has an empty path component"));
        }
        if component == ".." {
            return Err(format!(
                "`{pattern}` leaves the configuration directory; `..` is not allowed"
            ));
        }
        if component == "**" {
            continue;
        }
        if component.contains("**") {
            return Err(format!("`{pattern}`: `**` must be a whole path component"));
        }
    }
    // Every class must close, or the pattern is a typo rather than a matcher.
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '[' {
            continue;
        }
        let mut closed = false;
        for inner in characters.by_ref() {
            if inner == ']' {
                closed = true;
                break;
            }
        }
        if !closed {
            return Err(format!("`{pattern}` has an unclosed `[` class"));
        }
    }
    Ok(())
}

/// Whether one relative path matches one validated pattern.
pub(super) fn matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    match_segments(&pattern, &path)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        // `**` crosses segments, but not a hidden one: a wildcard never matches a
        // leading dot, and `**` is a wildcard.
        Some((&"**", rest)) => (0..=path.len())
            .any(|skip| path[..skip].iter().all(|name| !name.starts_with('.')) && match_segments(rest, &path[skip..])),
        Some((segment, rest)) => match path.split_first() {
            Some((first, remaining)) if match_segment(segment, first) => match_segments(rest, remaining),
            _ => false,
        },
    }
}

/// Whether one name matches one pattern segment.
fn match_segment(pattern: &str, name: &str) -> bool {
    // A wildcard never matches a hidden name: `.git` needs `.git`, not `*`.
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    match_here(&pattern, 0, &name, 0)
}

fn match_here(pattern: &[char], pattern_at: usize, name: &[char], name_at: usize) -> bool {
    let mut pattern_at = pattern_at;
    let mut name_at = name_at;
    while pattern_at < pattern.len() {
        match pattern[pattern_at] {
            '*' => {
                let mut next = pattern_at;
                while next < pattern.len() && pattern[next] == '*' {
                    next += 1;
                }
                if next == pattern.len() {
                    return true;
                }
                for skip in name_at..=name.len() {
                    if match_here(pattern, next, name, skip) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if name_at >= name.len() {
                    return false;
                }
                name_at += 1;
                pattern_at += 1;
            }
            '[' => {
                let Some(target) = name.get(name_at).copied() else {
                    return false;
                };
                match class(pattern, pattern_at, target) {
                    Some((matched, next)) => {
                        if !matched {
                            return false;
                        }
                        pattern_at = next;
                        name_at += 1;
                    }
                    // A class that does not close matches nothing; `validate`
                    // has already refused it, so this is unreachable in practice.
                    None => return false,
                }
            }
            literal => {
                if name.get(name_at) != Some(&literal) {
                    return false;
                }
                name_at += 1;
                pattern_at += 1;
            }
        }
    }
    name_at == name.len()
}

/// Match one `[...]` class against `target`, answering whether it matched and
/// where the class ended.
fn class(pattern: &[char], start: usize, target: char) -> Option<(bool, usize)> {
    let mut index = start + 1;
    let negated = matches!(pattern.get(index), Some('!') | Some('^'));
    if negated {
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    while index < pattern.len() {
        let current = pattern[index];
        if current == ']' && !first {
            return Some((if negated { !matched } else { matched }, index + 1));
        }
        first = false;
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            if current <= target && target <= pattern[index + 2] {
                matched = true;
            }
            index += 3;
        } else {
            if current == target {
                matched = true;
            }
            index += 1;
        }
    }
    None
}

/// Every file under `root` whose path relative to it matches `pattern`.
///
/// `pattern` is relative to `root`, so the caller passes the directory of the file
/// that made the call. Results are sorted by their relative path's bytes, which
/// is the order the reference documents promise and does not depend on the
/// directory iteration order of the filesystem.
pub(super) fn expand(root: &Path, pattern: &str) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            // A directory reached through a symbolic link is not followed: `**`
            // would otherwise leave the configuration tree.
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if file_type.is_symlink() && path.is_dir() {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let Some(text) = relative.to_str() else {
                // A name that is not UTF-8 cannot be matched against a Lua string,
                // and a configuration file with such a name is not loadable.
                continue;
            };
            if matches(pattern, text) {
                found.push((text.to_owned(), path));
            }
        }
    }
    found.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_stays_inside_one_segment() {
        assert!(matches("*.lua", "a.lua"));
        assert!(!matches("*.lua", "parts/a.lua"));
        assert!(matches("parts/*.lua", "parts/a.lua"));
        assert!(!matches("parts/*.lua", "parts/sub/a.lua"));
    }

    #[test]
    fn a_double_star_crosses_segments() {
        assert!(matches("**/*.lua", "a.lua"));
        assert!(matches("**/*.lua", "a/b/c.lua"));
        assert!(matches("conf.d/**/*.lua", "conf.d/a.lua"));
        assert!(matches("conf.d/**/*.lua", "conf.d/x/y/a.lua"));
        assert!(!matches("conf.d/**/*.lua", "other/a.lua"));
    }

    #[test]
    fn a_wildcard_does_not_match_a_hidden_name() {
        assert!(!matches("*", ".hidden"));
        assert!(matches(".*", ".hidden"));
        assert!(!matches("**/*.lua", ".cache/a.lua"));
        assert!(matches(".cache/**/*.lua", ".cache/a.lua"));
    }

    #[test]
    fn question_and_class_match_one_character() {
        assert!(matches("a?.lua", "ab.lua"));
        assert!(!matches("a?.lua", "abc.lua"));
        assert!(matches("[ab].lua", "a.lua"));
        assert!(!matches("[ab].lua", "c.lua"));
        assert!(matches("[a-z].lua", "q.lua"));
        assert!(matches("[!a-z].lua", "Q.lua"));
        assert!(!matches("[!a-z].lua", "q.lua"));
    }

    #[test]
    fn a_bad_pattern_is_refused() {
        for pattern in [
            "",
            "/etc/*.lua",
            "../*.lua",
            "a/../b.lua",
            "a//b.lua",
            "a/[bc.lua",
            "a**/b.lua",
        ] {
            assert!(validate(pattern).is_err(), "{pattern} should be refused");
        }
        for pattern in ["*.lua", "conf.d/**/*.lua", "[ab].lua", "**/*.lua"] {
            assert!(validate(pattern).is_ok(), "{pattern} should be accepted");
        }
    }
}
