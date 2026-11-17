//! Reading `cargo metadata` output.
//!
//! The dependency rules are statements about declared edges, so only the
//! package names and their `dependencies` arrays are needed. That lets the check
//! run with `--no-deps`: no registry access, no resolution and no lockfile
//! churn, which keeps it fast and offline on a build node.

use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// The packages of one workspace and the dependency edges they declare.
#[derive(Debug)]
pub struct Graph {
    pub packages: Vec<Package>,
}

#[derive(Debug)]
pub struct Package {
    pub name: String,
    pub deps: Vec<Dep>,
    /// Whether the package has a binary target. Only a binary may be a composition root, so a rule
    /// that gates a feature on the root reads this.
    pub is_binary: bool,
    /// How the manifest declares its lint levels. `cargo metadata` does not report them, so the
    /// manifest is read when the document names it.
    pub lints: Lints,
}

/// How a manifest declares its lints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lints {
    /// The document gave no manifest path (a hand-written fixture), so nothing was read.
    Unread,
    /// `[lints] workspace = true`: the workspace table, `unsafe_code = "forbid"` included.
    Inherited,
    /// A table of the crate's own.
    Own {
        /// The level of `lints.rust.unsafe_code`, when set.
        unsafe_code: Option<String>,
        /// Whether `lints.clippy.all` is `deny` at priority -1, as the workspace table has it.
        clippy_all_denied: bool,
    },
    /// No `[lints]` table at all.
    Missing,
}

#[derive(Debug)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// The features this package enables on the dependency.
    pub features: Vec<String>,
}

/// How a dependency is declared. `Normal` is a regular build and runtime
/// dependency; `Dev` and `Build` are the kinds that do not ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

impl DepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DepKind::Normal => "normal",
            DepKind::Dev => "development",
            DepKind::Build => "build",
        }
    }
}

/// Read the workspace that contains `directory`.
///
/// `cargo metadata` resolves the workspace by walking up from the working
/// directory, so `cargo xtask deps` checks the tree it was run in, not the tree
/// the xtask binary was compiled in. That distinction matters: a copied or
/// renamed checkout with a shared `target/` would otherwise be checked against
/// the wrong metadata and pass a rule it breaks.
pub fn load(directory: &Path) -> Result<Graph, String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(directory)
        .output()
        .map_err(|error| format!("could not run `cargo metadata`: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`cargo metadata` failed: {}", stderr.trim()));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("`cargo metadata` wrote non-UTF-8 output: {error}"))?;
    from_json(&stdout)
}

/// Parse the part of `cargo metadata --format-version 1` output the rules read.
pub fn from_json(json: &str) -> Result<Graph, String> {
    let root: Value = serde_json::from_str(json).map_err(|error| format!("invalid metadata JSON: {error}"))?;
    let raw_packages = root
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("metadata has no `packages` array")?;
    let mut packages = Vec::with_capacity(raw_packages.len());
    for raw_package in raw_packages {
        let name = raw_package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("a package in the metadata has no `name`")?
            .to_owned();
        let raw_deps = raw_package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("package `{name}` has no `dependencies` array"))?;
        let mut deps = Vec::with_capacity(raw_deps.len());
        for raw_dep in raw_deps {
            let dep_name = raw_dep
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("a dependency of `{name}` has no `name`"))?;
            // `kind` is JSON null for a normal dependency, `"dev"` or `"build"`.
            let kind = match raw_dep.get("kind").and_then(Value::as_str) {
                None => DepKind::Normal,
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                Some(other) => {
                    return Err(format!(
                        "dependency `{dep_name}` of `{name}` has unknown kind `{other}`"
                    ));
                }
            };
            let features = raw_dep
                .get("features")
                .and_then(Value::as_array)
                .map(|features| features.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            deps.push(Dep {
                name: dep_name.to_owned(),
                kind,
                features,
            });
        }
        let is_binary = has_binary_target(raw_package);
        let lints = read_lints(raw_package, &name)?;
        packages.push(Package {
            name,
            deps,
            is_binary,
            lints,
        });
    }
    Ok(Graph { packages })
}

/// The lint declaration of the manifest `raw_package` names, or [`Lints::Unread`] when it names none.
fn read_lints(raw_package: &Value, name: &str) -> Result<Lints, String> {
    let Some(path) = raw_package.get("manifest_path").and_then(Value::as_str) else {
        return Ok(Lints::Unread);
    };
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("could not read `{path}` for `{name}`: {error}"))?;
    parse_lints(&text).map_err(|error| format!("`{path}` of `{name}`: {error}"))
}

/// The lint declaration in one manifest's text.
fn parse_lints(manifest: &str) -> Result<Lints, String> {
    let document: toml::Table = toml::from_str(manifest).map_err(|error| format!("invalid TOML: {error}"))?;
    let Some(lints) = document.get("lints") else {
        return Ok(Lints::Missing);
    };
    let lints = lints.as_table().ok_or("`lints` is not a table")?;
    if lints.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
        return Ok(Lints::Inherited);
    }
    // A level is a bare string (`unsafe_code = "deny"`) or a table (`{ level = "deny", priority = -1 }`).
    let level = |entry: &toml::Value| -> Option<String> {
        match entry {
            toml::Value::String(level) => Some(level.clone()),
            toml::Value::Table(table) => table.get("level").and_then(toml::Value::as_str).map(str::to_owned),
            _ => None,
        }
    };
    let unsafe_code = lints
        .get("rust")
        .and_then(toml::Value::as_table)
        .and_then(|rust| rust.get("unsafe_code"))
        .and_then(level);
    let clippy_all_denied = lints
        .get("clippy")
        .and_then(toml::Value::as_table)
        .and_then(|clippy| clippy.get("all"))
        .is_some_and(|all| {
            level(all).as_deref() == Some("deny")
                && all
                    .as_table()
                    .and_then(|table| table.get("priority"))
                    .and_then(toml::Value::as_integer)
                    == Some(-1)
        });
    Ok(Lints::Own {
        unsafe_code,
        clippy_all_denied,
    })
}

/// Whether a package's `targets` include a binary: `cargo metadata` lists each target with its kinds.
fn has_binary_target(raw_package: &Value) -> bool {
    raw_package
        .get("targets")
        .and_then(Value::as_array)
        .is_some_and(|targets| {
            targets.iter().any(|target| {
                target
                    .get("kind")
                    .and_then(Value::as_array)
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bin")))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_that_inherits_the_workspace_lints_is_inherited() {
        assert_eq!(parse_lints("[lints]\nworkspace = true\n"), Ok(Lints::Inherited));
    }

    #[test]
    fn a_manifest_that_restates_the_table_reports_its_levels() {
        let manifest =
            "[lints.rust]\nunsafe_code = \"deny\"\n\n[lints.clippy]\nall = { level = \"deny\", priority = -1 }\n";
        assert_eq!(
            parse_lints(manifest),
            Ok(Lints::Own {
                unsafe_code: Some("deny".to_owned()),
                clippy_all_denied: true,
            })
        );
    }

    #[test]
    fn a_restated_table_without_the_clippy_levels_does_not_count_them() {
        let manifest = "[lints.rust]\nunsafe_code = \"allow\"\n\n[lints.clippy]\nall = \"deny\"\n";
        assert_eq!(
            parse_lints(manifest),
            Ok(Lints::Own {
                unsafe_code: Some("allow".to_owned()),
                clippy_all_denied: false,
            })
        );
    }

    #[test]
    fn a_manifest_without_a_lints_table_is_missing_one() {
        assert_eq!(parse_lints("[package]\nname = \"a\"\n"), Ok(Lints::Missing));
    }
}
