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
///
/// `gnoblin-capture` joins them for the same reason as `gnoblin-output`: it holds the
/// capture session rules and no mechanism at all. A frame's pixels reach it through the
/// `CaptureSource` trait, which the engine implements, so the crate names no Smithay
/// type, no device and no clock, and the plan's Renderer card is what makes it the
/// *owner* of those sessions rather than a helper of the protocol layer.
///
/// `gnoblin-output` joins them for the same reason as `gnoblin-wm`: it is pure
/// policy. It holds no Smithay, DRM, udev or Wayland type, reads no clock and
/// touches no hardware — an output arrives as a descriptor and a configuration
/// becomes a layout of rectangles (`crates/gnoblin-output/src/lib.rs`). The plan
/// files it under the mechanism layer, but the mechanism is the backend that
/// performs the mode sets; keeping it here is what lets the Wayland engine lay out
/// its outputs without an engine-to-engine edge, which is the edge the plan forbids.
const FOUNDATION: &[&str] = &[
    "gnoblin-authority",
    "gnoblin-capture",
    "gnoblin-client",
    "gnoblin-config",
    "gnoblin-lock",
    "gnoblin-model",
    "gnoblin-output",
    "gnoblin-proto",
    "gnoblin-schema",
    "gnoblin-types",
    "gnoblin-wm",
];

/// The shared primitives: the root of every other crate's dependency graph.
const TYPES: &str = "gnoblin-types";

/// The mechanism layer: Smithay glue, renderer, input, backends and Xwayland. These
/// reach the policy core through `gnoblin-model` and `gnoblin-output` types and each
/// other through `gnoblin-backend` traits. Capture is not here: it is pure policy, and
/// [`FOUNDATION`] says why.
const ENGINE: &[&str] = &[
    "gnoblin-backend",
    "gnoblin-input",
    "gnoblin-render",
    "gnoblin-wayland",
    "gnoblin-xwayland",
];

/// The one engine crate every mechanism crate may depend on: the pure scene graph.
///
/// The plan says engine crates meet "through traits in `gnoblin-backend` and types
/// in `gnoblin-model`", and this one exception is what makes the render half of that
/// statement implementable. A frame *is* the scene's paint order and damage: a
/// description that could not name the scene would either duplicate the scene graph
/// or drop its damage on the floor, and `gnoblin-backend` is the crate that carries
/// the frame description, so it is the hub that needed the edge first.
///
/// The edge is safe because `gnoblin-render` depends on `gnoblin-types` alone. It
/// cannot pull Smithay, a protocol or a policy crate in behind a caller, it depends
/// on no engine crate itself (that direction stays a violation), and no cycle is
/// possible because nothing depends on the mechanism crates except each other and
/// the composition root. Every other engine-to-engine edge still fails.
const HUB_SCENE: &str = "gnoblin-render";

/// The second engine crate a mechanism crate may depend on: the seat's delivery order.
///
/// The plan puts input in the mechanism layer and says mechanism crates meet "through
/// traits in `gnoblin-backend` and types in `gnoblin-model`". Serving a seat cannot be
/// written inside that shape: a key arrives, and the *order* it is offered in — the grab,
/// then a binding, then the client — is a rule, not a type. `gnoblin-input` is where that
/// rule lives, and a protocol engine that restated it would be a second answer to "does a
/// binding beat a client", which is exactly the divergence these rules exist to prevent.
///
/// The edge is safe for the same reasons the scene's is, and `gnoblin-input` is smaller
/// still: it depends on `gnoblin-types` alone, so it cannot pull Smithay, a device or a
/// policy crate in behind a caller, and it names no engine crate itself. That direction
/// stays a violation, and so does every other engine-to-engine edge.
const HUB_INPUT: &str = "gnoblin-input";

/// Crates that talk to the compositor as clients, so they may not link it.
///
/// `gnoblin-wasm` joins them because a module host is an IPC client by design
/// (design/rust-compositor.md, D6): it reaches the object model through
/// `gnoblin-client` and nothing else, so a module fault cannot be a compositor
/// fault. The edge also has to point this way for a mechanical reason: the
/// compositor is what will host this crate, and a crate that named it would make
/// that edge a cycle.
const CLIENT_SIDE: &[&str] = &[
    "gnoblin-lua",
    "gnoblin-services",
    "gnoblin-wasm",
    "gnoblin-portal-backend",
];

/// The binary whose own dependencies [`binary_roots`] constrains.
///
/// Exactly one crate, and deliberately not the whole [`COMPAT_HOSTS`] set: a role crate that
/// serves an adapter is still a client, and the rules over a client are
/// `client-side-isolation`'s, not this one's.
const SESSION_BINARY: &str = "gnoblin";

/// The processes that link the compat adapters.
///
/// Only the session binary links them: it is the composition root that hands the services
/// role its linked agents, such as the gcr prompter. The services role itself names no
/// adapter, so a library or role crate that did would leak one into a process that does not
/// serve it. Nothing else may name one.
const COMPAT_HOSTS: &[&str] = &["gnoblin"];

/// What the session binary itself may depend on.
///
/// Design/rust-compositor.md, "Project structure": `gnoblin/compositor` is the
/// composition root and "the only crate that knows every subsystem", and the binary is
/// the supervisor that starts one role per process. A binary that named an engine or a
/// policy crate directly would be a second composition root: two places deciding which
/// backend runs, which IPC server serves and how a snapshot becomes an object.
///
/// The three others are role entry points, named because the binary is what runs a role:
/// `gnoblin-session` for the guardian, and `gnoblin-lua` and `gnoblin-services` for the two
/// roles the guardian starts. Each is the top of its own process, so an edge to it decides
/// nothing the composition root decides — the compositor, the engine and the policy crates
/// stay out of reach either way.
const BINARY_ROOTS: &[&str] = &[
    "gnoblin-compositor",
    "gnoblin-session",
    "gnoblin-lua",
    "gnoblin-services",
];

/// The logging facade: the macros every crate logs through. It names no workspace crate (see
/// [`log_isolation`]), so it cannot become a second composition root; every crate may depend on it.
const LOG_CRATE: &str = "gnoblin-log";

/// The logging composition: the sinks wired up for one process. Only a binary may name it (see
/// [`log_init_roots`]), and the session binary is the one that does.
const LOG_INIT_CRATE: &str = "gnoblin-log-init";

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
        id: "portal-layering",
        check: portal_layering,
    },
    Rule {
        id: "portal-crates",
        check: portal_crates,
    },
    Rule {
        id: "dev-crates",
        check: dev_crates,
    },
    Rule {
        id: "types-roots",
        check: types_roots,
    },
    Rule {
        id: "binary-roots",
        check: binary_roots,
    },
    Rule {
        id: "log-isolation",
        check: log_isolation,
    },
    Rule {
        id: "log-init-roots",
        check: log_init_roots,
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

/// Engine crates reach each other only through `gnoblin-backend`'s traits, and none of them
/// depends on the composition root. The two hub edges are the exceptions: see
/// [`HUB_SCENE`] and [`HUB_INPUT`].
///
/// A **development** edge is not this rule's business. The rule guards what ships: a
/// library that names another mechanism crate pulls the whole mechanism in behind every
/// caller. A test target does the opposite of that — `gnoblin-xwayland`'s real-Xwayland
/// tests run a `gnoblin-wayland` session, and the only way to do that from the crate that
/// owns the policy is to link it in the test binary. The edge cannot ship, because a
/// development dependency is not compiled for a dependent.
fn engine_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if !is_engine(&package.name) {
        return None;
    }
    if dep.kind == DepKind::Dev {
        return None;
    }
    if dep.name == "gnoblin-compositor" {
        return Some("engine crates do not depend on the composition root".to_owned());
    }
    if package.name == HUB_SCENE && is_engine(&dep.name) {
        // The exceptions below are one-directional, and this is the direction that has to
        // stay shut: the scene crate is a leaf, so an edge from it into the mechanism would
        // make every crate that names the scene pull Smithay in.
        return Some(format!(
            "the pure scene crate depends on no engine crate, not even the hub; `{}` is an engine crate",
            dep.name
        ));
    }
    if package.name == HUB_INPUT && is_engine(&dep.name) && dep.name != "gnoblin-backend" {
        // The same direction, for the same reason: `gnoblin-input` is the seat's rule book
        // and a leaf of the mechanism, so an edge from it into the mechanism would make
        // every crate that asks "where does this key go" pull a compositor in behind it.
        // The backend is the one engine crate the plan lets every mechanism crate name, and
        // the input policy needs it to translate an event's device name into a stage.
        return Some(format!(
            "the input policy crate depends on no engine crate but gnoblin-backend; `{}` is an engine crate",
            dep.name
        ));
    }
    if is_engine(&dep.name) && dep.name != "gnoblin-backend" {
        if dep.name == HUB_SCENE || dep.name == HUB_INPUT {
            return None;
        }
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
    if !is_client_side(&package.name) {
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

/// Whether `name` is an IPC client under [`client_side_isolation`]: a listed crate, or any
/// service crate. The `gnoblin-service-*` crates are the agents the services role serves, and
/// `gnoblin-service-core` and `gnoblin-service-testkit` are the shared frame and the test
/// kit under them, so they run in the same client process and get the same rule.
fn is_client_side(name: &str) -> bool {
    CLIENT_SIDE.contains(&name) || name.starts_with("gnoblin-service-")
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

/// Nothing outside `compat/` links a compat adapter except the processes that
/// serve them.
fn compat_host(package: &Package, dep: &Dep) -> Option<String> {
    if !is_compat(&dep.name) || is_compat(&package.name) || COMPAT_HOSTS.contains(&package.name.as_str()) {
        return None;
    }
    Some(format!(
        "only {} link a compat adapter, so the adapters cannot leak into a library",
        COMPAT_HOSTS
            .iter()
            .map(|host| format!("`{host}`"))
            .collect::<Vec<_>>()
            .join(" and ")
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

/// The portal's dialog models (design/rust-compositor.md, D19).
///
/// `gnoblin-diag` holds the built-in dialog models (its `portal` module), which the
/// compositor's negotiation answers with. Only the compositor may depend on it: the
/// relay `gnoblin-portal-backend` stays a thin, removable relay and names neither
/// the compositor nor the models.
fn portal_layering(package: &Package, dep: &Dep) -> Option<String> {
    const DIAG: &str = "gnoblin-diag";

    if dep.name == DIAG && package.name != "gnoblin-compositor" {
        return Some(format!(
            "the portal dialog models in `{DIAG}` are used only by the compositor, where portal.ui requests land (D19); `{}` may not depend on it",
            package.name
        ));
    }
    None
}

/// The portal's UI crates, each with the only dependencies it may name (the crates of
/// the `gnoblin-portal-*` family). The core holds the dialog models, the theme, the CPU
/// painter and the widgets: it names egui and the primitive crates only, never an
/// engine crate. An allowlist, not a denylist, so a new edge is a decision rather than a leak.
const PORTAL_ALLOWED: &[(&str, &[&str])] = &[("gnoblin-portal-core", &["egui", "gnoblin-schema", "gnoblin-types"])];

/// Each portal UI crate names only the dependencies in [`PORTAL_ALLOWED`].
fn portal_crates(package: &Package, dep: &Dep) -> Option<String> {
    let (_, allowed) = PORTAL_ALLOWED.iter().find(|(name, _)| *name == package.name)?;
    if allowed.contains(&dep.name.as_str()) {
        return None;
    }
    Some(format!(
        "`{}` may depend only on {}; `{}` is not one of them",
        package.name,
        allowed.join(", "),
        dep.name
    ))
}

/// What one process may name.
///
/// The rule is about a process's *own* dependencies, not about the dependencies of the
/// crates it links. `gnoblin` may name the composition root, the guardian and the two role
/// entry crates, and nothing else of the workspace: a binary that named an engine or a
/// policy crate directly would be a second composition root — two places deciding which
/// backend runs, which IPC server serves and how a snapshot becomes an object.
///
/// A role entry crate is a *client*, and this rule does not reach through it: `gnoblin-services`
/// depends on `gnoblin-client`, the object model and the primitives because that is what a
/// services role is, and `gnoblin-lua` on the client and the configuration layer for the same
/// reason. Constraining those edges here would be a second copy of `client-side-isolation`,
/// written from the other end, and it would forbid the one thing the role crates exist to do.
///
/// A dev-dependency is exempt for the reason the plan gives: a test target may drive the
/// engine directly to prove what the binary does with it, and that edge cannot ship.
fn binary_roots(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != SESSION_BINARY || dep.kind == DepKind::Dev {
        return None;
    }
    if BINARY_ROOTS.contains(&dep.name.as_str())
        || dep.name == LOG_CRATE
        || dep.name == LOG_INIT_CRATE
        || is_compat(&dep.name)
    {
        return None;
    }
    // Only the workspace's own crates can be a second composition root; `clap` and
    // `tempfile` cannot, and holding the binary to a naming rule for the crate
    // registry would fail for a reason that has nothing to do with the rule.
    if !is_workspace_crate(&dep.name) {
        return None;
    }
    Some(format!(
        "the session binary depends on the composition root, the guardian and a role entry crate, not on `{}`; a second composition root decides the same things twice",
        dep.name
    ))
}

/// Whether a dependency is a crate of this workspace rather than one from a registry
/// or a git pin.
///
/// The workspace's members are `gnoblin-*`, the session binary `gnoblin`, the portal
/// backend `gnoblin-portal-backend` and `xtask`; every other name in a member's dependency
/// list comes from outside the workspace.
fn is_workspace_crate(name: &str) -> bool {
    name.starts_with("gnoblin") || name == "xtask"
}

/// The logging stack sits beneath every process that logs, so each piece names only the pieces
/// beneath it: the facade names no workspace crate, each sink names the facade, and the
/// composition names the facade and both sinks. A wider edge would drag that crate's graph into
/// every role that logs.
fn log_isolation(package: &Package, dep: &Dep) -> Option<String> {
    let allowed: &[&str] = match package.name.as_str() {
        LOG_CRATE => &[],
        "gnoblin-log-console" | "gnoblin-log-store" => &[LOG_CRATE],
        LOG_INIT_CRATE => &[LOG_CRATE, "gnoblin-log-console", "gnoblin-log-store"],
        _ => return None,
    };
    if !is_workspace_crate(&dep.name) || allowed.contains(&dep.name.as_str()) {
        return None;
    }
    Some(format!(
        "`{}` is part of the logging stack and may not depend on the workspace crate `{}`",
        package.name, dep.name
    ))
}

/// The composition installs the log for a process, so only a binary may depend on it: a library
/// that installed a subscriber would decide the log for a process it does not own.
fn log_init_roots(package: &Package, dep: &Dep) -> Option<String> {
    if dep.name != LOG_INIT_CRATE
        || dep.kind == DepKind::Dev
        || package.name == SESSION_BINARY
        || package.name == "gnoblin-portal-backend"
    {
        return None;
    }
    Some(format!(
        "`{LOG_INIT_CRATE}` installs the log for a process and may only be depended on by a binary, not by `{}`",
        package.name
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
    fn the_session_binary_may_depend_on_the_composition_root_and_the_guardian() {
        assert_eq!(
            violations(&[(
                "gnoblin",
                &["gnoblin-compositor", "gnoblin-session", "gnoblin-compat-gtk-shell1"],
            )]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_session_binary_may_depend_on_the_logging_facade_and_composition() {
        assert_eq!(
            violations(&[("gnoblin", &["gnoblin-compositor", "gnoblin-log", "gnoblin-log-init"])]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_portal_core_may_name_egui_and_the_primitive_crates_only() {
        assert_eq!(
            violations(&[("gnoblin-portal-core", &["egui", "gnoblin-schema", "gnoblin-types"])]),
            Vec::<String>::new()
        );
        let found = violations(&[("gnoblin-portal-core", &["egui", "gnoblin-wayland"])]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("[portal-crates]"), "{found:?}");
    }

    #[test]
    fn the_logging_facade_may_not_depend_on_a_workspace_crate() {
        let found = violations(&[("gnoblin-log", &["gnoblin-types"])]);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("[log-isolation]"), "{found:?}");
    }

    #[test]
    fn the_sinks_may_depend_on_the_facade_alone() {
        assert_eq!(
            violations(&[
                ("gnoblin-log-console", &["gnoblin-log"]),
                ("gnoblin-log-store", &["gnoblin-log"]),
            ]),
            Vec::<String>::new()
        );
        let found = violations(&[("gnoblin-log-store", &["gnoblin-types"])]);
        assert!(
            found.iter().any(|message| message.contains("[log-isolation]")),
            "{found:?}"
        );
    }

    #[test]
    fn only_a_binary_may_depend_on_the_log_composition() {
        let found = violations(&[("gnoblin-wayland", &["gnoblin-log-init"])]);
        assert!(
            found.iter().any(|message| message.contains("[log-init-roots]")),
            "{found:?}"
        );
        assert_eq!(
            violations(&[("gnoblin-portal-backend", &["gnoblin-log-init"])]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_session_binary_may_depend_on_an_external_crate() {
        // `clap` parses the one flag and `tempfile` is the test's; neither is a
        // subsystem.
        assert_eq!(
            violations(&[("gnoblin", &["gnoblin-compositor", "gnoblin-session", "clap"])]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_session_binary_may_name_every_role_entry_crate() {
        // The binary is what runs a role, so it names the four crates that own one.
        assert_eq!(
            violations(&[(
                "gnoblin",
                &[
                    "gnoblin-compositor",
                    "gnoblin-session",
                    "gnoblin-lua",
                    "gnoblin-services",
                ],
            )]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_role_entry_crate_may_depend_on_the_client_and_the_model() {
        // The failure this prevents: `binary-roots` reaching through the binary into the
        // crates it links, so a services role could not name the client it is built on.
        // `gnoblin-services` is a client crate; its edges are `client-side-isolation`'s to
        // judge and that rule allows every foundation crate.
        assert_eq!(
            violations(&[(
                "gnoblin-services",
                &["gnoblin-client", "gnoblin-model", "gnoblin-proto", "gnoblin-types"],
            )]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations(&[(
                "gnoblin-lua",
                &["gnoblin-client", "gnoblin-config", "gnoblin-model", "gnoblin-proto"],
            )]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_role_entry_crate_may_not_depend_on_the_engine() {
        // The edge the plan forbids for a role, and the rule that forbids it is the client
        // one: a role that linked the compositor would run inside the session it talks to.
        let reported = violations(&[("gnoblin-services", &["gnoblin-wayland"])]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-services -> gnoblin-wayland [client-side-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn a_service_crate_may_not_depend_on_the_engine() {
        // Each `gnoblin-service-*` crate is a role's agent, served in the services process, so it is a
        // client like `gnoblin-services` and may not link the engine.
        let reported = violations(&[("gnoblin-service-polkit", &["gnoblin-wayland"])]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-service-polkit -> gnoblin-wayland [client-side-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn the_session_binary_may_not_depend_on_a_subsystem() {
        let reported = violations(&[("gnoblin", &["gnoblin-compositor", "gnoblin-wayland"])]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin -> gnoblin-wayland [binary-roots]"),
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

    /// A metadata document with one package per `(name, dependencies)` pair.
    fn graph(packages: &[(&str, &[&str])]) -> Graph {
        let packages: Vec<serde_json::Value> = packages
            .iter()
            .map(|(name, deps)| {
                let deps: Vec<serde_json::Value> = deps
                    .iter()
                    .map(|dep| serde_json::json!({ "name": dep, "kind": null }))
                    .collect();
                serde_json::json!({ "name": name, "dependencies": deps })
            })
            .collect();
        let document = serde_json::json!({ "packages": packages });
        from_json(&document.to_string()).expect("the fixture parses")
    }

    /// Every violation in a fixture, as the report prints them.
    fn violations(packages: &[(&str, &[&str])]) -> Vec<String> {
        check(&graph(packages)).iter().map(Violation::to_string).collect()
    }

    #[test]
    fn the_hub_may_depend_on_the_pure_scene_crate() {
        // The frame a backend draws is the scene's damage, so this edge is the one the
        // render path cannot exist without.
        assert_eq!(
            violations(&[
                ("gnoblin-backend", &["gnoblin-render", "gnoblin-types"]),
                ("gnoblin-render", &["gnoblin-types"]),
            ]),
            Vec::<String>::new()
        );
    }

    /// A test target may link a sibling engine to run it; a shipped dependency may not.
    #[test]
    fn an_engine_crate_may_link_another_only_as_a_development_dependency() {
        let shipping = serde_json::json!({
            "packages": [
                { "name": "gnoblin-xwayland", "dependencies": [
                    { "name": "gnoblin-wayland", "kind": null }
                ] },
                { "name": "gnoblin-wayland", "dependencies": [] },
            ]
        });
        let graph = from_json(&shipping.to_string()).expect("the fixture parses");
        assert_eq!(
            check(&graph).iter().map(Violation::to_string).collect::<Vec<_>>(),
            vec![
                "gnoblin-xwayland -> gnoblin-wayland [engine-isolation]: engine crates reach each other only \
                 through gnoblin-backend traits; `gnoblin-wayland` is an engine crate"
                    .to_owned()
            ]
        );

        let testing = serde_json::json!({
            "packages": [
                { "name": "gnoblin-xwayland", "dependencies": [
                    { "name": "gnoblin-wayland", "kind": "dev" }
                ] },
                { "name": "gnoblin-wayland", "dependencies": [] },
            ]
        });
        let graph = from_json(&testing.to_string()).expect("the fixture parses");
        assert_eq!(
            check(&graph).iter().map(Violation::to_string).collect::<Vec<_>>(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn any_engine_crate_may_name_the_pure_scene_crate() {
        // The scene crate is dependency-free, so naming it cannot pull mechanism in
        // behind a caller: a capture path and a protocol engine may both read it.
        assert_eq!(
            violations(&[
                ("gnoblin-wayland", &["gnoblin-backend", "gnoblin-render"]),
                ("gnoblin-render", &["gnoblin-types"]),
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_hub_may_not_depend_on_another_mechanism_crate() {
        let reported = violations(&[
            ("gnoblin-backend", &["gnoblin-wayland"]),
            ("gnoblin-wayland", &["gnoblin-backend"]),
        ]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-backend -> gnoblin-wayland [engine-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn any_engine_crate_may_name_the_input_hub() {
        // The seat's delivery order is a rule, not a device: a protocol engine that
        // restated it would answer "does a binding beat a client" a second time.
        assert_eq!(
            violations(&[
                ("gnoblin-wayland", &["gnoblin-backend", "gnoblin-input"]),
                ("gnoblin-input", &["gnoblin-types"]),
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_input_hub_may_name_the_backend_but_no_other_mechanism_crate() {
        // The exception is one-directional: `gnoblin-input` is a leaf of the mechanism, so
        // an edge from it would pull a compositor in behind every crate that asks where a
        // key goes.
        for forbidden in ["gnoblin-wayland", "gnoblin-render", "gnoblin-xwayland"] {
            let reported = violations(&[("gnoblin-input", &[forbidden]), ("gnoblin-wayland", &["gnoblin-input"])]);
            assert_eq!(
                reported.len(),
                1,
                "expected one violation for {forbidden}, got {reported:#?}"
            );
            assert!(
                reported[0].starts_with(&format!("gnoblin-input -> {forbidden} [engine-isolation]")),
                "unexpected violation: {}",
                reported[0]
            );
        }
        // The one engine crate the plan lets every mechanism crate name stays open: the
        // input policy translates an event's device name into a stage from the backend.
        assert_eq!(
            violations(&[
                ("gnoblin-input", &["gnoblin-backend", "gnoblin-types"]),
                ("gnoblin-wayland", &["gnoblin-input"]),
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_engine_crate_still_may_not_depend_on_another_mechanism_crate() {
        // The hub is the only exception: a protocol engine may not name the backend's
        // sibling engines directly.
        let reported = violations(&[
            (
                "gnoblin-wayland",
                &["gnoblin-backend", "gnoblin-input", "gnoblin-xwayland"],
            ),
            ("gnoblin-input", &["gnoblin-types"]),
        ]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-wayland -> gnoblin-xwayland [engine-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn the_pure_scene_crate_may_not_depend_on_an_engine_crate() {
        // The exception is one-directional: the scene is a leaf, so an edge back into
        // the mechanism would make every caller of the scene pull Smithay in.
        let reported = violations(&[
            ("gnoblin-render", &["gnoblin-backend"]),
            ("gnoblin-backend", &["gnoblin-render"]),
        ]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-render -> gnoblin-backend [engine-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn a_foundation_crate_may_not_name_the_pure_scene_crate() {
        // The exception is for mechanism crates only: a policy crate that reached the
        // renderer would make the window manager depend on the render path.
        let reported = violations(&[
            ("gnoblin-wm", &["gnoblin-render"]),
            ("gnoblin-render", &["gnoblin-types"]),
        ]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].contains("[foundation-purity]"),
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
