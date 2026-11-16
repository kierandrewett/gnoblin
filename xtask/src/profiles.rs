//! The `profile-unwind` check, run by `cargo xtask deps`: no Cargo profile may set `panic = "abort"`.
//!
//! The shell host contains a panic at every boundary it calls (design/modular-framework.md, 5.7): a
//! shell callback, a window-manager provider and a runtime module each run under `catch_unwind`, and
//! `gnoblin-js`'s supervisor counts a panic as a strike. An aborting profile turns the first of them
//! into the end of the session, silently, so the root manifest states `panic = "unwind"` and this
//! check rejects the opposite in the places Cargo reads a profile from: the workspace manifest and
//! the workspace's Cargo configuration.

use std::fs;
use std::path::Path;

/// Where a profile may be declared, relative to the workspace root.
const SOURCES: &[&str] = &["Cargo.toml", ".cargo/config.toml"];

/// Every profile that sets `panic = "abort"`, one line each. A missing file has no profiles.
pub fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for source in SOURCES {
        let path = root.join(source);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        found.extend(abort_profiles(source, &text)?);
    }
    Ok(found)
}

/// The profiles in one file's text that set `panic = "abort"`.
fn abort_profiles(source: &str, text: &str) -> Result<Vec<String>, String> {
    let document: toml::Table = toml::from_str(text).map_err(|error| format!("invalid TOML in {source}: {error}"))?;
    let Some(profiles) = document.get("profile").and_then(toml::Value::as_table) else {
        return Ok(Vec::new());
    };
    Ok(profiles
        .iter()
        .filter(|(_, profile)| profile.get("panic").and_then(toml::Value::as_str) == Some("abort"))
        .map(|(name, _)| {
            format!("{source}: `[profile.{name}]` sets `panic = \"abort\"`; core and shells need unwinding panics")
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_that_aborts_is_reported_by_name() {
        let text = "[profile.release]\npanic = \"abort\"\n\n[profile.dev]\npanic = \"unwind\"\n";
        let found = abort_profiles("Cargo.toml", text).expect("parses");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("[profile.release]"), "{found:?}");
    }

    #[test]
    fn a_custom_profile_that_aborts_is_reported() {
        let found = abort_profiles(
            "Cargo.toml",
            "[profile.dist]\ninherits = \"release\"\npanic = \"abort\"\n",
        )
        .unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn unwinding_and_unset_profiles_pass() {
        let text = "[profile.dev]\npanic = \"unwind\"\n\n[profile.release]\nopt-level = 3\n";
        assert_eq!(abort_profiles("Cargo.toml", text), Ok(Vec::new()));
        assert_eq!(
            abort_profiles("Cargo.toml", "[package]\nname = \"a\"\n"),
            Ok(Vec::new())
        );
    }

    #[test]
    fn the_real_workspace_sets_unwinding_panics_and_no_abort() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert_eq!(check(&root), Ok(Vec::new()));
        let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("root manifest");
        for profile in ["dev", "release"] {
            let document: toml::Table = toml::from_str(&manifest).expect("root manifest parses");
            let panic = document
                .get("profile")
                .and_then(|profiles| profiles.get(profile))
                .and_then(|profile| profile.get("panic"))
                .and_then(toml::Value::as_str);
            assert_eq!(panic, Some("unwind"), "[profile.{profile}] states its panic strategy");
        }
    }
}
