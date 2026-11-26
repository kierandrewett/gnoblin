//! `config init`: write a starter shell in TypeScript, Python or Rust.
//!
//! The three starters are the same desktop (the stock one, as statements against the shell's objects) in three languages:
//!
//! - TypeScript is the packaged default shell (`shells/default/shell.ts`), written as `shell.ts` with the editor files
//!   of `types init`. It is read from the install, because a session with no shell file runs the same text.
//! - Python is `shells/starters/python/shell.py`, written as `shell.py`, with a `pyrightconfig.json` that points Pyright
//!   at the installed stubs when they are there.
//! - Rust is the Cargo project in `shells/starters/rust`: a `cdylib` crate in `rust-shell/` of the config directory,
//!   with the commands that build it and make `gnoblin` find the library.
//!
//! The Python and Rust starters are compiled into `gnoblinctl`: no session runs them as a default, so nothing installs
//! them.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use gnoblin_cli_style::{ErrorReport, LineOutcome, error_block, status_line};
use gnoblin_config::Environment;
use serde_json::{Value as Json, json};

use crate::js::{self, INIT_FILE};

/// The language of the starter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StarterLanguage {
    TypeScript,
    Python,
    Rust,
}

impl StarterLanguage {
    /// The name `--language` takes, and `language` in the JSON.
    pub(crate) fn name(self) -> &'static str {
        match self {
            StarterLanguage::TypeScript => "typescript",
            StarterLanguage::Python => "python",
            StarterLanguage::Rust => "rust",
        }
    }

    /// The language a `--language` value names; the parser only lets the known names through.
    pub(crate) fn from_name(name: &str) -> Option<StarterLanguage> {
        match name {
            "typescript" | "ts" => Some(StarterLanguage::TypeScript),
            "python" | "py" => Some(StarterLanguage::Python),
            "rust" | "rs" => Some(StarterLanguage::Rust),
            _ => None,
        }
    }

    /// The command that writes this starter over what is there.
    fn force_command(self) -> String {
        match self {
            StarterLanguage::TypeScript => "gnoblinctl config init --force".to_owned(),
            other => format!("gnoblinctl config init --language {} --force", other.name()),
        }
    }
}

/// What `config init` was asked.
#[derive(Debug)]
pub(crate) struct InitOptions {
    /// The config directory; the default is `$XDG_CONFIG_HOME/gnoblin`.
    pub(crate) dir: Option<PathBuf>,
    pub(crate) language: StarterLanguage,
    pub(crate) force: bool,
    /// A checkout of the Gnoblin source that the Rust starter depends on; the default is the repository on GitHub.
    pub(crate) sdk: Option<PathBuf>,
    pub(crate) json: bool,
}

/// The Python starter.
const PYTHON_STARTER: &str = include_str!("../../../shells/starters/python/shell.py");
/// The file it is written as.
const PYTHON_FILE: &str = "shell.py";
/// The file that tells Pyright where the stubs are, written beside it.
const PYRIGHT_FILE: &str = "pyrightconfig.json";
/// Where the stubs are installed, below the prefix's `share`.
const PYTHON_STUBS: &str = "types/python";

/// The Rust starter's manifest, with `@SDK_CORE@` and `@SDK_STD@` for the two dependencies, and its source.
const RUST_MANIFEST_TEMPLATE: &str = include_str!("../../../shells/starters/rust/Cargo.toml.in");
const RUST_SOURCE_TEXT: &str = include_str!("../../../shells/starters/rust/src/lib.rs");
/// The project directory, in the config directory.
const RUST_PROJECT_DIR: &str = "rust-shell";
const RUST_MANIFEST: &str = "Cargo.toml";
const RUST_SOURCE: &str = "src/lib.rs";
const RUST_IGNORE: &str = ".gitignore";
const RUST_IGNORE_TEXT: &str = "/target\n";
/// What the starter's `[lib] name` builds, in the `target` directory.
const RUST_LIBRARY: &str = "libmyshell.so";
/// The Gnoblin repository, which holds the SDK crates.
const SDK_REPOSITORY: &str = "https://github.com/kierandrewett/gnoblin";

/// The error block for a failed command, and the exit code 1.
fn fail(err: &mut dyn Write, summary: String, why: Option<String>, hint: &str) -> u8 {
    let report = ErrorReport {
        summary,
        why,
        hints: vec![hint.to_owned()],
    };
    let _ = writeln!(err, "{}", error_block(&js::theme(io::stderr().is_terminal()), &report));
    1
}

/// Writes `text` to `path`, refusing to replace a file unless `force`.
fn write_new(path: &Path, text: &str, force: bool) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// The file that makes `config init` refuse: the one it would write, or another shell file of the directory that would
/// be equally preferred beside it, with the reason.
fn existing_shell_file(dir: &Path, file: &str) -> Option<(PathBuf, String)> {
    let init = dir.join(file);
    if init.exists() {
        return Some((init, "the shell already has a config to read".to_owned()));
    }
    js::rival_shell_file(dir, file).map(|rival| {
        (
            rival,
            format!("a {file} written here would make the two equally preferred, and no shell would start"),
        )
    })
}

/// The numbered steps, as the text prints them.
fn steps_text(next: &[String]) -> String {
    next.iter()
        .enumerate()
        .map(|(number, step)| format!("\n  {}. {step}", number + 1))
        .collect()
}

/// A word for a shell: bare when it is plain, else in single quotes.
fn shell_word(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':' | '@' | '=' | ','));
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// `config init [DIR] [--language L] [--force] [--sdk PATH]`: write a short, commented starter, and say what to do next.
/// Nothing is replaced unless `--force` is given, so a config you have is never lost to a first-run command.
pub(crate) fn config_init(options: &InitOptions, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Some(dir) = options.dir.clone().or_else(|| Environment::from_env().config_dir()) else {
        return fail(
            err,
            "neither XDG_CONFIG_HOME nor HOME is set, so there is no config directory".to_owned(),
            None,
            "name the directory: `gnoblinctl config init DIR`",
        );
    };
    match options.language {
        StarterLanguage::TypeScript => init_typescript(options, &dir, out, err),
        StarterLanguage::Python => init_python(options, &dir, out, err),
        StarterLanguage::Rust => init_rust(options, &dir, out, err),
    }
}

/// Refuses when `file` or a rival is in `dir` and `--force` was not given. Returns the exit code of the refusal.
fn refuse_existing(options: &InitOptions, file: &str, dir: &Path, err: &mut dyn Write) -> Option<u8> {
    if options.force {
        return None;
    }
    let (existing, why) = existing_shell_file(dir, file)?;
    Some(fail(
        err,
        format!("{} already exists, so nothing was written", existing.display()),
        Some(why),
        &format!(
            "edit that file, or run `{}` to write the starter over it",
            options.language.force_command()
        ),
    ))
}

/// Prints the report: the JSON document, or the first line, the lines of the files and the steps.
fn report(
    options: &InitOptions,
    document: Json,
    headline: &str,
    lines: &str,
    next: &[String],
    out: &mut dyn Write,
) -> u8 {
    let result = if options.json {
        writeln!(out, "{document}")
    } else {
        let theme = js::theme(io::stdout().is_terminal());
        writeln!(
            out,
            "{}\n{lines}\n\nNext:{}",
            status_line(&theme, LineOutcome::Ok, headline),
            steps_text(next)
        )
    };
    u8::from(result.is_err())
}

/// The TypeScript starter: the packaged default shell, and the editor files.
fn init_typescript(options: &InitOptions, dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    if let Some(code) = refuse_existing(options, INIT_FILE, dir, err) {
        return code;
    }
    let init = dir.join(INIT_FILE);
    let replaced = init.exists();
    // Read first: a starter that is not installed must not leave an empty `shell.ts` behind.
    let written = js::starter_text().and_then(|text| write_new(&init, &text, options.force));
    if let Err(error) = written {
        return fail(
            err,
            format!("cannot write {}: {error}", init.display()),
            None,
            "check that the directory is writable, or name another: `gnoblinctl config init DIR`",
        );
    }
    let editor = match js::write_editor_files(dir) {
        Ok(editor) => editor,
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
    let mut document = js::editor_json(&editor, dir);
    document["language"] = json!(options.language.name());
    document["init"] = json!(init);
    document["replaced"] = json!(replaced);
    document["next"] = json!(next);
    let verb = if replaced { "replaced" } else { "written" };
    let lines = js::editor_lines(&js::theme(io::stdout().is_terminal()), &editor, dir);
    report(
        options,
        document,
        &format!("{INIT_FILE} {verb}: {}", init.display()),
        &lines,
        &next,
        out,
    )
}

/// The directory of the installed Python stubs, when there is one: `share/gnoblin/types/python` beside the library the
/// shell finds its stock sources in.
fn python_stubs() -> Option<PathBuf> {
    js::stock_dirs()
        .into_iter()
        .filter_map(|library| Some(library.parent()?.join(PYTHON_STUBS)))
        .find(|stubs| stubs.join("gnoblin/__init__.pyi").is_file())
}

/// What became of `pyrightconfig.json`.
enum PyrightOutcome {
    Created(PathBuf),
    Kept(PathBuf),
    NoStubs,
}

/// Writes the `pyrightconfig.json` that points Pyright at `stubs`, unless the directory has one.
fn write_pyright(dir: &Path, stubs: Option<&Path>) -> io::Result<PyrightOutcome> {
    let path = dir.join(PYRIGHT_FILE);
    let Some(stubs) = stubs else {
        return Ok(PyrightOutcome::NoStubs);
    };
    let text = serde_json::to_string_pretty(&json!({ "extraPaths": [stubs] })).map_err(io::Error::other)?;
    match write_new(&path, &format!("{text}\n"), false) {
        Ok(()) => Ok(PyrightOutcome::Created(path)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(PyrightOutcome::Kept(path)),
        Err(error) => Err(error),
    }
}

/// The Python starter: `shell.py`, and a `pyrightconfig.json` for the editor when the stubs are installed.
fn init_python(options: &InitOptions, dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    if let Some(code) = refuse_existing(options, PYTHON_FILE, dir, err) {
        return code;
    }
    let init = dir.join(PYTHON_FILE);
    let replaced = init.exists();
    if let Err(error) = write_new(&init, PYTHON_STARTER, options.force) {
        return fail(
            err,
            format!("cannot write {}: {error}", init.display()),
            None,
            "check that the directory is writable, or name another: `gnoblinctl config init --language python DIR`",
        );
    }
    let stubs = python_stubs();
    let pyright = match write_pyright(dir, stubs.as_deref()) {
        Ok(pyright) => pyright,
        Err(error) => {
            return fail(
                err,
                format!("{} was written, but {PYRIGHT_FILE} was not: {error}", init.display()),
                None,
                "check that the directory is writable",
            );
        }
    };
    let theme = js::theme(io::stdout().is_terminal());
    let stubs_text = stubs.as_ref().map(|stubs| stubs.display().to_string());
    let line = match (&pyright, &stubs_text) {
        (PyrightOutcome::Created(path), Some(stubs)) => status_line(
            &theme,
            LineOutcome::Ok,
            &format!(
                "{PYRIGHT_FILE} created: {} (finds the stubs in {stubs})",
                path.display()
            ),
        ),
        (PyrightOutcome::Kept(path), _) => status_line(
            &theme,
            LineOutcome::Info,
            &format!(
                "{PYRIGHT_FILE} kept, it already exists: {}; add the stubs to its \"extraPaths\" for an editor to see the types",
                path.display()
            ),
        ),
        _ => status_line(
            &theme,
            LineOutcome::Warn,
            &format!(
                "{PYRIGHT_FILE} not written: the Python stubs are not installed (share/gnoblin/{PYTHON_STUBS} in the prefix), \
                 so an editor sees no types"
            ),
        ),
    };
    let next = vec![
        format!("Open {} in your editor.", init.display()),
        format!("Check it with `gnoblinctl check {}`.", init.display()),
        "Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request."
            .to_owned(),
    ];
    let pyright_path = match &pyright {
        PyrightOutcome::Created(path) | PyrightOutcome::Kept(path) => Some(path.clone()),
        PyrightOutcome::NoStubs => None,
    };
    let document = json!({
        "language": options.language.name(),
        "init": init,
        "replaced": replaced,
        "stubs": stubs,
        "pyrightconfig": pyright_path,
        "pyrightconfig_created": matches!(pyright, PyrightOutcome::Created(_)),
        "next": next,
    });
    let verb = if replaced { "replaced" } else { "written" };
    report(
        options,
        document,
        &format!("{PYTHON_FILE} {verb}: {}", init.display()),
        &line,
        &next,
        out,
    )
}

/// How the starter's manifest names a crate of the SDK.
fn sdk_dependency(sdk: Option<&Path>, name: &str) -> Result<String, String> {
    let Some(sdk) = sdk else {
        return Ok(format!("{{ git = {} }}", json!(SDK_REPOSITORY)));
    };
    let crate_dir = sdk.join("crates").join(name);
    if !crate_dir.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} is not a Gnoblin checkout: it has no crates/{name}/Cargo.toml",
            sdk.display()
        ));
    }
    let crate_dir = std::path::absolute(&crate_dir).map_err(|error| format!("{}: {error}", crate_dir.display()))?;
    let text = crate_dir.to_str().ok_or_else(|| {
        format!(
            "{} is not valid UTF-8, which a Cargo manifest needs",
            crate_dir.display()
        )
    })?;
    Ok(format!("{{ path = {} }}", json!(text)))
}

/// The Rust starter: a Cargo project in `rust-shell/` of the config directory, and the commands that build and run it.
fn init_rust(options: &InitOptions, dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let project = dir.join(RUST_PROJECT_DIR);
    let manifest = project.join(RUST_MANIFEST);
    let source = project.join(RUST_SOURCE);
    let replaced = manifest.exists() || source.exists();
    if !options.force
        && let Some(existing) = [&manifest, &source].into_iter().find(|path| path.exists())
    {
        return fail(
            err,
            format!("{} already exists, so nothing was written", existing.display()),
            Some("the project already has a shell to build".to_owned()),
            &format!(
                "edit that file, or run `{}` to write the starter over it",
                StarterLanguage::Rust.force_command()
            ),
        );
    }
    let dependencies = sdk_dependency(options.sdk.as_deref(), "gnoblin-core")
        .and_then(|core| sdk_dependency(options.sdk.as_deref(), "gnoblin-std").map(|std| (core, std)));
    let (core, std) = match dependencies {
        Ok(pair) => pair,
        Err(message) => {
            return fail(
                err,
                message,
                None,
                "name the root of a checkout of https://github.com/kierandrewett/gnoblin, or leave out --sdk",
            );
        }
    };
    let manifest_text = RUST_MANIFEST_TEMPLATE
        .replace("@SDK_CORE@", &core)
        .replace("@SDK_STD@", &std);
    let written = write_new(&manifest, &manifest_text, options.force)
        .and_then(|()| write_new(&source, RUST_SOURCE_TEXT, options.force))
        .and_then(
            |()| match write_new(&project.join(RUST_IGNORE), RUST_IGNORE_TEXT, false) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                other => other,
            },
        );
    if let Err(error) = written {
        return fail(
            err,
            format!("cannot write the project in {}: {error}", project.display()),
            None,
            "check that the directory is writable, or name another: `gnoblinctl config init --language rust DIR`",
        );
    }
    // The commands name absolute paths, so they run from any directory.
    let absolute = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let project_path = absolute(&project);
    let manifest_path = absolute(&manifest);
    let config_dir = absolute(dir);
    let library = project_path.join("target/debug").join(RUST_LIBRARY);
    let build = format!(
        "cargo build --manifest-path {}",
        shell_word(&manifest_path.display().to_string())
    );
    let run = format!("gnoblin {}", shell_word(&library.display().to_string()));
    let install = format!(
        "cp {} {}",
        shell_word(&library.display().to_string()),
        shell_word(&config_dir.join("shell.so").display().to_string())
    );
    let next = vec![
        format!("Open {} in your editor.", absolute(&source).display()),
        format!("Build it with `{build}`."),
        format!("Run it once, from a terminal, with `{run}`."),
        format!(
            "To run it at every start, copy the library into the config directory with `{install}`, or set \
             `[shell] path = \"{}\"` in {}.",
            library.display(),
            config_dir.join("config.toml").display()
        ),
    ];
    let theme = js::theme(io::stdout().is_terminal());
    let lines = [&manifest_path, &absolute(&source)]
        .map(|path| status_line(&theme, LineOutcome::Ok, &format!("written: {}", path.display())))
        .join("\n");
    let document = json!({
        "language": options.language.name(),
        "init": absolute(&source),
        "manifest": manifest_path,
        "project": project_path,
        "replaced": replaced,
        "build": build,
        "library": library,
        "next": next,
    });
    let verb = if replaced { "replaced" } else { "written" };
    report(
        options,
        document,
        &format!("Rust shell project {verb}: {}", project_path.display()),
        &lines,
        &next,
        out,
    )
}
