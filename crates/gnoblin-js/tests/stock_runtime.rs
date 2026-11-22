//! Which stock library a shell runs on (`gnoblin_js` `choice`): decided from the shell's static import graph before any module
//! evaluates, so a shell never has the TypeScript library's object model and the native library's classes together.
//!
//! The import log of the loader (`JsShell::embedded_modules`) says what a generation loaded. A shell that imports a package entry,
//! or no gnoblin module at all (it reads the global `gnoblin`), or that reaches the XR extension or a dynamic `import()`, loads the
//! TypeScript host and none of the native classes; a shell that imports only the generated object modules loads those and no
//! TypeScript module. A reload that would change the choice is refused and the old generation keeps serving.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gnoblin_js::JsRoleLinks;
use gnoblin_testkit::compositor::FakeCompositor;

mod common;
use common::*;

const HOST: &str = "gnoblin-core/host";
const OBJECTS: &str = "import { Gnoblin } from \"gnoblin-core/generated/objects\";\n";

fn loaded(rig: &Rig) -> Vec<String> {
    rig.shell.embedded_modules()
}

/// A shell over a core whose directory holds `files`, `shell.ts` being the entry.
fn rig_files(files: &[(&str, &str)]) -> Rig {
    let comp = FakeCompositor::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    for (name, text) in files {
        fs::write(root.join(name), text).expect("a module is written");
    }
    let (config, clock) = timed_config(&root);
    let (mut shell, core) = native::shell_over_core(&comp, Arc::new(clock.clone()), config);
    let load_diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    Rig {
        shell,
        comp,
        load_diagnostics,
        clock,
        core: Some(core),
        _dir: dir,
    }
}

#[test]
fn a_shell_that_imports_gnoblin_core_runs_on_the_typescript_library() {
    let mut rig = rig_with(
        "import { gnoblin } from \"gnoblin-core\";\n",
        "(globalThis as any).seen = gnoblin.clients.length;",
    );
    let modules = loaded(&rig);
    assert!(modules.iter().any(|name| name == HOST), "{modules:?}");
    assert!(
        modules.iter().all(|name| !name.contains("generated/object")),
        "no native class is loaded beside the TypeScript object model: {modules:?}"
    );
    assert_eq!(rig.eval("globalThis.seen"), "0");
}

#[test]
fn a_shell_that_imports_no_gnoblin_module_and_reads_the_global_runs_on_the_typescript_library() {
    let mut rig = rig_with("", "(globalThis as any).seen = gnoblin.clients.length;");
    assert!(loaded(&rig).iter().any(|name| name == HOST), "{:?}", loaded(&rig));
    assert_eq!(rig.eval("globalThis.seen"), "0");
}

#[test]
fn a_shell_that_imports_only_the_generated_modules_loads_no_typescript_module() {
    let mut rig = rig_setup(
        OBJECTS,
        "(globalThis as any).seen = Gnoblin.get().clients.length;",
        |comp| {
            comp.add_desktop(1000.0, 800.0);
            comp.add_node(3, ROOT, "window", vec![("title", text("one"))]);
        },
    );
    rig.settle();
    let modules = loaded(&rig);
    assert!(modules.iter().all(|name| name != HOST), "{modules:?}");
    assert!(
        modules
            .iter()
            .all(|name| name.starts_with("gnoblin-core/generated/") || name.starts_with("gnoblin-std/generated/")),
        "only the generated object modules are loaded: {modules:?}"
    );
    assert_eq!(
        rig.eval("globalThis.seen"),
        "1",
        "the generated classes read the core's windows"
    );
}

#[test]
fn a_helper_that_reaches_the_xr_extension_puts_the_whole_shell_on_the_typescript_library() {
    let rig = rig_files(&[
        (
            "shell.ts",
            "import \"./helper\";\nimport { Gnoblin } from \"gnoblin-core/generated/objects\";\nvoid Gnoblin;\n",
        ),
        ("helper.ts", "import \"gnoblin-std/extensions/xr/exports\";\n"),
    ]);
    assert!(loaded(&rig).iter().any(|name| name == HOST), "{:?}", loaded(&rig));
}

#[test]
fn a_dynamic_import_in_the_graph_puts_the_whole_shell_on_the_typescript_library() {
    let rig = rig_files(&[
        ("shell.ts", &format!("{OBJECTS}import \"./lazy\";\nvoid Gnoblin;\n")),
        ("lazy.ts", "export const later = (name: string) => import(name);\n"),
    ]);
    assert!(loaded(&rig).iter().any(|name| name == HOST), "{:?}", loaded(&rig));
}

// -- a reload keeps the library -------------------------------------------------------------------------------------

fn role_over(
    init: &str,
) -> (
    gnoblin_js::JsRole,
    std::path::PathBuf,
    (tempfile::TempDir, native::NativeCore),
) {
    let comp = FakeCompositor::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), init).expect("shell.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    let core = native::NativeCore::new(&comp, Arc::clone(&config.shell.clock));
    config.shell.js.natives.push(core.embedded().std_native());
    config.shell.js.natives.push(core.embedded().object_native());
    let role = gnoblin_js::JsRole::over_link(core.link(), JsRoleLinks::default(), config).expect("a role");
    (role, root, (dir, core))
}

fn marker(role: &mut gnoblin_js::JsRole) -> String {
    role.call_script(|ctx| ctx.globals().get::<_, String>("marker"))
        .expect("the marker is read")
}

#[test]
fn a_reload_that_changes_the_library_is_refused_and_the_old_shell_keeps_serving() {
    let generated = format!("{OBJECTS}(globalThis as any).marker = \"generated one\";\nvoid Gnoblin;\n");
    let (mut role, root, _keep) = role_over(&generated);
    assert_eq!(marker(&mut role), "generated one");

    // Now the entry needs the TypeScript library (a package entry): refused, naming the cause.
    fs::write(
        root.join("shell.ts"),
        "import { gnoblin } from \"gnoblin-core\";\n(globalThis as any).marker = \"typescript\";\nvoid gnoblin;\n",
    )
    .expect("shell.ts is rewritten");
    let report = role.reload_now();
    assert!(!report.ok, "the reload that changes the library is refused");
    let text = report
        .errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(text.contains("restart gnoblin to change the stock library"), "{text}");
    assert!(text.contains("gnoblin-core"), "the cause is named: {text}");
    assert_eq!(marker(&mut role), "generated one", "the old generation still serves");
    role.tick().expect("the old shell still runs a frame");

    // A reload that keeps the library is taken.
    fs::write(
        root.join("shell.ts"),
        format!("{OBJECTS}(globalThis as any).marker = \"generated two\";\nvoid Gnoblin;\n"),
    )
    .expect("shell.ts is rewritten");
    let report = role.reload_now();
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(marker(&mut role), "generated two");
}

#[test]
fn a_reload_from_the_typescript_library_to_the_generated_one_is_refused_the_same_way() {
    let (mut role, root, _keep) = role_over(
        "import { gnoblin } from \"gnoblin-core\";\n(globalThis as any).marker = \"typescript\";\nvoid gnoblin;\n",
    );
    assert_eq!(marker(&mut role), "typescript");
    fs::write(
        root.join("shell.ts"),
        format!("{OBJECTS}(globalThis as any).marker = \"generated\";\nvoid Gnoblin;\n"),
    )
    .expect("shell.ts is rewritten");
    let report = role.reload_now();
    assert!(!report.ok);
    let text = report
        .errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(text.contains("restart gnoblin to change the stock library"), "{text}");
    assert_eq!(marker(&mut role), "typescript");

    // Staying on the library is taken.
    fs::write(
        root.join("shell.ts"),
        "import { gnoblin } from \"gnoblin-core\";\n(globalThis as any).marker = \"typescript two\";\nvoid gnoblin;\n",
    )
    .expect("shell.ts is rewritten");
    assert!(role.reload_now().ok);
    assert_eq!(marker(&mut role), "typescript two");
}

#[test]
fn a_first_load_with_a_missing_import_fails_as_it_always_did_on_the_typescript_library() {
    let comp = FakeCompositor::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), format!("{OBJECTS}import \"./nowhere\";\n")).expect("shell.ts");
    let (config, clock) = timed_config(&root);
    let (mut shell, _core) = native::shell_over_core(&comp, Arc::new(clock), config);
    let error = shell
        .load_entry(Path::new("shell.ts"))
        .expect_err("the import is missing");
    let text = error.to_string();
    assert!(text.contains("./nowhere"), "{text}");
    assert!(
        shell
            .embedded_modules()
            .iter()
            .all(|name| !name.contains("generated/object")),
        "an unreadable graph runs on the TypeScript library, where the shell started: {:?}",
        shell.embedded_modules()
    );
}
