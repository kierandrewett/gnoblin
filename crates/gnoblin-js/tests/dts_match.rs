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
const NOT_YET_IN_RUNTIME: &[&str] = &[
    // JsOutputs adds `gnoblin.outputs` (the live outputs as a collection).
    "outputs",
];

/// The members the runtime keeps for itself: the hooks `defineConfig` and the inspector call.
const INTERNAL: &[&str] = &["constructor", "__register", "__effectiveConfig"];

/// The names `interface Gnoblin { .. }` declares, from the static declarations.
fn declared_members(dts: &str) -> BTreeSet<String> {
    let start = dts.find("export interface Gnoblin {").expect("the d.ts declares interface Gnoblin");
    let body = &dts[start..];
    let end = body.find("\n    }\n").expect("interface Gnoblin ends");
    let mut names = BTreeSet::new();
    for line in body[..end].lines().skip(1) {
        // A member sits at the interface's own indent; its doc comment and any wrapped line sit deeper or start
        // with a comment marker.
        let Some(rest) = line.strip_prefix("        ") else { continue };
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
    assert!(declared.contains("clients") && declared.contains("process"), "the d.ts parse found the members: {declared:?}");

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
