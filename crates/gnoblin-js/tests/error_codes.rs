//! One scheme of error codes for the script API (`docs/shell-api/errors.md`, "Error codes").
//!
//! Three schemes existed: untyped kebab-case `GnoblinError.code` strings, the snake_case `OperationErrorCode`, and
//! bespoke classes with no code. The specification gives no codes, so the snake_case set the host already spoke is the
//! one, extended where the kebab codes said more. Three halves hold it:
//!
//! - the docs table and the `ErrorCode` of the library (the type scripts import) list the same codes,
//! - no source throws a code that is not one of them, and
//! - runs of throw sites show a `code` from the set, and an error class a script catches by type carries one too.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::policy::world;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The members of `export type ErrorCode = | "a" | "b" ...;` in `text`.
fn union_members(text: &str) -> BTreeSet<String> {
    let start = text
        .find("export type ErrorCode =")
        .expect("the file declares ErrorCode");
    let body = &text[start..];
    let end = body.find(';').expect("the union ends");
    body[..end].split('"').skip(1).step_by(2).map(str::to_owned).collect()
}

/// The first column of the "Error codes" table of the errors page.
fn documented_codes() -> BTreeSet<String> {
    let page = read("docs/shell-api/errors.md");
    let start = page.find("## Error codes").expect("the page has the section");
    let rest = &page[start + 3..];
    let section = &rest[..rest.find("\n## ").unwrap_or(rest.len())];
    section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split('`').next())
        .map(str::to_owned)
        .collect()
}

fn typescript_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("a directory")
            .map(|entry| entry.expect("an entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, into);
            } else if path.extension().is_some_and(|ext| ext == "ts") {
                into.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo().join("data/js"), &mut files);
    files
}

#[test]
fn the_docs_and_the_library_list_the_same_codes() {
    let library = union_members(&read("data/js/gnoblin-core/errors.ts"));
    let documented = documented_codes();
    assert_eq!(library, documented, "errors.ts and docs/shell-api/errors.md");
    assert!(
        library.iter().all(|code| !code.contains('-')),
        "the codes are snake_case: {library:?}"
    );
    assert_eq!(library.len(), 13);
}

/// Every string literal that follows `opener(` in `text` as its first argument, ignoring white space.
fn first_literals(text: &str, opener: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(opener) {
        let mut rest = text[from + at + opener.len()..].trim_start();
        from += at + opener.len();
        if let Some(after) = rest.strip_prefix('"') {
            rest = after;
            if let Some(end) = rest.find('"') {
                found.push(rest[..end].to_owned());
            }
        }
    }
    found
}

#[test]
fn no_source_throws_a_code_outside_the_scheme() {
    let allowed = union_members(&read("data/js/gnoblin-core/errors.ts"));
    let mut bad = Vec::new();
    for path in typescript_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable source");
        for opener in [
            "new GnoblinError(",
            "new SceneError(",
            "world.error(",
            "backend.error(",
            "host.error(",
            "refuse(",
            ".fail(",
        ] {
            for literal in first_literals(&text, opener) {
                if !allowed.contains(&literal) {
                    bad.push(format!("{}: {opener}\"{literal}\"", path.display()));
                }
            }
        }
    }
    assert!(bad.is_empty(), "codes outside the scheme:\n{}", bad.join("\n"));
}

#[test]
fn the_old_kebab_case_codes_are_gone_from_everything_a_script_reads() {
    let old = [
        "\"read-only\"",
        "\"spawn-failed\"",
        "\"unknown-workspace\"",
        "\"not-a-member\"",
        "\"no-output\"",
        "\"no-seat\"",
        "\"output-conflict\"",
        "\"forbidden\"",
        "\"nested\"",
        "OperationErrorCode",
    ];
    let mut found = Vec::new();
    for path in typescript_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable source");
        for (index, line) in text.lines().enumerate() {
            // The host's words for a refused write are matched in `errors.ts` and in the places that compare them with
            // what the host answered; they are never a `code` a script sees.
            let host_word = line.contains("case \"") || line.contains("=== \"") || line.contains("!== \"");
            if !host_word && old.iter().any(|name| line.contains(name)) {
                found.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "old codes are back:\n{}", found.join("\n"));
    for page in [
        "docs/shell-api/errors.md",
        "docs/shell-api/types.md",
        "docs/shell-api/windows.md",
        "docs/shell-api/input.md",
        "docs/shell-api/processes.md",
    ] {
        let text = read(page);
        for gone in [
            "`spawn-failed`",
            "`read-only`",
            "`destroyed`",
            "`detached`",
            "`duplicate`",
            "`no-seat`",
        ] {
            assert!(!text.contains(gone), "{page} still names {gone}");
        }
    }
}

/// `name|code|instanceof GnoblinError|instanceof TypeError` of what `source` throws.
fn caught(rig: &mut common::Rig, source: &str) -> String {
    rig.eval(&format!(
        "(() => {{ try {{ {source}; return 'no error'; }} catch (e) {{ \
         return [e.name, e.code, e instanceof GnoblinError, e instanceof TypeError].join('|'); }} }})()"
    ))
}

#[test]
fn what_the_throw_sites_throw_carries_a_documented_code() {
    let mut rig = world(1, false, "new Layout()", "");
    let allowed = union_members(&read("data/js/gnoblin-core/errors.ts"));
    for (source, expected) in [
        // a workspace that is not there
        (
            "gnoblin.workspaces.show('nowhere')",
            "GnoblinError|not_found|true|false",
        ),
        // a second workspace with a taken id
        (
            "gnoblin.workspaces.add(new Workspace({ id: '1' }))",
            "GnoblinError|already_exists|true|false",
        ),
        // a collection the compositor owns
        ("gnoblin.outputs.clear()", "GnoblinError|read_only|true|false"),
        // a scene root that the compositor owns
        ("gnoblin.scene.root.destroy()", "SceneError|denied|true|false"),
        // a trigger the grammar refuses is a TypeError a script can still branch on
        ("parseTrigger('Super+!')", "TypeError|invalid_argument|false|true"),
    ] {
        let got = caught(&mut rig, source);
        assert_eq!(got, expected, "{source}");
        let code = got.split('|').nth(1).expect("a code");
        assert!(allowed.contains(code), "{source}: {code} is not a documented code");
    }
    // two drags on one chord are refused at registration, and the refusal has a code too
    let conflict = rig.eval(
        "(() => { try { new FloatingLayout({ triggers: { move: 'Super+MouseLeft+Drag', resize: 'Super+MouseLeft+Drag' } }); \
         return 'no error'; } catch (e) { return e.name + '|' + e.code; } })()",
    );
    assert_eq!(conflict, "BindingConflictError|already_exists");
}
