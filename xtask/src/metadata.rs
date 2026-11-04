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
}

#[derive(Debug)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
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
            deps.push(Dep {
                name: dep_name.to_owned(),
                kind,
            });
        }
        packages.push(Package { name, deps });
    }
    Ok(Graph { packages })
}
