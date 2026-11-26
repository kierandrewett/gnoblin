//! The stock library: the TypeScript every generation can import as `gnoblin-core` and `gnoblin-std`.
//!
//! The hand-written sources live in `data/js/gnoblin-core/` and `data/js/gnoblin-std/` and are installed as data under
//! `<prefix>/share/gnoblin/js/<package>/` (`cargo xtask dist`). They are read at run time: the loader finds a package by the
//! directories of [`stock_search_path`], and a package that is in none of them is an import error that says where it looked
//! (`gnoblin-std is not installed (looked in: ...)`). A shell that imports only `gnoblin-core` therefore runs on a prefix that has
//! no std library, which is the point of the split.
//!
//! The generated modules (`gnoblin-core/generated/*`, `gnoblin-std/generated/*`: the schema's classes, the ABI constants, the verbs,
//! the std classes and records) are the exception. `gnoblin-sdkgen` renders them from the schema and the ABI crates in `build.rs`
//! and this crate embeds them, so a module can never describe a different catalog from the libraries the runtime was built with.
//! They appear under their usual paths, beside the files of the package that is present, and win over an installed file of the same
//! path.
//!
//! [`stock_sources_in`] reads the packages once and returns the name to TypeScript map [`JsConfig::embed_typescript`](crate::JsConfig)
//! takes. The modules import one another by their package names (`gnoblin-std/layout`), because the loader resolves a relative import
//! only against a file module. The import name of a file is its path under the library root without `.ts`, and the `index.ts` of a
//! directory is that directory's name (`gnoblin-core/index.ts` is `gnoblin-core`), the rule TypeScript applies to a directory.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use crate::choice::StockRuntime;
use crate::error::JsTsError;
use crate::loader::JsTypeScriptModule;
use crate::ts::strip_shared;

/// The variable that names the one directory the stock library is read from, instead of searching for it: for a person working
/// on the stock shell, and for a test that installs a library of its own.
pub const STOCK_DIR_VARIABLE: &str = "GNOBLIN_JS_STOCK_DIR";

/// The directory of the data prefix that holds the library: `<prefix>/share/gnoblin/js`.
const INSTALLED: &str = "share/gnoblin/js";

/// Where a source tree keeps the library.
const SOURCE_TREE: &str = "data/js";

/// The packaged default shell under a data directory (`share/gnoblin/default-shell/shell.ts`): the starter, as installed.
const DEFAULT_SHELL: &str = "share/gnoblin/default-shell/shell.ts";

/// The starter in a source tree: the file `cargo xtask dist` installs as the packaged default shell.
const SOURCE_STARTER: &str = "shells/default/shell.ts";

/// The package every shell needs: the machinery, with no desktop policy. Its entry is `gnoblin-core`.
pub(crate) const CORE_PACKAGE: &str = "gnoblin-core";

/// The optional package of desktop policy, which imports `gnoblin-core` and is never imported by it.
pub(crate) const STD_PACKAGE: &str = "gnoblin-std";

/// The packages of the library, core first.
pub const PACKAGES: [&str; 2] = [CORE_PACKAGE, STD_PACKAGE];

/// The import name the engine runs the starter under when a shell has no user entry (or its entry could not load), if it was given one.
pub(crate) const STARTER_MODULE_NAME: &str = "gnoblin-std/starter";

/// The import name of the object model of the `gnoblin` global (`gnoblin.clients`, `Client`, the listeners): the
/// stock module `data/js/gnoblin-core/host.ts`, which the shell imports to build a generation and which the
/// `gnoblin-core` entry re-exports. `gnoblin-std/host` is what adds the desktop policy to it, and a script that
/// imports `gnoblin-std` is what loads that.
pub(crate) const HOST_MODULE_NAME: &str = "gnoblin-core/host";

/// What a package entry has beyond the stock index: the exports of the package's host module, so one import line
/// reaches `Client` and `ClientListener` alike (`Workspace` and `startDecorations` for `gnoblin-std`). The stock
/// sources stay as they are because a host module is only evaluated when something imports it. The shell appends this to the
/// index it embeds and [`editor_library_in`] appends it to the index an editor reads, so the two cannot disagree.
pub(crate) fn host_export(package: &str) -> String {
    format!("\nexport * from \"{package}/host\";\n")
}

/// The directories the stock library is looked for in, in order: `$GNOBLIN_JS_STOCK_DIR` alone when it is set and not empty; else
/// `$XDG_DATA_HOME/gnoblin/js`, each `$XDG_DATA_DIRS` entry's `gnoblin/js`, and the library of the prefix `anchors` sit in
/// (`share/gnoblin/js`, or `data/js` of a source tree) for each of an anchor's four nearest ancestors. An anchor is a file that
/// is installed with the library: the runtime library, or the executable of a tool. A directory may not exist; the list is what a
/// failed import names.
pub fn stock_search_path(env: &dyn Fn(&str) -> Option<String>, anchors: &[PathBuf]) -> Vec<PathBuf> {
    if let Some(dir) = env(STOCK_DIR_VARIABLE).filter(|dir| !dir.is_empty()) {
        return vec![PathBuf::from(dir)];
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in data_directories(env) {
        dirs.push(base.join("gnoblin/js"));
    }
    for anchor in anchors {
        for ancestor in anchor.ancestors().skip(1).take(4) {
            dirs.push(ancestor.join(INSTALLED));
            dirs.push(ancestor.join(SOURCE_TREE));
        }
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// The data directories of the XDG rule, user first: a relative or empty value is ignored, and the defaults are `$HOME/.local/share`
/// and `/usr/local/share:/usr/share`.
fn data_directories(env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let absolute = |text: String| Some(PathBuf::from(text)).filter(|path| path.is_absolute());
    let mut dirs = Vec::new();
    match env("XDG_DATA_HOME").and_then(absolute) {
        Some(home) => dirs.push(home),
        None => dirs.extend(
            env("HOME")
                .and_then(absolute)
                .map(|home| home.join(".local").join("share")),
        ),
    }
    let system = env("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    dirs.extend(system.split(':').filter_map(|dir| absolute(dir.to_owned())));
    dirs
}

/// The packaged default shell (`<prefix>/share/gnoblin/default-shell/shell.ts`, which is `shells/default/shell.ts` of a source
/// tree) the way [`stock_search_path`] finds the library: through the prefix `anchors` sit in.
pub fn default_shell_path(anchors: &[PathBuf]) -> Option<PathBuf> {
    anchors
        .iter()
        .flat_map(|anchor| anchor.ancestors().skip(1).take(4))
        .flat_map(|ancestor| [ancestor.join(DEFAULT_SHELL), ancestor.join(SOURCE_STARTER)])
        .find(|path| path.is_file())
}

/// The starter of the source tree that holds `stock_dir` (`<root>/data/js`): `<root>/shells/default/shell.ts`. An installed
/// library directory is not laid out as `data/js`, so it has none.
pub fn source_starter(stock_dir: &Path) -> Option<PathBuf> {
    if !stock_dir.ends_with(SOURCE_TREE) {
        return None;
    }
    let root = stock_dir.ancestors().nth(Path::new(SOURCE_TREE).components().count())?;
    Some(root.join(SOURCE_STARTER)).filter(|path| path.is_file())
}

/// A stock library that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StockError {
    /// A package's directory is there, but a file of it could not be read.
    #[error("cannot read the stock library file {path}: {reason}")]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        reason: String,
    },
}

/// The import name a script imports `file` (a path under the library root, with `/` separators) by.
fn import_name(file: &str) -> String {
    let stem = file
        .strip_suffix(".ts")
        .or_else(|| file.strip_suffix(".js"))
        .unwrap_or(file);
    stem.strip_suffix("/index").unwrap_or(stem).to_owned()
}

/// Every `.ts` file under `dir`, sorted, as paths relative to `root` with `/` separators.
fn collect(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), StockError> {
    let unreadable = |path: &Path, error: std::io::Error| StockError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    };
    let entries = fs::read_dir(dir).map_err(|error| unreadable(dir, error))?;
    for entry in entries {
        let path = entry.map_err(|error| unreadable(dir, error))?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "ts")
            && let Ok(relative) = path.strip_prefix(root)
        {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// The first of `dirs` that holds `package` (its `index.ts` is there), if any.
fn package_root<'a>(dirs: &'a [PathBuf], package: &str) -> Option<&'a Path> {
    dirs.iter()
        .map(PathBuf::as_path)
        .find(|dir| dir.join(package).join("index.ts").is_file())
}

// `GENERATED`: the modules `gnoblin-sdkgen` rendered from the schema and the ABI crates when this crate was built, by path under the
// library root (`gnoblin-core/generated/abi.ts`). `build.rs` writes the table; nothing of it is committed.
include!(concat!(env!("OUT_DIR"), "/stock_embedded.rs"));

/// Where an embedded module reports its source from: it has no file, so the path is a label under this directory.
const EMBEDDED_ROOT: &str = "<generated>";

/// One module of the stock library: a file under a library root, or a generated module the runtime was built with.
struct Listed {
    /// The file, or for an embedded module a label under [`EMBEDDED_ROOT`]: what a stack frame and a diagnostic name.
    path: PathBuf,
    /// The path under the library root, with `/` separators.
    file: String,
    /// The import name.
    name: String,
    /// The text of an embedded module; `None` for a file to read.
    embedded: Option<&'static str>,
}

impl Listed {
    /// Whether the module is declarations only (`objects.d.ts`): an editor reads it, the engine never runs it.
    fn is_declarations(&self) -> bool {
        self.file.ends_with(".d.ts")
    }

    /// Whether the module is JavaScript the generator wrote (`objects.js`): the engine runs it, and an editor reads its declarations.
    fn is_javascript(&self) -> bool {
        self.file.ends_with(".js")
    }
}

impl Listed {
    /// The TypeScript of the module.
    fn read(&self) -> Result<String, StockError> {
        match self.embedded {
            Some(source) => Ok(source.to_owned()),
            None => fs::read_to_string(&self.path).map_err(|error| StockError::Unreadable {
                path: self.path.clone(),
                reason: error.to_string(),
            }),
        }
    }
}

/// The stock library as the modules of each package that `dirs` hold, in path order: the files of the package's directory and the
/// generated modules of the package, which this runtime carries. A generated module wins over a file of the same path (an install
/// that predates it), so what runs is always what this build generated. A package no directory holds is absent, which is how a
/// prefix with no std library reads.
fn listing(dirs: &[PathBuf]) -> Result<Vec<Listed>, StockError> {
    let mut found = Vec::new();
    for package in PACKAGES {
        let Some(root) = package_root(dirs, package) else {
            continue;
        };
        let mut files = Vec::new();
        collect(root, &root.join(package), &mut files)?;
        let prefix = format!("{package}/");
        let embedded: Vec<(&'static str, &'static str)> = GENERATED
            .iter()
            .copied()
            .filter(|(file, _)| file.starts_with(&prefix))
            .collect();
        files.retain(|file| !embedded.iter().any(|(generated, _)| generated == file));
        let mut modules: Vec<Listed> = files
            .into_iter()
            .map(|file| Listed {
                path: root.join(&file),
                name: import_name(&file),
                file,
                embedded: None,
            })
            .collect();
        modules.extend(embedded.into_iter().map(|(file, source)| Listed {
            path: Path::new(EMBEDDED_ROOT).join(file),
            file: file.to_owned(),
            name: import_name(file),
            embedded: Some(source),
        }));
        modules.sort_by(|a, b| a.file.cmp(&b.file));
        found.extend(modules);
    }
    Ok(found)
}

/// The stock modules by import name as TypeScript, which is how the shell embeds them
/// ([`JsConfig::embed_typescript`](crate::JsConfig)): nothing is stripped until a module is loaded, and a module the bytecode cache
/// holds is never stripped at all. A package that none of `dirs` holds is not in the map.
///
/// # Errors
///
/// A file of a package that is there cannot be read.
pub fn stock_sources_in(dirs: &[PathBuf]) -> Result<HashMap<String, JsTypeScriptModule>, StockError> {
    let mut modules = HashMap::new();
    for listed in listing(dirs)?.into_iter().filter(|listed| !listed.is_declarations()) {
        let source = listed.read()?;
        modules.insert(
            listed.name,
            JsTypeScriptModule {
                path: listed.path,
                source: Cow::Owned(source),
            },
        );
    }
    Ok(modules)
}

/// The stock library as an editor and a type checker read it: `(path under the library root, TypeScript)` for every module in
/// path order, so each starts with its package directory (`gnoblin-std/scene/node.ts`). These are the real sources, byte for byte,
/// except the package entries (`gnoblin-core/index.ts`, `gnoblin-std/index.ts`), which are the modules the runtime builds (the
/// stock index plus [`host_export`]). The generated modules are the ones this runtime was built with. `gnoblinctl types init` writes
/// them next to the user's config, so `import ... from "gnoblin-std/scene/node"` opens the file the shell runs.
///
/// # Errors
///
/// A file of a package that is there cannot be read.
pub fn editor_library_in(dirs: &[PathBuf]) -> Result<Vec<(String, String)>, StockError> {
    let mut library = Vec::new();
    for listed in listing(dirs)?.into_iter().filter(|listed| !listed.is_javascript()) {
        let mut text = listed.read()?;
        if PACKAGES.contains(&listed.name.as_str()) {
            text.push_str(&host_export(&listed.name));
        }
        library.push((listed.file, text));
    }
    Ok(library)
}

/// The stock modules by import name, as JavaScript, ready to embed in a [`JsConfig`](crate::JsConfig). A file that does not strip
/// becomes a module that throws the reason when imported.
///
/// # Errors
///
/// A file of a package that is there cannot be read.
pub fn stock_modules_in(dirs: &[PathBuf]) -> Result<HashMap<String, String>, StockError> {
    Ok(stock_sources_in(dirs)?
        .into_iter()
        .map(|(name, module)| {
            let javascript = javascript_of(&name, &module.path, &module.source);
            (name, javascript)
        })
        .collect())
}

/// The starter as the embedded TypeScript module [`STARTER_MODULE_NAME`], read from `path`.
pub(crate) fn starter_module(path: &Path) -> Result<JsTypeScriptModule, StockError> {
    let source = fs::read_to_string(path).map_err(|error| StockError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    Ok(JsTypeScriptModule {
        path: path.to_path_buf(),
        source: Cow::Owned(source),
    })
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

/// The module the engine imports into a generation on the native library to read the document the core's input manager declares (the
/// device settings the script wrote). It installs `__gnoblinDocument` and nothing else.
///
/// It keeps the input manager it reads. A frame reads the revision, and a manager made for each read is a script object that is dropped
/// as soon as the read ends, so the engine queues the cleanup of its reference as a promise job; a job left at the end of a frame is a
/// frame due at once (`FrameGate::plan_next_frame`), and that frame read the revision again, so a shell that held the document ran a
/// frame on every turn.
pub(crate) const DOCUMENT_MODULE: &str = "gnoblin-core/generated/document";

const DOCUMENT_SOURCE: &str = "import { InputManager } from \"gnoblin-core/generated/objects\";\nlet manager;\nglobalThis.__gnoblinDocument = {\n  revision: () => (manager ??= InputManager.get()).documentRevision,\n  text: () => (manager ??= InputManager.get()).document,\n};\n";

/// The package entry `gnoblin-core` on the native library: the core's object classes, and the global `gnoblin` a shell reads without an
/// import. A generation on the native library imports it before the shell, so the global is always there.
pub(crate) const CORE_ENTRY_MODULE: &str = "gnoblin-core";

const CORE_ENTRY_SOURCE: &str = "import { gnoblin } from \"gnoblin-core/generated/objects\";\nexport * from \"gnoblin-core/generated/objects\";\nglobalThis.gnoblin = gnoblin;\n";

/// The package entry `gnoblin-std` on the native library: the std's object classes, which also put the std members on the core's root
/// (`gnoblin.workspaces`) when the module evaluates.
const STD_ENTRY_MODULE: &str = "gnoblin-std";

const STD_ENTRY_SOURCE: &str = "import \"gnoblin-core\";\nexport * from \"gnoblin-std/generated/objects\";\n";

/// The modules of the native library by import name: the object classes the generator rendered when this crate was built, the
/// package entries over them, and the engine's own module that reads the document, as the embedded TypeScript the loader strips
/// (they are plain JavaScript, which strips to itself).
fn native_modules() -> HashMap<String, JsTypeScriptModule> {
    let module = |name: &str, source: &'static str| {
        (
            name.to_owned(),
            JsTypeScriptModule {
                path: Path::new(EMBEDDED_ROOT).join(format!("native/{name}.js")),
                source: Cow::Borrowed(source),
            },
        )
    };
    NATIVE
        .iter()
        .map(|(name, source)| module(name, source))
        .chain([
            module(DOCUMENT_MODULE, DOCUMENT_SOURCE),
            module(CORE_ENTRY_MODULE, CORE_ENTRY_SOURCE),
            module(STD_ENTRY_MODULE, STD_ENTRY_SOURCE),
        ])
        .collect()
}

/// The stock library of a shell: both of the libraries a generation may run on (see [`StockRuntime`]), each built when the first
/// generation that needs it asks, and which of them the running generation uses.
///
/// The TypeScript library is read from `dirs` and its package entries get [`host_export`]; the native one is the table the build
/// embedded. The starter (the shell that runs when there is no entry) is part of both, under [`STARTER_MODULE_NAME`].
#[derive(Debug)]
pub struct StockLibrary {
    dirs: Vec<PathBuf>,
    starter: Option<JsTypeScriptModule>,
    typescript: OnceLock<Result<Arc<HashMap<String, JsTypeScriptModule>>, StockError>>,
    generated: OnceLock<Arc<HashMap<String, JsTypeScriptModule>>>,
    serving: AtomicU8,
}

/// What [`StockLibrary::serving`] holds before any generation that runs a user's code is installed.
const SERVING_NONE: u8 = 0;
const SERVING_GENERATED: u8 = 1;
const SERVING_TYPESCRIPT: u8 = 2;

impl StockLibrary {
    /// A library whose TypeScript half is read from `dirs` and whose starter is `starter`.
    pub fn new(dirs: Vec<PathBuf>, starter: Option<JsTypeScriptModule>) -> Self {
        Self {
            dirs,
            starter,
            typescript: OnceLock::new(),
            generated: OnceLock::new(),
            serving: AtomicU8::new(SERVING_NONE),
        }
    }

    /// The directories the TypeScript library is read from.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// The modules of `runtime` by import name.
    ///
    /// # Errors
    ///
    /// The TypeScript library has a file that cannot be read.
    pub fn modules(&self, runtime: StockRuntime) -> Result<Arc<HashMap<String, JsTypeScriptModule>>, StockError> {
        match runtime {
            StockRuntime::Generated => Ok(Arc::clone(self.generated.get_or_init(|| {
                let mut modules = native_modules();
                if let Some(starter) = &self.starter {
                    modules.insert(STARTER_MODULE_NAME.to_owned(), starter.clone());
                }
                Arc::new(modules)
            }))),
            StockRuntime::Typescript => self
                .typescript
                .get_or_init(|| {
                    let mut stock = stock_sources_in(&self.dirs)?;
                    for package in PACKAGES {
                        if let Some(index) = stock.get_mut(package) {
                            index.source.to_mut().push_str(&host_export(package));
                        }
                    }
                    // The explicit object paths (`gnoblin-std/generated/objects`) are served on this library too, as they were before
                    // the native one existed: a shell that mixes them with the package entries is on this library, and runs as it ran.
                    for (name, module) in native_modules() {
                        if name.ends_with("generated/objects") || name.ends_with("generated/object_base") {
                            stock.entry(name).or_insert(module);
                        }
                    }
                    if let Some(starter) = &self.starter {
                        stock.insert(STARTER_MODULE_NAME.to_owned(), starter.clone());
                    }
                    Ok(Arc::new(stock))
                })
                .clone(),
        }
    }

    /// The shell that runs when there is no entry, if the shell has one.
    pub(crate) fn starter(&self) -> Option<&JsTypeScriptModule> {
        self.starter.as_ref()
    }

    /// The library the generation that runs the user's code uses, once there is one. A reload that would change it is refused.
    pub fn serving(&self) -> Option<StockRuntime> {
        match self.serving.load(Ordering::Acquire) {
            SERVING_GENERATED => Some(StockRuntime::Generated),
            SERVING_TYPESCRIPT => Some(StockRuntime::Typescript),
            _ => None,
        }
    }

    /// Records the library of the generation that now runs the user's code.
    pub(crate) fn note_serving(&self, runtime: StockRuntime) {
        let value = match runtime {
            StockRuntime::Generated => SERVING_GENERATED,
            StockRuntime::Typescript => SERVING_TYPESCRIPT,
        };
        self.serving.store(value, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The library of the source tree, which is what a prefix's `share/gnoblin/js` is made from.
    fn source_tree() -> Vec<PathBuf> {
        vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js")]
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn every_stock_module_strips_to_javascript() {
        let modules = stock_modules_in(&source_tree()).expect("the source tree's library reads");
        assert!(modules.len() > 100, "the library is read: {}", modules.len());
        for (name, source) in &modules {
            assert!(!source.starts_with("throw new Error"), "{name}: {source}");
        }
    }

    /// The modules that import one another at run time (the stripped JavaScript has no type-only imports) form a
    /// tree of layers, never a loop: a loop makes the order modules evaluate in depend on which one was imported first.
    #[test]
    fn stock_modules_do_not_import_each_other_in_a_cycle() {
        let modules = stock_modules_in(&source_tree()).expect("the library reads");
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
        let sources = stock_sources_in(&source_tree()).expect("the library reads");
        let names: BTreeSet<&str> = sources.keys().map(String::as_str).collect();
        assert!(names.contains("gnoblin-core"), "the core index.ts is the package entry");
        assert!(names.contains("gnoblin-std"), "the std index.ts is the package entry");
        assert!(names.contains("gnoblin-std/layouts/arc"));
        assert!(names.contains("gnoblin-core/host"));
        assert!(names.contains("gnoblin-std/host"));
        // A module that sits beside a directory of the same name is that directory's one import path.
        for barrel in [
            "gnoblin-std/scene",
            "gnoblin-std/style",
            "gnoblin-core/input",
            "gnoblin-std/ui",
        ] {
            assert!(names.contains(barrel), "{barrel}");
        }
        // The `index.ts` of a subdirectory is that directory's import name, so the guarded XR entry keeps its path.
        assert!(names.contains("gnoblin-std/extensions/xr"));
        assert!(!names.contains("gnoblin-std/extensions/xr/index"));
        assert!(
            !names.contains("gnoblin-core/index"),
            "the package entry is the package name"
        );
        // The bare `gnoblin` specifier is gone: a script names the package it wants.
        assert!(
            !names
                .iter()
                .any(|name| *name == "gnoblin" || name.starts_with("gnoblin/"))
        );
    }

    /// The editor reads the sources the shell reads: the same files, the same bytes, and for a package entry the same text the shell
    /// builds, so a type the editor shows is the type the runtime has.
    #[test]
    fn the_editor_library_is_the_installed_sources() {
        let dirs = source_tree();
        let sources = stock_sources_in(&dirs).expect("the library reads");
        let library = editor_library_in(&dirs).expect("the library reads");
        // The generated classes of the object protocol are declarations for the editor here; their JavaScript is the native library's
        // (`NATIVE`), which is not part of the TypeScript library.
        let (declarations, library): (Vec<_>, Vec<_>) =
            library.into_iter().partition(|(file, _)| file.ends_with(".d.ts"));
        assert_eq!(declarations.len(), 3, "object_base and the two objects declarations");
        assert!(
            sources
                .values()
                .all(|module| module.path.extension().is_none_or(|extension| extension != "js"))
        );
        assert!(library.iter().all(|(file, _)| !file.ends_with(".js")));
        assert!(sources.keys().all(|name| !name.ends_with(".d")));
        assert_eq!(library.len(), sources.len());
        for (file, text) in &library {
            let source = &sources[&import_name(file)].source;
            if PACKAGES.contains(&import_name(file).as_str()) {
                assert_eq!(*text, format!("{source}{}", host_export(&import_name(file))));
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

    /// `gnoblin-std` depends on `gnoblin-core`, never the reverse: a shell that only needs the machinery runs on
    /// `gnoblin-core` and loads no std module, so no core module may import one. What std adds to the object model it
    /// adds itself, when it is imported (`data/js/gnoblin-core/host/hooks.ts` and `data/js/gnoblin-std/host/install.ts`).
    #[test]
    fn no_core_module_imports_a_std_module() {
        let mut found: Vec<(String, String)> = Vec::new();
        for (name, module) in stock_sources_in(&source_tree()).expect("the library reads") {
            if !(name == CORE_PACKAGE || name.starts_with("gnoblin-core/")) {
                continue;
            }
            for target in specifiers(&module.source) {
                if target == STD_PACKAGE || target.starts_with("gnoblin-std/") {
                    found.push((name.clone(), target));
                }
            }
        }
        found.sort();
        found.dedup();
        assert!(found.is_empty(), "core modules import std modules: {found:#?}");
    }

    #[test]
    fn the_search_path_is_the_override_alone_or_the_data_directories_then_the_prefix() {
        let anchor = PathBuf::from("/opt/gnoblin/lib64/gnoblin/libgnoblin-runtime-js.so");
        let only = stock_search_path(
            &env_of(&[(STOCK_DIR_VARIABLE, "/srv/stock")]),
            std::slice::from_ref(&anchor),
        );
        assert_eq!(only, [PathBuf::from("/srv/stock")], "the override is exclusive");
        let ignored = stock_search_path(&env_of(&[(STOCK_DIR_VARIABLE, "")]), &[]);
        assert!(!ignored.is_empty(), "an empty override is no override");

        let dirs = stock_search_path(
            &env_of(&[
                ("XDG_DATA_HOME", "/home/k/.local/share"),
                ("XDG_DATA_DIRS", "/a:rel:/b"),
            ]),
            &[anchor],
        );
        assert_eq!(dirs[0], Path::new("/home/k/.local/share/gnoblin/js"));
        assert_eq!(dirs[1], Path::new("/a/gnoblin/js"));
        assert_eq!(dirs[2], Path::new("/b/gnoblin/js"), "a relative entry is ignored");
        assert!(
            dirs.contains(&PathBuf::from("/opt/gnoblin/share/gnoblin/js")),
            "the prefix of the anchor is searched: {dirs:?}"
        );
    }

    #[test]
    fn a_package_no_directory_holds_is_absent() {
        let scratch = tempfile::tempdir().expect("a directory");
        let core = scratch.path().join("gnoblin-core");
        fs::create_dir_all(&core).expect("the package directory");
        fs::write(core.join("index.ts"), "export const x = 1;\n").expect("an index");
        let sources = stock_sources_in(&[scratch.path().to_path_buf()]).expect("a core-only library reads");
        let names: BTreeSet<&str> = sources.keys().map(String::as_str).collect();
        assert!(names.contains("gnoblin-core"));
        assert!(
            names.iter().all(|name| !name.starts_with("gnoblin-std")),
            "a prefix without the std package serves no std module, generated or not: {names:?}"
        );
        assert!(
            names.contains("gnoblin-core/generated/abi"),
            "the generated modules of a present package come with the runtime, not with the prefix"
        );
    }

    /// The generated modules are the runtime's own: a file of the same path in the library directory (an install that predates the
    /// runtime) does not replace one.
    #[test]
    fn a_generated_module_wins_over_an_installed_file_of_its_path() {
        let scratch = tempfile::tempdir().expect("a directory");
        let generated = scratch.path().join("gnoblin-core/generated");
        fs::create_dir_all(&generated).expect("the package directory");
        fs::write(scratch.path().join("gnoblin-core/index.ts"), "export const x = 1;\n").expect("an index");
        fs::write(generated.join("abi.ts"), "export const OLD_INSTALL_MARKER = true;\n")
            .expect("a stale generated file");
        let sources = stock_sources_in(&[scratch.path().to_path_buf()]).expect("the library reads");
        let abi = &sources["gnoblin-core/generated/abi"];
        assert!(!abi.source.contains("OLD_INSTALL_MARKER"), "{}", abi.source);
        assert!(abi.path.starts_with(EMBEDDED_ROOT), "{}", abi.path.display());
    }

    /// Every module the generator renders for the TypeScript target is embedded, under the path the hand-written modules import it by.
    #[test]
    fn the_generated_modules_of_both_packages_are_embedded() {
        let files: BTreeSet<&str> = GENERATED.iter().map(|(file, _)| *file).collect();
        for expected in [
            "gnoblin-core/generated/abi.ts",
            "gnoblin-core/generated/schema.ts",
            "gnoblin-core/generated/verbs.ts",
            "gnoblin-std/generated/classes.ts",
            "gnoblin-std/generated/native.ts",
            "gnoblin-core/generated/object_base.d.ts",
            "gnoblin-std/generated/objects.d.ts",
        ] {
            assert!(files.contains(expected), "{expected} is not embedded: {files:?}");
        }
        assert!(
            files.iter().all(|file| !file.ends_with(".js")),
            "the native modules are not part of the TypeScript library: {files:?}"
        );
        let native: BTreeSet<&str> = NATIVE.iter().map(|(name, _)| *name).collect();
        for expected in [
            "gnoblin-core/generated/object_base",
            "gnoblin-core/generated/objects",
            "gnoblin-std/generated/objects",
        ] {
            assert!(
                native.contains(expected),
                "{expected} is not in the native library: {native:?}"
            );
        }
        assert!(
            GENERATED
                .iter()
                .chain(NATIVE.iter())
                .all(|(_, source)| source.starts_with("// GENERATED by gnoblin-sdkgen"))
        );
    }
}
