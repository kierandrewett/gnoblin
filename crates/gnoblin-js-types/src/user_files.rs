//! The files an editor needs in the user's configuration directory.
//!
//! The types of the library are its real TypeScript sources, so they are written out as files and the type checker is
//! pointed at them; nothing declares the library a second time. The layout, under the configuration directory:
//!
//! ```text
//! tsconfig.json              the user's: extends .gnoblin/tsconfig.json; written once, never touched again
//! .gnoblin/                  everything below is rewritten on every call; commit none of it
//!   .gitignore               "*", so a dotfiles repository ignores the directory
//!   VERSION                  the version that wrote the directory
//!   tsconfig.json            strict ES2023, bundler resolution, and the two `paths` that map the library
//!   gnoblin.d.ts             the globals (`gnoblin`, `console`)
//!   lib/gnoblin-core/**.ts   the sources of `gnoblin-core`: `gnoblin-core` is lib/gnoblin-core/index.ts,
//!                            `gnoblin-core/host/client` is lib/gnoblin-core/host/client.ts, ...
//!   lib/gnoblin-std/**.ts    the sources of `gnoblin-std`: `gnoblin-std/scene/node` is
//!                            lib/gnoblin-std/scene/node.ts, ...
//! ```
//!
//! A refresh reaches the editor without a step: the sources, the declarations and the generated `tsconfig.json` are
//! the ones of the running binary after any call, a file whose bytes already match is left alone (a watcher on the
//! directory sees no event on an ordinary start), and a source the library no longer has is removed so that its import
//! stops resolving. The user's `tsconfig.json` is theirs once it exists.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::render::render_dts;

/// The directory under the configuration directory that holds everything generated.
pub const GENERATED_DIR: &str = ".gnoblin";
/// The declaration file's name in [`GENERATED_DIR`].
pub const TYPES_FILE: &str = "gnoblin.d.ts";
/// The user's editor configuration, in the configuration directory.
pub const TSCONFIG_FILE: &str = "tsconfig.json";
/// The generated editor configuration, in [`GENERATED_DIR`]: the one the user's extends.
pub const GENERATED_TSCONFIG_FILE: &str = "tsconfig.json";
/// Where the library sources go, relative to [`GENERATED_DIR`].
pub const LIBRARY_DIR: &str = "lib";
/// The file in [`GENERATED_DIR`] that names the version that wrote it.
pub const VERSION_FILE: &str = "VERSION";

/// What [`ensure_user_files`] did to the user's `tsconfig.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TsconfigOutcome {
    /// The file was absent and this call wrote it.
    Created(PathBuf),
    /// The file was the one an earlier release wrote, which names a `gnoblin.d.ts` that no longer holds the library;
    /// this call wrote the current one over it.
    Migrated(PathBuf),
    /// The file is the user's and extends the generated configuration; it was left alone.
    Kept,
    /// The file is the user's and does not extend the generated configuration, so the editor does not see the types.
    /// It was left alone: the caller says what to add.
    NotLinked,
}

/// What [`ensure_user_files`] left in the configuration directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsWritten {
    /// The generated directory, `<config>/.gnoblin`.
    pub generated: PathBuf,
    /// The declaration file, which is always current after the call.
    pub types: PathBuf,
    /// The directory the library sources were written to: `import ... from "gnoblin-std/x"` is `<library>/gnoblin-std/x.ts`.
    pub library: PathBuf,
    /// How many library sources the directory holds.
    pub library_files: usize,
    /// The user's `tsconfig.json`: written, replaced, or left alone.
    pub tsconfig: TsconfigOutcome,
}

/// The first line of the `gnoblin.d.ts` that earlier releases wrote beside `shell.ts`. A file that starts with it is
/// ours and is removed once the declarations live in [`GENERATED_DIR`]; a file the user wrote there is not touched.
const LEGACY_TYPES_HEADER: &str = "// gnoblin.d.ts: the types of the Gnoblin JavaScript and TypeScript API.";

/// The generated `tsconfig.json`: what an editor or `tsc` needs to check a configuration written in TypeScript.
///
/// The options follow design/js-runtime.md: strict, ES modules resolved the way a bundler does, `.ts` extensions
/// importable (the runtime resolves them), and no emit, because nothing here compiles: the runtime strips types itself.
/// Each package, `gnoblin-core` and `gnoblin-std`, is mapped with its submodules to the real sources
/// (`lib/gnoblin-core/index.ts` and `lib/gnoblin-core/*`, the same for `gnoblin-std`), which is what makes
/// go-to-definition open them and `Camera3D` extend the one `Node`. The declaration file is listed in
/// `files` so the editor loads it whatever the user's `include` matches. Every path is relative to this file.
#[must_use]
pub fn render_generated_tsconfig() -> String {
    let config = json!({
        "compilerOptions": {
            "target": "es2023",
            "lib": ["es2023"],
            "module": "esnext",
            "moduleResolution": "bundler",
            "strict": true,
            "noEmit": true,
            "isolatedModules": true,
            "allowImportingTsExtensions": true,
            // No `@types` discovery: the globals are declared by the file named below.
            "types": [],
            "paths": {
                "gnoblin-core": [format!("./{LIBRARY_DIR}/gnoblin-core/index.ts")],
                "gnoblin-core/*": [format!("./{LIBRARY_DIR}/gnoblin-core/*")],
                "gnoblin-std": [format!("./{LIBRARY_DIR}/gnoblin-std/index.ts")],
                "gnoblin-std/*": [format!("./{LIBRARY_DIR}/gnoblin-std/*")],
            },
        },
        "files": [format!("./{TYPES_FILE}")],
    });
    pretty(&config)
}

/// The user's `tsconfig.json` when there is none: it extends the generated one and leaves what to check to the user.
#[must_use]
pub fn render_user_tsconfig() -> String {
    let config = json!({
        "extends": format!("./{GENERATED_DIR}/{GENERATED_TSCONFIG_FILE}"),
        "include": ["./**/*.ts"],
    });
    pretty(&config)
}

fn pretty(config: &Value) -> String {
    // A `Value` always serialises; the fallback keeps the function total without a panic.
    let mut text = serde_json::to_string_pretty(config).unwrap_or_default();
    text.push('\n');
    text
}

/// The `tsconfig.json` that earlier releases wrote: it maps `gnoblin` to the one `gnoblin.d.ts` beside it. Compared as
/// a value, so the order of its keys does not matter.
fn legacy_user_tsconfig() -> Value {
    json!({
        "compilerOptions": {
            "target": "es2023",
            "lib": ["es2023"],
            "module": "esnext",
            "moduleResolution": "bundler",
            "strict": true,
            "noEmit": true,
            "isolatedModules": true,
            "allowImportingTsExtensions": true,
            "types": [],
            "paths": { "gnoblin": ["./gnoblin.d.ts"] },
        },
        "files": ["./gnoblin.d.ts"],
        "include": ["./**/*.ts"],
    })
}

/// Write the editor files into `config_dir`, creating the directory when it is missing.
///
/// `library` is the real library: `(path under lib/, TypeScript)` for every module, such as
/// `("gnoblin-std/scene/node.ts", "...")` (`gnoblin_js::editor_library` is the stock one). It is written as given, so
/// the files are the sources and not a copy that could be edited into a second truth. See the module documentation for
/// the layout and what is rewritten.
///
/// # Errors
///
/// An [`io::Error`] when the directory or a file cannot be written, or of kind [`io::ErrorKind::InvalidInput`] when a
/// library path is absolute or leaves the library directory.
pub fn ensure_user_files(config_dir: &Path, library: &[(String, String)]) -> io::Result<JsWritten> {
    let wanted = library_paths(library)?;

    let generated = config_dir.join(GENERATED_DIR);
    let library_dir = generated.join(LIBRARY_DIR);
    fs::create_dir_all(&library_dir)?;
    for (file, text) in library {
        let path = library_dir.join(file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_if_changed(&path, text)?;
    }
    prune(&library_dir, &library_dir, &wanted)?;

    let types = generated.join(TYPES_FILE);
    write_if_changed(&types, &render_dts())?;
    write_if_changed(&generated.join(GENERATED_TSCONFIG_FILE), &render_generated_tsconfig())?;
    write_if_changed(
        &generated.join(VERSION_FILE),
        &format!("gnoblin {}\n", env!("CARGO_PKG_VERSION")),
    )?;
    // The directory is generated: a dotfiles repository that holds the configuration must not pick it up.
    write_if_changed(&generated.join(".gitignore"), "*\n")?;

    remove_legacy_types(&config_dir.join(TYPES_FILE))?;
    let tsconfig = ensure_user_tsconfig(&config_dir.join(TSCONFIG_FILE))?;
    Ok(JsWritten {
        generated,
        types,
        library: library_dir,
        library_files: library.len(),
        tsconfig,
    })
}

/// The library's paths as the set the writer keeps, after checking that each stays inside the library directory.
fn library_paths(library: &[(String, String)]) -> io::Result<BTreeSet<PathBuf>> {
    library
        .iter()
        .map(|(file, _)| {
            let path = PathBuf::from(file);
            let contained = path.components().all(|part| matches!(part, Component::Normal(_)));
            if path.as_os_str().is_empty() || !contained {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("the library file {file:?} is not a relative path inside the library"),
                ));
            }
            Ok(path)
        })
        .collect()
}

/// Write `text` to `path` unless the file already holds exactly those bytes.
fn write_if_changed(path: &Path, text: &str) -> io::Result<()> {
    match fs::read(path) {
        Ok(current) if current == text.as_bytes() => Ok(()),
        Ok(_) => fs::write(path, text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::write(path, text),
        Err(error) => Err(error),
    }
}

/// Remove what the library no longer has from `dir`: a file not in `wanted` (paths relative to `root`), and a directory
/// that is left empty. Left in place, a module the library dropped would still resolve for the editor.
fn prune(root: &Path, dir: &Path, wanted: &BTreeSet<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // `file_type` does not follow a link, so a link someone put here is removed and never walked through.
        if entry.file_type()?.is_dir() {
            prune(root, &path, wanted)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)?;
            }
        } else if !path.strip_prefix(root).is_ok_and(|relative| wanted.contains(relative)) {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Remove the `gnoblin.d.ts` an earlier release wrote beside `shell.ts`, when it is that file: it declares `gnoblin`
/// again and would clash with the declarations in [`GENERATED_DIR`] for any `include` that reaches it.
fn remove_legacy_types(path: &Path) -> io::Result<()> {
    match fs::read_to_string(path) {
        Ok(text) if text.starts_with(LEGACY_TYPES_HEADER) => fs::remove_file(path),
        Ok(_) => Ok(()),
        // A file that is not text is not ours.
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::InvalidData) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Create the user's `tsconfig.json` when absent; replace the one an earlier release wrote; leave any other alone.
fn ensure_user_tsconfig(path: &Path) -> io::Result<TsconfigOutcome> {
    // `create_new` makes "absent" and "create" one step, so a file that appears between a check and a write is never
    // overwritten.
    match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(render_user_tsconfig().as_bytes())?;
            return Ok(TsconfigOutcome::Created(path.to_path_buf()));
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let text = fs::read_to_string(path)?;
    if serde_json::from_str::<Value>(&text).is_ok_and(|config| config == legacy_user_tsconfig()) {
        fs::write(path, render_user_tsconfig())?;
        return Ok(TsconfigOutcome::Migrated(path.to_path_buf()));
    }
    // Anything that names the generated file extends it, whatever else the user wrote around it.
    let generated = format!("{GENERATED_DIR}/{GENERATED_TSCONFIG_FILE}");
    Ok(if text.contains(&generated) {
        TsconfigOutcome::Kept
    } else {
        TsconfigOutcome::NotLinked
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).expect("the tsconfig is JSON")
    }

    /// A two-module library with a nested file, enough to show the layout.
    fn library() -> Vec<(String, String)> {
        vec![
            (
                "gnoblin-std/index.ts".to_owned(),
                "export * from \"gnoblin-std/scene\";\n".to_owned(),
            ),
            (
                "gnoblin-std/scene.ts".to_owned(),
                "export { Node } from \"gnoblin-std/scene/node\";\n".to_owned(),
            ),
            (
                "gnoblin-std/scene/node.ts".to_owned(),
                "export class Node {}\n".to_owned(),
            ),
        ]
    }

    fn files(config: &Path) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, into: &mut Vec<String>) {
            for entry in fs::read_dir(dir).expect("a directory") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    walk(&path, root, into);
                } else {
                    into.push(
                        path.strip_prefix(root)
                            .expect("under root")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        let mut found = Vec::new();
        walk(config, config, &mut found);
        found.sort();
        found
    }

    #[test]
    fn the_generated_tsconfig_is_strict_noemit_bundler_resolution_and_maps_the_real_sources() {
        let config = parse(&render_generated_tsconfig());
        let options = &config["compilerOptions"];
        assert_eq!(options["strict"], true);
        assert_eq!(options["module"], "esnext");
        assert_eq!(options["moduleResolution"], "bundler");
        assert_eq!(options["noEmit"], true);
        assert_eq!(options["allowImportingTsExtensions"], true);
        assert_eq!(options["paths"]["gnoblin-core"][0], "./lib/gnoblin-core/index.ts");
        assert_eq!(options["paths"]["gnoblin-core/*"][0], "./lib/gnoblin-core/*");
        assert_eq!(options["paths"]["gnoblin-std"][0], "./lib/gnoblin-std/index.ts");
        assert_eq!(options["paths"]["gnoblin-std/*"][0], "./lib/gnoblin-std/*");
        assert_eq!(config["files"][0], "./gnoblin.d.ts");
    }

    #[test]
    fn the_user_tsconfig_extends_the_generated_one_and_names_nothing_else_of_it() {
        let config = parse(&render_user_tsconfig());
        assert_eq!(config["extends"], "./.gnoblin/tsconfig.json");
        assert_eq!(config["include"][0], "./**/*.ts");
        assert!(
            config.get("compilerOptions").is_none(),
            "an upgrade must reach the options"
        );
    }

    #[test]
    fn the_tsconfigs_are_the_same_text_every_time() {
        assert_eq!(render_generated_tsconfig(), render_generated_tsconfig());
        assert_eq!(render_user_tsconfig(), render_user_tsconfig());
    }

    #[test]
    fn an_empty_directory_gets_the_layout() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let written = ensure_user_files(dir.path(), &library()).expect("the files are written");
        assert_eq!(written.generated, dir.path().join(".gnoblin"));
        assert_eq!(written.types, dir.path().join(".gnoblin/gnoblin.d.ts"));
        assert_eq!(written.library, dir.path().join(".gnoblin/lib"));
        assert_eq!(written.library_files, 3);
        assert_eq!(
            written.tsconfig,
            TsconfigOutcome::Created(dir.path().join(TSCONFIG_FILE))
        );
        assert_eq!(
            files(dir.path()),
            [
                ".gnoblin/.gitignore",
                ".gnoblin/VERSION",
                ".gnoblin/gnoblin.d.ts",
                ".gnoblin/lib/gnoblin-std/index.ts",
                ".gnoblin/lib/gnoblin-std/scene.ts",
                ".gnoblin/lib/gnoblin-std/scene/node.ts",
                ".gnoblin/tsconfig.json",
                "tsconfig.json",
            ]
        );
        assert_eq!(
            fs::read_to_string(written.library.join("gnoblin-std/scene/node.ts")).expect("a source"),
            "export class Node {}\n",
            "the sources are written as given"
        );
        let types = fs::read_to_string(&written.types).expect("the types are readable");
        assert!(types.contains("declare const gnoblin:"));
        assert!(types.contains("declare const console:"));
        assert!(
            !types.contains("declare module"),
            "the file declares the two globals and nothing else"
        );
        let version = fs::read_to_string(written.generated.join(VERSION_FILE)).expect("the version");
        assert_eq!(version, format!("gnoblin {}\n", env!("CARGO_PKG_VERSION")));
        assert_eq!(
            fs::read_to_string(written.generated.join(".gitignore")).expect("ignore"),
            "*\n"
        );
    }

    #[test]
    fn a_missing_directory_is_created() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let nested = dir.path().join("config").join("gnoblin");
        ensure_user_files(&nested, &library()).expect("the files are written");
        assert!(nested.join(".gnoblin/gnoblin.d.ts").is_file());
    }

    #[test]
    fn a_refresh_rewrites_the_generated_files_and_keeps_the_users_tsconfig() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let written = ensure_user_files(dir.path(), &library()).expect("the first call");
        fs::write(&written.types, "stale").expect("stale types");
        fs::write(written.generated.join(GENERATED_TSCONFIG_FILE), "{}").expect("stale tsconfig");
        fs::write(written.library.join("gnoblin-std/scene/node.ts"), "edited").expect("an edited source");
        fs::write(
            dir.path().join(TSCONFIG_FILE),
            "{ \"extends\": \"./.gnoblin/tsconfig.json\", \"mine\": true }",
        )
        .expect("user tsconfig");

        let again = ensure_user_files(dir.path(), &library()).expect("the second call");

        assert_eq!(
            again.tsconfig,
            TsconfigOutcome::Kept,
            "an existing tsconfig is not rewritten"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(TSCONFIG_FILE)).expect("tsconfig"),
            "{ \"extends\": \"./.gnoblin/tsconfig.json\", \"mine\": true }"
        );
        assert!(
            fs::read_to_string(&again.types)
                .expect("types")
                .starts_with("// gnoblin.d.ts")
        );
        assert_eq!(
            fs::read_to_string(again.generated.join(GENERATED_TSCONFIG_FILE)).expect("generated tsconfig"),
            render_generated_tsconfig()
        );
        assert_eq!(
            fs::read_to_string(again.library.join("gnoblin-std/scene/node.ts")).expect("source"),
            "export class Node {}\n",
            "an edited source is the library again"
        );
    }

    #[test]
    fn a_source_the_library_dropped_stops_resolving() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        ensure_user_files(dir.path(), &library()).expect("the first call");
        let kept = vec![("gnoblin-std/index.ts".to_owned(), "export {};\n".to_owned())];
        let written = ensure_user_files(dir.path(), &kept).expect("the second call");
        assert!(written.library.join("gnoblin-std/index.ts").is_file());
        assert!(!written.library.join("gnoblin-std/scene.ts").exists());
        assert!(
            !written.library.join("gnoblin-std/scene").exists(),
            "an emptied directory goes too"
        );
    }

    #[test]
    fn current_files_are_not_rewritten() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let written = ensure_user_files(dir.path(), &library()).expect("the first call");
        let touched = [written.types.clone(), written.library.join("gnoblin-std/scene/node.ts")];
        let mtime = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).expect("mtime");
        let before: Vec<_> = touched.iter().map(|path| mtime(path)).collect();
        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure_user_files(dir.path(), &library()).expect("the second call");
        let after: Vec<_> = touched.iter().map(|path| mtime(path)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_tsconfig_an_earlier_release_wrote_is_replaced() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // The comparison is of values, so the order and the layout of the keys do not matter.
        let legacy = serde_json::to_string_pretty(&legacy_user_tsconfig()).expect("json");
        fs::write(dir.path().join(TSCONFIG_FILE), legacy).expect("legacy tsconfig");
        fs::write(
            dir.path().join(TYPES_FILE),
            format!("{LEGACY_TYPES_HEADER}\ndeclare const gnoblin: 1;\n"),
        )
        .expect("legacy declarations");

        let written = ensure_user_files(dir.path(), &library()).expect("the call");

        assert_eq!(
            written.tsconfig,
            TsconfigOutcome::Migrated(dir.path().join(TSCONFIG_FILE))
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(TSCONFIG_FILE)).expect("tsconfig"),
            render_user_tsconfig()
        );
        assert!(
            !dir.path().join(TYPES_FILE).exists(),
            "the old declarations would clash with the new ones"
        );
    }

    #[test]
    fn a_tsconfig_or_declarations_the_user_wrote_are_not_touched() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        fs::write(dir.path().join(TSCONFIG_FILE), "{ \"mine\": true }").expect("user tsconfig");
        fs::write(dir.path().join(TYPES_FILE), "declare const mine: 1;\n").expect("user declarations");

        let written = ensure_user_files(dir.path(), &library()).expect("the call");

        assert_eq!(written.tsconfig, TsconfigOutcome::NotLinked);
        assert_eq!(
            fs::read_to_string(dir.path().join(TSCONFIG_FILE)).expect("tsconfig"),
            "{ \"mine\": true }"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(TYPES_FILE)).expect("declarations"),
            "declare const mine: 1;\n"
        );
    }

    #[test]
    fn a_library_path_that_leaves_the_library_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        for bad in ["../escape.ts", "/abs.ts", ""] {
            let library = vec![(bad.to_owned(), "x".to_owned())];
            let error = ensure_user_files(dir.path(), &library).expect_err("a path outside the library");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
        assert!(
            !dir.path().join(".gnoblin").exists(),
            "nothing is written before the paths are checked"
        );
    }

    #[test]
    fn a_file_where_the_directory_should_be_is_an_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").expect("a file");
        assert!(ensure_user_files(&blocker.join("config"), &library()).is_err());
    }
}
