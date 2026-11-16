//! `check` for a TypeScript or JavaScript module, and `types init`.
//!
//! `check FILE` runs the module in the dry-run host, the evaluation `gnoblin_js::evaluate_document` makes: the
//! shell's own library and objects over an inert tree (the root and one seat, no windows, no
//! outputs). A spec-style `init.ts` that builds workspaces, binds keys and attaches listeners loads as written, and
//! what a session refuses is refused here: a syntax error, an import that does not resolve, a name the `gnoblin`
//! module does not export, a bad trigger, a refused value and a top-level throw, all at the line the author wrote, in
//! `.ts` coordinates. It needs no session and no Node. A file that reads live state sees an empty world. `--types` then
//! runs an external type checker when one is installed; none is embedded (design/js-research/ts-in-quickjs.md,
//! section 6), and a machine without one skips that step with a plain line and exit 0.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_cli_style::{
    ColorChoice, ErrorReport, LineOutcome, OutputEnvironment, OutputTheme, error_block, status_line,
};
use gnoblin_config::SearchPaths;
use gnoblin_js::{JsError, JsErrorKind, STARTER_CONFIG, editor_library};
use gnoblin_js_types::{
    GENERATED_DIR, GENERATED_TSCONFIG_FILE, JsWritten, TSCONFIG_FILE, TYPES_FILE, TsconfigOutcome, ensure_user_files,
};
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

/// Runs `file` in the dry-run host: `gnoblin_js::evaluate_document`, the evaluation of an
/// `init.ts` over an inert tree. The shell's own library and objects run over an inert tree (the
/// root and one seat, no windows, no outputs), so a spec-style file loads as written and every check a session makes
/// is made here: a syntax error, an import that does not resolve, a name the `gnoblin` module does not export, a bad
/// trigger, a refused value, a throw. Nothing starts and nothing is bound.
///
/// A file that decides something from live state sees an empty world, as it does at first boot. Reading a count or
/// walking a collection is fine; dereferencing the first window is a throw, as it is in a session with no windows.
fn evaluate(file: &Path) -> Result<(), JsError> {
    gnoblin_js::evaluate_document(file).map(drop)
}

/// Loads `source` as an `init.ts` in the dry-run host before a command writes it, so a file that would not load is said
/// so and not found at the next login. Returns the first problem as `file:line:column: message`.
pub(crate) fn validate_config(source: &str) -> Result<(), String> {
    let scratch = Scratch::new().map_err(|error| format!("cannot make a scratch directory: {error}"))?;
    let entry = scratch.0.join(INIT_FILE);
    std::fs::write(&entry, source).map_err(|error| format!("cannot write {}: {error}", entry.display()))?;
    evaluate(&entry).map_err(|error| describe(&error))
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

/// The directory `types init` writes into when none is named: `$XDG_CONFIG_HOME/gnoblin`, else
/// `$HOME/.config/gnoblin`, the same directory the runtime reads `init.ts` from.
pub(crate) fn default_config_dir(paths: &SearchPaths) -> Option<PathBuf> {
    match (&paths.config_home, &paths.home) {
        (Some(config_home), _) => Some(config_home.join("gnoblin")),
        (None, Some(home)) => Some(home.join(".config").join("gnoblin")),
        (None, None) => None,
    }
}

/// The name of the configuration `check --types` hands the compiler, in the scratch directory.
const CHECK_TSCONFIG_FILE: &str = "check.tsconfig.json";

/// Write the editor files into `dir`: the library as the shell embeds it, the schema declarations and the tsconfigs.
/// `types init`, `config init`, `migrate` and `check --types` all write the same files through this.
pub(crate) fn write_editor_files(dir: &Path) -> io::Result<JsWritten> {
    ensure_user_files(dir, &editor_library())
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
                "library refreshed: {} ({} files, the real sources of `gnoblin`)",
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
        for name in ["init.ts", "a/b.mjs", "x.js", "y.mts"] {
            assert!(is_module(Path::new(name)), "{name}");
        }
        for name in ["init.lua", "config.json", "init", "init.tsx.bak"] {
            assert!(!is_module(Path::new(name)), "{name}");
        }
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
    fn the_compiler_is_given_the_library_and_the_declarations_and_its_output_is_kept_unchanged() {
        let dir = scratch_dir("tsc");
        let file = dir.join("init.ts");
        std::fs::write(&file, "export {};\n").expect("write init.ts");
        // `$5` is the tsconfig, after `--noEmit --pretty false -p`. The script proves the declarations and the real
        // library sources exist beside it, echoes a diagnostic and fails.
        let tool = fake_tsc(
            &dir,
            "d=\"$(dirname \"$5\")/.gnoblin\"\ntest -f \"$d/gnoblin.d.ts\" || exit 9\ntest -f \"$d/lib/gnoblin-std/scene/node.ts\" || exit 7\ngrep -q init.ts \"$5\" || exit 8\necho \"init.ts(1,1): error TS0000: fake\"\nexit 2",
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
