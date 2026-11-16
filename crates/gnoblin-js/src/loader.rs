//! The module resolver and loader for one generation.
//!
//! Three kinds of specifier resolve:
//!
//! - `./x.ts`, `../x.ts`, `./x.js` and extensionless `./x`: files under the config root. The path is
//!   normalised, then canonicalised, and the result must still be under the canonical root, so neither a
//!   `..` that climbs out nor a symlink that points out can reach a file the author did not put there.
//!   A `./x.js` that does not exist falls back to `./x.ts`, because TypeScript authors write the output
//!   extension in imports.
//! - `gnoblin-core`, `gnoblin-std` and their `<package>/<module>` submodules: modules the host registered with their
//!   JavaScript source.
//! - An absolute path, only for the entry the host passes (an import with no importing module).
//!
//! `.ts` and `.mts` files go through [`strip_types`](crate::ts::strip_types); `.js` and `.mjs` pass through.
//! A module's name inside QuickJS is its canonical path, so a stack frame names the file it came from and
//! `generation` can remap it through the source map kept here.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gnoblin_config::Language;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Error, Module, Result};

use crate::bytecode::{Digest, JsBytecodeCache};
use crate::error::{JsError, JsLoadError, JsLocation};
use crate::stock::PACKAGES;
use crate::ts::{JsStripped, position, strip_shared, strip_types};

mod files;
use files::resolve_file;

/// The source map of a `.ts` module: made when it was stripped, or still to be made from its source.
#[derive(Debug)]
enum MapSlot {
    /// Stripped in this process: `path` is the source file a stack frame names, and `stripped` holds the map.
    Ready { path: PathBuf, stripped: Arc<JsStripped> },
    /// A user's module whose bytecode came from the cache, so the types were never stripped in this process.
    Pending { path: PathBuf, text: String },
    /// A stock module whose bytecode came from the cache: its text is in the table the loader was given.
    PendingStock {
        typescript: Arc<HashMap<String, JsTypeScriptModule>>,
    },
}

/// Where a position in stripped JavaScript is in the author's source.
#[derive(Debug)]
pub(crate) struct Remapped {
    /// The source file.
    pub(crate) file: String,
    /// The 1-based line in it.
    pub(crate) line: u32,
    /// The 1-based column in it.
    pub(crate) column: u32,
}

/// What the resolver and loader of one generation share with it: the source maps of the `.ts` files
/// loaded so far, and the structured error behind the last failed load.
///
/// QuickJS carries a failed load as a `ReferenceError` with only a message, so the typed
/// [`JsError`] is kept here and `generation` swaps it in when the message matches.
#[derive(Debug, Default)]
pub struct JsLoadState {
    maps: RefCell<HashMap<String, MapSlot>>,
    /// Every file the loader read, canonical: the module graph the hot-reload watcher observes.
    files: RefCell<BTreeSet<PathBuf>>,
    /// Every embedded module the loader handed out, by name: the import log of the library.
    embedded: RefCell<BTreeSet<String>>,
    failure: RefCell<Option<JsFailure>>,
}

/// A failed load: the message QuickJS will carry and the typed error behind it.
#[derive(Debug)]
struct JsFailure {
    message: String,
    error: JsError,
}

impl JsLoadState {
    /// An empty state, shared between a resolver, a loader and their generation.
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// The canonical path of every file module loaded so far, sorted.
    pub(crate) fn files(&self) -> Vec<PathBuf> {
        self.files.borrow().iter().cloned().collect()
    }

    /// The name of every embedded module (stock library or host-registered) loaded so far, sorted.
    pub(crate) fn embedded(&self) -> Vec<String> {
        self.embedded.borrow().iter().cloned().collect()
    }

    /// Records `error` as the last load failure and returns the message to hand to QuickJS.
    fn fail(&self, error: JsError) -> String {
        let message = error.to_string();
        *self.failure.borrow_mut() = Some(JsFailure {
            message: message.clone(),
            error,
        });
        message
    }

    /// Takes the recorded failure if the exception text `message` carries its message.
    ///
    /// A script can catch a failed `import()` and throw something else later; matching on the text keeps
    /// that stale failure from replacing the real error.
    pub(crate) fn take_failure(&self, message: &str) -> Option<JsError> {
        let mut slot = self.failure.borrow_mut();
        if slot.as_ref().is_some_and(|f| message.contains(&f.message)) {
            return slot.take().map(|f| f.error);
        }
        None
    }

    /// Forgets any recorded failure; `generation` calls it at the start of every budgeted call.
    pub(crate) fn clear_failure(&self) {
        self.failure.borrow_mut().take();
    }

    /// Maps a 1-based `line` and `column` in the stripped JavaScript of module `file` to the file and 1-based
    /// position in the `.ts` source, or `None` when `file` has no source map (plain JavaScript). The file is the
    /// module's own name for a file module and the path of the stock source for a stock module.
    ///
    /// A module whose bytecode came from the cache was never stripped in this process, so its map is made here, from
    /// the source, the first time a stack names it (a stock module's through the shared strip). The stripper is
    /// deterministic, so it is the map the compiled code was made with.
    pub(crate) fn remap(&self, file: &str, line: u32, column: u32) -> Option<Remapped> {
        let mut maps = self.maps.borrow_mut();
        let made = match maps.get(file)? {
            MapSlot::Ready { .. } => None,
            MapSlot::Pending { path, text } => Some(strip_types(path, text).ok().map(|stripped| MapSlot::Ready {
                path: path.clone(),
                stripped: Arc::new(stripped),
            })),
            MapSlot::PendingStock { typescript } => Some(typescript.get(file).and_then(|module| {
                strip_shared(&module.path, &module.source)
                    .ok()
                    .map(|stripped| MapSlot::Ready {
                        path: module.path.clone(),
                        stripped,
                    })
            })),
        };
        if let Some(made) = made {
            match made {
                Some(slot) => {
                    maps.insert(file.to_owned(), slot);
                }
                None => {
                    maps.remove(file);
                    return None;
                }
            }
        }
        let MapSlot::Ready { path, stripped } = maps.get(file)? else {
            return None;
        };
        let map = stripped.map.as_ref()?;
        let table = map.generate_lookup_table();
        // QuickJS counts from 1, the source map from 0.
        let token = map.lookup_token_approx(&table, line.saturating_sub(1), column.saturating_sub(1))?;
        Some(Remapped {
            file: path.display().to_string(),
            line: token.get_src_line() + 1,
            column: token.get_src_col() + 1,
        })
    }
}

/// A module the host embeds as TypeScript: the stock library. It is stripped and compiled when something imports it
/// and the bytecode cache has no copy, so a start that finds every module cached never runs the stripper.
#[derive(Clone, Debug)]
pub struct JsTypeScriptModule {
    /// Where the text came from, for the stripper's diagnostics.
    pub path: PathBuf,
    /// The TypeScript.
    pub source: Cow<'static, str>,
}

/// The resolver: turns an import specifier and its importer into a module name.
#[derive(Debug)]
pub struct JsResolver {
    root: PathBuf,
    embedded: Arc<HashMap<String, String>>,
    typescript: Arc<HashMap<String, JsTypeScriptModule>>,
    state: Rc<JsLoadState>,
}

impl JsResolver {
    /// A resolver confined to `root`, which must exist; it is canonicalised here so every later
    /// comparison is between canonical paths.
    pub fn new(
        root: &Path,
        embedded: Arc<HashMap<String, String>>,
        typescript: Arc<HashMap<String, JsTypeScriptModule>>,
        state: Rc<JsLoadState>,
    ) -> Result<Self> {
        let root = fs::canonicalize(root)
            .map_err(|error| Error::new_loading_message(root.display().to_string(), error.to_string()))?;
        Ok(Self {
            root,
            embedded,
            typescript,
            state,
        })
    }

    fn resolve_name(&self, base: &str, name: &str) -> std::result::Result<String, JsLoadError> {
        let relative = name.starts_with("./") || name.starts_with("../");
        if !relative && !Path::new(name).is_absolute() {
            return if self.embedded.contains_key(name) || self.typescript.contains_key(name) {
                Ok(name.to_owned())
            } else if PACKAGES.iter().any(|package| {
                name == *package || name.strip_prefix(*package).is_some_and(|rest| rest.starts_with('/'))
            }) {
                Err(JsLoadError::NoEmbedded { name: name.to_owned() })
            } else {
                Err(JsLoadError::Unresolvable { name: name.to_owned() })
            };
        }
        let found = resolve_file(&self.root, base, name)?;
        found
            .to_str()
            .map(str::to_owned)
            .ok_or(JsLoadError::NotUtf8 { path: found })
    }
}

impl Resolver for JsResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        self.resolve_name(base, name).map_err(|error| {
            let message = self.state.fail(at_import(base, name, error));
            Error::new_resolving_message(base, name, message)
        })
    }
}

/// `error` as a [`JsError`] placed at the import that caused it, so a missing module is reported at
/// the line of the `import` that names it rather than with no position.
///
/// The position is found by looking for the quoted specifier in the importer's source on disk. That is
/// the first textual match, so a string that merely looks like the specifier earlier in the file can
/// win; it is only a failure report, and the specifier is in the message either way. When the importer
/// is not a readable file, or the text is not found, the importer is named in the message instead.
fn at_import(base: &str, name: &str, error: JsLoadError) -> JsError {
    let mut error = JsError::from(error);
    let importer = Path::new(base);
    if !importer.is_absolute() {
        return error;
    }
    let site = fs::read_to_string(importer).ok().and_then(|source| {
        let offset = ['"', '\'']
            .iter()
            .filter_map(|quote| source.find(&format!("{quote}{name}{quote}")))
            .min()?;
        Some(position(&source, offset))
    });
    match site {
        Some((line, column)) => {
            error.location = Some(JsLocation {
                file: importer.to_path_buf(),
                line,
                column,
            });
        }
        None => error.message = format!("{} (imported from {base})", error.message),
    }
    error
}

/// What a module is made from, before it is stripped or compiled.
struct ModuleText<'a> {
    kind: Language,
    text: Cow<'a, str>,
    /// The path the stripper reports diagnostics against, and the path of a `.ts` file whose map is kept.
    path: PathBuf,
    /// Whether a stack frame in this module is remapped through a source map: a `.ts` file, the stock library
    /// included (its frames name the stock `.ts` file and line).
    mapped: bool,
    /// Whether the module is stock: its text does not change while the process runs, so the types are stripped once
    /// per process and the result, source map included, is shared by every generation.
    shared: bool,
    /// Whether the bytecode cache may hold this module. Test modules embedded as JavaScript are not worth a file.
    cacheable: bool,
}

/// The loader: reads a resolved module and hands QuickJS its bytecode or its JavaScript.
#[derive(Debug)]
pub struct JsLoader {
    root: PathBuf,
    embedded: Arc<HashMap<String, String>>,
    typescript: Arc<HashMap<String, JsTypeScriptModule>>,
    cache: JsBytecodeCache,
    state: Rc<JsLoadState>,
}

impl JsLoader {
    /// A loader for the same `root` and embedded modules as the [`JsResolver`] it is paired with. Modules the
    /// `cache` holds are loaded from bytecode, and what it lacks is compiled and offered to it.
    pub fn new(
        root: &Path,
        embedded: Arc<HashMap<String, String>>,
        typescript: Arc<HashMap<String, JsTypeScriptModule>>,
        cache: JsBytecodeCache,
        state: Rc<JsLoadState>,
    ) -> Result<Self> {
        let root = fs::canonicalize(root)
            .map_err(|error| Error::new_loading_message(root.display().to_string(), error.to_string()))?;
        Ok(Self {
            root,
            embedded,
            typescript,
            cache,
            state,
        })
    }

    /// The text of module `name` and what it is.
    fn origin(&self, name: &str) -> std::result::Result<ModuleText<'_>, JsError> {
        if let Some(source) = self.embedded.get(name) {
            return Ok(ModuleText {
                kind: Language::JavaScript,
                text: Cow::Borrowed(source),
                path: PathBuf::from(name),
                mapped: false,
                shared: false,
                cacheable: false,
            });
        }
        if let Some(module) = self.typescript.get(name) {
            return Ok(ModuleText {
                kind: Language::TypeScript,
                text: Cow::Borrowed(&module.source),
                path: module.path.clone(),
                mapped: true,
                shared: true,
                cacheable: true,
            });
        }
        let path = Path::new(name);
        // The resolver already confined the name; checking again keeps a loader that is handed a name by
        // another route from reading outside the root.
        let canonical = fs::canonicalize(path).map_err(|error| read_error(path, &error))?;
        if !canonical.starts_with(&self.root) {
            return Err(JsLoadError::Escape {
                name: name.to_owned(),
                path: canonical,
                root: self.root.clone(),
            }
            .into());
        }
        // Recorded before the read, so a file that fails to parse is still in the graph the watcher
        // observes: fixing it must trigger a reload.
        self.state.files.borrow_mut().insert(canonical.clone());
        let text = fs::read_to_string(&canonical).map_err(|error| read_error(&canonical, &error))?;
        let kind = match canonical.extension().and_then(|ext| ext.to_str()) {
            Some("ts" | "mts") => Language::TypeScript,
            Some("js" | "mjs") => Language::JavaScript,
            _ => return Err(JsLoadError::Extension { path: canonical }.into()),
        };
        Ok(ModuleText {
            kind,
            text: Cow::Owned(text),
            path: canonical,
            mapped: kind == Language::TypeScript,
            shared: false,
            cacheable: true,
        })
    }

    /// The JavaScript for `origin`, stripping its types and recording its source map when that is kept.
    fn javascript(&self, name: &str, origin: ModuleText<'_>) -> std::result::Result<String, JsError> {
        match origin.kind {
            Language::JavaScript => Ok(origin.text.into_owned()),
            Language::TypeScript if origin.shared => match strip_shared(&origin.path, &origin.text) {
                Ok(stripped) => {
                    let code = stripped.code.clone();
                    self.state.maps.borrow_mut().insert(
                        name.to_owned(),
                        MapSlot::Ready {
                            path: origin.path,
                            stripped,
                        },
                    );
                    Ok(code)
                }
                Err(error) => Ok(crate::stock::failing_module(name, &error)),
            },
            Language::TypeScript => {
                let mut stripped = strip_types(&origin.path, &origin.text)?;
                let code = std::mem::take(&mut stripped.code);
                self.state.maps.borrow_mut().insert(
                    name.to_owned(),
                    MapSlot::Ready {
                        path: origin.path,
                        stripped: Arc::new(stripped),
                    },
                );
                Ok(code)
            }
        }
    }

    /// Reports `error` to QuickJS as the load's failure, keeping the typed error for `generation`.
    fn refuse(&self, name: &str, error: JsError) -> Error {
        let message = self.state.fail(error);
        Error::new_loading_message(name, message)
    }
}

impl Loader for JsLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js>> {
        let origin = self.origin(name).map_err(|error| self.refuse(name, error))?;
        if self.embedded.contains_key(name) || self.typescript.contains_key(name) {
            self.state.embedded.borrow_mut().insert(name.to_owned());
        }
        let digest: Option<Digest> = origin
            .cacheable
            .then(|| self.cache.digest(origin.kind, name, &origin.text))
            .flatten();
        if let Some(digest) = &digest
            && let Some(module) = self.cache.declare(ctx, digest, name)
        {
            if origin.shared {
                self.state.maps.borrow_mut().insert(
                    name.to_owned(),
                    MapSlot::PendingStock {
                        typescript: Arc::clone(&self.typescript),
                    },
                );
            } else if origin.mapped {
                self.state.maps.borrow_mut().insert(
                    name.to_owned(),
                    MapSlot::Pending {
                        path: origin.path,
                        text: origin.text.into_owned(),
                    },
                );
            }
            return Ok(module);
        }
        let code = self
            .javascript(name, origin)
            .map_err(|error| self.refuse(name, error))?;
        let module = Module::declare(ctx.clone(), name, code)?;
        if let Some(digest) = &digest {
            self.cache.store(digest, name, &module);
        }
        Ok(module)
    }
}

fn read_error(path: &Path, error: &std::io::Error) -> JsError {
    JsLoadError::Read {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
    .into()
}
