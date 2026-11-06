//! `cargo xtask layout`: keep references to the root `src/` and `patches/` trees deliberate, and
//! refuse the retired `src/data` tree outright.
//!
//! Two root trees hold things the product reads. `src/` is the C tree: the C components still
//! compile from it, and the build scripts read its sources to generate files. `patches/` is the patch
//! series the build applies to the pinned upstream sources. The Lua runtime and its shipped assets
//! moved out of `src/data` to `data/` at the repository root, so a reference to `src/data` names a
//! path that no longer exists. Such a reference is a hard failure, and no allow-list entry can cover it.
//!
//! Most references to `src/` and `patches/` are correct build inputs, so the rule does not ban them.
//! It makes each one reviewed: a reference that is not on the allow-list fails, and so does an entry
//! that matches no reference, so the list cannot rot. The two kinds of file are scanned differently:
//!
//! - Rust sources under `crates/` and `xtask/` are tokenised. A string literal whose value is a
//!   root-relative path (`src/...`, `./src/...`, `patches/...`) is a root reference. Any literal that
//!   names `src` then `data` as consecutive components is a retired reference. Comments and doc
//!   attributes are prose, so they are exempt.
//! - Every other text file under the scanned roots is read line by line. A `src/` or `patches/` that
//!   is root-relative is a root reference, and the retired tree named as a whole path on a line is a
//!   retired reference. `../src/` and `crates/x/src/` are not root references, because they point
//!   below a crate or out of the tree, and neither is `xsrc/`.
//!
//! A reviewed exception goes in `xtask/layout-allow.toml`, one `[[allow]]` entry per file or
//! directory, each with a `reason`. Only root references can be allowed; a retired reference fails
//! whatever the list says.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use proc_macro2::{Delimiter, TokenStream, TokenTree};

use crate::allow;
use crate::logging::collect_rust_files;

const ALLOW_FILE: &str = "xtask/layout-allow.toml";

/// The directories whose Rust sources are tokenised, relative to the repository root.
const RUST_ROOTS: &[&str] = &["crates", "xtask"];

/// The directories whose other text files are scanned for root-relative `src/` and `patches/`.
/// `crates` and `xtask` appear here too: their manifests, READMEs and XML are text, while their
/// `.rs` files are tokenised instead and skipped by the text walk.
const TEXT_ROOTS: &[&str] = &[
    "cmake", "crates", "design", "docs", ".github", "nix", "pkgsmith", "scripts", "tests", "xtask",
];

/// The files at the repository root that are scanned as text.
const TEXT_FILES: &[&str] = &[
    "build.sh",
    "CMakeLists.txt",
    "Cargo.toml",
    "Makefile",
    "flake.nix",
    "justfile",
];

/// Directories never scanned: VCS metadata, build output, and the vendored submodules, whose
/// `src/` is their own upstream tree rather than this repository's.
const SKIPPED_DIRS: &[&str] = &[".git", ".omp", "node_modules", "subprojects", "target"];

/// The path segments that name the retired location. A Rust literal is a retired reference when
/// these two segments appear next to each other, so `src/database` is not one.
const RETIRED_SEGMENTS: [&str; 2] = ["src", "data"];

/// The root-relative trees a path can start with. A reference to either is a root reference unless
/// it is reviewed.
const ROOT_PREFIXES: [&str; 2] = ["src/", "patches/"];

/// One reference the rule found: the line it is on, what it is, and whether it names the retired
/// tree. A retired reference is never allowed, since the tree it names no longer exists.
#[derive(Debug, PartialEq, Eq)]
struct Finding {
    line: usize,
    what: String,
    retired: bool,
}

/// Run the layout check on the tree the command was run in.
pub fn run() -> ExitCode {
    let root = Path::new(".");
    match check(root) {
        Ok(violations) if violations.is_empty() => {
            println!(
                "[layout] no reference to the root src/ or patches/ tree lacks a reviewed entry, and none names the retired tree"
            );
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

/// Scan every Rust and text file, applying the allow-list, and report stale entries. A retired
/// reference is reported whatever the allow-list says, and it does not count as using an entry.
fn check(root: &Path) -> Result<Vec<String>, String> {
    let allow = allow::load(root, ALLOW_FILE)?;
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut violations = Vec::new();
    let rust = scanned_sources(root)?;
    let text = scanned_text(root)?;
    let files = rust
        .into_iter()
        .map(|file| (file, true))
        .chain(text.into_iter().map(|file| (file, false)));
    for (file, is_rust) in files {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let source = match fs::read_to_string(&file) {
            Ok(source) => source,
            // Binary assets such as fonts and images hold no paths to check.
            Err(error) if error.kind() == ErrorKind::InvalidData && !is_rust => continue,
            Err(error) => return Err(format!("could not read {relative}: {error}")),
        };
        let findings = if is_rust {
            scan_source(&source).map_err(|error| format!("{relative}: {error}"))?
        } else {
            scan_text(&source)
        };
        for finding in findings {
            if finding.retired {
                violations.push(format!(
                    "{relative}:{}: {}; the retired src/data tree moved to data/ and is never allowed",
                    finding.line, finding.what
                ));
                continue;
            }
            match allow.iter().position(|entry| allow::covers(&entry.path, &relative)) {
                Some(index) => {
                    used.insert(index);
                }
                None => violations.push(format!(
                    "{relative}:{}: {}; a build input needs a reviewed entry in {ALLOW_FILE}",
                    finding.line, finding.what
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

/// Every `.rs` file under the Rust roots, sorted so the report is stable.
fn scanned_sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for scanned in RUST_ROOTS {
        let directory = root.join(scanned);
        if directory.is_dir() {
            collect_rust_files(&directory, &mut files)?;
        }
    }
    files.sort();
    Ok(files)
}

/// Every text file the rule scans, sorted so the report is stable. Rust sources are excluded here
/// because they are tokenised, and the allow-list file is excluded because it spells the paths out.
fn scanned_text(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for scanned in TEXT_ROOTS {
        let directory = root.join(scanned);
        if directory.is_dir() {
            collect_text_files(&directory, &mut files)?;
        }
    }
    for name in TEXT_FILES {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    files.retain(|file| !file.ends_with(ALLOW_FILE));
    files.sort();
    Ok(files)
}

/// Recursively collect the non-Rust files under `directory`, skipping `SKIPPED_DIRS`.
fn collect_text_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(directory).map_err(|error| format!("could not read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read {}: {error}", directory.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                collect_text_files(&path, files)?;
            }
        } else if path.extension().is_none_or(|extension| extension != "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Report every string literal in `source` that is a root reference or a retired one. Comments and
/// doc attributes are skipped.
fn scan_source(source: &str) -> Result<Vec<Finding>, String> {
    let stream: TokenStream = source.parse().map_err(|error| format!("does not tokenise: {error}"))?;
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut findings = Vec::new();
    scan_tokens(&tokens, &mut findings);
    Ok(findings)
}

fn scan_tokens(tokens: &[TokenTree], findings: &mut Vec<Finding>) {
    let mut index = 0;
    while index < tokens.len() {
        if let Some(next) = doc_attribute_end(tokens, index) {
            index = next;
            continue;
        }
        match &tokens[index] {
            TokenTree::Group(group) => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                scan_tokens(&inner, findings);
            }
            TokenTree::Literal(literal) => {
                if let Ok(string) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                    let value = string.value();
                    if let Some(finding) = literal_finding(literal.span().start().line, &value) {
                        findings.push(finding);
                    }
                }
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
        index += 1;
    }
}

/// The finding for a Rust string value, if it is a retired reference or a root reference. The
/// retired check comes first, so a value that names the old tree is never filed as a root reference.
fn literal_finding(line: usize, value: &str) -> Option<Finding> {
    if names_retired_tree(value) {
        return Some(Finding {
            line,
            what: format!("string literal `{value}` names the retired src/data tree"),
            retired: true,
        });
    }
    let trimmed = value.strip_prefix("./").unwrap_or(value);
    if ROOT_PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix)) {
        return Some(Finding {
            line,
            what: format!("string literal `{value}` names a root-relative src/ or patches/ path"),
            retired: false,
        });
    }
    None
}

/// Report every root-relative `src/` or `patches/` in a text file, and every retired reference, line
/// by line.
fn scan_text(text: &str) -> Vec<Finding> {
    let retired = RETIRED_SEGMENTS.join("/");
    let mut findings = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line_number = number + 1;
        if names_retired_text(line, &retired) {
            findings.push(Finding {
                line: line_number,
                what: "names the retired src/data tree".to_string(),
                retired: true,
            });
        }
        for prefix in ROOT_PREFIXES {
            for (at, _) in line.match_indices(prefix) {
                if is_root_relative(line, at) {
                    findings.push(Finding {
                        line: line_number,
                        what: format!("`{prefix}` is reached as a root-relative path"),
                        retired: false,
                    });
                }
            }
        }
    }
    findings
}

/// Whether the `src/` or `patches/` that starts at byte `at` of `line` names a path from the
/// repository root. It does not when a longer path precedes it (`crates/x/src/`, `../src/`), and a
/// `${ROOT}/src/` does, because the variable stands for the root.
fn is_root_relative(line: &str, at: usize) -> bool {
    let head = &line[..at];
    // `./src/` names the root explicitly. `../src/` climbs out of the current directory, so it is not.
    let head = match head.strip_suffix("./") {
        Some(rest) if rest.ends_with('.') || rest.ends_with('/') => return false,
        Some(rest) => rest,
        None => head,
    };
    // A `/` before the tree is a directory above it, unless a variable that holds the root precedes it.
    if let Some(parent) = head.strip_suffix('/') {
        return parent.ends_with('}');
    }
    // A name character before the tree means a longer name (`xsrc/`, `subprojects.src/`), not the tree.
    !matches!(head.chars().next_back(), Some(c) if is_name_char(c))
}

/// Whether a line names the retired tree as a whole path. The tree must not run on into a longer
/// name on either side, so `src/database` and `xsrc/data` are not it. A full stop after the tree
/// ends a sentence, so it does not extend the name, but a following letter, digit, `_` or `-` does.
fn names_retired_text(line: &str, tree: &str) -> bool {
    line.match_indices(tree).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + tree.len()..].chars().next();
        !before.is_some_and(is_name_char) && !after.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    })
}

/// Whether a character can be part of a file or directory name, as the scanner reads one.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
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

    /// The retired tree spelt as the rule matches it. Tests build it at run time, so no string
    /// literal in this file names the tree, which would make the file itself a retired reference.
    fn retired_tree() -> String {
        RETIRED_SEGMENTS.join("/")
    }

    #[test]
    fn a_literal_naming_the_retired_tree_is_a_retired_hit() {
        let source = format!("fn f() {{ let p = \"crates/x/{}/lua/init.lua\"; }}", retired_tree());
        let findings = scan_source(&source).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].line, 1);
        assert!(findings[0].retired, "{findings:?}");
    }

    #[test]
    fn a_raw_literal_naming_the_retired_tree_is_a_retired_hit() {
        let source = format!("const P: &str = r#\"{}/layouts\"#;", retired_tree());
        let findings = scan_source(&source).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].retired, "{findings:?}");
    }

    #[test]
    fn a_literal_rooted_in_src_or_patches_is_a_root_hit() {
        for source in [
            "const P: &str = \"src/wit.rs\";",
            "const P: &str = \"./src/generate.rs\";",
            "const P: &str = \"patches/mutter/series\";",
        ] {
            let findings = scan_source(source).unwrap();
            assert_eq!(findings.len(), 1, "{source}");
            assert!(!findings[0].retired, "{source}");
        }
    }

    #[test]
    fn crate_paths_prose_and_climbing_paths_are_not_hits() {
        let source = "const A: &str = \"crates/x/src/lib.rs\";\n\
                      const B: &str = \"panicked at src/main.rs:3\";\n\
                      const C: &str = \"../src/a.h\";";
        assert_eq!(scan_source(source).unwrap(), Vec::new());
    }

    #[test]
    fn the_new_location_and_similar_names_are_not_hits() {
        let source = "const A: &str = \"data/gnoblin/lua/init.lua\";\n\
                      const B: &str = \"srcs/database.rs\";";
        assert_eq!(scan_source(source).unwrap(), Vec::new());
    }

    #[test]
    fn comments_and_doc_attributes_are_not_hits() {
        let tree = retired_tree();
        let source = format!(
            "// see {tree} for the old layout\n/// The Lua runtime lived at {tree}.\n#[doc = \"{tree}\"]\nfn f() {{}}"
        );
        assert_eq!(scan_source(&source).unwrap(), Vec::new());
    }

    #[test]
    fn test_code_is_scanned_too() {
        let source = format!("#[cfg(test)] mod t {{ fn g() {{ let _ = \"{}\"; }} }}", retired_tree());
        let findings = scan_source(&source).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].retired, "{findings:?}");
    }

    #[test]
    fn a_text_reference_to_a_root_tree_is_a_hit_on_its_line() {
        let findings = scan_text("set -e\ncp -r src/cursor out\n");
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].line, 2);
        assert!(!findings[0].retired, "{findings:?}");
    }

    #[test]
    fn text_references_through_a_root_variable_or_dot_slash_are_hits() {
        for line in [
            "ROOT / \"src/config/lua\"",
            "versions = ${GNOBLIN_ROOT}/src/gnome-versions.json",
            "./src/tools/gnoblin-env.sh",
            "apply patches/mutter/series",
        ] {
            assert_eq!(scan_text(line).len(), 1, "{line}");
        }
    }

    #[test]
    fn text_paths_below_a_directory_or_climbing_out_are_not_hits() {
        for line in [
            "#include \"../src/a.h\"",
            "src = crates/x/src/lib.rs",
            "the xsrc/ mirror and subprojects.src/ name",
            "the src tree has no slash",
            "build/mutter/src/core",
        ] {
            assert_eq!(scan_text(line), Vec::new(), "{line}");
        }
    }

    #[test]
    fn a_text_reference_to_the_retired_tree_is_retired_wherever_it_sits() {
        let tree = retired_tree();
        for line in [
            format!("cp -r {tree} out"),
            format!("the code moved out of {tree}."),
            format!("ROOT / \"{tree}\""),
        ] {
            let findings = scan_text(&line);
            assert!(findings.iter().any(|finding| finding.retired), "{line}: {findings:?}");
        }
    }

    #[test]
    fn a_text_name_that_runs_on_from_the_retired_tree_is_not_retired() {
        let tree = retired_tree();
        for line in [
            format!("the {tree}base file"),
            format!("the x{tree} mirror"),
            format!("{tree}_v2"),
            format!("{tree}-old"),
        ] {
            let findings = scan_text(&line);
            assert!(!findings.iter().any(|finding| finding.retired), "{line}: {findings:?}");
        }
    }
}
