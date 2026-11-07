//! `cargo xtask logging`: forbid stock logging in the workspace.
//!
//! Every crate logs through the `gnoblin-log` macros, so a record carries its component, its role and
//! its level, and reaches the console and the database. Two rules enforce that:
//!
//! - **Dependencies.** Only the crates in [`MAY_NAME`] may declare a direct dependency on a logging
//!   crate. Everything else reaches `tracing` through `gnoblin-log`, and nothing names `log` or
//!   `env_logger`.
//! - **Sources.** Library and daemon sources under `crates/*/src` must not call `tracing::` macros or the
//!   stock print macros, or a retired `*_in!` macro. Tests are exempt: `#[test]` functions,
//!   `#[cfg(test)]` items, files named `tests.rs` or `*_test(s).rs`, `build.rs` and `examples/`.
//!   Comments and string literals never count, because the scanner reads tokens rather than text.
//!
//! A reviewed exception goes in `xtask/logging-allow.toml`, one `[[allow]]` entry per file or crate,
//! each with a `reason`. An entry that matches no hit fails too, so the list cannot rot.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use proc_macro2::{Delimiter, TokenStream, TokenTree};

use crate::allow;
use crate::metadata::{self, Graph};

const ALLOW_FILE: &str = "xtask/logging-allow.toml";

/// Every logging crate a workspace crate may name directly, by the crate that may name it.
///
/// A crate absent from this table names none of them. `gnoblin-log` is the one logging crate: the
/// facade over `tracing`, and the sinks and the composition behind its features, which need
/// `tracing-subscriber`.
const MAY_NAME: &[(&str, &[&str])] = &[("gnoblin-log", &["tracing", "tracing-subscriber"])];

/// The logging crates the table governs. Any direct dependency on one outside [`MAY_NAME`] fails.
const LOGGING_CRATES: &[&str] = &["tracing", "tracing-subscriber", "log", "env_logger"];

/// The stock macros that write to stdout or stderr, or print debug output, in library code.
const PRINT_MACROS: &[&str] = &["println", "print", "eprintln", "eprint", "dbg"];

/// The `*_in!` macros that took the component as a string. `logger!` replaced them, so a call to
/// one is a violation.
const RETIRED_MACROS: &[&str] = &["error_in", "warn_in", "info_in", "debug_in", "trace_in"];

/// One source-level violation: the line and what was found there.
#[derive(Debug, PartialEq, Eq)]
pub struct Hit {
    pub line: usize,
    pub what: String,
}

/// Run every logging check on the tree the command was run in.
pub fn run() -> ExitCode {
    let root = Path::new(".");
    let graph = match metadata::load(root) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("[logging] {error}");
            return ExitCode::from(2);
        }
    };
    let mut violations = check_dependencies(&graph);
    match check_sources(root) {
        Ok(found) => violations.extend(found),
        Err(error) => {
            eprintln!("[logging] {error}");
            return ExitCode::from(2);
        }
    }
    if violations.is_empty() {
        println!("[logging] {} packages checked, no violations", graph.packages.len());
        return ExitCode::SUCCESS;
    }
    for violation in &violations {
        println!("[logging] {violation}");
    }
    eprintln!("[logging] {} violation(s) of the logging rules", violations.len());
    ExitCode::FAILURE
}

/// Report every direct dependency on a logging crate that [`MAY_NAME`] does not permit.
pub fn check_dependencies(graph: &Graph) -> Vec<String> {
    let mut violations = Vec::new();
    for package in &graph.packages {
        let allowed: &[&str] = MAY_NAME
            .iter()
            .find(|(name, _)| *name == package.name)
            .map_or(&[], |(_, crates)| *crates);
        for dep in &package.deps {
            let name = dep.name.as_str();
            if LOGGING_CRATES.contains(&name) && !allowed.contains(&name) {
                violations.push(format!(
                    "{} declares a {} dependency on `{name}`; log through a gnoblin-log `logger!` module instead",
                    package.name,
                    dep.kind.as_str(),
                ));
            }
        }
    }
    violations
}

/// Scan every crate's `src` tree, applying the allow-list, and report stale entries.
fn check_sources(root: &Path) -> Result<Vec<String>, String> {
    let allow = allow::load(root, ALLOW_FILE)?;
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut violations = Vec::new();
    for file in library_sources(root)? {
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
                    "{relative}:{}: {} in library code; log through a gnoblin-log `logger!` module, or allow it in {ALLOW_FILE}",
                    hit.line, hit.what,
                )),
            }
        }
    }
    for (index, entry) in allow.iter().enumerate() {
        if !used.contains(&index) {
            violations.push(format!(
                "{ALLOW_FILE}: `{}` matches no logging hit; remove the entry",
                entry.path
            ));
        }
    }
    Ok(violations)
}

/// Every `.rs` file under `crates/*/src`, skipping the exempt test-only files.
fn library_sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let crates = root.join("crates");
    let mut files = Vec::new();
    let entries = fs::read_dir(&crates).map_err(|error| format!("could not read {}: {error}", crates.display()))?;
    for entry in entries {
        let src = entry.map_err(|error| error.to_string())?.path().join("src");
        if src.is_dir() {
            collect_rust_files(&src, &mut files)?;
        }
    }
    files.retain(|file| !is_exempt_file(file));
    files.sort();
    Ok(files)
}

pub(crate) fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(directory).map_err(|error| format!("could not read {}: {error}", directory.display()))?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Files that hold only test code, whatever module declares them.
fn is_exempt_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    let in_examples = path.components().any(|part| part.as_os_str() == "examples");
    name == "tests.rs" || name.ends_with("_test.rs") || name.ends_with("_tests.rs") || name == "build.rs" || in_examples
}

/// Every violation in one source file, skipping test-gated items.
///
/// The file is read as tokens, so comments and string literals are never mistaken for calls.
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
        if is_punct(tokens.get(index), '#')
            && let Some(TokenTree::Group(group)) = tokens.get(index + 1)
            && group.delimiter() == Delimiter::Bracket
            && is_test_gate(group.stream())
        {
            index = skip_item(tokens, index + 2);
            continue;
        }
        match &tokens[index] {
            TokenTree::Group(group) => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                scan_tokens(&inner, hits);
            }
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                if PRINT_MACROS.contains(&name.as_str()) && is_punct(tokens.get(index + 1), '!') {
                    hits.push(Hit {
                        line: ident.span().start().line,
                        what: format!("`{name}!`"),
                    });
                } else if name == "tracing"
                    && let Some(path) = tracing_macro_path(tokens, index)
                {
                    hits.push(Hit {
                        line: ident.span().start().line,
                        what: format!("`{path}!`"),
                    });
                } else if RETIRED_MACROS.contains(&name.as_str()) && is_punct(tokens.get(index + 1), '!') {
                    hits.push(Hit {
                        line: ident.span().start().line,
                        what: format!("`{name}!`, retired: declare a `logger!` and call its level macro"),
                    });
                }
            }
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
        index += 1;
    }
}

/// The path of a `tracing::...!` macro call starting at `start`, if one does.
fn tracing_macro_path(tokens: &[TokenTree], start: usize) -> Option<String> {
    let mut path = String::from("tracing");
    let mut index = start + 1;
    while is_punct(tokens.get(index), ':')
        && is_punct(tokens.get(index + 1), ':')
        && let Some(TokenTree::Ident(segment)) = tokens.get(index + 2)
    {
        path.push_str("::");
        path.push_str(&segment.to_string());
        index += 3;
    }
    (path != "tracing" && is_punct(tokens.get(index), '!')).then_some(path)
}

fn is_punct(token: Option<&TokenTree>, character: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == character)
}

/// Whether an attribute's tokens gate the item after it to test builds: `#[test]`, or a `cfg` that
/// names `test` without negating it.
fn is_test_gate(attribute: TokenStream) -> bool {
    let tokens: Vec<TokenTree> = attribute.into_iter().collect();
    let names = identifiers(&tokens);
    match names.first().map(String::as_str) {
        Some("test") => names.len() == 1,
        Some("cfg") => names.iter().any(|name| name == "test") && !names.iter().any(|name| name == "not"),
        _ => false,
    }
}

fn identifiers(tokens: &[TokenTree]) -> Vec<String> {
    let mut names = Vec::new();
    for token in tokens {
        match token {
            TokenTree::Ident(ident) => names.push(ident.to_string()),
            TokenTree::Group(group) => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                names.extend(identifiers(&inner));
            }
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
    names
}

/// The index just past the item that starts at `index`: its body, or its `;`.
fn skip_item(tokens: &[TokenTree], mut index: usize) -> usize {
    while is_punct(tokens.get(index), '#') && matches!(tokens.get(index + 1), Some(TokenTree::Group(_))) {
        index += 2;
    }
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => return index + 1,
            TokenTree::Punct(punct) if punct.as_char() == ';' => return index + 1,
            _ => index += 1,
        }
    }
    tokens.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Dep, DepKind, Package};

    fn package(name: &str, deps: &[(&str, DepKind)]) -> Package {
        Package {
            name: name.to_owned(),
            deps: deps
                .iter()
                .map(|(name, kind)| Dep {
                    name: (*name).to_owned(),
                    kind: *kind,
                    features: Vec::new(),
                })
                .collect(),
            is_binary: false,
        }
    }

    #[test]
    fn only_the_table_may_name_a_logging_crate() {
        let graph = Graph {
            packages: vec![
                package(
                    "gnoblin-log",
                    &[("tracing", DepKind::Normal), ("tracing-subscriber", DepKind::Normal)],
                ),
                package(
                    "gnoblin-wm",
                    &[("tracing", DepKind::Normal), ("serde", DepKind::Normal)],
                ),
                package("gnoblin-lua", &[("env_logger", DepKind::Dev)]),
                package("gnoblin-render", &[("log", DepKind::Build)]),
            ],
        };
        let violations = check_dependencies(&graph);
        assert_eq!(violations.len(), 3, "{violations:?}");
        assert!(violations[0].starts_with("gnoblin-wm declares a normal dependency on `tracing`"));
        assert!(violations[1].contains("gnoblin-lua declares a development dependency on `env_logger`"));
        assert!(violations[2].contains("gnoblin-render declares a build dependency on `log`"));
    }

    #[test]
    fn a_macro_call_is_a_violation() {
        assert_eq!(scan_source("fn f() { println!(\"x\"); }").unwrap().len(), 1);
        assert_eq!(scan_source("fn f() { eprint!(\"x\"); dbg!(1); }").unwrap().len(), 2);
        assert_eq!(scan_source("fn f() { tracing::info!(\"x\"); }").unwrap().len(), 1);
        assert_eq!(
            scan_source("fn f() { tracing::subscriber::set_default(); }")
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn a_retired_in_macro_is_a_violation() {
        assert_eq!(scan_source("fn f() { warn_in!(\"x\", \"y\"); }").unwrap().len(), 1);
        assert_eq!(scan_source("fn f() { log::warn!(\"x\"); }").unwrap().len(), 0);
    }

    #[test]
    fn a_comment_or_a_string_is_not_a_call() {
        let source = "
            // println!(\"a line comment\");
            /* eprintln!(\"a block comment\"); */
            //! tracing::info!(\"a doc comment\");
            fn f() -> &'static str {
                \"println!(\\\"in a string\\\")\"
            }
        ";
        assert_eq!(scan_source(source).unwrap(), Vec::new());
    }

    #[test]
    fn test_only_items_are_exempt() {
        let source = "
            #[test]
            fn a_test() { println!(\"fine\"); }

            #[cfg(test)]
            mod tests {
                fn helper() { eprintln!(\"fine\"); }
            }

            #[cfg(all(test, feature = \"x\"))]
            fn gated() { dbg!(1); }

            #[cfg(not(test))]
            fn shipped() { eprintln!(\"flagged\"); }
        ";
        let hits = scan_source(source).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].what, "`eprintln!`");
        assert_eq!(hits[0].line, 14);
    }

    #[test]
    fn the_line_of_the_call_is_reported() {
        let hits = scan_source("fn f() {\n    let x = 1;\n    print!(\"{x}\");\n}\n").unwrap();
        assert_eq!(
            hits,
            vec![Hit {
                line: 3,
                what: "`print!`".to_owned()
            }]
        );
    }

    #[test]
    fn exempt_files_are_test_only() {
        assert!(is_exempt_file(Path::new("crates/a/src/gles/tests.rs")));
        assert!(is_exempt_file(Path::new("crates/a/src/model_test.rs")));
        assert!(is_exempt_file(Path::new("crates/a/build.rs")));
        assert!(!is_exempt_file(Path::new("crates/a/src/lib.rs")));
    }
}
