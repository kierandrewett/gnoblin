//! Every member of `gnoblin` that the runtime has is declared in `gnoblin-stock.d.ts`, and every member the d.ts
//! declares is there at runtime (`design/js-parity.md`, finding 5: "types promise what the runtime lacks").
//!
//! The runtime's members come from a booted `JsShell` over the fake link: the own and prototype properties of the
//! `gnoblin` global, which is what an editor completes and what a script can call. The declared ones come from
//! `interface Gnoblin` in `crates/gnoblin-js-types/static/gnoblin-stock.d.ts`, the text the generated `gnoblin.d.ts`
//! ends with. The test lives here and not in `gnoblin-js-types` because that crate depends on the schema
//! and nothing else of the workspace: a shell to boot is not something it may link.

mod common;

use std::collections::BTreeSet;

use common::rig_with;

/// Members the d.ts declares that a lane has not yet put in the runtime. Each entry names the lane that lands it and
/// is deleted by that lane: the test fails once the runtime has the member, so an entry cannot outlive its reason.
const NOT_YET_IN_RUNTIME: &[&str] = &[];

/// The members the runtime keeps for itself: the hooks `defineConfig` and the inspector call.
const INTERNAL: &[&str] = &["constructor", "__register", "__effectiveConfig"];

/// The names `interface Gnoblin { .. }` declares, from the static declarations.
fn declared_members(dts: &str) -> BTreeSet<String> {
    let start = dts
        .find("export interface Gnoblin {")
        .expect("the d.ts declares interface Gnoblin");
    let body = &dts[start..];
    let end = body.find("\n    }\n").expect("interface Gnoblin ends");
    let mut names = BTreeSet::new();
    for line in body[..end].lines().skip(1) {
        // A member sits at the interface's own indent; its doc comment and any wrapped line sit deeper or start
        // with a comment marker.
        let Some(rest) = line.strip_prefix("        ") else {
            continue;
        };
        if rest.starts_with([' ', '/', '*']) {
            continue;
        }
        let rest = rest.strip_prefix("readonly ").unwrap_or(rest);
        let name: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        if !name.is_empty() && rest[name.len()..].starts_with(['(', ':', '?', '<']) {
            names.insert(name);
        }
    }
    names
}

/// The names the `gnoblin` global has: its own properties and its class's.
fn runtime_members() -> BTreeSet<String> {
    let mut rig = rig_with("", "export {};\n");
    let listing = rig.eval(
        "(() => {
            const names = new Set(Object.getOwnPropertyNames(gnoblin));
            for (const name of Object.getOwnPropertyNames(Object.getPrototypeOf(gnoblin))) names.add(name);
            return [...names].join(',');
        })()",
    );
    listing
        .split(',')
        .filter(|name| !INTERNAL.contains(name))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_runtime_and_the_declarations_name_the_same_members_of_gnoblin() {
    let dts = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-js-types/static/gnoblin-stock.d.ts"
    ))
    .expect("the stock declarations are checked in");
    let declared = declared_members(&dts);
    let runtime = runtime_members();
    assert!(
        declared.contains("clients") && declared.contains("process"),
        "the d.ts parse found the members: {declared:?}"
    );

    let undeclared: Vec<&String> = runtime.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "the runtime has members the d.ts does not declare: {undeclared:?}"
    );

    let absent: Vec<&String> = declared.difference(&runtime).collect();
    let allowed: Vec<&str> = NOT_YET_IN_RUNTIME.to_vec();
    let unexpected: Vec<&&String> = absent.iter().filter(|name| !allowed.contains(&name.as_str())).collect();
    assert!(
        unexpected.is_empty(),
        "the d.ts declares members the runtime lacks (remove the declaration or implement it): {unexpected:?}"
    );
    let landed: Vec<&&str> = allowed.iter().filter(|name| runtime.contains(**name)).collect();
    assert!(
        landed.is_empty(),
        "the runtime has these now: delete them from NOT_YET_IN_RUNTIME: {landed:?}"
    );
}

// ---- the module's exports --------------------------------------------------------------------------------------------

/// Value exports the d.ts declares in `declare module "gnoblin"` that a lane has not put in the runtime, each with the
/// lane that lands it. The test fails once the runtime has the export, so an entry cannot outlive its reason.
const DECLARED_NOT_IN_RUNTIME: &[(&str, &str)] = &[];

/// Exports the runtime has that the d.ts does not declare, each with why. A name here is not a promise to a script
/// author: it is plumbing the stock modules share. The test fails once the d.ts declares the name.
const RUNTIME_NOT_DECLARED: &[(&str, &str)] = &[
    // The classes behind the objects of the `gnoblin` global. A script reaches the object through `gnoblin`, whose type
    // names an interface for it, and never constructs one.
    ("AuthenticationManager", MANAGER),
    ("DialogManager", MANAGER),
    ("IdleMonitor", MANAGER),
    ("InputManager", MANAGER),
    ("LaunchFeedback", MANAGER),
    (
        "Collection",
        "typed as an interface; the class is the runtime's and a script does not construct one",
    ),
    (
        "MutableCollection",
        "typed as an interface; the class is the runtime's and a script does not construct one",
    ),
    // The window menu's internals.
    (
        "MenuItem",
        "an entry of the stock window menu, built by the menu itself",
    ),
    ("MenuPanel", "the stock window menu's panel, built by the menu itself"),
    ("MENU_COLOURS", "the stock window menu's palette"),
    ("defaultMenuItems", "the stock window menu's entries"),
    // Tuning constants of the stock modules, read by their tests.
    ("CLOSE_FADE_MS", CONSTANT),
    ("CLOSE_RESTORE_MS", CONSTANT),
    ("DEFAULT_LAUNCH_TIMEOUT_MS", CONSTANT),
    ("MAX_IDLE_MS", CONSTANT),
    ("MAX_IDLE_WATCHES", CONSTANT),
    ("MAX_LAUNCHES", CONSTANT),
    ("MAX_LAUNCH_TIMEOUT_MS", CONSTANT),
    ("MIN_LAUNCH_TIMEOUT_MS", CONSTANT),
    (
        "normalizeApplication",
        "checks the application a launch names, for `gnoblin.launches`",
    ),
    // The pipeline `defineConfig` runs, which the host and the tests call.
    ("CONFIG_KEYS", CONFIG_PIPELINE),
    ("DOCUMENT_LISTS", CONFIG_PIPELINE),
    ("DOCUMENT_SECTIONS", CONFIG_PIPELINE),
    ("defaultConfig", CONFIG_PIPELINE),
    ("desugarConfig", CONFIG_PIPELINE),
    ("mergeConfig", CONFIG_PIPELINE),
    ("outputRequest", CONFIG_PIPELINE),
    ("planStockLauncher", CONFIG_PIPELINE),
    // Helpers the stock layouts and bindings share.
    ("checkGap", LAYOUT_HELPER),
    ("clamp", LAYOUT_HELPER),
    ("insetRect", LAYOUT_HELPER),
    ("placeRect", LAYOUT_HELPER),
    ("tile", LAYOUT_HELPER),
    ("tilingGeometry", LAYOUT_HELPER),
    (
        "checkTrigger",
        "validates a trigger text for the bindings; a script uses `parseTrigger`",
    ),
    ("claimOf", "the seat claim a trigger needs, read by the input module"),
    (
        "resolveTriggers",
        "turns a layout's `triggers` option into bindings, for the stock layouts",
    ),
    // The UI toolkit's hooks, which the host calls once a frame and the stock modules use.
    ("attachUi", UI_HOOK),
    ("estimateText", UI_HOOK),
    ("onUiFrame", UI_HOOK),
    ("onUiRequest", UI_HOOK),
    ("runUiFrame", UI_HOOK),
    ("uiFocus", UI_HOOK),
    ("uiFocused", UI_HOOK),
    ("uiLocked", UI_HOOK),
    ("uiNodeGone", UI_HOOK),
    ("uiNow", UI_HOOK),
    ("uiPointer", UI_HOOK),
    ("uiReleaseAll", UI_HOOK),
    ("uiScreen", UI_HOOK),
    ("uiSeatAction", UI_HOOK),
    ("utf8", UI_HOOK),
];

const MANAGER: &str = "the class behind a `gnoblin` member, which the `Gnoblin` interface types as an interface";
const CONSTANT: &str = "a tuning constant of a stock module, read by its tests";
const CONFIG_PIPELINE: &str = "part of the pipeline `defineConfig` runs, called by the host and the tests";
const LAYOUT_HELPER: &str = "a helper the stock layouts share";
const UI_HOOK: &str = "a hook of the UI toolkit that the host and the stock modules call";

/// The body of `declare module "gnoblin" { ... }`: from its opening line to the closing brace at the margin.
fn gnoblin_module(dts: &str) -> &str {
    let open = "declare module \"gnoblin\" {\n";
    let start = dts.find(open).expect("the d.ts declares the module gnoblin") + open.len();
    let body = &dts[start..];
    let end = body.find("\n}\n").expect("the module gnoblin ends at the margin");
    &body[..end]
}

/// The first identifier of `text`.
fn identifier(text: &str) -> String {
    text.chars()
        .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
        .collect()
}

/// The names a module body exports as values: its classes, functions, consts, enums, namespaces and the names a
/// re-export (`export { A, B } from "..."`) lists. Interfaces and type aliases have no runtime value.
fn declared_exports(module: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in module.lines() {
        let Some(rest) = line.strip_prefix("    export ") else {
            continue;
        };
        if let Some(list) = rest.strip_prefix('{') {
            let list = list.split('}').next().unwrap_or_default();
            names.extend(
                list.split(',')
                    .map(|item| identifier(item.trim()))
                    .filter(|name| !name.is_empty()),
            );
            continue;
        }
        let rest = rest.strip_prefix("declare ").unwrap_or(rest);
        let rest = rest.strip_prefix("abstract ").unwrap_or(rest);
        for keyword in ["class ", "function ", "const ", "let ", "var ", "enum ", "namespace "] {
            if let Some(after) = rest.strip_prefix(keyword) {
                names.insert(identifier(after));
            }
        }
    }
    names
}

/// The export names of the runtime's `gnoblin` module: `Object.keys` of its namespace in a booted shell.
fn runtime_exports() -> BTreeSet<String> {
    let mut rig = rig_with(
        "import * as namespace from 'gnoblin';\n",
        "(globalThis as any).exported = Object.keys(namespace).join(',');",
    );
    rig.eval("globalThis.exported")
        .split(',')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A declared export the runtime lacks, or a runtime export the d.ts lacks, that is not allow-listed; and an allow-list
/// entry that no longer applies.
fn export_problems(
    declared: &BTreeSet<String>,
    runtime: &BTreeSet<String>,
    declared_only: &[(&str, &str)],
    runtime_only: &[(&str, &str)],
) -> Vec<String> {
    let mut problems = Vec::new();
    for name in declared.difference(runtime) {
        if !declared_only.iter().any(|(allowed, _)| allowed == name) {
            problems.push(format!(
                "`{name}` is declared in the d.ts and is not exported at runtime"
            ));
        }
    }
    for name in runtime.difference(declared) {
        if !runtime_only.iter().any(|(allowed, _)| allowed == name) {
            problems.push(format!(
                "`{name}` is exported at runtime and is not declared in the d.ts"
            ));
        }
    }
    for (name, lane) in declared_only {
        if runtime.contains(*name) {
            problems.push(format!(
                "`{name}` is exported now: delete it from DECLARED_NOT_IN_RUNTIME ({lane})"
            ));
        } else if !declared.contains(*name) {
            problems.push(format!(
                "`{name}` is no longer declared: delete it from DECLARED_NOT_IN_RUNTIME ({lane})"
            ));
        }
    }
    for (name, why) in runtime_only {
        if declared.contains(*name) {
            problems.push(format!(
                "`{name}` is declared now: delete it from RUNTIME_NOT_DECLARED ({why})"
            ));
        } else if !runtime.contains(*name) {
            problems.push(format!(
                "`{name}` is no longer exported: delete it from RUNTIME_NOT_DECLARED ({why})"
            ));
        }
    }
    problems
}

#[test]
fn the_runtime_exports_what_the_module_declares_and_declares_what_it_exports() {
    let dts = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-js-types/static/gnoblin-stock.d.ts"
    ))
    .expect("the stock declarations are checked in");
    let declared = declared_exports(gnoblin_module(&dts));
    let runtime = runtime_exports();
    assert!(
        declared.contains("Client") && declared.contains("defineConfig") && declared.contains("RULE_DEFAULTS"),
        "the d.ts parse found a class, a function and a const: {declared:?}"
    );
    assert!(
        runtime.contains("Client") && runtime.contains("defineConfig"),
        "the runtime's namespace listing found them: {runtime:?}"
    );
    let problems = export_problems(&declared, &runtime, DECLARED_NOT_IN_RUNTIME, RUNTIME_NOT_DECLARED);
    assert!(
        problems.is_empty(),
        "the d.ts and the runtime disagree on the exports of `gnoblin`:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_export_check_reports_each_kind_of_disagreement() {
    let set = |names: &[&str]| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<String>>()
    };
    let problems = export_problems(
        &set(&["A", "B", "Late", "Now"]),
        &set(&["A", "C", "Ghost", "Now"]),
        &[("Late", "SomeLane"), ("Now", "OtherLane"), ("Gone", "ThirdLane")],
        &[("Ghost", "plumbing"), ("A", "stale reason")],
    );
    assert_eq!(
        problems,
        [
            "`B` is declared in the d.ts and is not exported at runtime",
            "`C` is exported at runtime and is not declared in the d.ts",
            "`Now` is exported now: delete it from DECLARED_NOT_IN_RUNTIME (OtherLane)",
            "`Gone` is no longer declared: delete it from DECLARED_NOT_IN_RUNTIME (ThirdLane)",
            "`A` is declared now: delete it from RUNTIME_NOT_DECLARED (stale reason)",
        ]
    );
}

#[test]
fn the_export_parse_reads_classes_functions_consts_and_reexports_and_skips_types() {
    let module = "    export interface Shape {}\n    export type Alias = number;\n    export abstract class Base {}\n    \
                  export class Derived extends Base {\n        export x = 1;\n    }\n    export function make(): void;\n    \
                  export namespace make {\n    }\n    export const LIMIT: number;\n    export { Camera3D, View } from \"x\";\n";
    assert_eq!(
        declared_exports(module),
        ["Base", "Camera3D", "Derived", "LIMIT", "View", "make"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>()
    );
}
