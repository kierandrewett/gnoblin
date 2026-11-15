//! The module resolver and loader for one generation.
//!
//! Three kinds of specifier resolve:
//!
//! - `./x.ts`, `../x.ts`, `./x.js` and extensionless `./x`: files under the config root. The path is
//!   normalised, then canonicalised, and the result must still be under the canonical root, so neither a
//!   `..` that climbs out nor a symlink that points out can reach a file the author did not put there.
//!   A `./x.js` that does not exist falls back to `./x.ts`, because TypeScript authors write the output
//!   extension in imports.
//! - `gnoblin` and `gnoblin/<module>`: modules the host registered with their JavaScript source.
//! - An absolute path, only for the entry the host passes (an import with no importing module).
//!
//! `.ts` and `.mts` files go through [`strip_types`](crate::ts::strip_types); `.js` and `.mjs` pass through.
//! A module's name inside QuickJS is its canonical path, so a stack frame names the file it came from and
//! `generation` can remap it through the source map kept here.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gnoblin_config::Language;
use oxc_sourcemap::SourceMap;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Error, Module, Result};

use crate::bytecode::{Digest, JsBytecodeCache};
use crate::error::{JsError, JsLoadError, JsLocation};
use crate::ts::{position, strip_types};

/// The source map of a `.ts` module: made when it was stripped, or still to be made from its source.
#[derive(Debug)]
enum MapSlot {
    Ready(SourceMap<'static>),
    /// The module's bytecode came from the cache, so the types were never stripped in this process.
    Pending {
        path: PathBuf,
        text: String,
    },
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

    /// Maps a 1-based `line` and `column` in the stripped JavaScript of module `file` to the 1-based
    /// position in the `.ts` source, or `None` when `file` has no source map (plain JavaScript).
    ///
    /// A module whose bytecode came from the cache was never stripped in this process, so its map is made here, from
    /// the source, the first time a stack names it. The stripper is deterministic, so it is the map the compiled
    /// code was made with.
    pub(crate) fn remap(&self, file: &str, line: u32, column: u32) -> Option<(u32, u32)> {
        let mut maps = self.maps.borrow_mut();
        let slot = maps.get_mut(file)?;
        if let MapSlot::Pending { path, text } = slot {
            match strip_types(path, text).ok().and_then(|stripped| stripped.map) {
                Some(map) => *slot = MapSlot::Ready(map),
                None => {
                    maps.remove(file);
                    return None;
                }
            }
        }
        let MapSlot::Ready(map) = maps.get(file)? else {
            return None;
        };
        let table = map.generate_lookup_table();
        // QuickJS counts from 1, the source map from 0.
        let token = map.lookup_token_approx(&table, line.saturating_sub(1), column.saturating_sub(1))?;
        Some((token.get_src_line() + 1, token.get_src_col() + 1))
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
            } else if name == "gnoblin" || name.starts_with("gnoblin/") {
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
    /// Whether a stack frame in this module is remapped through a source map: a user's `.ts` file. The stock
    /// library has none (its stack frames name the stripped text).
    mapped: bool,
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
                cacheable: false,
            });
        }
        if let Some(module) = self.typescript.get(name) {
            return Ok(ModuleText {
                kind: Language::TypeScript,
                text: Cow::Borrowed(&module.source),
                path: module.path.clone(),
                mapped: false,
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
            cacheable: true,
        })
    }

    /// The JavaScript for `origin`, stripping its types and recording its source map when that is kept.
    fn javascript(&self, name: &str, origin: ModuleText<'_>) -> std::result::Result<String, JsError> {
        match origin.kind {
            Language::JavaScript => Ok(origin.text.into_owned()),
            Language::TypeScript if origin.mapped => {
                let stripped = strip_types(&origin.path, &origin.text)?;
                if let Some(map) = stripped.map {
                    self.state
                        .maps
                        .borrow_mut()
                        .insert(name.to_owned(), MapSlot::Ready(map));
                }
                Ok(stripped.code)
            }
            Language::TypeScript => Ok(crate::stock::javascript_of(name, &origin.path, &origin.text)),
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
        let digest: Option<Digest> = origin
            .cacheable
            .then(|| self.cache.digest(origin.kind, name, &origin.text))
            .flatten();
        if let Some(digest) = &digest
            && let Some(module) = self.cache.declare(ctx, digest, name)
        {
            if origin.mapped {
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

/// Resolves the file import `name` made by module `base` to a canonical path under `root`.
///
/// `root` must be canonical. `base` is the importing module's name: a canonical file path, or empty for
/// an import the host makes with no importing module, in which case `name` may also be an absolute path
/// and relative names count from `root`.
pub(crate) fn resolve_file(root: &Path, base: &str, name: &str) -> std::result::Result<PathBuf, JsLoadError> {
    let target = Path::new(name);
    let joined = if target.is_absolute() {
        if !base.is_empty() {
            return Err(JsLoadError::Unresolvable { name: name.to_owned() });
        }
        target.to_path_buf()
    } else if base.is_empty() {
        root.join(target)
    } else {
        let importer = Path::new(base);
        match importer.parent() {
            Some(dir) if importer.is_absolute() => dir.join(target),
            _ => {
                return Err(JsLoadError::NoFileBase {
                    name: name.to_owned(),
                    base: base.to_owned(),
                });
            }
        }
    };
    let normal = normalise(&joined);
    let escape = |path: PathBuf| JsLoadError::Escape {
        name: name.to_owned(),
        path,
        root: root.to_path_buf(),
    };
    // Lexical check first: a `..` that climbs out is refused without touching the file system.
    if !normal.starts_with(root) {
        return Err(escape(normal));
    }
    let candidates = candidates(&normal);
    for candidate in &candidates {
        // `canonicalize` follows symlinks and fails for a missing file, which moves on to the next
        // candidate. A symlink that points out of the root is an error, not a miss.
        let Ok(canonical) = fs::canonicalize(candidate) else {
            continue;
        };
        if !canonical.starts_with(root) {
            return Err(escape(canonical));
        }
        if !canonical.is_file() {
            return Err(JsLoadError::NotAFile { path: canonical });
        }
        return Ok(canonical);
    }
    Err(JsLoadError::NotFound {
        name: name.to_owned(),
        tried: candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

/// The files that can satisfy `path`, best first.
fn candidates(path: &Path) -> Vec<PathBuf> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("js") => vec![path.to_path_buf(), path.with_extension("ts")],
        Some("mjs") => vec![path.to_path_buf(), path.with_extension("mts")],
        Some(_) => vec![path.to_path_buf()],
        None => ["ts", "js", "mjs"]
            .iter()
            .map(|ext| {
                let mut name = OsString::from(path);
                name.push(".");
                name.push(ext);
                PathBuf::from(name)
            })
            .collect(),
    }
}

/// `path` with `.` and `..` folded away without touching the file system. A `..` at the root is dropped,
/// which cannot make a path leave the root: the caller compares against it afterwards.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{candidates, normalise, resolve_file};
    use crate::error::JsLoadError;

    /// A canonical temp directory with a `root` inside it and a sibling `outside` directory.
    struct Tree {
        _dir: TempDir,
        root: PathBuf,
        outside: PathBuf,
    }

    fn tree() -> Tree {
        let dir = tempfile::tempdir().expect("a temp dir");
        let base = fs::canonicalize(dir.path()).expect("the temp dir exists");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(root.join("sub")).expect("root/sub");
        fs::create_dir_all(&outside).expect("outside");
        Tree {
            _dir: dir,
            root,
            outside,
        }
    }

    fn write(path: &Path, text: &str) {
        fs::write(path, text).expect("the file is written");
    }

    fn base_of(root: &Path, file: &str) -> String {
        root.join(file).to_str().expect("utf-8 temp path").to_owned()
    }

    #[test]
    fn relative_ts_resolves_to_its_canonical_path() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("x.ts"), "");
        let found = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./x.ts").expect("x.ts is under the root");
        assert_eq!(found, t.root.join("x.ts"));
    }

    #[test]
    fn js_specifier_falls_back_to_ts_but_prefers_an_existing_js() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("only_ts.ts"), "");
        write(&t.root.join("both.ts"), "");
        write(&t.root.join("both.js"), "");
        let base = base_of(&t.root, "a.ts");
        assert_eq!(
            resolve_file(&t.root, &base, "./only_ts.js").expect("falls back to .ts"),
            t.root.join("only_ts.ts")
        );
        assert_eq!(
            resolve_file(&t.root, &base, "./both.js").expect("the .js exists"),
            t.root.join("both.js")
        );
    }

    #[test]
    fn extensionless_specifier_tries_ts_then_js() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("m.js"), "");
        let base = base_of(&t.root, "a.ts");
        assert_eq!(resolve_file(&t.root, &base, "./m").expect("m.js"), t.root.join("m.js"));
        write(&t.root.join("m.ts"), "");
        assert_eq!(resolve_file(&t.root, &base, "./m").expect("m.ts"), t.root.join("m.ts"));
    }

    #[test]
    fn parent_that_stays_inside_the_root_is_allowed() {
        let t = tree();
        write(&t.root.join("shared.ts"), "");
        write(&t.root.join("sub/a.ts"), "");
        let found = resolve_file(&t.root, &base_of(&t.root, "sub/a.ts"), "../shared.ts").expect("inside the root");
        assert_eq!(found, t.root.join("shared.ts"));
    }

    #[test]
    fn parent_that_climbs_out_of_the_root_is_rejected() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.outside.join("secret.ts"), "");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "../outside/secret.ts")
            .expect_err("the path leaves the root");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn symlinked_file_pointing_out_of_the_root_is_rejected() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.outside.join("secret.ts"), "");
        symlink(t.outside.join("secret.ts"), t.root.join("link.ts")).expect("a symlink");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./link.ts").expect_err("the link points out");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn symlinked_directory_pointing_out_of_the_root_is_rejected() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.outside.join("secret.ts"), "");
        symlink(&t.outside, t.root.join("dir")).expect("a symlink");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./dir/secret.ts")
            .expect_err("the directory link points out");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn symlink_that_stays_inside_the_root_is_allowed() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("real.ts"), "");
        symlink(t.root.join("real.ts"), t.root.join("link.ts")).expect("a symlink");
        let found = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./link.ts").expect("inside the root");
        assert_eq!(found, t.root.join("real.ts"));
    }

    #[test]
    fn absolute_import_is_rejected_from_a_module_but_allowed_for_the_host_entry() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("x.ts"), "");
        let absolute = t.root.join("x.ts");
        let name = absolute.to_str().expect("utf-8 temp path");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), name)
            .expect_err("a module may not import by absolute path");
        assert!(matches!(error, JsLoadError::Unresolvable { .. }), "{error}");
        assert_eq!(resolve_file(&t.root, "", name).expect("the host entry"), absolute);
        let outside = t.outside.join("x.ts");
        write(&outside, "");
        let error = resolve_file(&t.root, "", outside.to_str().expect("utf-8")).expect_err("outside the root");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn missing_file_names_what_was_tried() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./nope.js").expect_err("no such file");
        let JsLoadError::NotFound { tried, .. } = &error else {
            panic!("expected NotFound, got {error}");
        };
        assert!(tried.contains("nope.js") && tried.contains("nope.ts"), "{tried}");
    }

    #[test]
    fn relative_import_from_an_embedded_module_is_rejected() {
        let t = tree();
        let error = resolve_file(&t.root, "gnoblin", "./x.ts").expect_err("gnoblin is not a file");
        assert!(matches!(error, JsLoadError::NoFileBase { .. }), "{error}");
    }

    #[test]
    fn normalise_folds_dots_and_never_climbs_past_the_root_directory() {
        assert_eq!(normalise(Path::new("/a/./b/../c")), Path::new("/a/c"));
        assert_eq!(normalise(Path::new("/../..")), Path::new("/"));
    }

    #[test]
    fn candidates_follow_the_extension() {
        assert_eq!(
            candidates(Path::new("/r/x.js")),
            [Path::new("/r/x.js"), Path::new("/r/x.ts")]
        );
        assert_eq!(
            candidates(Path::new("/r/x")),
            [Path::new("/r/x.ts"), Path::new("/r/x.js"), Path::new("/r/x.mjs")]
        );
        assert_eq!(candidates(Path::new("/r/x.ts")), [Path::new("/r/x.ts")]);
    }
}
