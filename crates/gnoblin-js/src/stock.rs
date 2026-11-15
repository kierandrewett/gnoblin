//! The stock library: the TypeScript every generation can import as the embedded `gnoblin` modules.
//!
//! The sources live in `data/js/gnoblin/` and the whole directory is compiled into the binary (`build.rs` lists it).
//! [`stock_modules`] strips their types once and returns the name to JavaScript map
//! [`JsConfig::embed`](crate::JsConfig::embed) takes. The modules import one another by their embedded names
//! (`gnoblin/layout`), because the loader resolves a relative import only against a file module.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use crate::loader::JsTypeScriptModule;
use crate::ts::strip_types;

/// The `init.ts` `gnoblinctl config init` writes (`data/js/starter/init.ts`). It is the readable, commented text of
/// the stock default, which the shell runs as `installDefaults` of the `gnoblin/starter` module when there is no
/// `init.ts`; a test holds the two statement for statement. The examples in it are comments to switch on.
pub const STARTER_CONFIG: &str = include_str!("../../../data/js/starter/init.ts");

// `MODULES`: `(import name, file under data/js/gnoblin, source)` for every `.ts` file under that directory, written by
// `build.rs` so a new module needs no edit here. `gnoblin` is the user-facing module and only re-exports the others;
// `gnoblin/host` is the object model.
include!(concat!(env!("OUT_DIR"), "/stock_modules.rs"));

/// The stock modules by import name, as JavaScript, ready to embed in a [`JsConfig`](crate::JsConfig).
///
/// A source that fails to strip cannot happen for the files compiled in (a test imports every one), but a
/// panic in a compositor-side process is worse than an error where it is used, so such a module becomes
/// one that throws the reason when imported.
pub fn stock_modules() -> HashMap<String, String> {
    stock_modules_in(None)
}

/// Like [`stock_modules`], reading each module from `dir` (laid out as `data/js/gnoblin`) when the file is there.
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
            let path = dir.map_or_else(|| Path::new("data/js/gnoblin").join(file), |dir| dir.join(file));
            let source = match dir.and_then(|_| std::fs::read_to_string(&path).ok()) {
                Some(text) => Cow::Owned(text),
                None => Cow::Borrowed(*compiled),
            };
            ((*name).to_owned(), JsTypeScriptModule { path, source })
        })
        .collect()
}

/// The JavaScript of the stock module `name`, whose TypeScript is `source` (read from `path`). A source that does not
/// strip becomes a module that throws the reason when it is imported, because a panic or a missing module in a
/// compositor-side process is worse than an error where the module is used.
pub(crate) fn javascript_of(name: &str, path: &Path, source: &str) -> String {
    match strip_types(path, source) {
        Ok(stripped) => stripped.code,
        Err(error) => {
            let reason = error.to_string().replace(['\\', '"', '\n'], " ");
            format!("throw new Error(\"stock module {name} does not compile: {reason}\");\n")
        }
    }
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js/gnoblin");
        let mut on_disk = Vec::new();
        walk(&root, &mut on_disk, &root);
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
        assert!(names.contains(&"gnoblin"), "index.ts is the user-facing module");
        assert!(names.contains(&"gnoblin/layouts/arc"));
        assert!(names.contains(&"gnoblin/host"));
    }
}
