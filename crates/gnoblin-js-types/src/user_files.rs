//! The files an editor needs in the user's configuration directory: `gnoblin.d.ts` and a
//! `tsconfig.json` that finds it.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::render::render_dts;
use crate::stock::stock_catalog;

/// The declaration file's name in the configuration directory.
pub const TYPES_FILE: &str = "gnoblin.d.ts";
/// The editor configuration's name in the configuration directory.
pub const TSCONFIG_FILE: &str = "tsconfig.json";

/// What [`ensure_user_files`] left in the configuration directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsWritten {
    /// The declaration file, which is always current after the call.
    pub types: PathBuf,
    /// The `tsconfig.json`, when this call created it. `None` means the user's file was
    /// already there and was left alone.
    pub tsconfig: Option<PathBuf>,
}

/// The `tsconfig.json` an editor needs to check a configuration written in TypeScript.
///
/// `types_path` is where the declaration file sits, as the `tsconfig.json` should name it
/// (relative to the `tsconfig.json`, or absolute). The module `gnoblin` is mapped to it with
/// `paths`, and it is listed in `files` so the editor loads it whatever `include` matches.
///
/// The options follow design/js-runtime.md: strict, ES modules resolved the way a bundler
/// does, `.ts` extensions importable (the runtime resolves them), and no emit, because
/// nothing here compiles: the runtime strips types itself.
#[must_use]
pub fn render_tsconfig(types_path: &Path) -> String {
    let types = types_path.to_string_lossy();
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
            // No `@types` discovery: the one global is declared by the file named below.
            "types": [],
            "paths": { "gnoblin": [types] },
        },
        "files": [types],
        "include": ["./**/*.ts"],
    });
    // A `Value` always serialises; the fallback keeps the function total without a panic.
    let mut text = serde_json::to_string_pretty(&config).unwrap_or_default();
    text.push('\n');
    text
}

/// Write the editor files into `config_dir`, creating the directory when it is missing.
///
/// `gnoblin.d.ts` is generated from the stock schema and refreshed on every call, so a
/// compositor upgrade reaches the editor without a step; it is rewritten only when its
/// bytes changed, so a watcher on the directory sees no event on an ordinary start.
/// `tsconfig.json` belongs to the user once it exists: it is written only when absent.
///
/// # Errors
///
/// An [`io::Error`] when the directory or a file cannot be written, or when the embedded
/// schema does not load.
pub fn ensure_user_files(config_dir: &Path) -> io::Result<JsWritten> {
    let catalog = stock_catalog().map_err(io::Error::other)?;
    let types_text = render_dts(&catalog);

    fs::create_dir_all(config_dir)?;
    let types = config_dir.join(TYPES_FILE);
    let current = match fs::read_to_string(&types) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if current.as_deref() != Some(types_text.as_str()) {
        fs::write(&types, types_text)?;
    }

    let tsconfig_path = config_dir.join(TSCONFIG_FILE);
    // `create_new` makes "absent" and "create" one step, so a file that appears between a
    // check and a write is never overwritten.
    let tsconfig = match fs::OpenOptions::new().write(true).create_new(true).open(&tsconfig_path) {
        Ok(mut file) => {
            file.write_all(render_tsconfig(Path::new(&format!("./{TYPES_FILE}"))).as_bytes())?;
            Some(tsconfig_path)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
        Err(error) => return Err(error),
    };
    Ok(JsWritten { types, tsconfig })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("the tsconfig is JSON")
    }

    #[test]
    fn the_tsconfig_is_strict_noemit_bundler_resolution() {
        let config = parse(&render_tsconfig(Path::new("./gnoblin.d.ts")));
        let options = &config["compilerOptions"];
        assert_eq!(options["strict"], true);
        assert_eq!(options["module"], "esnext");
        assert_eq!(options["moduleResolution"], "bundler");
        assert_eq!(options["noEmit"], true);
        assert_eq!(options["allowImportingTsExtensions"], true);
        assert_eq!(options["paths"]["gnoblin"][0], "./gnoblin.d.ts");
        assert_eq!(config["files"][0], "./gnoblin.d.ts");
    }

    #[test]
    fn the_tsconfig_escapes_the_types_path() {
        let config = parse(&render_tsconfig(Path::new("/home/a \"b\"/gnoblin.d.ts")));
        assert_eq!(
            config["compilerOptions"]["paths"]["gnoblin"][0],
            "/home/a \"b\"/gnoblin.d.ts"
        );
    }

    #[test]
    fn the_tsconfig_is_the_same_text_every_time() {
        let path = Path::new("./gnoblin.d.ts");
        assert_eq!(render_tsconfig(path), render_tsconfig(path));
    }

    #[test]
    fn an_empty_directory_gets_both_files() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let written = ensure_user_files(dir.path()).expect("the files are written");
        assert_eq!(written.types, dir.path().join(TYPES_FILE));
        assert_eq!(written.tsconfig, Some(dir.path().join(TSCONFIG_FILE)));
        let types = fs::read_to_string(&written.types).expect("the types are readable");
        assert!(types.contains("declare module \"gnoblin\""));
        let config = parse(&fs::read_to_string(dir.path().join(TSCONFIG_FILE)).expect("tsconfig"));
        assert_eq!(config["compilerOptions"]["paths"]["gnoblin"][0], "./gnoblin.d.ts");
    }

    #[test]
    fn a_missing_directory_is_created() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let nested = dir.path().join("config").join("gnoblin");
        ensure_user_files(&nested).expect("the files are written");
        assert!(nested.join(TYPES_FILE).is_file());
    }

    #[test]
    fn the_declarations_are_refreshed_and_the_tsconfig_is_kept() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        ensure_user_files(dir.path()).expect("the first call");
        fs::write(dir.path().join(TYPES_FILE), "stale").expect("stale types");
        fs::write(dir.path().join(TSCONFIG_FILE), "{ \"mine\": true }").expect("user tsconfig");

        let written = ensure_user_files(dir.path()).expect("the second call");

        assert_eq!(written.tsconfig, None, "an existing tsconfig is not rewritten");
        assert_eq!(
            fs::read_to_string(dir.path().join(TSCONFIG_FILE)).expect("tsconfig"),
            "{ \"mine\": true }"
        );
        let types = fs::read_to_string(dir.path().join(TYPES_FILE)).expect("types");
        assert_ne!(types, "stale");
        assert!(types.starts_with("// gnoblin.d.ts"));
    }

    #[test]
    fn current_declarations_are_not_rewritten() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        ensure_user_files(dir.path()).expect("the first call");
        let path = dir.path().join(TYPES_FILE);
        let before = fs::metadata(&path).and_then(|meta| meta.modified()).expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure_user_files(dir.path()).expect("the second call");
        let after = fs::metadata(&path).and_then(|meta| meta.modified()).expect("mtime");
        assert_eq!(before, after);
    }

    #[test]
    fn a_file_where_the_directory_should_be_is_an_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").expect("a file");
        assert!(ensure_user_files(&blocker.join("config")).is_err());
    }
}
