//! `check` for a TypeScript or JavaScript module, and `types init`.
//!
//! `check FILE` runs the module in the dry-run host, the launcher's `check` role: the
//! shell's own library and objects over an inert tree (the root and one seat, no windows, no
//! outputs). A spec-style `shell.ts` that builds workspaces, binds keys and attaches listeners loads as written, and
//! what a session refuses is refused here: a syntax error, an import that does not resolve, a name the `gnoblin`
//! module does not export, a bad trigger, a refused value and a top-level throw, all at the line the author wrote, in
//! `.ts` coordinates. It needs no session and no Node. A file that reads live state sees an empty world. `--types` then
//! runs an external type checker when one is installed; none is embedded, and a machine without one skips that step
//! with a plain line and exit 0.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_cli_style::{
    ColorChoice, ErrorReport, LineOutcome, OutputEnvironment, OutputTheme, error_block, status_line,
};
use gnoblin_config::Environment;
use gnoblin_js::{JsError, JsErrorKind, default_shell_path, editor_library_in, source_starter, stock_search_path};
use gnoblin_js_types::{
    GENERATED_DIR, GENERATED_TSCONFIG_FILE, JsWritten, TSCONFIG_FILE, TYPES_FILE, TsconfigOutcome, ensure_user_files,
};
use gnoblin_shell_discovery::{KNOWN_EXTENSIONS, SHELL_STEM};
use serde_json::{Value as Json, json};

/// The extensions the shell runtime loads as modules, and so the ones `check` sends down the
/// JavaScript path. Anything else keeps the configuration check.
const MODULE_EXTENSIONS: &[&str] = &["ts", "mts", "js", "mjs"];

/// The tools `--types` looks for on `PATH`, best first.
const TSC_NAMES: &[&str] = &["tsc", "tsc6", "tsgo"];

/// What `--types` prints when no type checker is installed.
const NO_TSC: &str =
    "type check skipped: no TypeScript compiler found; install one with `npm i -g typescript` or set GNOBLIN_TSC";

/// Whether `check` runs `path` through the JavaScript runtime rather than the configuration check.
pub(crate) fn is_module(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| MODULE_EXTENSIONS.contains(&extension))
}

/// `check FILE` for a module: the dry-run host, then the types when asked.
pub(crate) fn check(file: &Path, types: bool, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let theme = theme(io::stderr().is_terminal());
    let loaded = evaluate(file);
    let checked = if types {
        Some(type_check(file, &TscSearch::from_environment(file)))
    } else {
        None
    };
    let code = u8::from(loaded.is_err() || checked.as_ref().is_some_and(|checked| !checked.passed()));
    let written = if json {
        write_json(file, &loaded, checked.as_ref(), out)
    } else {
        write_text(file, &loaded, checked.as_ref(), out, err, &theme)
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

/// Runs `file` in the dry-run host: the launcher's `check` role (`gnoblin --role check FILE`), which loads the module as a session
/// does, in a core generation of its own over an inert tree (the root and one seat, no windows, no outputs). The shell's own
/// libraries run (a module that imports `gnoblin-std` runs it over the installed std library), so a spec-style file loads as written
/// and every check a session makes is made here: a syntax error, an import that does not resolve, a name the `gnoblin` module does
/// not export, a bad trigger, a refused value, a throw. Nothing starts and nothing is bound.
///
/// A file that decides something from live state sees an empty world, as it does at first boot. Reading a count or
/// walking a collection is fine; dereferencing the first window is a throw, as it is in a session with no windows.
fn evaluate(file: &Path) -> Result<(), JsError> {
    let failed = |message: String| JsError {
        kind: JsErrorKind::Load,
        message,
        location: None,
        stack: None,
    };
    let entry =
        std::fs::canonicalize(file).map_err(|error| failed(format!("cannot read {}: {error}", file.display())))?;
    let launcher = launcher().ok_or_else(|| {
        failed(
            "the dry-run host is the `gnoblin` launcher, and none was found beside gnoblinctl (set GNOBLIN_LAUNCHER to its path)"
                .to_owned(),
        )
    })?;
    let mut command = Command::new(&launcher);
    command
        .args(["--role", "check"])
        .arg(&entry)
        .env("GNOBLIN_START_FAILURE", "json")
        .stdin(std::process::Stdio::null());
    // A build tree staged by `cargo xtask stage` keeps its runtimes in `runtimes/` beside the launcher; an installed prefix keeps them
    // where the core finds them.
    if std::env::var_os("GNOBLIN_RUNTIME_DIR").is_none()
        && let Some(runtimes) = launcher.parent().map(|dir| dir.join("runtimes"))
        && runtimes.is_dir()
    {
        command.env("GNOBLIN_RUNTIME_DIR", runtimes);
    }
    let output = command
        .output()
        .map_err(|error| failed(format!("cannot run {}: {error}", launcher.display())))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    match lines.next() {
        Some("ok") => Ok(()),
        Some("error") => Err(start_failure(&lines.collect::<Vec<_>>().join("\n"))),
        _ => {
            let said = String::from_utf8_lossy(&output.stderr);
            Err(JsError {
                kind: JsErrorKind::Engine,
                message: format!(
                    "the dry-run host {} said nothing about {}: {}",
                    launcher.display(),
                    entry.display(),
                    said.lines().next().unwrap_or("it printed no verdict")
                ),
                location: None,
                stack: None,
            })
        }
    }
}

/// The `gnoblin` launcher: `$GNOBLIN_LAUNCHER`, or the one installed beside this executable (an installed prefix's `bin/`, or the
/// directory above a test binary's `deps/`).
fn launcher() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("GNOBLIN_LAUNCHER") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..2 {
        let candidate = dir.join("gnoblin");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// The failure the check role printed: the JSON object the JavaScript runtime gives (`gnoblin-runtime-js`, `GNOBLIN_START_FAILURE`),
/// somewhere in the words of the loader, or else the words themselves.
fn start_failure(words: &str) -> JsError {
    let parsed = words
        .find("{\"kind\":")
        .and_then(|at| serde_json::from_str::<Json>(&words[at..]).ok());
    if let Some(object) = parsed
        && let Some(kind) = object["kind"].as_str().and_then(JsErrorKind::from_word)
    {
        let number = |key: &str| object[key].as_u64().and_then(|n| u32::try_from(n).ok());
        let location = match (object["file"].as_str(), number("line"), number("column")) {
            (Some(file), Some(line), Some(column)) => Some(gnoblin_js::JsLocation {
                file: PathBuf::from(file),
                line,
                column,
            }),
            _ => None,
        };
        return JsError {
            kind,
            message: object["message"].as_str().unwrap_or_default().to_owned(),
            location,
            stack: None,
        };
    }
    JsError {
        kind: JsErrorKind::Engine,
        message: words.to_owned(),
        location: None,
        stack: None,
    }
}

/// Files installed with this executable, which is where the prefix is looked for from.
fn anchors() -> Vec<PathBuf> {
    std::env::current_exe().ok().into_iter().collect()
}

/// The directories the stock library is looked for in: `$GNOBLIN_JS_STOCK_DIR`, the data directories, and the prefix this
/// executable is installed in (`share/gnoblin/js`), as the runtime looks.
fn stock_dirs() -> Vec<PathBuf> {
    stock_search_path(&|name| std::env::var(name).ok(), &anchors())
}

/// The starter, the text of the packaged default shell: the `shells/default/shell.ts` of a stock directory laid out as a source tree, else
/// `<prefix>/share/gnoblin/default-shell/shell.ts`.
fn starter_text() -> io::Result<String> {
    let path = stock_dirs()
        .into_iter()
        .find_map(|dir| source_starter(&dir))
        .or_else(|| default_shell_path(&anchors()))
        .ok_or_else(|| {
            io::Error::other("the packaged default shell (share/gnoblin/default-shell/shell.ts) is not installed")
        })?;
    std::fs::read_to_string(&path).map_err(|error| io::Error::other(format!("cannot read {}: {error}", path.display())))
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

/// Print the result for a person: the verdict on standard output, a problem on standard error, and
/// the type checker's own words wherever it wrote them.
fn write_text(
    file: &Path,
    loaded: &Result<(), JsError>,
    types: Option<&TypeCheck>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    theme: &OutputTheme,
) -> io::Result<()> {
    match loaded {
        Ok(()) => {
            let line = status_line(
                &self::theme(io::stdout().is_terminal()),
                LineOutcome::Ok,
                &format!("{}  loads in the dry-run shell", shown(file)),
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
    loaded: &Result<(), JsError>,
    types: Option<&TypeCheck>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut document = match loaded {
        Ok(()) => json!({"ok": true, "file": file, "problems": []}),
        Err(error) => json!({
            "ok": false,
            "file": file,
            "problems": [{
                "kind": error.kind.word(),
                "file": error.location.as_ref().map(|at| &at.file),
                "line": error.location.as_ref().map(|at| at.line),
                "column": error.location.as_ref().map(|at| at.column),
                "message": error.message,
            }],
        }),
    };
    if let Some(types) = types {
        document["types"] = types.json();
        document["ok"] = Json::Bool(loaded.is_ok() && types.passed());
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

/// Run the external type checker over `file` with the real library and the generated declarations.
///
/// The editor files come from `write_editor_files`, written into a scratch directory so a check never touches the
/// user's directory: the library sources, `gnoblin.d.ts` and the generated `tsconfig.json`. The configuration the
/// compiler is given extends that one and names the checked file and the declarations alone (`files`), rather than
/// every `.ts` below the scratch directory, so the compiler sees the module graph the runtime would load and nothing
/// else, and a `gnoblin.d.ts` the user already keeps beside the file cannot be declared twice.
fn type_check(file: &Path, search: &TscSearch) -> TypeCheck {
    let Some(tool) = search.find() else {
        return TypeCheck::Skipped;
    };
    let run = || -> Result<TypeCheck, String> {
        let entry = file
            .canonicalize()
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        let scratch = Scratch::new().map_err(|error| format!("cannot create a scratch directory: {error}"))?;
        let written =
            write_editor_files(&scratch.0).map_err(|error| format!("cannot write the type files: {error}"))?;
        let config_path = scratch.0.join(CHECK_TSCONFIG_FILE);
        let mut config = json!({
            "extends": format!("./{GENERATED_DIR}/{GENERATED_TSCONFIG_FILE}"),
            "files": [written.types, entry],
        });
        if entry
            .extension()
            .is_some_and(|extension| extension == "js" || extension == "mjs")
        {
            config["compilerOptions"] = json!({ "allowJs": true, "checkJs": true });
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

/// The name of the configuration `check --types` hands the compiler, in the scratch directory.
const CHECK_TSCONFIG_FILE: &str = "check.tsconfig.json";

/// Write the editor files into `dir`: the library as the shell embeds it, the global declarations and the tsconfigs.
/// `types init`, `config init` and `check --types` all write the same files through this.
pub(crate) fn write_editor_files(dir: &Path) -> io::Result<JsWritten> {
    let dirs = stock_dirs();
    let library = editor_library_in(&dirs).map_err(io::Error::other)?;
    if library.is_empty() {
        let looked = dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(io::Error::other(format!(
            "the stock library is not installed (looked in: {looked})"
        )));
    }
    ensure_user_files(dir, &library)
}

/// What the editor-file commands print after `written` was written into `dir`: the library, the declarations and what
/// became of the user's `tsconfig.json`.
pub(crate) fn editor_lines(theme: &OutputTheme, written: &JsWritten, dir: &Path) -> String {
    let line = |outcome: LineOutcome, text: String| status_line(theme, outcome, &text);
    let tsconfig_path = dir.join(TSCONFIG_FILE);
    let mut lines = vec![
        line(
            LineOutcome::Ok,
            format!(
                "library refreshed: {} ({} files, the real sources of `gnoblin-core` and `gnoblin-std`)",
                written.library.display(),
                written.library_files
            ),
        ),
        line(
            LineOutcome::Ok,
            format!("{TYPES_FILE} refreshed: {}", written.types.display()),
        ),
    ];
    match &written.tsconfig {
        TsconfigOutcome::Created(path) => {
            lines.push(line(
                LineOutcome::Ok,
                format!("{TSCONFIG_FILE} created: {}", path.display()),
            ));
        }
        TsconfigOutcome::Migrated(path) => lines.push(line(
            LineOutcome::Ok,
            format!(
                "{TSCONFIG_FILE} replaced, it was the one an earlier release wrote and no longer finds the types: {}",
                path.display()
            ),
        )),
        TsconfigOutcome::Kept => lines.push(line(
            LineOutcome::Info,
            format!("{TSCONFIG_FILE} kept, it already exists: {}", tsconfig_path.display()),
        )),
        TsconfigOutcome::NotLinked => {
            let generated = format!("{GENERATED_DIR}/{GENERATED_TSCONFIG_FILE}");
            lines.push(line(
                LineOutcome::Warn,
                format!(
                    "{TSCONFIG_FILE} kept, but it does not extend {generated}, so the editor does not see the types: \
                     add \"extends\": \"./{generated}\" to {}",
                    tsconfig_path.display()
                ),
            ));
        }
    }
    lines.join("\n")
}

/// The same report as JSON: where the files are, and what became of the user's `tsconfig.json`.
pub(crate) fn editor_json(written: &JsWritten, dir: &Path) -> Json {
    json!({
        "types": written.types,
        "library": written.library,
        "library_files": written.library_files,
        "tsconfig": dir.join(TSCONFIG_FILE),
        "tsconfig_created": matches!(written.tsconfig, TsconfigOutcome::Created(_)),
        "tsconfig_replaced": matches!(written.tsconfig, TsconfigOutcome::Migrated(_)),
        "tsconfig_linked": !matches!(written.tsconfig, TsconfigOutcome::NotLinked),
    })
}

/// `types init [DIR]`: write the library sources, `gnoblin.d.ts` and, when absent, `tsconfig.json`, so an editor works
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
        None => match Environment::from_env().config_dir() {
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
    let written = match write_editor_files(&dir) {
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
        writeln!(out, "{}", editor_json(&written, &dir))
    } else {
        writeln!(
            out,
            "{}",
            editor_lines(&theme(io::stdout().is_terminal()), &written, &dir)
        )
    };
    u8::from(result.is_err())
}

/// The file `config init` writes: the conventional user shell, which discovery finds in the config directory.
const INIT_FILE: &str = "shell.ts";

/// A conventional shell file of `dir` other than `keep` that is there. Writing `keep` beside it would make the two
/// equally preferred, which discovery refuses to guess between, so the session would not start.
pub(crate) fn rival_shell_file(dir: &Path, keep: &str) -> Option<PathBuf> {
    KNOWN_EXTENSIONS
        .iter()
        .map(|extension| format!("{SHELL_STEM}.{extension}"))
        .filter(|name| name != keep)
        .map(|name| dir.join(name))
        .find(|file| file.exists())
}

/// Writes the starter `shell.ts`, refusing to replace a file unless `force`.
fn write_starter(dir: &Path, init: &Path, force: bool) -> io::Result<()> {
    // Read first: a starter that is not installed must not leave an empty `shell.ts` behind.
    let starter = starter_text()?;
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(init)?.write_all(starter.as_bytes())
}

/// `config init [DIR] [--force]`: write a short, commented `shell.ts` and the editor files (`types init`), and say what
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
        .or_else(|| Environment::from_env().config_dir())
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
        let found = if replaced {
            Some((init.clone(), "the shell already has a config to read".to_owned()))
        } else {
            rival_shell_file(&dir, INIT_FILE).map(|rival| {
                (
                    rival,
                    format!(
                        "a {INIT_FILE} written here would make the two equally preferred, and no shell would start"
                    ),
                )
            })
        };
        if let Some((existing, why)) = found {
            return fail(
                err,
                format!("{} already exists, so nothing was written", existing.display()),
                Some(why),
                "edit that file, or run `gnoblinctl config init --force` to write the starter over it",
            );
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
    let written = match write_editor_files(&dir) {
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
    let next = vec![
        format!("Open {} in your editor.", init.display()),
        format!("Check it with `gnoblinctl check --types {}`.", init.display()),
        "Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request."
            .to_owned(),
    ];
    let result = if json {
        let mut document = editor_json(&written, &dir);
        document["init"] = json!(init);
        document["replaced"] = json!(replaced);
        document["next"] = json!(next);
        writeln!(out, "{document}")
    } else {
        let theme = theme(io::stdout().is_terminal());
        let verb = if replaced { "replaced" } else { "written" };
        let steps: String = next
            .iter()
            .enumerate()
            .map(|(number, step)| format!("\n  {}. {step}", number + 1))
            .collect();
        let text = format!(
            "{}\n{}\n\nNext:{steps}",
            status_line(
                &theme,
                LineOutcome::Ok,
                &format!("{INIT_FILE} {verb}: {}", init.display())
            ),
            editor_lines(&theme, &written, &dir),
        );
        writeln!(out, "{text}")
    };
    u8::from(result.is_err())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_module_extensions_take_the_javascript_path() {
        for name in ["shell.ts", "a/b.mjs", "x.js", "y.mts"] {
            assert!(is_module(Path::new(name)), "{name}");
        }
        for name in ["config.json", "init", "init.tsx.bak"] {
            assert!(!is_module(Path::new(name)), "{name}");
        }
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
        let file = dir.join("shell.ts");
        std::fs::write(&file, "export {};\n").expect("write shell.ts");
        let result = type_check(&file, &TscSearch::default());
        assert!(matches!(result, TypeCheck::Skipped), "{result:?}");
        assert!(result.passed());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_compiler_is_given_the_library_and_the_declarations_and_its_output_is_kept_unchanged() {
        let dir = scratch_dir("tsc");
        let file = dir.join("shell.ts");
        std::fs::write(&file, "export {};\n").expect("write shell.ts");
        // `$5` is the tsconfig, after `--noEmit --pretty false -p`. The script proves the declarations and the real
        // library sources exist beside it, echoes a diagnostic and fails.
        let tool = fake_tsc(
            &dir,
            "d=\"$(dirname \"$5\")/.gnoblin\"\ntest -f \"$d/gnoblin.d.ts\" || exit 9\ntest -f \"$d/lib/gnoblin-std/scene/node.ts\" || exit 7\ngrep -q shell.ts \"$5\" || exit 8\necho \"shell.ts(1,1): error TS0000: fake\"\nexit 2",
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
        assert_eq!(String::from_utf8_lossy(&stdout), "shell.ts(1,1): error TS0000: fake\n");
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
