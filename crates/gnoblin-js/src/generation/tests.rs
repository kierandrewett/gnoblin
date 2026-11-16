//! Tests of this module.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;

use super::{JsConfig, JsGeneration, split_frame};
use crate::error::JsErrorKind;

/// A canonical temp config root.
struct Root {
    _dir: TempDir,
    path: PathBuf,
}

fn root_with(files: &[(&str, &str)]) -> Root {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = fs::canonicalize(dir.path()).expect("the temp dir exists");
    for (name, text) in files {
        let file = path.join(name);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("the parent directory");
        }
        fs::write(file, text).expect("the module is written");
    }
    Root { _dir: dir, path }
}

fn generation(root: &Root) -> JsGeneration {
    JsGeneration::new(&JsConfig::new(&root.path), 1).expect("a generation")
}

fn global_number(generation: &JsGeneration, name: &'static str) -> Option<i32> {
    generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, Option<i32>>(name))
        .expect("reading a global")
}

#[test]
fn evaluates_a_module_and_imports_a_ts_sibling() {
    let root = root_with(&[
        (
            "init.ts",
            "import { answer } from './x.ts';\nconst n: number = answer;\n(globalThis as any).result = n;\n",
        ),
        ("x.ts", "export const answer: number = 42;\n"),
    ]);
    let generation = generation(&root);
    generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
    assert_eq!(global_number(&generation, "result"), Some(42));
}

#[test]
fn module_paths_lists_the_entry_and_its_transitive_imports() {
    let root = root_with(&[
        (
            "init.ts",
            "import './lib/a.ts';\nimport { gnoblin } from 'gnoblin-core';\n",
        ),
        ("lib/a.ts", "import '../b.js';\n"),
        ("b.js", "globalThis.b = 1;\n"),
        ("unused.ts", "export {};\n"),
    ]);
    let config = JsConfig::new(&root.path).embed("gnoblin-core", "export const gnoblin = 1;\n");
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    assert!(generation.module_paths().is_empty(), "nothing is loaded before eval");
    generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
    assert_eq!(
        generation.module_paths(),
        vec![
            root.path.join("b.js"),
            root.path.join("init.ts"),
            root.path.join("lib/a.ts"),
        ]
    );
}

#[test]
fn js_import_falls_back_to_the_ts_file_and_js_files_pass_through() {
    let root = root_with(&[
        (
            "init.js",
            "import { a } from './a.js';\nimport { b } from './b.js';\nglobalThis.sum = a + b;\n",
        ),
        ("a.ts", "export const a: number = 1;\n"),
        ("b.js", "export const b = 2;\n"),
    ]);
    let generation = generation(&root);
    generation.eval_module(Path::new("init.js")).expect("init.js evaluates");
    assert_eq!(global_number(&generation, "sum"), Some(3));
}

#[test]
fn embedded_modules_resolve_by_name() {
    let root = root_with(&[(
        "init.ts",
        "import { shell } from 'gnoblin-core';\nimport { grid } from 'gnoblin-std/layouts';\n(globalThis as any).out = shell + grid;\n",
    )]);
    let config = JsConfig::new(&root.path)
        .embed("gnoblin-core", "export const shell = 10;")
        .embed("gnoblin-std/layouts", "export const grid = 5;");
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
    assert_eq!(global_number(&generation, "out"), Some(15));
}

#[test]
fn unknown_embedded_and_bare_modules_are_load_errors() {
    let root = root_with(&[
        ("a.ts", "import 'gnoblin-core/missing';\n"),
        ("b.ts", "import 'lodash';\n"),
    ]);
    let generation = generation(&root);
    for entry in ["a.ts", "b.ts"] {
        let error = generation
            .eval_module(Path::new(entry))
            .expect_err("the import cannot resolve");
        assert_eq!(error.kind, JsErrorKind::Load, "{entry}: {error}");
    }
}

#[test]
fn a_missing_import_is_reported_at_the_importing_line() {
    let root = root_with(&[
        // The imports are used: the stripper drops an import nothing refers to, as TypeScript does.
        (
            "init.ts",
            "const a = 1;\nimport { b } from './sub/b.ts';\nexport const x = a + b;\n",
        ),
        ("sub/b.ts", "import { c } from './absent.ts';\nexport const b = c;\n"),
    ]);
    let generation = generation(&root);
    let error = generation
        .eval_module(Path::new("init.ts"))
        .expect_err("./absent.ts does not exist");
    assert_eq!(error.kind, JsErrorKind::Load, "{error}");
    assert!(error.message.contains("./absent.ts"), "{error}");
    let at = error.location.as_ref().expect("the import has a position");
    assert_eq!(at.file, root.path.join("sub/b.ts"));
    assert_eq!((at.line, at.column), (1, 19), "{error}");
}

#[test]
fn import_that_climbs_out_of_the_root_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let base = fs::canonicalize(dir.path()).expect("exists");
    fs::create_dir_all(base.join("root")).expect("root");
    fs::write(base.join("secret.ts"), "globalThis.leaked = 1;\n").expect("secret");
    fs::write(base.join("root/init.ts"), "import '../secret.ts';\n").expect("init");
    let generation = JsGeneration::new(&JsConfig::new(base.join("root")), 1).expect("a generation");
    let error = generation
        .eval_module(Path::new("init.ts"))
        .expect_err("the import leaves the root");
    assert_eq!(error.kind, JsErrorKind::Load, "{error}");
    assert!(error.message.contains("outside the config root"), "{error}");
    assert_eq!(global_number(&generation, "leaked"), None);
}

#[test]
fn syntax_error_in_an_imported_ts_file_is_reported_in_ts_coordinates() {
    let root = root_with(&[
        ("init.ts", "import './bad.ts';\n"),
        ("bad.ts", "const ok = 1;\nconst broken = ;\n"),
    ]);
    let generation = generation(&root);
    let error = generation
        .eval_module(Path::new("init.ts"))
        .expect_err("bad.ts does not parse");
    assert_eq!(error.kind, JsErrorKind::Ts, "{error}");
    let at = error.location.expect("a location");
    assert_eq!(at.file, root.path.join("bad.ts"));
    assert_eq!((at.line, at.column), (2, 16));
}

#[test]
fn throw_is_remapped_to_the_ts_line() {
    // The enum and interface change the line numbers of the stripped JavaScript, so a frame that
    // is not remapped would name the wrong line.
    let source = "interface Shape {\n    sides: number;\n}\nenum Mode {\n    A,\n    B,\n}\nexport function explode(shape: Shape): never {\n    throw new Error('boom ' + Mode.B + shape.sides);\n}\nexplode({ sides: 3 });\n";
    let root = root_with(&[("init.ts", source)]);
    let generation = generation(&root);
    let error = generation
        .eval_module(Path::new("init.ts"))
        .expect_err("the module throws");
    assert_eq!(error.kind, JsErrorKind::Exception, "{error}");
    assert_eq!(error.message, "boom 13");
    let at = error.location.as_ref().expect("a location");
    assert_eq!(at.file, root.path.join("init.ts"));
    assert_eq!(at.line, 9, "{error:?}");
    let stack = error.stack.as_deref().expect("a stack");
    assert!(stack.contains("init.ts:9:"), "{stack}");
    assert!(
        stack.contains("init.ts:11:"),
        "the caller frame is remapped too: {stack}"
    );
}

#[test]
fn throw_in_an_imported_module_names_that_module() {
    let root = root_with(&[
        ("init.ts", "import { fail } from './lib.ts';\nfail();\n"),
        (
            "lib.ts",
            "type Unused = string;\n\nexport function fail(): void {\n    throw new TypeError('nope');\n}\n",
        ),
    ]);
    let generation = generation(&root);
    let error = generation.eval_module(Path::new("init.ts")).expect_err("fail() throws");
    let at = error.location.as_ref().expect("a location");
    assert_eq!(at.file, root.path.join("lib.ts"));
    assert_eq!(at.line, 4);
}

#[test]
fn throw_in_plain_javascript_keeps_its_own_coordinates() {
    let root = root_with(&[("init.js", "\n\nthrow new Error('plain');\n")]);
    let generation = generation(&root);
    let error = generation
        .eval_module(Path::new("init.js"))
        .expect_err("the module throws");
    let at = error.location.as_ref().expect("a location");
    assert_eq!((at.file.as_path(), at.line), (root.path.join("init.js").as_path(), 3));
}

#[test]
fn column_is_one_based_in_ts_and_in_js_coordinates() {
    // The stripped code is not indented, so the statement's column moves from 1 to 3 when it is mapped
    // back to the two-space-indented source line.
    let root = root_with(&[
        (
            "init.ts",
            "interface Unused {}\nenum E { A }\nconst gone: any = null;\n  gone.x;\n",
        ),
        ("init.js", "const a = null;\n  a.x;\n"),
    ]);
    let generation = generation(&root);
    let ts = generation.eval_module(Path::new("init.ts")).expect_err("gone is null");
    let at = ts.location.expect("a location");
    assert_eq!((at.line, at.column), (4, 3));
    let js = generation.eval_module(Path::new("init.js")).expect_err("a is null");
    let at = js.location.expect("a location");
    assert_eq!((at.line, at.column), (2, 3));
}

#[test]
fn non_error_throw_is_reported_by_its_text() {
    let root = root_with(&[("init.js", "throw 'just a string';\n")]);
    let generation = generation(&root);
    let error = generation
        .eval_module(Path::new("init.js"))
        .expect_err("the module throws");
    assert_eq!(error.kind, JsErrorKind::Exception);
    assert_eq!(error.message, "just a string");
}

#[test]
fn an_infinite_loop_is_interrupted_by_the_budget() {
    let root = root_with(&[]);
    let generation = generation(&root);
    // The loop ends by itself after 30 s, so an engine that failed to interrupt it fails this test with a
    // message, not by hanging the run. How long the interrupt took is not asserted: it is the machine's speed.
    let error = generation
        .call_budgeted(Duration::from_millis(200), |ctx| {
            ctx.eval::<(), _>("for (const end = Date.now() + 30000; Date.now() < end;) {}")
        })
        .expect_err("the loop is interrupted before its own deadline");
    assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
}

#[test]
fn script_cannot_catch_the_interrupt() {
    let root = root_with(&[]);
    let generation = generation(&root);
    let error = generation
        .call_budgeted(Duration::from_millis(100), |ctx| {
            ctx.eval::<(), _>("try { for (;;) {} } catch (e) {} finally { globalThis.finalised = 1; }")
        })
        .expect_err("the interrupt cannot be caught");
    assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
}

/// A host function that blocks without computing, as a thread that the scheduler does not run does: wall time passes
/// and the script's own CPU time does not.
fn install_pause(ctx: &rquickjs::Ctx<'_>) {
    let pause = rquickjs::Function::new(ctx.clone(), |ms: u64| std::thread::sleep(Duration::from_millis(ms)))
        .expect("the pause function");
    ctx.globals().set("pause", pause).expect("the pause global");
}

#[test]
fn time_the_script_spends_waiting_for_the_scheduler_is_not_charged_to_its_budget() {
    // The budget bounds a script that hangs, and a machine that does not run the thread for a while is not a script
    // that hangs. Twenty pauses of 60 ms are 1.2 s of wall time against a 500 ms budget, with a few milliseconds of
    // the script's own work: the call must finish. (How long the work takes is not asserted, so nothing here depends
    // on the machine's speed beyond it being far under 500 ms of CPU.)
    let root = root_with(&[]);
    let generation = generation(&root);
    let finished = generation
        .call_budgeted(Duration::from_millis(500), |ctx| {
            install_pause(ctx);
            ctx.eval::<i32, _>(
                "let n = 0; for (let i = 0; i < 20; i++) { pause(60); for (let j = 0; j < 20000; j++) { n++; } } n",
            )
        })
        .expect("a stalled thread is not a runaway script");
    assert_eq!(finished, 400_000);
}

#[test]
fn a_script_stalled_for_longer_than_the_wall_clock_backstop_is_still_stopped() {
    // The CPU budget alone would let a call that never computes run on for ever: the wall clock stops it at
    // `WALL_BACKSTOP_FACTOR` times the budget. A 20 ms budget makes that 160 ms, and the pauses add up to 600 ms.
    let root = root_with(&[]);
    let generation = generation(&root);
    let error = generation
        .call_budgeted(Duration::from_millis(20), |ctx| {
            install_pause(ctx);
            ctx.eval::<(), _>("for (let i = 0; i < 20; i++) { pause(30); for (let j = 0; j < 20000; j++) {} }")
        })
        .expect_err("the backstop stops a call that is never scheduled");
    assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
    assert!(
        error.message.contains("wall") && error.message.contains("20 ms"),
        "the report names the limit and the clock that stopped it: {error}"
    );
}

#[test]
fn a_trip_reports_the_limit_the_time_used_and_where_the_script_was() {
    let root = root_with(&[]);
    let generation = generation(&root);
    let error = generation
        .call_budgeted(Duration::from_millis(100), |ctx| {
            ctx.eval::<(), _>("function spin() { for (;;) {} }\nfunction listener() { spin(); }\nlistener();")
        })
        .expect_err("the loop is interrupted");
    assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
    let text = error.to_string();
    assert!(text.contains("100 ms"), "the limit: {text}");
    assert!(text.contains("CPU"), "the clock the limit is on: {text}");
    assert!(text.contains("spin"), "the call it was in: {text}");
    assert!(text.contains("listener"), "and who called it: {text}");
}

#[test]
fn infinite_loop_in_a_module_is_a_budget_error_and_the_generation_stays_usable() {
    let root = root_with(&[("init.ts", "export const n: number = 1;\nwhile (true) {}\n")]);
    let mut config = JsConfig::new(&root.path);
    config.stage_budget = Duration::from_millis(100);
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    let error = generation
        .eval_module(Path::new("init.ts"))
        .expect_err("the module loops");
    assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
    // The next call gets a fresh budget.
    let sum = generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.eval::<i32, _>("1 + 2"))
        .expect("a later call runs");
    assert_eq!(sum, 3);
}

#[test]
fn promise_job_chain_that_never_ends_is_a_budget_error() {
    let root = root_with(&[(
        "init.js",
        "const spin = () => Promise.resolve().then(spin);\nspin();\nawait new Promise(() => {});\n",
    )]);
    let mut config = JsConfig::new(&root.path);
    config.stage_budget = Duration::from_millis(100);
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    let error = generation
        .eval_module(Path::new("init.js"))
        .expect_err("the jobs never end");
    assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
}

#[test]
fn top_level_await_that_nothing_resolves_is_an_error() {
    let root = root_with(&[("init.js", "await new Promise(() => {});\n")]);
    let generation = generation(&root);
    let error = generation
        .eval_module(Path::new("init.js"))
        .expect_err("the await never settles");
    assert_eq!(error.kind, JsErrorKind::Exception, "{error}");
    assert!(error.message.contains("never settled"), "{error}");
}

#[test]
fn top_level_await_that_a_job_resolves_completes() {
    let root = root_with(&[(
        "init.js",
        "const v = await Promise.resolve(7);\nglobalThis.awaited = v;\n",
    )]);
    let generation = generation(&root);
    generation.eval_module(Path::new("init.js")).expect("the await settles");
    assert_eq!(global_number(&generation, "awaited"), Some(7));
}

#[test]
fn memory_limit_trips() {
    let root = root_with(&[]);
    let mut config = JsConfig::new(&root.path);
    config.memory_limit = 8 << 20;
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    let error = generation
        .call_budgeted(Duration::from_secs(10), |ctx| {
            ctx.eval::<(), _>("const keep = []; for (;;) { keep.push(new Array(10000).fill(1)); }")
        })
        .expect_err("the heap grows past the limit");
    assert_eq!(error.kind, JsErrorKind::Memory, "{error}");
}

#[test]
fn generations_share_no_state() {
    let root = root_with(&[("init.js", "globalThis.marker = 1;\n")]);
    let first = generation(&root);
    let second = generation(&root);
    first.eval_module(Path::new("init.js")).expect("init.js evaluates");
    assert_eq!(global_number(&first, "marker"), Some(1));
    assert_eq!(global_number(&second, "marker"), None);
}

#[test]
fn call_budgeted_returns_a_thrown_value_with_a_message() {
    let root = root_with(&[]);
    let generation = generation(&root);
    let error = generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.eval::<(), _>("null.x"))
        .expect_err("null has no properties");
    assert_eq!(error.kind, JsErrorKind::Exception);
    assert!(error.message.contains("null"), "{error}");
}

#[test]
fn frames_are_split_around_their_position() {
    let frame = split_frame("    at explode (/r/init.ts:9:11)").expect("a frame");
    assert_eq!(
        (frame.before, frame.file, frame.line, frame.column, frame.after),
        ("    at explode (", "/r/init.ts", 9, 11, ")")
    );
    let frame = split_frame("    at /r/init.ts:11:1").expect("a bare frame");
    assert_eq!(
        (frame.before, frame.file, frame.line, frame.column, frame.after),
        ("    at ", "/r/init.ts", 11, 1, "")
    );
    assert!(split_frame("    at parseInt (native)").is_none());
    assert!(split_frame("no frame here").is_none());
}
