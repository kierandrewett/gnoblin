//! `check` for a TypeScript or JavaScript module, and `types init`.
//!
//! `check FILE` loads the module graph with the runtime's own resolver, loader and TypeScript
//! stripper in a throwaway generation, so a syntax error, an import that does not resolve, a
//! cycle that fails and a top-level throw are reported at the line the author wrote, in `.ts`
//! coordinates. It needs no session and no Node. `--types` then runs an external type
//! checker when one is installed; none is embedded (design/js-research/ts-in-quickjs.md,
//! section 6), and a machine without one skips that step with a plain line and exit 0.
//!
//! The throwaway generation has no host: the compositor's objects do not exist while checking.
//! A module that imports `gnoblin` or `gnoblin/<module>`, or reads the global `gnoblin`, would
//! fail on that alone, so those names are bound to an inert stub that accepts any use. That keeps
//! the check to the author's own code. What the stub cannot catch (a wrong member of `gnoblin`)
//! is what `--types` is for.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gnoblin_cli_style::{
    ColorChoice, ErrorReport, LineOutcome, OutputEnvironment, OutputTheme, error_block, status_line,
};
use gnoblin_config::SearchPaths;
use gnoblin_js::rquickjs::Value;
use gnoblin_js::{JsConfig, JsError, JsErrorKind, JsGeneration, STARTER_CONFIG};
use gnoblin_js_types::{TSCONFIG_FILE, TYPES_FILE, ensure_user_files};
use serde_json::{Value as Json, json};

/// The extensions the shell runtime loads as modules, and so the ones `check` sends down the
/// JavaScript path. Anything else keeps the configuration check.
const MODULE_EXTENSIONS: &[&str] = &["ts", "mts", "js", "mjs"];

/// The wall-clock budget for one evaluation. A configuration's top level only declares things, so
/// a second is far more than it needs, and a loop that never ends is reported quickly.
const CHECK_BUDGET: Duration = Duration::from_secs(1);

/// The heap limit for a check, in bytes. Smaller than the runtime's: nothing here is a real session.
const CHECK_MEMORY: usize = 32 << 20;

/// How many host names one check will stub before it gives up. Each round learns one name, so this
/// bounds a module that imports a very long list from `gnoblin`.
const MAX_STUB_ROUNDS: usize = 512;

/// The tools `--types` looks for on `PATH`, best first.
const TSC_NAMES: &[&str] = &["tsc", "tsc6", "tsgo"];

/// What `--types` prints when no type checker is installed.
const NO_TSC: &str =
    "type check skipped: no TypeScript compiler found; install one with `npm i -g typescript` or set GNOBLIN_TSC";

/// An inert value that accepts any property read, call or construction and returns itself, so the
/// author's top-level code can use `gnoblin` without a host. `then` and symbols read as undefined so
/// awaiting or spreading it does not hang or throw.
const STUB: &str = "const stub = new Proxy(function () {}, {\n\
    get: (_, key) => key === 'then' ? undefined : key === Symbol.toPrimitive ? () => '' : typeof key === 'symbol' ? undefined : stub,\n\
    apply: () => stub,\n\
    construct: () => stub,\n\
});\n";

/// Whether `check` runs `path` through the JavaScript runtime rather than the configuration check.
pub(crate) fn is_module(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| MODULE_EXTENSIONS.contains(&extension))
}

/// `check FILE` for a module: the module graph, then the types when asked.
pub(crate) fn check(file: &Path, types: bool, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let theme = theme(io::stderr().is_terminal());
    let graph = load_graph(file);
    let checked = if types {
        Some(type_check(file, &TscSearch::from_environment(file)))
    } else {
        None
    };
    let code = u8::from(graph.is_err() || checked.as_ref().is_some_and(|checked| !checked.passed()));
    let written = if json {
        write_json(file, &graph, checked.as_ref(), out)
    } else {
        write_text(file, &graph, checked.as_ref(), out, err, &theme)
    };
    if written.is_err() {
        return 1;
    }
    code
}

/// The theme for a stream, from the environment.
pub(crate) fn theme(is_terminal: bool) -> OutputTheme {
    OutputTheme::resolve(ColorChoice::Auto, &OutputEnvironment::detect(is_terminal))
}

/// Load `file` and everything it imports in a throwaway generation. Returns how many files the graph
/// holds, or the first problem.
fn load_graph(file: &Path) -> Result<usize, JsError> {
    let unreadable = |reason: String| JsError {
        kind: JsErrorKind::Load,
        message: format!("cannot read {}: {reason}", file.display()),
        location: None,
        stack: None,
    };
    let entry = std::fs::canonicalize(file).map_err(|error| unreadable(error.to_string()))?;
    let root = entry
        .parent()
        .ok_or_else(|| unreadable("it has no directory".to_owned()))?;
    // Names the author imports from the host, learned one round at a time: see `host_gap`.
    let mut stubs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for _ in 0..MAX_STUB_ROUNDS {
        let mut config = JsConfig::new(root);
        config.stage_budget = CHECK_BUDGET;
        config.memory_limit = CHECK_MEMORY;
        for (module, names) in &stubs {
            config = config.embed(module.as_str(), stub_module(names));
        }
        let generation = JsGeneration::new(&config, 1)?;
        generation.call_budgeted(CHECK_BUDGET, |ctx| {
            ctx.eval::<Value<'_>, _>(format!("{STUB}globalThis.gnoblin = stub;"))
                .map(drop)
        })?;
        match generation.eval_module(&entry) {
            Ok(()) => return Ok(generation.module_paths().len()),
            Err(error) => match host_gap(&error) {
                Some(HostGap::Module(module)) => {
                    stubs.entry(module).or_default();
                }
                Some(HostGap::Export { module, name }) => {
                    stubs.entry(module).or_default().insert(name);
                }
                None => return Err(error),
            },
        }
    }
    Err(JsError {
        kind: JsErrorKind::Load,
        message: format!(
            "{} imports more than {MAX_STUB_ROUNDS} names from the shell modules",
            file.display()
        ),
        location: None,
        stack: None,
    })
}

/// The wall-clock budget for loading a converted config with the stock library: the library is parsed first, so this
/// is longer than [`CHECK_BUDGET`].
const VALIDATE_BUDGET: Duration = Duration::from_secs(10);

/// Loads `source` as an `init.ts` with the shell's own stock library and a host that registers nothing, so
/// `defineConfig` runs every check it runs in a session: an unknown key, a bad trigger, a rule that names a workspace
/// the config does not declare. `check` cannot do this, because it stubs the `gnoblin` module out. Returns the first
/// problem as `file:line:column: message`.
pub(crate) fn validate_define_config(source: &str) -> Result<(), String> {
    let scratch = Scratch::new().map_err(|error| format!("cannot make a scratch directory: {error}"))?;
    let entry = scratch.0.join(INIT_FILE);
    std::fs::write(&entry, source).map_err(|error| format!("cannot write {}: {error}", entry.display()))?;
    let run = || -> Result<(), JsError> {
        let mut config = JsConfig::new(&scratch.0);
        config.stage_budget = VALIDATE_BUDGET;
        for (name, module) in gnoblin_js::stock_modules() {
            config = config.embed(name, module);
        }
        let generation = JsGeneration::new(&config, 1)?;
        generation.call_budgeted(VALIDATE_BUDGET, |ctx| {
            ctx.eval::<Value<'_>, _>("globalThis.gnoblin = { __register() {} };")
                .map(drop)
        })?;
        generation.eval_module(&entry)
    };
    run().map_err(|error| describe(&error))
}

/// The source of an inert host module exporting `names` and a default.
fn stub_module(names: &BTreeSet<String>) -> String {
    let exports: String = names
        .iter()
        .map(|name| format!("export const {name} = stub;\n"))
        .collect();
    format!("{STUB}export default stub;\n{exports}")
}

/// Something the host would have provided that the check has not stubbed yet.
#[derive(Debug, PartialEq, Eq)]
enum HostGap {
    /// A `gnoblin` or `gnoblin/<module>` import with no module behind it.
    Module(String),
    /// A named import the stub for `module` does not export yet.
    Export { module: String, name: String },
}

/// What `error` says about a host module the check has not stubbed, if it is that and not a fault in the
/// author's code.
///
/// The runtime reports both cases as text: its own wording for a module no host registered, and the
/// engine's for a named import the module does not export. Only the `gnoblin` family and an
/// identifier name are taken, so an import of a missing export from the author's own file is reported.
fn host_gap(error: &JsError) -> Option<HostGap> {
    let message = error.message.as_str();
    let host = |module: &str| module == "gnoblin" || module.starts_with("gnoblin/");
    if error.kind == JsErrorKind::Load {
        let module = between(message, "no embedded module named `", "`")?;
        return host(module).then(|| HostGap::Module(module.to_owned()));
    }
    // The engine capitalises its message; ASCII lowercasing keeps every byte offset.
    let lower = message.to_ascii_lowercase();
    let start = lower.find("could not find export '")? + "could not find export '".len();
    let name = between(&message[start - 1..], "'", "'")?;
    let after = &message[start + name.len()..];
    let module = between(after, "in module '", "'")?;
    let identifier = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    (host(module) && identifier).then(|| HostGap::Export {
        module: module.to_owned(),
        name: name.to_owned(),
    })
}

/// The text of `text` between the first `open` and the next `close` after it.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let from = text.find(open)? + open.len();
    let length = text[from..].find(close)?;
    Some(&text[from..from + length])
}

/// A path as the user would type it: relative to the working directory when it is under it.
fn shown(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// The `file:line:column: message` line for a problem, or the bare message when it has no position.
fn describe(error: &JsError) -> String {
    match &error.location {
        Some(at) => format!("{}:{}:{}: {}", shown(&at.file), at.line, at.column, error.message),
        None => error.message.clone(),
    }
}

/// What to try, by what kind of problem it was.
fn hint(kind: JsErrorKind) -> Option<&'static str> {
    match kind {
        JsErrorKind::Ts => {
            Some("fix the syntax at that position; types are stripped here, not checked (see `--types`)")
        }
        JsErrorKind::Load => Some("import `./` or `../` paths under the config directory, or `gnoblin` modules"),
        JsErrorKind::Exception => Some("the module threw while loading, before any handler could run"),
        JsErrorKind::Budget => Some("top-level code must finish quickly; look for a loop that never ends"),
        JsErrorKind::Memory | JsErrorKind::Engine => None,
    }
}

/// The kind as a word a script can branch on.
fn kind_word(kind: JsErrorKind) -> &'static str {
    match kind {
        JsErrorKind::Ts => "syntax",
        JsErrorKind::Load => "load",
        JsErrorKind::Exception => "exception",
        JsErrorKind::Budget => "budget",
        JsErrorKind::Memory => "memory",
        JsErrorKind::Engine => "engine",
    }
}

/// Print the result for a person: the verdict on standard output, a problem on standard error, and
/// the type checker's own words wherever it wrote them.
fn write_text(
    file: &Path,
    graph: &Result<usize, JsError>,
    types: Option<&TypeCheck>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    theme: &OutputTheme,
) -> io::Result<()> {
    match graph {
        Ok(files) => {
            let noun = if *files == 1 { "file" } else { "files" };
            let line = status_line(
                &self::theme(io::stdout().is_terminal()),
                LineOutcome::Ok,
                &format!("{}  module graph loads ({files} {noun})", shown(file)),
            );
            writeln!(out, "{line}")?;
        }
        Err(error) => {
            let report = ErrorReport {
                summary: describe(error),
                why: None,
                hints: hint(error.kind).map(str::to_owned).into_iter().collect(),
            };
            writeln!(err, "{}", error_block(theme, &report))?;
        }
    }
    match types {
        Some(TypeCheck::Skipped) => writeln!(out, "{NO_TSC}")?,
        Some(TypeCheck::Ran { stdout, stderr, .. }) => {
            out.write_all(stdout)?;
            err.write_all(stderr)?;
        }
        Some(TypeCheck::Failed(message)) => {
            let report = ErrorReport {
                summary: message.clone(),
                why: None,
                hints: vec!["check the path in GNOBLIN_TSC, or run the compiler by hand".to_owned()],
            };
            writeln!(err, "{}", error_block(theme, &report))?;
        }
        None => {}
    }
    Ok(())
}

/// Print the result as one JSON document. The type checker's output is carried as text.
fn write_json(
    file: &Path,
    graph: &Result<usize, JsError>,
    types: Option<&TypeCheck>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut document = match graph {
        Ok(files) => json!({"ok": true, "file": file, "files": files, "problems": []}),
        Err(error) => json!({
            "ok": false,
            "file": file,
            "problems": [{
                "kind": kind_word(error.kind),
                "file": error.location.as_ref().map(|at| &at.file),
                "line": error.location.as_ref().map(|at| at.line),
                "column": error.location.as_ref().map(|at| at.column),
                "message": error.message,
            }],
        }),
    };
    if let Some(types) = types {
        document["types"] = types.json();
        document["ok"] = Json::Bool(graph.is_ok() && types.passed());
    }
    writeln!(out, "{document}")
}

/// What the type checker step did.
#[derive(Debug)]
enum TypeCheck {
    /// No compiler was found. That is not a failure: this machine just cannot do the step.
    Skipped,
    /// The compiler ran. Its output is kept as bytes and passed through unchanged.
    Ran {
        tool: PathBuf,
        passed: bool,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    /// The compiler was found but could not be run, or its inputs could not be written.
    Failed(String),
}

impl TypeCheck {
    /// Whether the step leaves the exit code alone.
    fn passed(&self) -> bool {
        match self {
            TypeCheck::Skipped => true,
            TypeCheck::Ran { passed, .. } => *passed,
            TypeCheck::Failed(_) => false,
        }
    }

    fn json(&self) -> Json {
        match self {
            TypeCheck::Skipped => json!({"ran": false, "message": NO_TSC}),
            TypeCheck::Ran {
                tool,
                passed,
                stdout,
                stderr,
            } => json!({
                "ran": true,
                "tool": tool,
                "passed": passed,
                "stdout": String::from_utf8_lossy(stdout),
                "stderr": String::from_utf8_lossy(stderr),
            }),
            TypeCheck::Failed(message) => json!({"ran": false, "error": message}),
        }
    }
}

/// Where `--types` looks for a compiler. Captured so a test supplies it instead of the process.
#[derive(Debug, Default)]
struct TscSearch {
    /// `$GNOBLIN_TSC`: used as given when set, with no search.
    explicit: Option<PathBuf>,
    /// The directory of the checked file, whose `node_modules/.bin` is tried next.
    config_dir: Option<PathBuf>,
    /// `$PATH`, for the names in [`TSC_NAMES`].
    path: Option<OsString>,
}

impl TscSearch {
    fn from_environment(file: &Path) -> Self {
        Self {
            explicit: std::env::var_os("GNOBLIN_TSC")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            config_dir: file
                .canonicalize()
                .ok()
                .and_then(|entry| entry.parent().map(Path::to_path_buf)),
            path: std::env::var_os("PATH"),
        }
    }

    /// The compiler to run, if there is one.
    fn find(&self) -> Option<PathBuf> {
        if let Some(explicit) = &self.explicit {
            return Some(explicit.clone());
        }
        if let Some(dir) = &self.config_dir {
            let local = dir.join("node_modules").join(".bin").join("tsc");
            if is_executable(&local) {
                return Some(local);
            }
        }
        let path = self.path.as_ref()?;
        let dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
        TSC_NAMES
            .iter()
            .flat_map(|name| dirs.iter().map(move |dir| dir.join(name)))
            .find(|candidate| is_executable(candidate))
    }
}

/// Whether `path` is a regular file with an execute bit.
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// A directory for the generated type files, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gnoblinctl-types-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run the external type checker over `file` with the generated declarations.
///
/// The declarations and the `tsconfig.json` come from `ensure_user_files`, written into a scratch
/// directory so a check never touches the user's directory. The `tsconfig.json` is then pointed at
/// the checked file alone (`files`), rather than at every `.ts` below the scratch directory, so the
/// compiler sees the module graph the runtime would load and nothing else, and a `gnoblin.d.ts` the
/// user already keeps beside it cannot be declared twice.
fn type_check(file: &Path, search: &TscSearch) -> TypeCheck {
    let Some(tool) = search.find() else {
        return TypeCheck::Skipped;
    };
    let run = || -> Result<TypeCheck, String> {
        let entry = file
            .canonicalize()
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        let scratch = Scratch::new().map_err(|error| format!("cannot create a scratch directory: {error}"))?;
        let written = ensure_user_files(&scratch.0).map_err(|error| format!("cannot write the type files: {error}"))?;
        let config_path = scratch.0.join(TSCONFIG_FILE);
        let text =
            std::fs::read_to_string(&config_path).map_err(|error| format!("cannot read the tsconfig: {error}"))?;
        let mut config: Json =
            serde_json::from_str(&text).map_err(|error| format!("the tsconfig is not JSON: {error}"))?;
        config["files"] = json!([written.types, entry]);
        if let Some(object) = config.as_object_mut() {
            object.remove("include");
        }
        if entry
            .extension()
            .is_some_and(|extension| extension == "js" || extension == "mjs")
        {
            config["compilerOptions"]["allowJs"] = Json::Bool(true);
            config["compilerOptions"]["checkJs"] = Json::Bool(true);
        }
        std::fs::write(&config_path, config.to_string())
            .map_err(|error| format!("cannot write the tsconfig: {error}"))?;
        let output = Command::new(&tool)
            .args(["--noEmit", "--pretty", "false", "-p"])
            .arg(&config_path)
            .output()
            .map_err(|error| format!("cannot run {}: {error}", tool.display()))?;
        Ok(TypeCheck::Ran {
            tool: tool.clone(),
            passed: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    };
    run().unwrap_or_else(TypeCheck::Failed)
}

/// The directory `types init` writes into when none is named: `$XDG_CONFIG_HOME/gnoblin`, else
/// `$HOME/.config/gnoblin`, the same directory the runtime reads `init.ts` from.
pub(crate) fn default_config_dir(paths: &SearchPaths) -> Option<PathBuf> {
    match (&paths.config_home, &paths.home) {
        (Some(config_home), _) => Some(config_home.join("gnoblin")),
        (None, Some(home)) => Some(home.join(".config").join("gnoblin")),
        (None, None) => None,
    }
}

/// `types init [DIR]`: write `gnoblin.d.ts` and, when absent, `tsconfig.json`, so an editor works
/// with no build step.
pub(crate) fn types_init(dir: Option<&Path>, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let theme_err = theme(io::stderr().is_terminal());
    let fail = |err: &mut dyn Write, summary: String, hint: &str| {
        let report = ErrorReport {
            summary,
            why: None,
            hints: vec![hint.to_owned()],
        };
        let _ = writeln!(err, "{}", error_block(&theme_err, &report));
        1
    };
    let dir = match dir {
        Some(dir) => dir.to_path_buf(),
        None => match default_config_dir(&SearchPaths::from_env()) {
            Some(dir) => dir,
            None => {
                return fail(
                    err,
                    "neither XDG_CONFIG_HOME nor HOME is set, so there is no config directory".to_owned(),
                    "name the directory: `gnoblinctl types init DIR`",
                );
            }
        },
    };
    let written = match ensure_user_files(&dir) {
        Ok(written) => written,
        Err(error) => {
            return fail(
                err,
                format!("cannot write the editor files in {}: {error}", dir.display()),
                "check that the directory is writable, or name another: `gnoblinctl types init DIR`",
            );
        }
    };
    let result = if json {
        writeln!(
            out,
            "{}",
            json!({
                "types": written.types,
                "tsconfig": dir.join(TSCONFIG_FILE),
                "tsconfig_created": written.tsconfig.is_some(),
            })
        )
    } else {
        let theme = theme(io::stdout().is_terminal());
        let types = status_line(
            &theme,
            LineOutcome::Ok,
            &format!("{TYPES_FILE} refreshed: {}", written.types.display()),
        );
        let config = match &written.tsconfig {
            Some(path) => status_line(
                &theme,
                LineOutcome::Ok,
                &format!("{TSCONFIG_FILE} created: {}", path.display()),
            ),
            None => status_line(
                &theme,
                LineOutcome::Info,
                &format!(
                    "{TSCONFIG_FILE} kept, it already exists: {}",
                    dir.join(TSCONFIG_FILE).display()
                ),
            ),
        };
        writeln!(out, "{types}\n{config}")
    };
    u8::from(result.is_err())
}

/// The file `config init` writes: the one the shell reads first.
const INIT_FILE: &str = "init.ts";

/// The entry the shell falls back to when there is no `init.ts`. A starter written beside it would shadow it, so
/// `config init` treats it like an existing config.
const INIT_FILE_JS: &str = "init.js";

/// Writes the starter `init.ts`, refusing to replace a file unless `force`.
fn write_starter(dir: &Path, init: &Path, force: bool) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(init)?.write_all(STARTER_CONFIG.as_bytes())
}

/// `config init [DIR] [--force]`: write a short, commented `init.ts` and the editor files (`types init`), and say what
/// to do next. Nothing is replaced unless `--force` is given, so a config you have is never lost to a first-run
/// command.
pub(crate) fn config_init(dir: Option<&Path>, force: bool, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let theme_err = theme(io::stderr().is_terminal());
    let fail = |err: &mut dyn Write, summary: String, why: Option<String>, hint: &str| {
        let report = ErrorReport {
            summary,
            why,
            hints: vec![hint.to_owned()],
        };
        let _ = writeln!(err, "{}", error_block(&theme_err, &report));
        1
    };
    let Some(dir) = dir
        .map(Path::to_path_buf)
        .or_else(|| default_config_dir(&SearchPaths::from_env()))
    else {
        return fail(
            err,
            "neither XDG_CONFIG_HOME nor HOME is set, so there is no config directory".to_owned(),
            None,
            "name the directory: `gnoblinctl config init DIR`",
        );
    };
    let init = dir.join(INIT_FILE);
    let replaced = init.exists();
    if !force {
        for name in [INIT_FILE, INIT_FILE_JS] {
            let existing = dir.join(name);
            if existing.exists() {
                let why = if name == INIT_FILE {
                    "the shell already has a config to read".to_owned()
                } else {
                    format!("an {INIT_FILE} written here would be read instead of it")
                };
                return fail(
                    err,
                    format!("{} already exists, so nothing was written", existing.display()),
                    Some(why),
                    "edit that file, or run `gnoblinctl config init --force` to write the starter over it",
                );
            }
        }
    }
    if let Err(error) = write_starter(&dir, &init, force) {
        return fail(
            err,
            format!("cannot write {}: {error}", init.display()),
            None,
            "check that the directory is writable, or name another: `gnoblinctl config init DIR`",
        );
    }
    let written = match ensure_user_files(&dir) {
        Ok(written) => written,
        Err(error) => {
            return fail(
                err,
                format!("{} was written, but the editor files were not: {error}", init.display()),
                None,
                "check that the directory is writable, then run `gnoblinctl types init`",
            );
        }
    };
    let mut next = vec![
        format!("Open {} in your editor.", init.display()),
        format!("Check it with `gnoblinctl check --types {}`.", init.display()),
        "Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request."
            .to_owned(),
    ];
    // The new `init.ts` takes precedence over an `init.lua` beside it, so the next session stops reading the Lua file.
    let lua = dir.join("init.lua");
    if lua.is_file() {
        next.push(format!(
            "{} is still there, and the TypeScript shell does not read it. Move what you need into init.ts (docs/migrating-from-lua.md).",
            lua.display()
        ));
    }
    let result = if json {
        writeln!(
            out,
            "{}",
            json!({
                "init": init,
                "replaced": replaced,
                "types": written.types,
                "tsconfig": dir.join(TSCONFIG_FILE),
                "tsconfig_created": written.tsconfig.is_some(),
                "next": next,
            })
        )
    } else {
        let theme = theme(io::stdout().is_terminal());
        let tsconfig = match &written.tsconfig {
            Some(path) => format!("{TSCONFIG_FILE} created: {}", path.display()),
            None => format!(
                "{TSCONFIG_FILE} kept, it already exists: {}",
                dir.join(TSCONFIG_FILE).display()
            ),
        };
        let tsconfig_outcome = if written.tsconfig.is_some() {
            LineOutcome::Ok
        } else {
            LineOutcome::Info
        };
        let verb = if replaced { "replaced" } else { "written" };
        let steps: String = next
            .iter()
            .enumerate()
            .map(|(number, step)| format!("\n  {}. {step}", number + 1))
            .collect();
        let text = format!(
            "{}\n{}\n{}\n\nNext:{steps}",
            status_line(
                &theme,
                LineOutcome::Ok,
                &format!("{INIT_FILE} {verb}: {}", init.display())
            ),
            status_line(
                &theme,
                LineOutcome::Ok,
                &format!("{TYPES_FILE} refreshed: {}", written.types.display())
            ),
            status_line(&theme, tsconfig_outcome, &tsconfig),
        );
        writeln!(out, "{text}")
    };
    u8::from(result.is_err())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: JsErrorKind, message: &str) -> JsError {
        JsError {
            kind,
            message: message.to_owned(),
            location: None,
            stack: None,
        }
    }

    #[test]
    fn only_module_extensions_take_the_javascript_path() {
        for name in ["init.ts", "a/b.mjs", "x.js", "y.mts"] {
            assert!(is_module(Path::new(name)), "{name}");
        }
        for name in ["init.lua", "config.json", "init", "init.tsx.bak"] {
            assert!(!is_module(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn an_unregistered_host_module_is_a_gap_and_another_bare_import_is_not() {
        let gap = host_gap(&error(JsErrorKind::Load, "no embedded module named `gnoblin/layouts`"));
        assert_eq!(gap, Some(HostGap::Module("gnoblin/layouts".to_owned())));
        let other = error(
            JsErrorKind::Load,
            "cannot resolve `lodash`: only `./` and `../` paths and embedded modules such as `gnoblin` can be imported",
        );
        assert_eq!(host_gap(&other), None);
    }

    #[test]
    fn a_missing_host_export_is_a_gap_and_a_missing_local_export_is_not() {
        let host = error(
            JsErrorKind::Exception,
            "Could not find export 'defineConfig' in module 'gnoblin'",
        );
        assert_eq!(
            host_gap(&host),
            Some(HostGap::Export {
                module: "gnoblin".to_owned(),
                name: "defineConfig".to_owned()
            })
        );
        let local = error(
            JsErrorKind::Exception,
            "Could not find export 'x' in module '/tmp/c/x.ts'",
        );
        assert_eq!(host_gap(&local), None);
    }

    #[test]
    fn the_stub_exports_a_default_and_each_name() {
        let names: BTreeSet<String> = ["a", "b"].iter().map(|name| (*name).to_owned()).collect();
        let source = stub_module(&names);
        assert!(source.contains("export default stub;"), "{source}");
        assert!(source.contains("export const a = stub;"), "{source}");
        assert!(source.contains("export const b = stub;"), "{source}");
    }

    #[test]
    fn the_config_directory_prefers_xdg_config_home() {
        let both = SearchPaths {
            home: Some(PathBuf::from("/home/u")),
            config_home: Some(PathBuf::from("/cfg")),
            override_path: None,
        };
        assert_eq!(default_config_dir(&both), Some(PathBuf::from("/cfg/gnoblin")));
        let home = SearchPaths {
            config_home: None,
            ..both.clone()
        };
        assert_eq!(
            default_config_dir(&home),
            Some(PathBuf::from("/home/u/.config/gnoblin"))
        );
        assert_eq!(default_config_dir(&SearchPaths::default()), None);
    }

    /// A shell script that stands in for the type checker, so the step is tested without Node.
    fn fake_tsc(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-tsc");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the fake compiler");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make it executable");
        path
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnoblinctl-js-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the test directory");
        dir
    }

    #[test]
    fn with_no_compiler_the_step_is_skipped_and_passes() {
        let dir = scratch_dir("none");
        let file = dir.join("init.ts");
        std::fs::write(&file, "export {};\n").expect("write init.ts");
        let result = type_check(&file, &TscSearch::default());
        assert!(matches!(result, TypeCheck::Skipped), "{result:?}");
        assert!(result.passed());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_compiler_is_given_the_generated_declarations_and_its_output_is_kept_unchanged() {
        let dir = scratch_dir("tsc");
        let file = dir.join("init.ts");
        std::fs::write(&file, "export {};\n").expect("write init.ts");
        // `$5` is the tsconfig, after `--noEmit --pretty false -p`. The script proves the declarations
        // exist beside it, echoes a diagnostic and fails.
        let tool = fake_tsc(
            &dir,
            "test -f \"$(dirname \"$5\")/gnoblin.d.ts\" || exit 9\ngrep -q init.ts \"$5\" || exit 8\necho \"init.ts(1,1): error TS0000: fake\"\nexit 2",
        );
        let search = TscSearch {
            explicit: Some(tool),
            ..TscSearch::default()
        };
        let TypeCheck::Ran {
            passed, stdout, stderr, ..
        } = type_check(&file, &search)
        else {
            panic!("the compiler runs");
        };
        assert!(!passed);
        assert_eq!(String::from_utf8_lossy(&stdout), "init.ts(1,1): error TS0000: fake\n");
        assert!(stderr.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_compiler_on_the_path_is_found_by_name_in_order() {
        let dir = scratch_dir("path");
        let tsgo = dir.join("tsgo");
        std::fs::write(&tsgo, "#!/bin/sh\n").expect("write tsgo");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tsgo, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let search = TscSearch {
            path: Some(dir.clone().into_os_string()),
            ..TscSearch::default()
        };
        assert_eq!(search.find(), Some(tsgo));
        let _ = std::fs::remove_dir_all(dir);
    }
}
