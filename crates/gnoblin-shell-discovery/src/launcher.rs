//! Launcher files: `name.toml` in a `shells/` directory, a lightweight reference and not a manifest.
//!
//! ```toml
//! # /usr/share/gnoblin/shells/tiler.toml
//! module = "tiler/libtiler.so"     # relative to <core libdir>/gnoblin/shells/ (arch-correct, multiarch-safe)
//! # path = "/opt/tiler/shell.py"   # alternative: absolute, `~/...`, or relative to this file
//! # language = "native"            # only to override detection
//! description = "A tiling shell"
//! ```
//!
//! Every key is optional but exactly one of `module` and `path`. A relative `path` is relative to the directory
//! that holds the launcher file *itself*: when the launcher is reached through a symlink, that is the directory of
//! the file the link points to, so a link to a launcher in a source tree keeps working.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::candidate::Target;
use crate::environment::Environment;
use crate::language::Language;
use crate::shell_config::describe;

/// The parsed keys of a launcher file.
#[derive(Debug, Default, PartialEq, Eq)]
struct Launcher {
    module: Option<String>,
    path: Option<String>,
    language: Option<Language>,
    description: Option<String>,
    warnings: Vec<String>,
}

/// Read the launcher file and find the module or script it names.
///
/// The error is one line that says what is wrong with the launcher or its target; the caller adds the file name.
pub(crate) fn resolve(file: &Path, environment: &Environment) -> Result<Target, String> {
    let text = fs::read_to_string(file).map_err(|error| format!("cannot read it: {error}"))?;
    let launcher = parse(&text)?;
    let real = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let directory = real.parent().unwrap_or(Path::new("/"));
    let path = match (&launcher.module, &launcher.path) {
        (Some(module), None) => environment
            .module_dir
            .as_ref()
            .ok_or_else(|| {
                "`module` is relative to the library directory, which is not known here; use `path`".to_owned()
            })?
            .join(module),
        (None, Some(path)) => {
            let expanded = environment
                .expand_home(path)
                .ok_or_else(|| "`path` starts with `~` and there is no home directory".to_owned())?;
            if expanded.is_absolute() {
                expanded
            } else {
                directory.join(expanded)
            }
        }
        (Some(_), Some(_)) => return Err("it has both `module` and `path`; keep one".to_owned()),
        (None, None) => return Err("it has neither `module` nor `path`".to_owned()),
    };
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(format!("its target {} is a directory; name the file", path.display()));
        }
        Ok(_) => {}
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Err(format!("its target {} does not exist", path.display()));
        }
        Err(error) => return Err(format!("its target {} cannot be read: {error}", path.display())),
    }
    if path.extension().is_some_and(|extension| extension == "toml") {
        return Err(format!("its target {} is another launcher file", path.display()));
    }
    let language = launcher.language.unwrap_or_else(|| Language::detect(&path));
    Ok(Target {
        path,
        language,
        launcher: Some(file.to_path_buf()),
        description: launcher.description,
        warnings: launcher.warnings,
    })
}

/// Parse the text of a launcher file. The error is one line.
fn parse(text: &str) -> Result<Launcher, String> {
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| describe(text, &error))?;
    let mut launcher = Launcher::default();
    for (key, value) in &table {
        let string = || match value {
            toml::Value::String(text) => Ok(text.clone()),
            _ => Err(format!("`{key}` must be a string")),
        };
        match key.as_str() {
            "module" => launcher.module = Some(non_empty(key, string()?)?),
            "path" => launcher.path = Some(non_empty(key, string()?)?),
            "language" => launcher.language = Some(Language::from_name(&non_empty(key, string()?)?)),
            "description" => launcher.description = Some(string()?),
            other => launcher.warnings.push(format!("unknown key `{other}`")),
        }
    }
    Ok(launcher)
}

fn non_empty(key: &str, text: String) -> Result<String, String> {
    if text.is_empty() {
        Err(format!("`{key}` is empty"))
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_read() {
        let launcher =
            parse("module = \"t/libt.so\"\nlanguage = \"native\"\ndescription = \"A tiler\"\n").expect("valid");
        assert_eq!(launcher.module.as_deref(), Some("t/libt.so"));
        assert_eq!(launcher.language, Some(Language::Native));
        assert_eq!(launcher.description.as_deref(), Some("A tiler"));
    }

    #[test]
    fn an_unknown_key_is_a_warning() {
        let launcher = parse("path = \"/a\"\nauthor = \"me\"\n").expect("valid");
        assert_eq!(launcher.warnings, vec!["unknown key `author`".to_owned()]);
    }

    #[test]
    fn a_key_of_the_wrong_type_or_an_empty_one_is_an_error() {
        assert_eq!(parse("path = 3\n"), Err("`path` must be a string".to_owned()));
        assert_eq!(parse("module = \"\"\n"), Err("`module` is empty".to_owned()));
    }

    #[test]
    fn a_syntax_error_names_its_line() {
        let error = parse("path = \"/a\"\nmodule = \n").expect_err("not TOML");
        assert!(error.starts_with("line "), "{error}");
    }
}
