//! `cargo xtask layout`: forbid Rust code from naming the old in-tree data location.
//!
//! The Lua runtime and its assets moved out of `src/data` to `data/` at the repository root. A
//! string literal in a crate or xtask source that still names `src/data` would point at a path that
//! no longer exists, and nothing else would catch it until the code ran. This rule scans every string
//! literal in `crates/**/*.rs` and `xtask/**/*.rs`, including test code, and fails on any that names
//! the segments `src` then `data` in order.
//!
//! Comments and doc attributes are exempt: the tokeniser drops comments, and a doc attribute is prose
//! rather than a path the code opens. Documentation, scripts and CMake are not scanned, because the
//! C-era scanners legitimately read `src/` C sources. A reviewed exception goes in
//! `xtask/layout-allow.toml`, one `[[allow]]` entry per file, with a `reason`. An entry that matches no
//! hit fails too, so the list cannot rot.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use proc_macro2::{Delimiter, TokenStream, TokenTree};

use crate::allow;
use crate::logging::{Hit, collect_rust_files};

const ALLOW_FILE: &str = "xtask/layout-allow.toml";

/// The directories whose Rust sources are scanned, relative to the repository root.
const SCANNED_ROOTS: &[&str] = &["crates", "xtask"];

/// The path segments that name the retired location. A literal is a hit when these two segments
/// appear next to each other, so `src/database` is not one.
const RETIRED_SEGMENTS: [&str; 2] = ["src", "data"];

/// Run the layout check on the tree the command was run in.
pub fn run() -> ExitCode {
    let root = Path::new(".");
    match check(root) {
        Ok(violations) if violations.is_empty() => {
            println!("[layout] no Rust literal names the retired src/data tree");
            ExitCode::SUCCESS
        }
        Ok(violations) => {
            for violation in &violations {
                println!("[layout] {violation}");
            }
            eprintln!("[layout] {} violation(s) of the layout rule", violations.len());
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("[layout] {error}");
            ExitCode::from(2)
        }
    }
}

/// Scan every scanned Rust file, applying the allow-list, and report stale entries.
fn check(root: &Path) -> Result<Vec<String>, String> {
    let allow = allow::load(root, ALLOW_FILE)?;
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut violations = Vec::new();
    for file in scanned_sources(root)? {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(&file).map_err(|error| format!("could not read {relative}: {error}"))?;
        let hits = scan_source(&source).map_err(|error| format!("{relative}: {error}"))?;
        for hit in hits {
            match allow.iter().position(|entry| allow::covers(&entry.path, &relative)) {
                Some(index) => {
                    used.insert(index);
                }
                None => violations.push(format!(
                    "{relative}:{}: {}; the Lua runtime and its assets live under data/",
                    hit.line, hit.what
                )),
            }
        }
    }
    for (index, entry) in allow.iter().enumerate() {
        if !used.contains(&index) {
            violations.push(format!(
                "{ALLOW_FILE}: `{}` matches no layout hit; remove the entry",
                entry.path
            ));
        }
    }
    Ok(violations)
}

/// Every `.rs` file under the scanned roots, sorted so the report is stable.
fn scanned_sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for scanned in SCANNED_ROOTS {
        let directory = root.join(scanned);
        if directory.is_dir() {
            collect_rust_files(&directory, &mut files)?;
        }
    }
    files.sort();
    Ok(files)
}

/// Report every string literal in `source` that names the retired segments. Doc attributes are
/// skipped.
pub fn scan_source(source: &str) -> Result<Vec<Hit>, String> {
    let stream: TokenStream = source.parse().map_err(|error| format!("does not tokenise: {error}"))?;
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut hits = Vec::new();
    scan_tokens(&tokens, &mut hits);
    Ok(hits)
}

fn scan_tokens(tokens: &[TokenTree], hits: &mut Vec<Hit>) {
    let mut index = 0;
    while index < tokens.len() {
        if let Some(next) = doc_attribute_end(tokens, index) {
            index = next;
            continue;
        }
        match &tokens[index] {
            TokenTree::Group(group) => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                scan_tokens(&inner, hits);
            }
            TokenTree::Literal(literal) => {
                if let Ok(string) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                    if names_retired_tree(&string.value()) {
                        hits.push(Hit {
                            line: literal.span().start().line,
                            what: format!("string literal `{}` names src/data", string.value()),
                        });
                    }
                }
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
        index += 1;
    }
}

/// The index just past a `#[doc = ...]` or `#![doc = ...]` attribute that starts at `index`, if one
/// does. Doc comments become these attributes in the token stream.
fn doc_attribute_end(tokens: &[TokenTree], index: usize) -> Option<usize> {
    if !is_punct(tokens.get(index), '#') {
        return None;
    }
    let mut cursor = index + 1;
    if is_punct(tokens.get(cursor), '!') {
        cursor += 1;
    }
    let Some(TokenTree::Group(group)) = tokens.get(cursor) else {
        return None;
    };
    if group.delimiter() != Delimiter::Bracket {
        return None;
    }
    let first = group.stream().into_iter().next();
    match first {
        Some(TokenTree::Ident(ident)) if ident == "doc" => Some(cursor + 1),
        _ => None,
    }
}

fn is_punct(token: Option<&TokenTree>, expected: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == expected)
}

/// Whether a literal's value names the retired segments as consecutive path components.
fn names_retired_tree(value: &str) -> bool {
    let parts: Vec<&str> = value.split('/').collect();
    parts.windows(2).any(|pair| pair == RETIRED_SEGMENTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_naming_the_retired_tree_is_a_hit() {
        let hits = scan_source("fn f() { let p = \"crates/x/src/data/lua/init.lua\"; }").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn a_raw_literal_naming_the_retired_tree_is_a_hit() {
        let hits = scan_source("const P: &str = r#\"src/data/layouts\"#;").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
    }

    #[test]
    fn the_new_location_and_similar_names_are_not_hits() {
        let source = "const A: &str = \"data/gnoblin/lua/init.lua\";\n\
                      const B: &str = \"src/database.rs\";\n\
                      const C: &str = \"crates/x/src/lib.rs\";";
        assert_eq!(scan_source(source).unwrap(), Vec::new());
    }

    #[test]
    fn comments_and_doc_attributes_are_not_hits() {
        let source = "// see src/data for the old layout\n\
                      /// The Lua runtime lived at src/data.\n\
                      #[doc = \"src/data\"]\n\
                      fn f() {}";
        assert_eq!(scan_source(source).unwrap(), Vec::new());
    }

    #[test]
    fn test_code_is_scanned_too() {
        let source = "#[cfg(test)] mod t { fn g() { let _ = \"src/data\"; } }";
        assert_eq!(scan_source(source).unwrap().len(), 1);
    }
}
