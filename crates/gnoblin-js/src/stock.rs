//! The stock library: the TypeScript every generation can import as the embedded `gnoblin-core` and `gnoblin-std`
//! modules.
//!
//! The sources live in `data/js/gnoblin-core/` and `data/js/gnoblin-std/` and both directories are compiled into the
//! binary (`build.rs` lists them). [`stock_modules`] strips their types once and returns the name to JavaScript map
//! [`JsConfig::embed`](crate::JsConfig::embed) takes. The modules import one another by their embedded names
//! (`gnoblin-std/layout`), because the loader resolves a relative import only against a file module.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use crate::error::JsTsError;
use crate::loader::JsTypeScriptModule;
use crate::ts::strip_shared;

/// The stock desktop, and the only description of it: `data/js/starter/init.ts`. `gnoblinctl config init` writes this
/// text byte for byte, and the shell runs the same text, as the embedded module [`STARTER_MODULE_NAME`], when there is
/// no `init.ts` (or it could not load). The examples in it are comments to switch on.
pub const STARTER_CONFIG: &str = include_str!("../../../data/js/starter/init.ts");

/// The import name the shell runs [`STARTER_CONFIG`] under as the entry of a generation that has no user entry.
pub(crate) const STARTER_MODULE_NAME: &str = "gnoblin-std/starter";

/// [`STARTER_CONFIG`] as the embedded TypeScript module [`STARTER_MODULE_NAME`], named by its path in the repository.
pub(crate) fn starter_module() -> JsTypeScriptModule {
    JsTypeScriptModule {
        path: Path::new("data/js/starter/init.ts").to_path_buf(),
        source: Cow::Borrowed(STARTER_CONFIG),
    }
}

// `MODULES`: `(import name, file under data/js, source)` for every `.ts` file of the two package directories, written
// by `build.rs` so a new module needs no edit here. A package's `index.ts` is the package entry (`gnoblin-core`,
// `gnoblin-std`) and only re-exports the others; `<package>/host` is the part of it that needs the shell host.
include!(concat!(env!("OUT_DIR"), "/stock_modules.rs"));

/// The package every shell needs: the machinery, with no desktop policy. Its entry is `gnoblin-core`.
pub(crate) const CORE_PACKAGE: &str = "gnoblin-core";

/// The optional package of desktop policy, which imports `gnoblin-core` and is never imported by it.
pub(crate) const STD_PACKAGE: &str = "gnoblin-std";

/// The packages of the library, core first.
pub(crate) const PACKAGES: [&str; 2] = [CORE_PACKAGE, STD_PACKAGE];

/// The import name of the object model of the `gnoblin` global (`gnoblin.clients`, `Client`, the listeners): the
/// stock module `data/js/gnoblin-core/host.ts`, which the shell imports to build a generation and which the
/// `gnoblin-core` entry re-exports. `gnoblin-std/host` is what adds the desktop policy to it, and a script that
/// imports `gnoblin-std` is what loads that.
pub(crate) const HOST_MODULE_NAME: &str = "gnoblin-core/host";

/// What a package entry has beyond the stock index: the exports of the package's host module, so one import line
/// reaches `Client` and `ClientListener` alike (`Workspace` and `startDecorations` for `gnoblin-std`). The stock
/// sources stay as they are because their own tests embed them without a host (a host module is only evaluated when
/// something imports it). The shell appends this to the index it embeds and [`editor_library`] appends it to the index
/// an editor reads, so the two cannot disagree.
pub(crate) fn host_export(package: &str) -> String {
    format!("\nexport * from \"{package}/host\";\n")
}

/// The stock library as an editor and a type checker read it: `(path under data/js, TypeScript)` for every module in
/// path order, so each starts with its package directory (`gnoblin-std/scene/node.ts`). These are the real sources,
/// byte for byte, except the package entries (`gnoblin-core/index.ts`, `gnoblin-std/index.ts`), which are the modules
/// the runtime builds (the stock index plus [`host_export`]). `gnoblinctl types init` writes them next to the user's
/// config, so `import ... from "gnoblin-std/scene/node"` opens the file the shell runs.
pub fn editor_library() -> Vec<(String, String)> {
    MODULES
        .iter()
        .map(|(name, file, source)| {
            let mut text = (*source).to_owned();
            if PACKAGES.contains(name) {
                text.push_str(&host_export(name));
            }
            ((*file).to_owned(), text)
        })
        .collect()
}

/// The stock modules by import name, as JavaScript, ready to embed in a [`JsConfig`](crate::JsConfig).
///
/// A source that fails to strip cannot happen for the files compiled in (a test imports every one), but a
/// panic in a compositor-side process is worse than an error where it is used, so such a module becomes
/// one that throws the reason when imported.
pub fn stock_modules() -> HashMap<String, String> {
    stock_modules_in(None)
}

/// Like [`stock_modules`], reading each module from `dir` (laid out as `data/js`: one directory per package) when the
/// file is there.
/// A file that is missing, or that cannot be read, is the compiled-in module: a half-installed directory must
/// not leave the shell without a stock library. A file that does not strip becomes a module that throws.
pub fn stock_modules_in(dir: Option<&Path>) -> HashMap<String, String> {
    stock_sources_in(dir)
        .into_iter()
        .map(|(name, module)| {
            let javascript = javascript_of(&name, &module.path, &module.source);
            (name, javascript)
        })
        .collect()
}

/// The stock modules by import name as TypeScript, which is how the shell embeds them
/// ([`JsConfig::embed_typescript`](crate::JsConfig::embed_typescript)): nothing is stripped until a module is
/// loaded, and a module the bytecode cache holds is never stripped at all. `dir` is read as in [`stock_modules_in`].
pub fn stock_sources_in(dir: Option<&Path>) -> HashMap<String, JsTypeScriptModule> {
    MODULES
        .iter()
        .map(|(name, file, compiled)| {
            let path = dir.map_or_else(|| Path::new("data/js").join(file), |dir| dir.join(file));
            let source = match dir.and_then(|_| std::fs::read_to_string(&path).ok()) {
                Some(text) => Cow::Owned(text),
                None => Cow::Borrowed(*compiled),
            };
            ((*name).to_owned(), JsTypeScriptModule { path, source })
        })
        .collect()
}

/// The JavaScript of the stock module `name`, whose TypeScript is `source` (read from `path`). The types are stripped
/// once per process ([`strip_shared`]), so every generation and every test that imports the library shares the result.
/// A source that does not strip becomes a module that throws the reason when it is imported, because a panic or a
/// missing module in a compositor-side process is worse than an error where the module is used.
pub(crate) fn javascript_of(name: &str, path: &Path, source: &str) -> String {
    match strip_shared(path, source) {
        Ok(stripped) => stripped.code.clone(),
        Err(error) => failing_module(name, &error),
    }
}

/// The module that stands in for stock module `name` when its source did not strip: it throws `error`'s text.
pub(crate) fn failing_module(name: &str, error: &JsTsError) -> String {
    let reason = error.to_string().replace(['\\', '"', '\n'], " ");
    format!("throw new Error(\"stock module {name} does not compile: {reason}\");\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stock_module_strips_to_javascript() {
        let modules = stock_modules();
        assert_eq!(modules.len(), MODULES.len());
        for (name, source) in &modules {
            assert!(!source.starts_with("throw new Error"), "{name}: {source}");
        }
    }

    /// The table is generated from the directory, so a file cannot be left out; this holds the generator to it.
    #[test]
    fn every_ts_file_under_the_stock_directory_is_embedded() {
        fn walk(dir: &Path, found: &mut Vec<String>, root: &Path) {
            for entry in std::fs::read_dir(dir).expect("the stock directory is readable") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    walk(&path, found, root);
                } else if path.extension().is_some_and(|extension| extension == "ts") {
                    let relative = path.strip_prefix(root).expect("under the root");
                    found.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js");
        let mut on_disk = Vec::new();
        for package in PACKAGES {
            walk(&root.join(package), &mut on_disk, &root);
        }
        on_disk.sort();
        let mut embedded: Vec<&str> = MODULES.iter().map(|(_, file, _)| *file).collect();
        embedded.sort_unstable();
        assert_eq!(embedded, on_disk);
    }

    /// The modules that import one another at run time (the stripped JavaScript has no type-only imports) form a
    /// tree of layers, never a loop: a loop makes the order modules evaluate in depend on which one was imported first.
    #[test]
    fn stock_modules_do_not_import_each_other_in_a_cycle() {
        let modules = stock_modules();
        // The module a line of stripped JavaScript imports or re-exports, if it does.
        fn specifier(line: &str) -> Option<&str> {
            let line = line.trim_start();
            if !(line.starts_with("import ") || line.starts_with("export ")) {
                return None;
            }
            let at = line
                .rfind(" from \"")
                .map(|at| at + " from \"".len())
                .or_else(|| line.strip_prefix("import \"").map(|_| "import \"".len()))?;
            let end = line[at..].find('"')?;
            Some(&line[at..at + end])
        }
        let imports_of = |name: &str| -> Vec<String> {
            modules[name]
                .lines()
                .filter_map(specifier)
                .filter(|target| modules.contains_key(*target))
                .map(str::to_owned)
                .collect()
        };
        // Depth-first search; `path` is the chain of modules being visited, so a repeat in it is a cycle.
        fn visit(
            name: &str,
            imports_of: &dyn Fn(&str) -> Vec<String>,
            path: &mut Vec<String>,
            done: &mut std::collections::HashSet<String>,
        ) {
            if done.contains(name) {
                return;
            }
            if let Some(at) = path.iter().position(|seen| seen == name) {
                let mut cycle = path[at..].to_vec();
                cycle.push(name.to_owned());
                panic!("stock modules import each other in a cycle: {}", cycle.join(" -> "));
            }
            path.push(name.to_owned());
            for next in imports_of(name) {
                visit(&next, imports_of, path, done);
            }
            path.pop();
            done.insert(name.to_owned());
        }
        let mut done = std::collections::HashSet::new();
        let mut names: Vec<&String> = modules.keys().collect();
        names.sort();
        for name in names {
            visit(name, &imports_of, &mut Vec::new(), &mut done);
        }
    }

    #[test]
    fn import_names_follow_the_path() {
        let names: Vec<&str> = MODULES.iter().map(|(name, _, _)| *name).collect();
        assert!(
            names.contains(&"gnoblin-core"),
            "the core index.ts is the package entry"
        );
        assert!(names.contains(&"gnoblin-std"), "the std index.ts is the package entry");
        assert!(names.contains(&"gnoblin-std/layouts/arc"));
        assert!(names.contains(&"gnoblin-core/host"));
        assert!(names.contains(&"gnoblin-std/host"));
        // A module that sits beside a directory of the same name is that directory's one import path.
        for barrel in [
            "gnoblin-std/scene",
            "gnoblin-std/style",
            "gnoblin-core/input",
            "gnoblin-std/ui",
        ] {
            assert!(names.contains(&barrel), "{barrel}");
        }
        // The `index.ts` of a subdirectory is that directory's import name, so the guarded XR entry keeps its path.
        assert!(names.contains(&"gnoblin-std/extensions/xr"));
        assert!(!names.contains(&"gnoblin-std/extensions/xr/index"));
        assert!(
            !names.contains(&"gnoblin-core/index"),
            "the package entry is the package name"
        );
        // The bare `gnoblin` specifier is gone: a script names the package it wants.
        assert!(
            !names
                .iter()
                .any(|name| *name == "gnoblin" || name.starts_with("gnoblin/"))
        );
    }

    /// The editor reads the sources the shell embeds: the same files, the same bytes, and for a package entry the same
    /// text the shell builds, so a type the editor shows is the type the runtime has.
    #[test]
    fn the_editor_library_is_the_embedded_sources() {
        let library = editor_library();
        assert_eq!(library.len(), MODULES.len());
        for ((name, file, source), (editor_file, text)) in MODULES.iter().zip(&library) {
            assert_eq!(file, editor_file);
            if PACKAGES.contains(name) {
                assert_eq!(*text, format!("{source}{}", host_export(name)));
            } else {
                assert_eq!(text, source, "{file}");
            }
        }
        for package in PACKAGES {
            let (_, index) = library
                .iter()
                .find(|(file, _)| file == &format!("{package}/index.ts"))
                .expect("the package entry is listed");
            assert!(index.ends_with(&format!("export * from \"{package}/host\";\n")));
        }
    }

    /// Every module specifier a TypeScript source imports or re-exports from, `import type` and `import("...")`
    /// included: the text between the quotes after `from`, after a bare `import`, and inside `import(`.
    fn specifiers(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        for line in source.lines() {
            let line = line.trim_start();
            if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
                continue;
            }
            let mut rest = line;
            while let Some(at) = rest
                .find("from \"")
                .or_else(|| rest.find("import(\""))
                .or_else(|| rest.strip_prefix("import \"").map(|_| 0))
            {
                let tail = &rest[at..];
                let Some(open) = tail.find('"') else { break };
                let Some(len) = tail[open + 1..].find('"') else { break };
                found.push(tail[open + 1..open + 1 + len].to_owned());
                rest = &tail[open + 1 + len + 1..];
            }
        }
        found
    }

    /// The core imports of std modules that still exist: each is a place `gnoblin-core` reaches into `gnoblin-std`
    /// and is being removed. The list only ever shrinks; the test fails on an entry whose import is gone.
    const CORE_TO_STD: &[(&str, &str)] = &[
        ("gnoblin-core/host/adoption", "gnoblin-std/host/desks"),
        ("gnoblin-core/host/adoption", "gnoblin-std/host/fitting"),
        ("gnoblin-core/host/adoption", "gnoblin-std/host/membership"),
        ("gnoblin-core/host/adoption", "gnoblin-std/host/work_area"),
        ("gnoblin-core/host/adoption", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/adoption", "gnoblin-std/host/workspace_state"),
        ("gnoblin-core/host/adoption", "gnoblin-std/host/workspaces"),
        ("gnoblin-core/host/client", "gnoblin-std/host/desks"),
        ("gnoblin-core/host/client", "gnoblin-std/host/membership"),
        ("gnoblin-core/host/client", "gnoblin-std/host/style_scopes"),
        ("gnoblin-core/host/client", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/client", "gnoblin-std/host/workspace_state"),
        ("gnoblin-core/host/client", "gnoblin-std/reposition"),
        ("gnoblin-core/host/client", "gnoblin-std/style/resolve"),
        ("gnoblin-core/host/client", "gnoblin-std/style/style"),
        ("gnoblin-core/host/client", "gnoblin-std/transition/engine"),
        ("gnoblin-core/host/client", "gnoblin-std/transition/transition"),
        ("gnoblin-core/host/client", "gnoblin-std/transition/types"),
        (
            "gnoblin-core/host/collection_events",
            "gnoblin-std/host/workspace_state",
        ),
        ("gnoblin-core/host/collection_state", "gnoblin-std/display/display"),
        (
            "gnoblin-core/host/collection_state",
            "gnoblin-std/display/output_profile",
        ),
        ("gnoblin-core/host/collection_state", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/collections", "gnoblin-std/host/style_scopes"),
        ("gnoblin-core/host/collections", "gnoblin-std/style/style"),
        ("gnoblin-core/host/console", "gnoblin-std/display/display"),
        ("gnoblin-core/host/console", "gnoblin-std/display/output_profile"),
        ("gnoblin-core/host/console", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/context", "gnoblin-std/display_manager/manager"),
        ("gnoblin-core/host/context", "gnoblin-std/extensions/xr/runtime"),
        ("gnoblin-core/host/context", "gnoblin-std/focus"),
        ("gnoblin-core/host/context", "gnoblin-std/frame/decorations"),
        ("gnoblin-core/host/context", "gnoblin-std/gesture_actions"),
        ("gnoblin-core/host/context", "gnoblin-std/host/desks"),
        ("gnoblin-core/host/context", "gnoblin-std/host/show"),
        ("gnoblin-core/host/context", "gnoblin-std/host/style_scopes"),
        ("gnoblin-core/host/context", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/context", "gnoblin-std/host/workspace_state"),
        ("gnoblin-core/host/context", "gnoblin-std/transition/engine"),
        ("gnoblin-core/host/context", "gnoblin-std/transition/staging"),
        ("gnoblin-core/host/entry", "gnoblin-std/host/layout_driver"),
        ("gnoblin-core/host/entry", "gnoblin-std/host/snapshot"),
        ("gnoblin-core/host/entry", "gnoblin-std/host/workspaces"),
        ("gnoblin-core/host/entry", "gnoblin-std/lock"),
        ("gnoblin-core/host/entry", "gnoblin-std/scene/registry"),
        ("gnoblin-core/host/entry", "gnoblin-std/ui/frame"),
        ("gnoblin-core/host/facade", "gnoblin-std/host/displays"),
        ("gnoblin-core/host/facade", "gnoblin-std/host/policy"),
        ("gnoblin-core/host/facade", "gnoblin-std/host/rollback"),
        ("gnoblin-core/host/facade", "gnoblin-std/host/staging"),
        ("gnoblin-core/host/facade", "gnoblin-std/host/xr_bridge"),
        ("gnoblin-core/host/frame", "gnoblin-std/host/desks"),
        ("gnoblin-core/host/frame", "gnoblin-std/host/membership"),
        ("gnoblin-core/host/frame", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/frame", "gnoblin-std/host/workspace_state"),
        ("gnoblin-core/host/frame", "gnoblin-std/host/workspaces"),
        ("gnoblin-core/host/frame", "gnoblin-std/scene/registry"),
        ("gnoblin-core/host/frame", "gnoblin-std/ui/frame"),
        ("gnoblin-core/host/frame", "gnoblin-std/window_state"),
        ("gnoblin-core/host/gnoblin", "gnoblin-std/extensions/xr/availability"),
        ("gnoblin-core/host/gnoblin", "gnoblin-std/host/show"),
        ("gnoblin-core/host/gnoblin", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/gnoblin", "gnoblin-std/scene/scene"),
        ("gnoblin-core/host/gnoblin", "gnoblin-std/transition/transition"),
        ("gnoblin-core/host/gnoblin", "gnoblin-std/transition/types"),
        ("gnoblin-core/host/input_bridge", "gnoblin-std/host/work_area"),
        ("gnoblin-core/host/input_bridge", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/input_bridge", "gnoblin-std/host/workspace_state"),
        ("gnoblin-core/host/input_bridge", "gnoblin-std/ui/state"),
        ("gnoblin-core/host/listeners", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/listeners", "gnoblin-std/window_state"),
        ("gnoblin-core/host/output", "gnoblin-std/host/desks"),
        ("gnoblin-core/host/output", "gnoblin-std/host/show"),
        ("gnoblin-core/host/output", "gnoblin-std/host/work_area"),
        ("gnoblin-core/host/output", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/roots", "gnoblin-std/display/display"),
        ("gnoblin-core/host/roots", "gnoblin-std/display/output_profile"),
        ("gnoblin-core/host/roots", "gnoblin-std/host/show"),
        ("gnoblin-core/host/roots", "gnoblin-std/host/work_area"),
        ("gnoblin-core/host/roots", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/roots", "gnoblin-std/host/workspaces"),
        ("gnoblin-core/host/seat", "gnoblin-std/focus"),
        ("gnoblin-core/host/services", "gnoblin-std/alerts"),
        ("gnoblin-core/host/services", "gnoblin-std/frame/client"),
        ("gnoblin-core/host/services", "gnoblin-std/frame/decorations"),
        ("gnoblin-core/host/services", "gnoblin-std/host/membership"),
        ("gnoblin-core/host/services", "gnoblin-std/host/workspace"),
        ("gnoblin-core/host/services", "gnoblin-std/lock"),
        ("gnoblin-core/host/services", "gnoblin-std/menus/windows"),
        ("gnoblin-core/host/services", "gnoblin-std/snap"),
        ("gnoblin-core/host/services", "gnoblin-std/window_state"),
        ("gnoblin-core/input/context", "gnoblin-std/display/display"),
    ];

    /// `gnoblin-std` depends on `gnoblin-core`, never the reverse: a shell that only needs the machinery runs on
    /// `gnoblin-core` and loads no std module, so no core module may import one.
    #[test]
    fn no_core_module_imports_a_std_module() {
        let mut found: Vec<(String, String)> = Vec::new();
        for (name, _, source) in MODULES {
            if !(*name == CORE_PACKAGE || name.starts_with("gnoblin-core/")) {
                continue;
            }
            for target in specifiers(source) {
                if target == STD_PACKAGE || target.starts_with("gnoblin-std/") {
                    found.push(((*name).to_owned(), target));
                }
            }
        }
        found.sort();
        found.dedup();
        let allowed: Vec<(String, String)> = CORE_TO_STD
            .iter()
            .map(|(from, to)| ((*from).to_owned(), (*to).to_owned()))
            .collect();
        let new: Vec<_> = found.iter().filter(|pair| !allowed.contains(pair)).collect();
        let stale: Vec<_> = allowed.iter().filter(|pair| !found.contains(pair)).collect();
        assert!(new.is_empty(), "core modules import std modules: {new:#?}");
        assert!(
            stale.is_empty(),
            "CORE_TO_STD lists imports that no longer exist: {stale:#?}"
        );
    }
}
