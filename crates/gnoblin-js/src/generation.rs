//! One generation: a QuickJS runtime and context with the limits the shell needs.
//!
//! A generation is the unit of hot reload (design/js-runtime.md, J5): reloading builds a new one and
//! drops the old, because QuickJS cannot evict a module. It owns exactly one [`Runtime`] and one
//! [`Context`], so two generations share no script state.
//!
//! Every entry into JavaScript goes through [`JsGeneration::call_budgeted`], which arms a wall-clock
//! deadline that the engine's interrupt handler polls. The handler's error cannot be caught by the
//! script, so `try { for (;;) {} } catch {}` still stops. A memory limit bounds the heap. An error that
//! escapes carries the file, line and column of the author's source: stack frames in stripped `.ts`
//! modules are remapped through the source map the loader kept.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rquickjs::context::intrinsic;
use rquickjs::convert::Coerced;
use rquickjs::{Context, Ctx, Error, Exception, FromJs, Module, Runtime, Value};

use crate::error::{JsError, JsErrorKind, JsLoadError, JsLocation};
use crate::loader::{JsLoadState, JsLoader, JsResolver};

/// What a generation is built from. Cheap to clone, so a host builds every generation from one value.
#[derive(Debug, Clone)]
pub struct JsConfig {
    /// The config root. Every file module must resolve under it.
    pub root: PathBuf,
    /// Modules the host embeds, by import name (`gnoblin`, `gnoblin/layouts`), with their JavaScript.
    pub embedded: Arc<HashMap<String, String>>,
    /// The wall-clock budget [`JsGeneration::eval_module`] gets for the import and its jobs.
    pub stage_budget: Duration,
    /// The heap limit in bytes; 0 means unlimited.
    pub memory_limit: usize,
    /// The native stack limit in bytes, which bounds recursion.
    pub max_stack_size: usize,
}

impl JsConfig {
    /// The defaults of design/js-runtime.md: a 5 s staging budget, a 64 MiB heap and a 512 KiB stack.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            embedded: Arc::new(HashMap::new()),
            stage_budget: Duration::from_secs(5),
            memory_limit: 64 << 20,
            max_stack_size: 512 << 10,
        }
    }

    /// Registers an embedded module: `import ... from "<name>"` runs `source`, JavaScript.
    #[must_use]
    pub fn embed(mut self, name: impl Into<String>, source: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.embedded).insert(name.into(), source.into());
        self
    }
}

/// No deadline armed.
const NO_DEADLINE: u64 = u64::MAX;

/// The deadline the engine's interrupt handler polls.
///
/// It is atomics rather than cells so a watchdog thread could arm or trip it later without a change here.
#[derive(Debug)]
struct JsBudget {
    epoch: Instant,
    /// Nanoseconds since `epoch` at which the running call must stop.
    deadline: AtomicU64,
    /// Set when the handler stopped a script, so the error can be classified after the unwind.
    tripped: AtomicBool,
}

impl JsBudget {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            deadline: AtomicU64::new(NO_DEADLINE),
            tripped: AtomicBool::new(false),
        }
    }

    fn now_ns(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(NO_DEADLINE - 1)
    }

    /// Starts a call that may run for `budget`. The guard disarms the deadline when dropped, also on panic.
    fn arm(&self, budget: Duration) -> ArmGuard<'_> {
        self.tripped.store(false, Ordering::Relaxed);
        let ns = u64::try_from(budget.as_nanos()).unwrap_or(u64::MAX);
        self.deadline
            .store(self.now_ns().saturating_add(ns).min(NO_DEADLINE - 1), Ordering::Relaxed);
        ArmGuard(self)
    }

    /// Whether the running call is out of time. Marks the call tripped when it is.
    fn expired(&self) -> bool {
        let deadline = self.deadline.load(Ordering::Relaxed);
        if deadline != NO_DEADLINE && self.now_ns() >= deadline {
            self.tripped.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }
}

/// Disarms the deadline of the call it was made for.
struct ArmGuard<'a>(&'a JsBudget);

impl Drop for ArmGuard<'_> {
    fn drop(&mut self) {
        self.0.deadline.store(NO_DEADLINE, Ordering::Relaxed);
    }
}

/// One QuickJS runtime and context, loaded from one config.
///
/// The fields drop in declaration order, context before runtime, so every value is freed while its
/// runtime is alive.
pub struct JsGeneration {
    context: Context,
    runtime: Runtime,
    budget: Arc<JsBudget>,
    load: Rc<JsLoadState>,
    config: JsConfig,
    id: u64,
}

impl std::fmt::Debug for JsGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsGeneration")
            .field("id", &self.id)
            .field("root", &self.config.root)
            .finish_non_exhaustive()
    }
}

impl JsGeneration {
    /// A fresh runtime and context for `config`, numbered `id`. Nothing has run in it yet.
    pub fn new(config: &JsConfig, id: u64) -> Result<Self, JsError> {
        let engine = |error: Error| JsError::new(JsErrorKind::Engine, error.to_string());
        let runtime = Runtime::new().map_err(engine)?;
        if config.memory_limit > 0 {
            runtime.set_memory_limit(config.memory_limit);
            // Collect before the limit is reached, so garbage does not count against the script.
            runtime.set_gc_threshold(config.memory_limit / 4);
        }
        runtime.set_max_stack_size(config.max_stack_size);

        let budget = Arc::new(JsBudget::new());
        let handler = Arc::clone(&budget);
        runtime.set_interrupt_handler(Some(Box::new(move || handler.expired())));

        let load = JsLoadState::new();
        let resolver = JsResolver::new(&config.root, Arc::clone(&config.embedded), Rc::clone(&load)).map_err(engine)?;
        let loader = JsLoader::new(&config.root, Arc::clone(&config.embedded), Rc::clone(&load)).map_err(engine)?;
        runtime.set_loader(resolver, loader);

        // The `Eval` intrinsic is not optional: QuickJS compiles modules for the host through it, and
        // the host's own `eval_module` fails with "eval is not supported" without it. So scripts have
        // `eval` too. The config is the user's own code, and the boundary that matters is the process
        // (design/js-runtime.md, J1), not the global object. `performance` is left out.
        let context = Context::builder()
            .with::<(
                intrinsic::Date,
                intrinsic::Eval,
                intrinsic::RegExpCompiler,
                intrinsic::RegExp,
                intrinsic::Json,
                intrinsic::Proxy,
                intrinsic::MapSet,
                intrinsic::TypedArrays,
                intrinsic::Promise,
                intrinsic::WeakRef,
            )>()
            .build(&runtime)
            .map_err(engine)?;

        Ok(Self {
            context,
            runtime,
            budget,
            load,
            config: config.clone(),
            id,
        })
    }

    /// The number the host gave this generation.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// A second handle on the same runtime and context. The engine frees them when the last handle is dropped.
    ///
    /// A reload stages a generation inside the [`JsHost`](crate::JsHost), and the shell that runs it keeps its own
    /// handle, so handing the staged generation to the shell does not take it from the host's bookkeeping (its
    /// id and module graph, which the watcher reads).
    #[must_use]
    pub fn share(&self) -> Self {
        Self {
            context: self.context.clone(),
            runtime: self.runtime.clone(),
            budget: Arc::clone(&self.budget),
            load: Rc::clone(&self.load),
            config: self.config.clone(),
            id: self.id,
        }
    }

    /// Bytes the engine currently has allocated for this generation.
    pub fn memory_used(&self) -> i64 {
        self.runtime.memory_usage().malloc_size
    }

    /// The canonical path of every file module this generation loaded, sorted: the entry and each
    /// module it imported, directly or not. Embedded modules are not files and are not listed.
    ///
    /// This is the set a file watcher observes for hot reload.
    pub fn module_paths(&self) -> Vec<PathBuf> {
        self.load.files()
    }

    /// Imports and evaluates the module at `entry`, a path under the config root (a relative path counts
    /// from the root), then runs its promise jobs until the module settles.
    ///
    /// The whole call has the config's `stage_budget`. A module that never settles, for example a
    /// top-level `await` on a promise nothing resolves, is an [`JsErrorKind::Exception`].
    pub fn eval_module(&self, entry: &Path) -> Result<(), JsError> {
        let path = if entry.is_absolute() {
            entry.to_path_buf()
        } else {
            self.config.root.join(entry)
        };
        let Some(specifier) = path.to_str().map(str::to_owned) else {
            return Err(JsLoadError::NotUtf8 { path }.into());
        };
        self.call_budgeted(self.config.stage_budget, |ctx| {
            let promise = Module::import(ctx, specifier)?;
            loop {
                if let Some(settled) = promise.result::<Value<'_>>() {
                    return settled.map(drop);
                }
                if self.budget.expired() || !ctx.execute_pending_job() {
                    // Out of time, or nothing left that could settle it.
                    return Err(Error::WouldBlock);
                }
            }
        })
    }

    /// Runs `f` with the generation's context, giving it at most `budget` of wall-clock time.
    ///
    /// When the time runs out the script is interrupted and the result is a [`JsErrorKind::Budget`]
    /// error, even if `f` swallowed the failure: the interrupt is not catchable, so a script that
    /// reached here did not finish. Out of memory is [`JsErrorKind::Memory`] and any other thrown value
    /// is [`JsErrorKind::Exception`], with file, line and column in the author's coordinates.
    pub fn call_budgeted<R>(
        &self,
        budget: Duration,
        f: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<R>,
    ) -> Result<R, JsError> {
        self.load.clear_failure();
        let _armed = self.budget.arm(budget);
        self.context.with(|ctx| {
            let outcome = f(&ctx);
            if self.budget.tripped() {
                // Drain the pending exception so the context is clean for the next call.
                let stack = ctx.has_exception().then(|| self.describe(&ctx, &ctx.catch()));
                return Err(self.budget_error(budget, stack));
            }
            outcome.map_err(|error| self.convert(&ctx, error))
        })
    }

    fn budget_error(&self, budget: Duration, thrown: Option<JsError>) -> JsError {
        let mut error = JsError::new(
            JsErrorKind::Budget,
            format!(
                "the script ran past its {} ms budget and was interrupted",
                budget.as_millis()
            ),
        );
        if let Some(thrown) = thrown {
            error.location = thrown.location;
            error.stack = thrown.stack;
        }
        error
    }

    /// The [`JsError`] for an engine error, reading the pending exception when there is one.
    fn convert<'js>(&self, ctx: &Ctx<'js>, error: Error) -> JsError {
        match error {
            Error::Exception => self.describe(ctx, &ctx.catch()),
            Error::Allocation => JsError::new(JsErrorKind::Memory, "the generation's memory limit was reached"),
            Error::WouldBlock => JsError::new(
                JsErrorKind::Exception,
                "the module never settled: it awaits a promise that nothing can resolve",
            ),
            other => JsError::new(JsErrorKind::Engine, other.to_string()),
        }
    }

    /// Describes a thrown `value`: its message, and its stack with every `.ts` frame remapped.
    pub(crate) fn describe<'js>(&self, ctx: &Ctx<'js>, value: &Value<'js>) -> JsError {
        let exception = value
            .as_object()
            .and_then(|object| Exception::from_object(object.clone()));
        let (message, stack) = match &exception {
            Some(exception) => (exception.message().unwrap_or_default(), exception.stack()),
            None => (
                Coerced::<String>::from_js(ctx, value.clone())
                    .map_or_else(|_| "a non-error value was thrown".to_owned(), |text| text.0),
                None,
            ),
        };
        if let Some(failure) = self.load.take_failure(&message) {
            return failure;
        }
        let kind = if message == "out of memory" {
            JsErrorKind::Memory
        } else {
            JsErrorKind::Exception
        };
        let (stack, location) = match stack {
            Some(text) => {
                let (remapped, location) = self.remap_stack(&text);
                (Some(remapped), location)
            }
            None => (None, None),
        };
        JsError {
            kind,
            message,
            location,
            stack,
        }
    }

    /// Rewrites every frame of `stack` whose file has a source map, and returns the location of the first
    /// frame that has a file and a line.
    fn remap_stack(&self, stack: &str) -> (String, Option<JsLocation>) {
        let mut out = String::with_capacity(stack.len());
        let mut first = None;
        for line in stack.lines() {
            let rewritten = match split_frame(line) {
                Some(frame) => {
                    let (line_no, column) = self
                        .load
                        .remap(frame.file, frame.line, frame.column)
                        .unwrap_or((frame.line, frame.column));
                    first.get_or_insert_with(|| JsLocation {
                        file: PathBuf::from(frame.file),
                        line: line_no,
                        column,
                    });
                    format!("{}{}:{line_no}:{column}{}", frame.before, frame.file, frame.after)
                }
                None => line.to_owned(),
            };
            out.push_str(&rewritten);
            out.push('\n');
        }
        (out, first)
    }
}

/// One stack line split around its `file:line:column`.
struct JsFrame<'a> {
    before: &'a str,
    file: &'a str,
    line: u32,
    column: u32,
    after: &'a str,
}

/// Splits a QuickJS frame (`    at fn (file:1:2)` or `    at file:1:2`), or `None` for a line that is
/// not a frame with a position, such as a native frame.
fn split_frame(line: &str) -> Option<JsFrame<'_>> {
    let trimmed = line.trim_end();
    let indent = trimmed.len() - trimmed.trim_start().len();
    let (head, tail) = trimmed.split_at(indent + trimmed.trim_start().strip_prefix("at ").map(|_| 3)?);
    let (before, location, after) = match tail
        .strip_suffix(')')
        .and_then(|body| body.rfind('(').map(|open| (body, open)))
    {
        Some((body, open)) => (&trimmed[..head.len() + open + 1], &body[open + 1..], ")"),
        None => (head, tail, ""),
    };
    let mut parts = location.rsplitn(3, ':');
    let column = parts.next()?.parse().ok()?;
    let line_no = parts.next()?.parse().ok()?;
    let file = parts.next()?;
    Some(JsFrame {
        before,
        file,
        line: line_no,
        column,
        after,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

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
            ("init.ts", "import './lib/a.ts';\nimport { gnoblin } from 'gnoblin';\n"),
            ("lib/a.ts", "import '../b.js';\n"),
            ("b.js", "globalThis.b = 1;\n"),
            ("unused.ts", "export {};\n"),
        ]);
        let config = JsConfig::new(&root.path).embed("gnoblin", "export const gnoblin = 1;\n");
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
            "import { shell } from 'gnoblin';\nimport { grid } from 'gnoblin/layouts';\n(globalThis as any).out = shell + grid;\n",
        )]);
        let config = JsConfig::new(&root.path)
            .embed("gnoblin", "export const shell = 10;")
            .embed("gnoblin/layouts", "export const grid = 5;");
        let generation = JsGeneration::new(&config, 1).expect("a generation");
        generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
        assert_eq!(global_number(&generation, "out"), Some(15));
    }

    #[test]
    fn unknown_embedded_and_bare_modules_are_load_errors() {
        let root = root_with(&[("a.ts", "import 'gnoblin/missing';\n"), ("b.ts", "import 'lodash';\n")]);
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
    fn infinite_loop_is_interrupted_within_twice_the_budget() {
        let root = root_with(&[]);
        let generation = generation(&root);
        let budget = Duration::from_millis(200);
        let started = Instant::now();
        let error = generation
            .call_budgeted(budget, |ctx| ctx.eval::<(), _>("while (true) {}"))
            .expect_err("the loop never ends");
        let elapsed = started.elapsed();
        assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
        assert!(elapsed >= budget, "stopped before the budget: {elapsed:?}");
        assert!(elapsed < budget * 2, "took {elapsed:?} for a {budget:?} budget");
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
}
