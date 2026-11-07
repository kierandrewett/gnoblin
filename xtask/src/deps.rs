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
    "gnoblin-cli-style",
    "gnoblin-client",
    "gnoblin-config",
    "gnoblin-cursor",
    "gnoblin-layers",
    "gnoblin-lock",
    "gnoblin-lua-pattern",
    "gnoblin-model",
    "gnoblin-output",
    "gnoblin-proto",
    "gnoblin-schema",
    "gnoblin-types",
    "gnoblin-wm",
];

/// The shared primitives: the root of every other crate's dependency graph.
const TYPES: &str = "gnoblin-types";

/// The mechanism layer: Smithay glue, renderer, input, the EIS server, backends and
/// Xwayland. These reach the policy core through `gnoblin-model` and `gnoblin-output`
/// types and each other through `gnoblin-backend` traits. Capture is not here: it is
/// pure policy, and [`FOUNDATION`] says why.
const ENGINE: &[&str] = &[
    "gnoblin-backend",
    "gnoblin-backend-drm",
    "gnoblin-input",
    "gnoblin-input-eis",
    "gnoblin-render-gles",
    "gnoblin-render-software",
    "gnoblin-render-vulkan",
    "gnoblin-render",
    "gnoblin-render-frame",
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

/// The third engine crate a mechanism crate may depend on: the renderer-neutral frame.
///
/// `gnoblin-render-frame` holds the seam every renderer draws through: the frame a
/// backend hands a renderer, the geometry and image types, the render error and the
/// shared shader sources. The GLES, Vulkan and software renderers each depend on it, and
/// the backend and the protocol engines name its types, so it is a hub in the same way
/// the scene is.
///
/// The edge is safe because the frame crate may depend on the scene and on nothing else
/// in the mechanism: it cannot name the backend, a renderer or a protocol engine, so a
/// caller that names the seam never pulls a renderer or a backend in behind it. The
/// direction back into the mechanism stays a violation (see [`engine_isolation`]).
const HUB_FRAME: &str = "gnoblin-render-frame";

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
    "xdg-desktop-portal-gnoblin",
];

/// The binary whose own dependencies [`binary_roots`] constrains.
///
/// Exactly one crate, and deliberately not the whole [`COMPAT_HOSTS`] set: a role crate that
/// serves an adapter is still a client, and the rules over a client are
/// `client-side-isolation`'s, not this one's.
const SESSION_BINARY: &str = "gnoblin";

/// The crate that wires the compat adapters into a process. It is a compat crate but not an
/// adapter, and the session binary is the only process that links it.
const COMPAT_HOST: &str = "gnoblin-compat-host";

/// The processes that link the compat adapters: the host, and nothing else.
///
/// The host is the one place that knows which adapters exist and how the session and services
/// roles wire them. The services role itself names no adapter, so a library or role crate that
/// did would leak one into a process that does not serve it. Nothing else may name one.
const COMPAT_HOSTS: &[&str] = &[COMPAT_HOST];

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

/// The logging facade: the macros every crate logs through, and the one crate that carries the
/// sinks behind its features. It names no workspace crate (see [`log_isolation`]), so it cannot
/// become a second composition root. Every crate may depend on it, but only a binary may enable a
/// sink or `init` (see [`log_features`]).
const LOG_CRATE: &str = "gnoblin-log";

/// The shared compat frame every compat crate depends on.
const COMPAT_CORE: &str = "gnoblin-compat-core";

/// Compat crates that speak the Smithay Wayland transport instead of D-Bus.
const COMPAT_WAYLAND: &[&str] = &[
    "gnoblin-compat-wayland-gtk-shell1",
    "gnoblin-compat-wayland-mutter-x11-interop",
];

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
        id: "pipewire-export-isolation",
        check: pipewire_export_isolation,
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
        id: "log-features",
        check: log_features,
    },
    Rule {
        id: "cursor-isolation",
        check: cursor_isolation,
    },
    Rule {
        id: "keyring-store-isolation",
        check: keyring_store_isolation,
    },
    Rule {
        id: "service-keyring-isolation",
        check: service_keyring_isolation,
    },
    Rule {
        id: "keyring-handoff-isolation",
        check: keyring_handoff_isolation,
    },
    Rule {
        id: "pam-gnoblin-keyring-isolation",
        check: pam_gnoblin_keyring_isolation,
    },
    Rule {
        id: "service-sessionidle-isolation",
        check: service_sessionidle_isolation,
    },
];

/// One broken edge.
pub struct DepViolation {
    package: String,
    dependency: String,
    rule: &'static str,
    message: String,
}

impl fmt::Display for DepViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A package-level rule names no edge, so it prints without the arrow.
        if self.dependency.is_empty() {
            return write!(formatter, "{} [{}]: {}", self.package, self.rule, self.message);
        }
        write!(
            formatter,
            "{} -> {} [{}]: {}",
            self.package, self.dependency, self.rule, self.message
        )
    }
}

/// Every violation of every rule in the graph, in package order.
pub fn check(graph: &Graph) -> Vec<DepViolation> {
    let mut violations = Vec::new();
    for package in &graph.packages {
        if let Some(message) = compat_naming(package) {
            violations.push(DepViolation {
                package: package.name.clone(),
                dependency: String::new(),
                rule: "compat-naming",
                message,
            });
        }
        for dep in &package.deps {
            for rule in RULES {
                if let Some(message) = (rule.check)(package, dep) {
                    violations.push(DepViolation {
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

/// Whether a crate is a compat adapter: a `gnoblin-compat-*` crate other than the host that
/// wires them. The host names every adapter, so it is exempt from the adapter rules.
fn is_adapter(name: &str) -> bool {
    is_compat(name) && name != COMPAT_HOST
}

/// Settings-layer adapters that write a toolkit's own configuration. They carry no transport
/// prefix: each is named `gnoblin-compat-<toolkit>-<what>`. Listing them makes an unprefixed name
/// a reviewed choice rather than drift.
const UNPREFIXED_COMPAT_ADAPTERS: &[&str] = &["gnoblin-compat-gtk-dconf-settings", "gnoblin-compat-qt-portal-settings"];

/// The transport prefixes a compat adapter that serves a protocol carries after `gnoblin-compat-`:
/// `dbus` for D-Bus, `wayland` for a Wayland protocol, `x11` for the Xwayland transport.
const COMPAT_TRANSPORT_PREFIXES: &[&str] = &["dbus", "wayland", "x11"];

/// Whether a compat adapter's name is allowed: one of the listed unprefixed settings adapters, or
/// `gnoblin-compat-<transport>-<what>` with a known transport and a non-empty `<what>`.
fn compat_name_is_allowed(name: &str) -> bool {
    if UNPREFIXED_COMPAT_ADAPTERS.contains(&name) {
        return true;
    }
    let Some(rest) = name.strip_prefix("gnoblin-compat-") else {
        return false;
    };
    COMPAT_TRANSPORT_PREFIXES.iter().any(|transport| {
        rest.strip_prefix(transport)
            .and_then(|what| what.strip_prefix('-'))
            .is_some_and(|what| !what.is_empty())
    })
}

/// The naming rule: every compat adapter is named for the transport it serves, or is one of the
/// listed unprefixed settings adapters. `gnoblin-compat-core` is the shared frame, not an adapter.
fn compat_naming(package: &Package) -> Option<String> {
    if !is_adapter(&package.name) || package.name == COMPAT_CORE || compat_name_is_allowed(&package.name) {
        return None;
    }
    Some(format!(
        "an adapter is `gnoblin-compat-<transport>-<what>` with transport dbus, wayland or x11, or one of the unprefixed settings adapters ({})",
        UNPREFIXED_COMPAT_ADAPTERS.join(", ")
    ))
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
    if package.name == HUB_FRAME && is_engine(&dep.name) && dep.name != HUB_SCENE {
        // The seam is below every renderer and the backend, so an edge from it upward would
        // make every crate that names a frame pull a renderer or a backend in behind it. Its
        // one engine dependency is the pure scene it describes.
        return Some(format!(
            "the render frame crate depends on no engine crate but the pure scene crate; `{}` is an engine crate",
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
    // The backend and the DRM backend name the renderers by crate, not through a trait: the
    // Vulkan renderer under the `vulkan` feature, the GLES renderer for the DRM path, and the
    // software renderer for the nested and headless paths. Each is a leaf of the renderer half,
    // so the edge cannot pull a policy or protocol crate in behind the backend.
    if (package.name == "gnoblin-backend" || package.name == "gnoblin-backend-drm")
        && (dep.name == "gnoblin-render-vulkan"
            || dep.name == "gnoblin-render-gles"
            || dep.name == "gnoblin-render-software")
    {
        return None;
    }
    if is_engine(&dep.name) && dep.name != "gnoblin-backend" {
        if dep.name == HUB_SCENE || dep.name == HUB_INPUT || dep.name == HUB_FRAME {
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

/// A compat crate depends only on `gnoblin-compat-core`, `gnoblin-client`, the services-role
/// library `gnoblin-service-core` (for the `LinkedAgent` an adapter's `service` module implements)
/// and its own transport, and never on another compat crate.
fn compat_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if !is_adapter(&package.name) {
        return None;
    }
    if dep.name == "gnoblin-service-core" {
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

/// Only `gnoblin-compat-host` links a compat adapter, and only the session binary links the host.
/// The adapters reach a process through the one crate that decides how each is wired, so nothing
/// else names one and no library can pull an adapter in behind the composition root.
fn compat_host(package: &Package, dep: &Dep) -> Option<String> {
    if dep.name == COMPAT_HOST {
        if package.name == SESSION_BINARY {
            return None;
        }
        return Some(format!(
            "only the session binary links `{COMPAT_HOST}`, the one place that wires the compat adapters into a process"
        ));
    }
    if !is_adapter(&dep.name) || is_compat(&package.name) || COMPAT_HOSTS.contains(&package.name.as_str()) {
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

/// The portal's built-in dialog screens and their dialog crates (design/rust-compositor.md,
/// D19). `gnoblin-portal` draws the dialogs, and only the compositor may depend on it,
/// where portal.ui requests land. The dialog crates are drawn by `gnoblin-portal` alone.
/// The egui layer `gnoblin-portal-ui` is named only by the screens, the dialog crates and
/// the compositor (for `paint::Frame`). The relay `xdg-desktop-portal-gnoblin` stays a
/// thin, removable relay and names none of them.
fn portal_layering(package: &Package, dep: &Dep) -> Option<String> {
    const SCREENS: &str = "gnoblin-portal";
    const UI: &str = "gnoblin-portal-ui";
    const DIALOGS: [&str; 3] = [
        "gnoblin-portal-consent",
        "gnoblin-portal-filechooser",
        "gnoblin-portal-appchooser",
    ];

    if dep.name == SCREENS && package.name != "gnoblin-compositor" {
        return Some(format!(
            "the portal dialog screens in `{SCREENS}` are used only by the compositor, where portal.ui requests land (D19); `{}` may not depend on it",
            package.name
        ));
    }
    if DIALOGS.iter().any(|name| dep.name == *name) && package.name != SCREENS {
        return Some(format!(
            "the portal dialog crates are drawn only by `{SCREENS}` (D19); `{}` may not depend on `{}`",
            package.name, dep.name
        ));
    }
    let may_name_ui =
        package.name == "gnoblin-compositor" || package.name == SCREENS || DIALOGS.contains(&package.name.as_str());
    if dep.name == UI && !may_name_ui {
        return Some(format!(
            "`{UI}` is the egui layer of the dialogs: only `{SCREENS}`, the dialog crates and `gnoblin-compositor` (for `paint::Frame`) may depend on it (D19); `{}` may not",
            package.name
        ));
    }
    None
}

/// The portal's own crates, each with the only dependencies it may name. The core holds the
/// dialog models only: plain data, no egui, and it names the primitive and schema crates.
/// The egui layer `gnoblin-portal-ui` holds the [`DialogUi`] trait, the shell that runs it
/// and the CPU painter: it names egui and the core, never a dialog crate. Each dialog UI
/// crate names egui, the core and the egui layer, never another UI crate or `gnoblin-portal`.
/// `gnoblin-portal` wires the UI crates into one constructor map. An allowlist, not a
/// denylist, so a new edge is a decision.
const PORTAL_ALLOWED: &[(&str, &[&str])] = &[
    ("gnoblin-portal-core", &["gnoblin-schema", "gnoblin-types"]),
    ("gnoblin-portal-ui", &["egui", "gnoblin-portal-core", "gnoblin-types"]),
    (
        "gnoblin-portal-consent",
        &["egui", "gnoblin-portal-core", "gnoblin-portal-ui", "gnoblin-types"],
    ),
    (
        "gnoblin-portal-filechooser",
        &["egui", "gnoblin-portal-core", "gnoblin-portal-ui", "gnoblin-types"],
    ),
    (
        "gnoblin-portal-appchooser",
        &["egui", "gnoblin-portal-core", "gnoblin-portal-ui", "gnoblin-types"],
    ),
    (
        "gnoblin-portal",
        &[
            "egui",
            "gnoblin-portal-core",
            "gnoblin-portal-ui",
            "gnoblin-portal-consent",
            "gnoblin-portal-filechooser",
            "gnoblin-portal-appchooser",
            "gnoblin-types",
            "png",
        ],
    ),
];

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

/// The crates the PipeWire export may name. It is fed plain values and a `PlaneSource` by
/// the wayland crate and names no other workspace crate beyond the capture, lock, types
/// and frame crate its frames are built from.
const PIPEWIRE_EXPORT_ALLOWED: &[&str] = &[
    "gnoblin-render-frame",
    "gnoblin-capture",
    "gnoblin-lock",
    "gnoblin-types",
    "pipewire",
];

/// Only `gnoblin-wayland` may name the PipeWire export, and only the export may link
/// `pipewire`. The PipeWire client is a large stack, so one crate owns it, and the export
/// names only the types listed in [`PIPEWIRE_EXPORT_ALLOWED`].
fn pipewire_export_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name == "gnoblin-pipewire-export" {
        if dep.name == "pipewire" || dep.name.starts_with("gnoblin-") {
            return (!PIPEWIRE_EXPORT_ALLOWED.contains(&dep.name.as_str())).then(|| {
                format!(
                    "`gnoblin-pipewire-export` may depend only on {}; `{}` is not one of them",
                    PIPEWIRE_EXPORT_ALLOWED.join(", "),
                    dep.name
                )
            });
        }
        return None;
    }
    if dep.name == "gnoblin-pipewire-export" && package.name != "gnoblin-wayland" {
        return Some(format!(
            "`{}` may not depend on `gnoblin-pipewire-export`; only `gnoblin-wayland` names the PipeWire export",
            package.name
        ));
    }
    if dep.name == "pipewire" && dep.kind == DepKind::Normal {
        return Some(format!(
            "`{}` may not link `pipewire`; only `gnoblin-pipewire-export` does",
            package.name
        ));
    }
    None
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
    if BINARY_ROOTS.contains(&dep.name.as_str()) || dep.name == LOG_CRATE || is_compat(&dep.name) {
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
/// backend `xdg-desktop-portal-gnoblin` and `xtask`; every other name in a member's dependency
/// list comes from outside the workspace.
fn is_workspace_crate(name: &str) -> bool {
    name.starts_with("gnoblin") || name == "xtask"
}

/// The logging stack sits beneath every process that logs, so the facade names no workspace crate:
/// a wider edge would drag that crate's graph into every role that logs.
fn log_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != LOG_CRATE || !is_workspace_crate(&dep.name) {
        return None;
    }
    Some(format!(
        "`{LOG_CRATE}` is the logging facade and may not depend on the workspace crate `{}`",
        dep.name
    ))
}

/// A library takes the logging facade alone. The features switch on the sinks and the composition,
/// and only a binary installs them for its process: a library that enabled `init` would decide the
/// log for a process it does not own, and one that enabled a sink would ship a database into every
/// role that logs. A dev-dependency is exempt, because a test target may drive a sink.
fn log_features(package: &Package, dep: &Dep) -> Option<String> {
    if dep.name != LOG_CRATE || dep.kind == DepKind::Dev || dep.features.is_empty() || package.is_binary {
        return None;
    }
    Some(format!(
        "`{}` enables the features {} of `{LOG_CRATE}`, but a library takes the facade alone: only a binary may enable a sink or `init`",
        package.name,
        dep.features.join(", ")
    ))
}

/// The cursor crate: the provider chain, theme lookup and image decoding for the pointer. It is a
/// foundation crate, so it names only the shared primitives and the logging facade among workspace
/// crates, and only the two crates that draw or configure the pointer may name it.
const CURSOR_CRATE: &str = "gnoblin-cursor";

/// The workspace crates `gnoblin-cursor` may depend on.
const CURSOR_DEPS: &[&str] = &["gnoblin-log", "gnoblin-types"];

/// The crates that name `gnoblin-cursor`: the compositor reads its settings and the Wayland
/// engine draws the pointer from its images.
const CURSOR_USERS: &[&str] = &["gnoblin-compositor", "gnoblin-wayland"];

/// Keeps the cursor crate light and its users explicit: it may name only [`CURSOR_DEPS`] among
/// workspace crates, and only [`CURSOR_USERS`] may depend on it.
fn cursor_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name == CURSOR_CRATE {
        if is_workspace_crate(&dep.name) && !CURSOR_DEPS.contains(&dep.name.as_str()) {
            return Some(format!(
                "`{CURSOR_CRATE}` may depend only on {} among workspace crates, not `{}`",
                CURSOR_DEPS.join(" and "),
                dep.name
            ));
        }
        return None;
    }
    if dep.name != CURSOR_CRATE || CURSOR_USERS.contains(&package.name.as_str()) {
        return None;
    }
    Some(format!(
        "only the pointer's users may depend on `{CURSOR_CRATE}`; `{}` is not one of them",
        package.name
    ))
}

/// The keyring's encrypted file. It is a leaf over the logging facade: the file format and its key
/// handling must not pull the compositor's graph into a process that only reads a keyring.
const KEYRING_STORE_CRATE: &str = "gnoblin-keyring-store";

/// Keeps the keyring store a leaf: it may name only the logging facade among workspace crates.
fn keyring_store_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != KEYRING_STORE_CRATE || !is_workspace_crate(&dep.name) || dep.name == LOG_CRATE {
        return None;
    }
    Some(format!(
        "`{KEYRING_STORE_CRATE}` may depend only on `{LOG_CRATE}` among workspace crates, not `{}`",
        dep.name
    ))
}

/// The Secret Service face is a thin layer over the keyring store. It may name only the store, the
/// hand-off protocol and the logging facade among workspace crates, so the bus code cannot reach
/// the compositor's graph.
fn service_keyring_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != SERVICE_KEYRING_CRATE || !is_workspace_crate(&dep.name) {
        return None;
    }
    if dep.name == LOG_CRATE || dep.name == KEYRING_STORE_CRATE || dep.name == KEYRING_HANDOFF_CRATE {
        return None;
    }
    Some(format!(
        "`{SERVICE_KEYRING_CRATE}` may depend only on `{KEYRING_STORE_CRATE}`, `{KEYRING_HANDOFF_CRATE}` and `{LOG_CRATE}` among workspace crates, not `{}`",
        dep.name
    ))
}

const SERVICE_KEYRING_CRATE: &str = "gnoblin-service-keyring";

/// The hand-off protocol is a leaf: the PAM module and the service both name it, so it may name no
/// workspace crate itself.
fn keyring_handoff_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != KEYRING_HANDOFF_CRATE || !is_workspace_crate(&dep.name) {
        return None;
    }
    Some(format!(
        "`{KEYRING_HANDOFF_CRATE}` may not depend on workspace crates, but depends on `{}`",
        dep.name
    ))
}

/// The PAM module runs inside every login's PAM stack, so it names only the hand-off protocol among
/// workspace crates: the store and the compositor's graph stay out of `login`.
fn pam_gnoblin_keyring_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != PAM_KEYRING_CRATE || !is_workspace_crate(&dep.name) || dep.name == KEYRING_HANDOFF_CRATE {
        return None;
    }
    Some(format!(
        "`{PAM_KEYRING_CRATE}` may depend only on `{KEYRING_HANDOFF_CRATE}` among workspace crates, not `{}`",
        dep.name
    ))
}

/// The SessionIdle agent is served on the ScreenSaver connection and reads the idle object through
/// the services role's host. It may name only the model it reads, the shared agent machinery, the
/// logging facade and the test kit, so it cannot reach the compositor's graph directly.
fn service_sessionidle_isolation(package: &Package, dep: &Dep) -> Option<String> {
    if package.name != SESSION_IDLE_CRATE || !is_workspace_crate(&dep.name) {
        return None;
    }
    if [SESSION_IDLE_MODEL, SESSION_IDLE_CORE, LOG_CRATE, SESSION_IDLE_TESTKIT].contains(&dep.name.as_str()) {
        return None;
    }
    Some(format!(
        "`{SESSION_IDLE_CRATE}` may depend only on `{SESSION_IDLE_MODEL}`, `{SESSION_IDLE_CORE}`, `{LOG_CRATE}` and `{SESSION_IDLE_TESTKIT}` among workspace crates, not `{}`",
        dep.name
    ))
}

const SESSION_IDLE_CRATE: &str = "gnoblin-service-sessionidle";
const SESSION_IDLE_MODEL: &str = "gnoblin-model";
const SESSION_IDLE_CORE: &str = "gnoblin-service-core";
const SESSION_IDLE_TESTKIT: &str = "gnoblin-service-testkit";

const KEYRING_HANDOFF_CRATE: &str = "gnoblin-keyring-handoff";
const PAM_KEYRING_CRATE: &str = "pam-gnoblin-keyring";

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
        let reported: Vec<String> = violations.iter().map(DepViolation::to_string).collect();
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with(
                "gnoblin-compat-dbus-mutter-screencast -> gnoblin-compat-dbus-mutter-clipboard [compat-isolation]"
            ),
            "unexpected violation: {}",
            reported[0]
        );
    }

    #[test]
    fn the_session_binary_may_depend_on_the_composition_root_and_the_guardian() {
        assert_eq!(
            violations(&[(
                "gnoblin",
                &["gnoblin-compositor", "gnoblin-session", "gnoblin-compat-host"],
            )]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_session_binary_links_the_compat_host_and_no_adapter() {
        let host_only = violations(&[("gnoblin", &["gnoblin-compositor", "gnoblin-compat-host"])]);
        assert!(
            !host_only.iter().any(|found| found.contains("[compat-host]")),
            "{host_only:?}"
        );
        let adapter = violations(&[("gnoblin", &["gnoblin-compat-wayland-gtk-shell1"])]);
        assert_eq!(
            adapter.iter().filter(|found| found.contains("[compat-host]")).count(),
            1,
            "{adapter:?}"
        );
    }

    #[test]
    fn only_the_compat_host_links_an_adapter_and_only_the_binary_links_the_host() {
        let through_host = violations(&[("gnoblin-compat-host", &["gnoblin-compat-wayland-gtk-shell1"])]);
        assert!(
            !through_host.iter().any(|found| found.contains("[compat-host]")),
            "{through_host:?}"
        );
        let library = violations(&[("gnoblin-compositor", &["gnoblin-compat-wayland-gtk-shell1"])]);
        assert_eq!(
            library.iter().filter(|found| found.contains("[compat-host]")).count(),
            1,
            "{library:?}"
        );
        let other_host = violations(&[("gnoblin-compositor", &["gnoblin-compat-host"])]);
        assert_eq!(
            other_host
                .iter()
                .filter(|found| found.contains("[compat-host]"))
                .count(),
            1,
            "{other_host:?}"
        );
    }
    #[test]
    fn no_adapter_links_the_compat_host() {
        let adapter = violations(&[("gnoblin-compat-dbus-mutter-clipboard", &["gnoblin-compat-host"])]);
        assert_eq!(
            adapter.iter().filter(|found| found.contains("[compat-host]")).count(),
            1,
            "{adapter:?}"
        );
    }

    #[test]
    fn the_services_core_links_no_compat_adapter() {
        let core = violations(&[("gnoblin-service-core", &["gnoblin-compat-dbus-mutter-clipboard"])]);
        assert_eq!(
            core.iter().filter(|found| found.contains("[compat-host]")).count(),
            1,
            "{core:?}"
        );
    }

    #[test]
    fn the_session_binary_may_depend_on_the_logging_facade() {
        assert_eq!(
            violations(&[("gnoblin", &["gnoblin-compositor", "gnoblin-log"])]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_portal_core_is_models_only_and_names_no_egui() {
        assert_eq!(
            violations(&[("gnoblin-portal-core", &["gnoblin-schema", "gnoblin-types"])]),
            Vec::<String>::new()
        );
        let found = violations(&[("gnoblin-portal-core", &["gnoblin-types", "egui"])]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("[portal-crates]"), "{found:?}");
        let found = violations(&[("gnoblin-portal-core", &["gnoblin-portal-ui"])]);
        assert!(found.iter().any(|found| found.contains("[portal-crates]")), "{found:?}");
        assert!(
            found.iter().any(|found| found.contains("[portal-layering]")),
            "{found:?}"
        );
    }

    #[test]
    fn the_portal_ui_crate_names_egui_and_the_core_only() {
        assert_eq!(
            violations(&[("gnoblin-portal-ui", &["egui", "gnoblin-portal-core", "gnoblin-types"])]),
            Vec::<String>::new()
        );
        let found = violations(&[("gnoblin-portal-ui", &["egui", "gnoblin-portal-consent"])]);
        assert!(found.iter().any(|found| found.contains("[portal-crates]")), "{found:?}");
        assert!(
            found.iter().any(|found| found.contains("[portal-layering]")),
            "{found:?}"
        );
    }

    #[test]
    fn a_portal_ui_crate_names_only_the_core_the_egui_layer_and_egui() {
        assert_eq!(
            violations(&[(
                "gnoblin-portal-consent",
                &["egui", "gnoblin-portal-core", "gnoblin-portal-ui", "gnoblin-types"]
            )]),
            Vec::<String>::new()
        );
        let found = violations(&[("gnoblin-portal-consent", &["egui", "png"])]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("[portal-crates]"), "{found:?}");
    }

    #[test]
    fn only_the_dialog_screens_and_the_compositor_may_name_the_egui_layer() {
        assert_eq!(
            violations(&[("gnoblin-compositor", &["gnoblin-portal-ui"])]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations(&[("gnoblin-portal-filechooser", &["gnoblin-portal-ui"])]),
            Vec::<String>::new()
        );
        let found = violations(&[("gnoblin-lua", &["gnoblin-portal-ui"])]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("[portal-layering]"), "{found:?}");
    }

    #[test]
    fn the_logging_facade_may_not_depend_on_a_workspace_crate() {
        let found = violations(&[("gnoblin-log", &["gnoblin-types"])]);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("[log-isolation]"), "{found:?}");
    }

    #[test]
    fn a_library_may_depend_on_the_logging_facade_without_features() {
        assert_eq!(
            violations(&[("gnoblin-wayland", &["gnoblin-log", "gnoblin-types"])]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_library_may_not_enable_a_logging_feature() {
        for feature in ["console", "pipeline", "journal", "init"] {
            let found = library_enabling_logging(false, &[feature]);
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("[log-features]"), "{found:?}");
        }
    }

    #[test]
    fn a_binary_may_enable_the_logging_features() {
        assert_eq!(library_enabling_logging(true, &["init"]), Vec::<String>::new());
        assert_eq!(library_enabling_logging(true, &["pipeline"]), Vec::<String>::new());
    }

    #[test]
    fn a_dev_dependency_may_enable_a_logging_feature() {
        let document = serde_json::json!({
            "packages": [{
                "name": "gnoblin-wayland",
                "targets": [{ "kind": ["lib"] }],
                "dependencies": [{
                    "name": "gnoblin-log",
                    "kind": "dev",
                    "features": ["pipeline"],
                }],
            }],
        });
        let graph = from_json(&document.to_string()).expect("the fixture parses");
        assert!(check(&graph).is_empty());
    }

    /// The violations for one package that enables a feature of the logging facade, which is a
    /// binary when `binary` is set and a library otherwise.
    fn library_enabling_logging(binary: bool, features: &[&str]) -> Vec<String> {
        let kind = if binary { "bin" } else { "lib" };
        let document = serde_json::json!({
            "packages": [{
                "name": "gnoblin-wayland",
                "targets": [{ "kind": [kind] }],
                "dependencies": [{
                    "name": "gnoblin-log",
                    "kind": null,
                    "features": features,
                }],
            }],
        });
        let graph = from_json(&document.to_string()).expect("the fixture parses");
        check(&graph).iter().map(DepViolation::to_string).collect()
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
    fn a_compat_adapter_may_link_the_services_library_but_not_the_engine() {
        // The `service` module of an adapter implements `LinkedAgent`, which lives in
        // `gnoblin-service-core`. That edge is allowed; the engine is still not, and
        // `gnoblin-service-core` itself stays under the client-side rule, which bans the engine.
        let allowed = violations(&[("gnoblin-compat-gtk-dconf-settings", &["gnoblin-service-core"])]);
        assert!(allowed.is_empty(), "expected no violation, got {allowed:#?}");
        let banned = violations(&[("gnoblin-compat-gtk-dconf-settings", &["gnoblin-compositor"])]);
        assert_eq!(banned.len(), 1, "expected one violation, got {banned:#?}");
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
        let reported: Vec<String> = violations.iter().map(DepViolation::to_string).collect();
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
        check(&graph(packages)).iter().map(DepViolation::to_string).collect()
    }

    /// The naming rule: an adapter names the transport it serves, and the two settings adapters
    /// that write a toolkit's own configuration are the only unprefixed names allowed.
    #[test]
    fn an_adapter_named_for_a_transport_passes_the_naming_rule() {
        assert_eq!(
            violations(&[
                ("gnoblin-compat-dbus-mutter-clipboard", &["gnoblin-compat-core"]),
                ("gnoblin-compat-wayland-gtk-shell1", &["gnoblin-compat-core"]),
                ("gnoblin-compat-x11-xsettings", &["gnoblin-compat-core"]),
                ("gnoblin-compat-core", &[]),
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_two_settings_adapters_carry_no_transport_prefix() {
        assert_eq!(
            violations(&[
                ("gnoblin-compat-gtk-dconf-settings", &["gnoblin-compat-core"]),
                ("gnoblin-compat-qt-portal-settings", &["gnoblin-compat-core"]),
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_unknown_unprefixed_adapter_fails_the_naming_rule() {
        let found = violations(&[("gnoblin-compat-mutter-clipboard", &["gnoblin-compat-core"])]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("gnoblin-compat-mutter-clipboard [compat-naming]: "),
            "{found:?}"
        );
    }

    #[test]
    fn an_unknown_transport_or_a_transport_without_a_name_fails_the_naming_rule() {
        for name in [
            "gnoblin-compat-file-gtk-dconf-settings",
            "gnoblin-compat-dbus",
            "gnoblin-compat-dbus-",
        ] {
            let found = violations(&[(name, &["gnoblin-compat-core"])]);
            assert_eq!(found.len(), 1, "{name}: {found:?}");
        }
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

    /// The DRM backend is an engine crate of the hardware half. It names the contract it
    /// implements, the GLES renderer it scans out through and the frame it draws, and no
    /// other engine crate: a renderer or the nested backend behind it would make the
    /// hardware path pull the other backends in.
    #[test]
    fn the_drm_backend_names_only_the_contract_the_renderer_and_the_frame() {
        assert_eq!(
            violations(&[
                (
                    "gnoblin-backend-drm",
                    &[
                        "gnoblin-backend",
                        "gnoblin-render-frame",
                        "gnoblin-render-gles",
                        "gnoblin-types",
                    ]
                ),
                ("gnoblin-backend", &["gnoblin-types"]),
                ("gnoblin-render-frame", &["gnoblin-render", "gnoblin-types"]),
                ("gnoblin-render", &["gnoblin-types"]),
            ]),
            Vec::<String>::new()
        );
    }

    /// The cursor crate names only the primitives and the logging facade, and only the pointer's
    /// two users depend on it.
    #[test]
    fn the_cursor_crate_names_only_its_primitives_and_the_logging_facade() {
        assert_eq!(
            violations(&[
                ("gnoblin-wayland", &["gnoblin-cursor"]),
                ("gnoblin-compositor", &["gnoblin-cursor"]),
                ("gnoblin-cursor", &["gnoblin-log", "gnoblin-types", "xcursor"]),
            ]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations(&[
                ("gnoblin-cursor", &["gnoblin-model", "gnoblin-types"]),
                ("gnoblin-services", &["gnoblin-cursor"]),
            ]),
            vec![
                "gnoblin-cursor -> gnoblin-model [cursor-isolation]: `gnoblin-cursor` may depend only on \
                 gnoblin-log and gnoblin-types among workspace crates, not `gnoblin-model`"
                    .to_owned(),
                "gnoblin-services -> gnoblin-cursor [cursor-isolation]: only the pointer's users may depend on \
                 `gnoblin-cursor`; `gnoblin-services` is not one of them"
                    .to_owned(),
            ]
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
            check(&graph).iter().map(DepViolation::to_string).collect::<Vec<_>>(),
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
            check(&graph).iter().map(DepViolation::to_string).collect::<Vec<_>>(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_render_frame_may_depend_on_the_scene_and_nothing_upward() {
        // Every renderer and the backend draw through the frame, so the frame must name
        // only the scene it describes. An upward edge would drag a renderer in behind any
        // crate that names the seam.
        assert_eq!(
            violations(&[
                ("gnoblin-render-frame", &["gnoblin-render", "gnoblin-types"]),
                ("gnoblin-render", &["gnoblin-types"]),
                ("gnoblin-backend", &["gnoblin-render-frame", "gnoblin-render"]),
                ("gnoblin-wayland", &["gnoblin-render-frame"]),
            ]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations(&[
                ("gnoblin-render-frame", &["gnoblin-backend"]),
                ("gnoblin-backend", &["gnoblin-types"]),
            ]),
            vec![
                "gnoblin-render-frame -> gnoblin-backend [engine-isolation]: the render frame crate depends on no \
                 engine crate but the pure scene crate; `gnoblin-backend` is an engine crate"
                    .to_owned()
            ]
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
    fn the_eis_server_may_name_the_backend_and_the_input_hub_only() {
        // The EIS server turns libei requests into seat events, so it needs the input hub's
        // event types and the backend's device names. Any other mechanism crate would pull a
        // compositor in behind a client-facing server.
        assert_eq!(
            violations(&[
                (
                    "gnoblin-input-eis",
                    &["gnoblin-backend", "gnoblin-input", "gnoblin-types"]
                ),
                ("gnoblin-input", &["gnoblin-types"]),
            ]),
            Vec::<String>::new()
        );
        let reported = violations(&[
            ("gnoblin-input-eis", &["gnoblin-wayland"]),
            ("gnoblin-wayland", &["gnoblin-backend"]),
        ]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-input-eis -> gnoblin-wayland [engine-isolation]"),
            "unexpected violation: {}",
            reported[0]
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
    fn only_the_wayland_crate_may_name_the_pipewire_export() {
        // The export is the one place `pipewire` is linked. A second crate naming it would
        // be a second owner of the PipeWire stack, so the rule rejects both the edge to the
        // export and a direct link to `pipewire`.
        let reported = violations(&[
            ("gnoblin-wayland", &["gnoblin-pipewire-export"]),
            ("gnoblin-session", &["gnoblin-pipewire-export"]),
            ("gnoblin-render", &["pipewire"]),
        ]);
        assert_eq!(reported.len(), 2, "expected two violations, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-session -> gnoblin-pipewire-export [pipewire-export-isolation]"),
            "unexpected violation: {}",
            reported[0]
        );
        assert!(
            reported[1].starts_with("gnoblin-render -> pipewire [pipewire-export-isolation]"),
            "unexpected violation: {}",
            reported[1]
        );
    }

    #[test]
    fn the_pipewire_export_may_name_only_its_allowed_crates() {
        // The export is fed plain values and a `PlaneSource` by the wayland crate, so it
        // must not name the wayland crate (a cycle) or any other workspace crate beyond the
        // frame-building set.
        let reported = violations(&[(
            "gnoblin-pipewire-export",
            &["gnoblin-capture", "gnoblin-wayland", "pipewire"],
        )]);
        assert_eq!(reported.len(), 1, "expected one violation, got {reported:#?}");
        assert!(
            reported[0].starts_with("gnoblin-pipewire-export -> gnoblin-wayland [pipewire-export-isolation]"),
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
        let reported: Vec<String> = violations.iter().map(DepViolation::to_string).collect();
        assert!(
            reported.is_empty(),
            "workspace violates its own rules:\n{}",
            reported.join("\n")
        );
    }
}
