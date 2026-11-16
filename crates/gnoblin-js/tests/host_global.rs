//! `globalThis.__gnoblinHost`, the bridge the shell calls into every frame, against a script that tries to replace it.
//!
//! The library reads the bridge once when it loads, but the shell reads `__gnoblinHost.entry` from the global on every
//! frame: the toolkit's frame, the lock screen's among it, is what `entry.ui` runs. A script that could assign or delete
//! the global would replace what the shell runs, so the global is defined read-only and non-configurable. The members of
//! the bridge stay assignable, and the tests that fail the shell's own module on purpose rely on that
//! (`crates/gnoblin/tests/js_lock_failures.rs`).

use std::fs;
use std::path::Path;

use gnoblin_js::JsShell;

mod common;
use common::{FakeCompositor, honest_config, rig_with};

#[test]
fn a_script_cannot_replace_delete_or_redefine_the_host_bridge() {
    let mut rig = rig_with("", "");
    rig.settle();
    // The shell evaluates in strict mode, where a write to a read-only global or a delete of it throws.
    let answer = rig.eval(
        "(() => {\n\
         \x20   const bridge = globalThis.__gnoblinHost;\n\
         \x20   const tried = [];\n\
         \x20   const attempt = (name, run) => { try { run(); tried.push(name + ':allowed'); } catch (error) { tried.push(name + ':threw'); } };\n\
         \x20   attempt('assign', () => { globalThis.__gnoblinHost = { entry: { ui: () => {} } }; });\n\
         \x20   attempt('delete', () => { delete globalThis.__gnoblinHost; });\n\
         \x20   attempt('define', () => { Object.defineProperty(globalThis, '__gnoblinHost', { value: {} }); });\n\
         \x20   const descriptor = Object.getOwnPropertyDescriptor(globalThis, '__gnoblinHost');\n\
         \x20   return [...tried, globalThis.__gnoblinHost === bridge, descriptor.writable, descriptor.configurable].join(' ');\n\
         })()",
    );
    assert_eq!(
        answer, "assign:threw delete:threw define:threw true false false",
        "no attempt took, the bridge is the same object, and it is neither writable nor configurable"
    );
    // The shell still runs its frame through the original bridge.
    let told = rig.settle();
    assert!(told.is_empty(), "{told:?}");
    assert_eq!(rig.eval("typeof __gnoblinHost.entry.ui"), "function");
}

#[test]
fn a_module_that_assigns_the_host_bridge_fails_to_load_and_the_bridge_stays() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        "globalThis.__gnoblinHost = { entry: { ui: () => {} } };\n",
    )
    .expect("init.ts");
    let mut shell = JsShell::new(Box::new(FakeCompositor::new()), honest_config(&root)).expect("the shell boots");
    let refused = shell
        .load_entry(Path::new("init.ts"))
        .expect_err("an entry that overwrites the bridge does not load");
    assert!(
        refused.to_string().contains("read-only"),
        "the script that tried is told why: {refused}"
    );
    // The bridge is the shell's own still.
    let kind = shell.eval("typeof __gnoblinHost.entry.ui").expect("the script runs");
    assert_eq!(kind.as_deref(), Some("function"));
}
