//! The dependency rules from design/rust-compositor.md, checked over the
//! workspace graph.
//!
//! The rules are what make concurrent lanes safe: a lane may change a crate
//! freely as long as no edge crosses the boundaries below. Every rule states one
//! bullet of the plan, and every violation names the edge so the fix is obvious.

use std::fmt;

use crate::metadata::{Dep, DepKind, Graph, Package};

/// Crates that define shared types, pure policy or the client half of the IPC.
///
/// The plan's "Dependency rules" first bullet names `gnoblin-schema`,
/// `gnoblin-proto`, `gnoblin-config`, `gnoblin-model`, `gnoblin-wm`,
/// `gnoblin-lock` and `gnoblin-authority`; `gnoblin-client` joins them because
/// the runtime, services, portal and shells link it and it must never pull a
/// compositor in behind itself. `gnoblin-types` joins them because it holds the
/// primitives all of them pass around.
const FOUNDATION: &[&str] = &[
    "gnoblin-authority",
    "gnoblin-client",
    "gnoblin-config",
    "gnoblin-lock",
    "gnoblin-model",
    "gnoblin-proto",
    "gnoblin-schema",
    "gnoblin-types",
    "gnoblin-wm",
];

/// The shared primitives: the root of every other crate's dependency graph.
const TYPES: &str = "gnoblin-types";

/// The mechanism layer: Smithay glue, renderer, input, backends, outputs,
/// capture and Xwayland. These reach the policy core through `gnoblin-model`
/// types and each other through `gnoblin-backend` traits.
const ENGINE: &[&str] = &[
    "gnoblin-backend",
    "gnoblin-capture",
    "gnoblin-input",
    "gnoblin-output",
    "gnoblin-render",
    "gnoblin-wayland",
    "gnoblin-xwayland",
];

/// Crates that talk to the compositor as clients, so they may not link it.
const CLIENT_SIDE: &[&str] = &["gnoblin-lua", "gnoblin-services", "xdg-desktop-portal-gnoblin"];

/// The one binary that links the compat adapters.
const COMPAT_HOST: &str = "gnoblin";

/// The shared compat frame every compat crate depends on.
const COMPAT_CORE: &str = "gnoblin-compat-core";

/// Compat crates that speak the Smithay Wayland transport instead of D-Bus.
const COMPAT_WAYLAND: &[&str] = &["gnoblin-compat-gtk-shell1", "gnoblin-compat-mutter-x11-interop"];

/// The one compat crate that speaks the Xwayland transport.
const COMPAT_XWAYLAND: &str = "gnoblin-compat-x11-xsettings";

/// External libraries a foundation crate may not depend on: Smithay, any
/// `wayland-*` crate, a GPU renderer, D-Bus or the Lua VM.
const FOUNDATION_BANNED_EXTERNALS: &[&str] = &["ash", "glium", "glutin", "mlua", "vulkano", "wgpu", "zbus"];

/// One rule: an id that CI prints and a predicate over a single edge.
struct Rule {
    /// Short stable name, printed as `[id]` so a failing job is greppable.
    id: &'static str,
    check: fn(&Package, &Dep) -> Option<String>,
}

const RULES: &[Rule] = &[
    Rule {
        id: "foundation-purity",
        check: foundation_purity,
    },
    Rule {
        id: "engine-isolation",
        check: engine_isolation,
    },
    Rule {
        id: "client-side-isolation",
        check: client_side_isolation,
    },
    Rule {
        id: "compat-isolation",
        check: compat_isolation,
    },
    Rule {
        id: "compat-host",
        check: compat_host,
    },
    Rule {
        id: "a11y-purity",
        check: a11y_purity,
    },
    Rule {
        id: "dev-crates",
        check: dev_crates,
    },
    Rule {
        id: "types-roots",
        check: types_roots,
    },
];

/// One broken edge.
pub struct Violation {
    package: String,
    dependency: String,
    rule: &'static str,
    message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} -> {} [{}]: {}",
            self.package, self.dependency, self.rule, self.message
        )
    }
}

/// Every violation of every rule in the graph, in package order.
pub fn check(graph: &Graph) -> Vec<Violation> {
    let mut violations = Vec::new();
    for package in &graph.packages {
        for dep in &package.deps {
            for rule in RULES {
                if let Some(message) = (rule.check)(package, dep) {
                    violations.push(Violation {
                        package: package.name.clone(),
                        dependency: dep.name.clone(),
                        rule: rule.id,
                        message,
                    });
                }
            }
        }
    }
    violations
}

fn is_foundation(name: &str) -> bool {
    FOUNDATION.contains(&name)
}

fn is_engine(name: &str) -> bool {
    ENGINE.contains(&name)
}

fn is_compat(name: &str) -> bool {
    name.starts_with("gnoblin-compat-")
}

/// Smithay, any `wayland-*` crate, a renderer, D-Bus or Lua.
fn is_banned_external(name: &str) -> bool {
    name.starts_with("smithay") || name.starts_with("wayland-") || FOUNDATION_BANNED_EXTERNALS.contains(&name)
}

/// A foundation crate stays free of mechanism: no engine crate, no compositor,
/// and none of Smithay, `wayland-*`, a renderer, `zbus` or `mlua`.
fn foundation_purity(package: &Package, dep: &Dep) -> Option<String> {
    if !is_foundation(&package.name) {
        return None;
    }
    if is_engine(&dep.name) || dep.name == "gnoblin-compositor" {
        return Some(format!(
            "foundation crates do not depend on mechanism; `{}` is an engine crate",
            dep.name
        ));
    }
    if is_banned_external(&dep.name) {
        return Some(format!("foundation crates do not depend on `{}`", dep.name));
    }
    None
}

/// Engine crates reach each other only through `gnoblin-backend` traits, and
/// none of them depends on the composition root.
fn engine_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if !is_engine(&package.name) {
        return None;
    }
    if dep.name == "gnoblin-compositor" {
        return Some("engine crates do not depend on the composition root".to_owned());
    }
    if is_engine(&dep.name) && dep.name != "gnoblin-backend" {
        return Some(format!(
            "engine crates reach each other only through gnoblin-backend traits; `{}` is an engine crate",
            dep.name
        ));
    }
    None
}

/// The runtime, the services role and the portal are IPC clients: they may not
/// link the compositor or the engine.
fn client_side_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if !CLIENT_SIDE.contains(&package.name.as_str()) {
        return None;
    }
    if dep.name == "gnoblin-compositor" || is_engine(&dep.name) {
        return Some(format!(
            "this crate is an IPC client and runs in its own process; `{}` is a compositor crate",
            dep.name
        ));
    }
    None
}

/// A compat crate depends only on `gnoblin-compat-core`, `gnoblin-client` and
/// its own transport, and never on another compat crate.
fn compat_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if !is_compat(&package.name) {
        return None;
    }
    if is_compat(&dep.name) && dep.name != COMPAT_CORE {
        return Some(format!(
            "a compat crate depends only on {} and its own transport, never on another adapter",
            COMPAT_CORE
        ));
    }
    if dep.name == "gnoblin-wayland" {
        if COMPAT_WAYLAND.contains(&package.name.as_str()) {
            return None;
        }
        return Some("only the Wayland redirect adapters use the gnoblin-wayland transport".to_owned());
    }
    if dep.name == "gnoblin-xwayland" {
        if package.name == COMPAT_XWAYLAND {
            return None;
        }
        return Some("only the XSETTINGS adapter uses the gnoblin-xwayland transport".to_owned());
    }
    if dep.name == "gnoblin-compositor" || is_engine(&dep.name) {
        return Some(format!(
            "a compat crate does not link the engine, and `{}` is part of it",
            dep.name
        ));
    }
    None
}

/// Nothing outside `compat/` links a compat adapter except the binary that
/// serves them.
fn compat_host(package: &Package, dep: &Dep) -> Option<String> {
    if !is_compat(&dep.name) || is_compat(&package.name) || package.name == COMPAT_HOST {
        return None;
    }
    Some(format!(
        "only the `{COMPAT_HOST}` binary links a compat adapter, so the adapters cannot leak into a library"
    ))
}

/// The accessibility crate is pure state machines: its tests run without a
/// session, so it depends on nothing outside the workspace foundation.
fn a11y_purity(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != "gnoblin-a11y" || is_foundation(&dep.name) {
        return None;
    }
    Some(format!(
        "gnoblin-a11y is a pure state-machine crate and may depend only on workspace foundation crates; `{}` is outside them",
        dep.name
    ))
}

/// `gnoblin-types` is the root of the graph: every crate may depend on it, and
/// it depends on nothing at all, not even another workspace crate.
///
/// The rule is one-directional on purpose. Nothing has to be enforced for an edge
/// toward the root; what would break the design is the reverse, a primitive that
/// pulls a policy or an engine crate in behind a value type.
fn types_roots(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != TYPES {
        return None;
    }
    Some(format!(
        "gnoblin-types holds the shared primitives and depends on nothing; it declares `{}`",
        dep.name
    ))
}

/// The test kit and the parity harness are development dependencies only.
fn dev_crates(package: &Package, dep: &Dep) -> Option<String> {
    if dep.name != "gnoblin-testkit" && dep.name != "gnoblin-parity" {
        return None;
    }
    if dep.kind == DepKind::Dev {
        return None;
    }
    Some(format!(
        "`{}` is a development dependency only, but `{}` declares it as a {} dependency",
        dep.name,
        package.name,
        dep.kind.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::from_json;

    /// Three compat crates where one adapter depends on another: the edge the
    /// plan forbids.
    const COMPAT_CHAIN: &str = include_str!("../fixtures/metadata-compat-chain.json");

    /// A policy crate that declares the test kit as a normal dependency.
    const TESTKIT_AS_NORMAL: &str = include_str!("../fixtures/metadata-testkit-normal.json");

    #[test]
    fn a_compat_adapter_depending_on_another_adapter_fails() {
        let graph = from_json(COMPAT_CHAIN).expect("fixture parses");
        let violations = check(&graph);
        let reported: Vec<String> = violations.iter().map(Violation::to_string).collect();
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0]
                .starts_with("gnoblin-compat-mutter-screencast -> gnoblin-compat-mutter-clipboard [compat-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn the_test_kit_as_a_normal_dependency_fails() {
        let graph = from_json(TESTKIT_AS_NORMAL).expect("fixture parses");
        let violations = check(&graph);
        let reported: Vec<String> = violations.iter().map(Violation::to_string).collect();
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-wm -> gnoblin-testkit [dev-crates]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn the_real_workspace_has_no_violations() {
        // `cargo test` runs a test binary with its package root as the working
        // directory, so the workspace root is one level above xtask.
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let graph = crate::metadata::load(&workspace_root).expect("cargo metadata for the workspace");
        assert!(!graph.packages.is_empty(), "workspace metadata lists no packages");
        let violations = check(&graph);
        let reported: Vec<String> = violations.iter().map(Violation::to_string).collect();
        assert!(
            reported.is_empty(),
            "workspace violates its own rules:\n{}",
            reported.join("\n")
        );
    }
}
