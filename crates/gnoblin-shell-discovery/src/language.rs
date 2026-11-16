//! The language a shell is written in, as far as discovery can tell.
//!
//! Detection is the first four steps of the loader's table: a `language` key in a launcher file or in `[shell]`
//! wins; else a file that starts with the ELF magic is native; else the extension decides. Nothing reads a shebang.
//! What to do with an [`Language::Other`] (look for a `libgnoblin-runtime-X.so`) is the loader's business, not
//! discovery's.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// The extensions of a module file, in the order the conventional names `shell.<ext>` are looked up and listed.
pub const KNOWN_EXTENSIONS: [&str; 6] = ["so", "py", "js", "mjs", "ts", "mts"];

/// The first four bytes of every ELF file.
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// How a shell is loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Language {
    /// A shared library: Rust, C, or anything that exports the shell entry symbol. Extension `.so`.
    Native,
    /// Python, run by the Python runtime module. Extension `.py`.
    Python,
    /// JavaScript, run by the JavaScript runtime module. Extensions `.js`, `.mjs`.
    JavaScript,
    /// TypeScript, run by the JavaScript runtime module. Extensions `.ts`, `.mts`.
    TypeScript,
    /// Any other name: a file with another extension, or another value of `language`. The loader looks for a
    /// runtime module named for it. The text is lowercase and may be empty for a file with no extension.
    Other(String),
}

impl Language {
    /// The name a row, a log line and the `language` key use: `native`, `python`, `js`, `ts`, or the other name.
    pub fn as_str(&self) -> &str {
        match self {
            Language::Native => "native",
            Language::Python => "python",
            Language::JavaScript => "js",
            Language::TypeScript => "ts",
            Language::Other(name) if name.is_empty() => "unknown",
            Language::Other(name) => name,
        }
    }

    /// The value of a `language` key: `native`, `python`, `js` or `ts`. Any other text is [`Language::Other`], which
    /// the loader looks up as a runtime module, so a new language needs no change here.
    pub fn from_name(name: &str) -> Language {
        match name {
            "native" => Language::Native,
            "python" => Language::Python,
            "js" => Language::JavaScript,
            "ts" => Language::TypeScript,
            other => Language::Other(other.to_owned()),
        }
    }

    /// The language of a known extension, or `None` for one that is not a module extension of
    /// [`KNOWN_EXTENSIONS`]. Case matters: `.PY` is not a Python shell.
    pub fn of_known_extension(extension: &str) -> Option<Language> {
        match extension {
            "so" => Some(Language::Native),
            "py" => Some(Language::Python),
            "js" | "mjs" => Some(Language::JavaScript),
            "ts" | "mts" => Some(Language::TypeScript),
            _ => None,
        }
    }

    /// The language the extension of `path` names. An unknown extension is [`Language::Other`] with the extension
    /// as its name; no extension is `Other("")`.
    pub fn of_path(path: &Path) -> Language {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        Language::of_known_extension(extension).unwrap_or_else(|| Language::Other(extension.to_owned()))
    }

    /// The language of the file at `path`: native when it starts with the ELF magic whatever it is called, else by
    /// extension. An unreadable file is classified by its extension alone, so a missing library is a load error
    /// with a name, not a language error.
    pub fn detect(path: &Path) -> Language {
        let mut magic = [0_u8; 4];
        let is_elf = File::open(path)
            .and_then(|mut file| file.read_exact(&mut magic))
            .is_ok()
            && magic == ELF_MAGIC;
        if is_elf {
            Language::Native
        } else {
            Language::of_path(path)
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_extension_names_its_language() {
        for (extension, language) in [
            ("so", Language::Native),
            ("py", Language::Python),
            ("js", Language::JavaScript),
            ("mjs", Language::JavaScript),
            ("ts", Language::TypeScript),
            ("mts", Language::TypeScript),
        ] {
            assert_eq!(
                Language::of_known_extension(extension),
                Some(language.clone()),
                "{extension}"
            );
            assert_eq!(Language::of_path(Path::new(&format!("/x/shell.{extension}"))), language);
        }
    }

    #[test]
    fn the_known_extensions_are_exactly_the_ones_that_name_a_language() {
        for extension in KNOWN_EXTENSIONS {
            assert!(Language::of_known_extension(extension).is_some(), "{extension}");
        }
    }

    #[test]
    fn an_unknown_extension_is_named_for_the_loader() {
        assert_eq!(
            Language::of_path(Path::new("/x/shell.lua")),
            Language::Other("lua".to_owned())
        );
        assert_eq!(Language::of_path(Path::new("/x/shell")), Language::Other(String::new()));
        assert_eq!(
            Language::of_path(Path::new("/x/shell.PY")),
            Language::Other("PY".to_owned())
        );
    }

    #[test]
    fn names_round_trip() {
        for name in ["native", "python", "js", "ts", "lua"] {
            assert_eq!(Language::from_name(name).as_str(), name);
        }
        assert_eq!(Language::of_path(Path::new("x")).as_str(), "unknown");
    }
}
