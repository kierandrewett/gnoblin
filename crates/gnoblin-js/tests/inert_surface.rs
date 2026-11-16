//! What the script API and the tree schema offer that nothing applies.
//!
//! A conformance run that counts "the script ran" as "the behaviour exists" hides every value that is accepted, kept and
//! read back and then used by nobody. Two lists of such values are kept here, each one reviewed by a person and each entry
//! saying why, and a test fails when the code and the list disagree:
//!
//! - **Tree properties.** Every property a tree class declares (`crates/gnoblin-schema/schema/tree`) has to be read
//!   or written by native code: its name appears as a quoted literal in the non-test source of the compositor, the
//!   renderers and the backends, or, for a record such as `effect`, as a field of the generated struct. A property that
//!   fails that is in [`UNREAD`] with the reason, and a property in [`UNREAD`] that has gained a reader is a stale entry.
//!   The scan over-approximates (a name another class reads counts), so the list is not a proof that everything else is
//!   applied; every entry is one that was checked by hand.
//! - **The library's own "not applied" notices.** A part of the library that accepts a value and tells the user it
//!   was not applied says so in its source. Each file that does is in [`NOTICES`] with the reason, and the page that
//!   documents the feature has to say "Not applied yet".
//!
//! The device settings (`Mouse`, `Touchpad`) are accepted and reported as not applied until the libinput applier lands
//! (issue gnoblin-v09s, another lane); `devices.ts` is listed as optional so that lane can take its notice out.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

fn walk(dir: &Path, ext: &str, into: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            walk(&path, ext, into);
        } else if path.extension().is_some_and(|found| found == ext) {
            into.push(path);
        }
    }
}

// ---- tree properties ---------------------------------------------------------------------------------------------

/// The crates whose non-test source reads or writes a tree property.
const NATIVE: &[&str] = &[
    "crates/gnoblin-render",
    "crates/gnoblin-backend",
    "crates/gnoblin-backend-drm",
    "crates/gnoblin-wayland",
    "crates/gnoblin-compositor",
    "crates/gnoblin-render-software",
    "crates/gnoblin-render-gles",
    "crates/gnoblin-render-vulkan",
    "crates/gnoblin-render-frame",
    "crates/gnoblin-capture",
    "crates/gnoblin",
    "crates/gnoblin-services",
    "extensions",
];

/// The tree properties nothing reads, each with why. `class.property`.
const UNREAD: &[(&str, &str)] = &[
    (
        "effect.algorithm",
        "the renderer has one blur kernel and reads no value from it; the schema says so and keeps the field so a list \
         written for a renderer with a choice still validates",
    ),
    (
        "node.tags",
        "stored and echoed by the tree (`tree_ipc.rs`) and read by nothing: the selector keys of the retired Lua API, and \
         no member of the script API uses them",
    ),
    (
        "screen.screen_x",
        "declared read-only and never published, so a read gives the default 0; `x` is the screen's place",
    ),
    (
        "screen.screen_y",
        "declared read-only and never published, so a read gives the default 0; `y` is the screen's place",
    ),
    (
        "screen.screen_width",
        "declared read-only and never published, so a read gives the default 0; `width` is the screen's size",
    ),
    (
        "screen.screen_height",
        "declared read-only and never published, so a read gives the default 0; `height` is the screen's size",
    ),
    (
        "workspace.index",
        "the compositor creates no workspace node (workspaces are the script's, `gnoblin.workspaces`), so the whole class \
         is unused: its `active`, `output` and `activate` go with it, and so does `window.workspace`, which is always nil",
    ),
    (
        "node.parent",
        "no native code reads or writes a value named `parent`: a node's parent is the tree's structure (`insert` ops and \
         `parented` changes), and the property only repeats it",
    ),
];

/// The non-test source of the native crates, each file cut at its first `#[cfg(test)]`.
fn native_source() -> Vec<String> {
    let root = repo();
    let mut files = Vec::new();
    for dir in NATIVE {
        walk(&root.join(dir), "rs", &mut files);
    }
    files
        .iter()
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            !name.contains("test") && !path.components().any(|part| part.as_os_str() == "tests")
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .map(|text| match text.find("#[cfg(test)]") {
            Some(at) => text[..at].to_owned(),
            None => text,
        })
        .collect()
}

/// Whether `text` holds `.name` as a field access.
fn has_field_access(text: &str, name: &str) -> bool {
    let needle = format!(".{name}");
    text.match_indices(&needle).any(|(at, _)| {
        text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_alphanumeric() || next == '_'))
    })
}

/// Every property of every tree class, as `(class, property, is_record)`.
fn tree_properties() -> Vec<(String, String, bool)> {
    let mut files = Vec::new();
    walk(&repo().join("crates/gnoblin-schema/schema/tree"), "toml", &mut files);
    let mut out = Vec::new();
    for path in files {
        // `messages.toml` declares the wire messages, which are not classes.
        if path.file_name().is_some_and(|name| name == "messages.toml") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("a schema file");
        let table: toml::Table = text.parse().expect("the schema file parses");
        for object in table
            .get("object")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let class = object.get("name").and_then(toml::Value::as_str).expect("a class name");
            let record = object.get("record").and_then(toml::Value::as_bool).unwrap_or(false);
            for property in object
                .get("property")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = property
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .expect("a property name");
                out.push((class.to_owned(), name.to_owned(), record));
            }
        }
    }
    out
}

#[test]
fn every_tree_property_is_read_by_native_code_or_is_listed_unread_with_its_reason() {
    let source = native_source();
    assert!(
        source.len() > 100,
        "the scan found the native sources: {} files",
        source.len()
    );
    let unread: BTreeSet<String> = tree_properties()
        .into_iter()
        .filter(|(_, name, record)| {
            let literal = format!("\"{name}\"");
            !source
                .iter()
                .any(|text| text.contains(&literal) || (*record && has_field_access(text, name)))
        })
        .map(|(class, name, _)| format!("{class}.{name}"))
        .collect();
    let listed: BTreeSet<String> = UNREAD.iter().map(|(name, _)| (*name).to_owned()).collect();
    let new: Vec<&String> = unread.difference(&listed).collect();
    let stale: Vec<&String> = listed.difference(&unread).collect();
    assert!(
        new.is_empty(),
        "tree properties no native code reads or writes: {new:?}. Wire each, remove it from the schema, or add it to UNREAD \
         with the reason and a note in the docs that it is not applied yet"
    );
    assert!(
        stale.is_empty(),
        "these are in UNREAD and a native reader was found: {stale:?}. Take them out of the list"
    );
    assert!(
        UNREAD.iter().all(|(_, why)| why.len() > 20),
        "every entry says why it is inert"
    );
}

// ---- the library's own "not applied" notices ----------------------------------------------------------------------

/// A file of the library that says a value was not applied.
struct Notice {
    /// Under `data/js/gnoblin`.
    file: &'static str,
    why: &'static str,
    /// The page that documents the feature, which has to say "Not applied yet". `None` for a file that is not a feature.
    doc: Option<&'static str>,
    /// Whether the entry may be gone: it belongs to work another lane is doing.
    optional: bool,
}

const OUTPUTS: &str = "docs/shell-api/outputs.md";
const STYLE: &str = "docs/js-style.md";

const NOTICES: &[Notice] = &[
    Notice {
        file: "display/output_profile.ts",
        why: "OutputProfile.vrr is accepted and kept: the output request has no variable refresh rate key",
        doc: Some(OUTPUTS),
        optional: false,
    },
    Notice {
        file: "display/types.ts",
        why: "the declaration of the same vrr setting",
        doc: Some(OUTPUTS),
        optional: false,
    },
    Notice {
        file: "display_manager/manager.ts",
        why: "Camera2D transforms, Camera3D and viewport.clearColor are kept and read back, and the shell draws flat \
              workspaces with no background colour behind them",
        doc: Some(OUTPUTS),
        optional: false,
    },
    Notice {
        file: "host/style_scopes.ts",
        why: "style properties no effect kind draws (background, filter, inset shadows) stay in computedStyle",
        doc: Some(STYLE),
        optional: false,
    },
    Notice {
        file: "host/styles.ts",
        why: "the same notice, raised where a style is applied to a window",
        doc: Some(STYLE),
        optional: false,
    },
    Notice {
        file: "style/map.ts",
        why: "the mapping from a style to effects, which lists what it could not map",
        doc: Some(STYLE),
        optional: false,
    },
    Notice {
        file: "layouts/strip.ts",
        why: "the strip layout writes depth for a renderer that draws it, and its fade of the ring is not applied: a node's \
              opacity is the window's own",
        doc: None,
        optional: false,
    },
    Notice {
        file: "session/types.ts",
        why: "describes the reload report of settings no process applies; not a feature",
        doc: None,
        optional: false,
    },
    Notice {
        file: "devices.ts",
        why: "the mouse and touchpad groups are accepted and validated and no device reads them until the libinput applier \
              lands (gnoblin-v09s, the InputSettings lane)",
        doc: None,
        optional: true,
    },
];

#[test]
fn every_place_the_library_says_not_applied_is_reviewed_and_its_page_says_not_applied_yet() {
    let library = repo().join("data/js/gnoblin");
    let mut files = Vec::new();
    walk(&library, "ts", &mut files);
    let found: BTreeSet<String> = files
        .iter()
        .filter(|path| {
            let text = fs::read_to_string(path).unwrap_or_default().to_lowercase();
            text.contains("not applied") || text.contains("unapplied")
        })
        .map(|path| {
            path.strip_prefix(&library)
                .expect("under the library")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let listed: BTreeSet<&str> = NOTICES.iter().map(|notice| notice.file).collect();
    let new: Vec<&String> = found.iter().filter(|file| !listed.contains(file.as_str())).collect();
    assert!(
        new.is_empty(),
        "files that say a value is not applied and are not reviewed: {new:?}. Add each to NOTICES with the reason, and \
         write \"Not applied yet\" on the page that documents the feature"
    );
    for notice in NOTICES {
        assert!(notice.why.len() > 20, "{}: say why", notice.file);
        if !notice.optional {
            assert!(
                found.contains(notice.file),
                "{} no longer says anything is not applied: take it out of NOTICES",
                notice.file
            );
        }
        if let Some(doc) = notice.doc {
            let page = fs::read_to_string(repo().join(doc)).unwrap_or_else(|error| panic!("{doc}: {error}"));
            assert!(
                page.contains("not applied yet"),
                "{doc} documents {} and does not say \"not applied yet\"",
                notice.file
            );
        }
    }
}
