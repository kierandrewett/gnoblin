//! A reviewed-exception file read by several xtask rules: a TOML list of `[[allow]]` entries, each
//! naming a file or a directory and the reason it is exempt. A rule decides what an entry covers;
//! this module only loads the file and checks that every entry says why.

use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowFile {
    #[serde(default)]
    allow: Vec<Allow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allow {
    /// A file or directory, relative to the repository root, with `/` separators.
    pub path: String,
    pub reason: String,
}

/// Load the entries of the exception file at `file`, relative to the repository root, rejecting
/// any entry with a blank reason.
pub fn load(root: &Path, file: &str) -> Result<Vec<Allow>, String> {
    let path = root.join(file);
    let text = fs::read_to_string(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let parsed: AllowFile =
        toml::from_str(&text).map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;
    for entry in &parsed.allow {
        if entry.reason.trim().is_empty() {
            return Err(format!("{file}: `{}` has no reason", entry.path));
        }
    }
    Ok(parsed.allow)
}

/// Whether an entry covers a relative path: the entry itself, or any path below it when the entry
/// names a directory.
pub fn covers(entry: &str, relative: &str) -> bool {
    let entry = entry.trim_end_matches('/');
    relative == entry || relative.starts_with(&format!("{entry}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_covers_its_file_or_its_directory_only() {
        assert!(covers("crates/gnoblin-parity", "crates/gnoblin-parity/src/main.rs"));
        assert!(covers(
            "crates/gnoblin-parity/src/main.rs",
            "crates/gnoblin-parity/src/main.rs"
        ));
        assert!(!covers(
            "crates/gnoblin-parity",
            "crates/gnoblin-parity-extra/src/lib.rs"
        ));
        assert!(!covers(
            "crates/gnoblin/src/compat.rs",
            "crates/gnoblin/src/compat_report.rs"
        ));
    }

    #[test]
    fn an_entry_without_a_reason_is_rejected() {
        let dir = std::env::temp_dir().join(format!("xtask-allow-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("allow.toml"), "[[allow]]\npath = \"a\"\nreason = \"  \"\n").unwrap();
        let error = load(&dir, "allow.toml").err().expect("a blank reason must fail");
        assert!(error.contains("`a` has no reason"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
