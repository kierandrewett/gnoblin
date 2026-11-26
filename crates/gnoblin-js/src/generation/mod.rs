//! One generation: a QuickJS runtime and context with the limits the shell needs.
//!
//! A generation is the unit of hot reload: reloading builds a new one and
//! drops the old, because QuickJS cannot evict a module. It owns exactly one [`Runtime`] and one
//! [`Context`], so two generations share no script state.
//!
//! Every entry into JavaScript goes through [`JsGeneration::call_budgeted`], which arms CPU-time and wall-clock
//! deadlines that the engine's interrupt handler polls. The handler's error cannot be caught by the
//! script, so `try { for (;;) {} } catch {}` still stops. A memory limit bounds the heap. An error that
//! escapes carries the file, line and column of the author's source: stack frames in stripped `.ts`
//! modules are remapped through the source map the loader kept.
//!
//! The tests are in `tests.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rquickjs::context::intrinsic;
use rquickjs::convert::Coerced;
use rquickjs::{Context, Ctx, Error, Exception, FromJs, Module, Runtime, Value};

use crate::budget::{STAGE_BUDGET, WALL_BACKSTOP_FACTOR};
use crate::bytecode::JsBytecodeCache;
use crate::choice::StockRuntime;
use crate::error::{JsError, JsErrorKind, JsLoadError, JsLocation};
use crate::loader::{JsLoadState, JsLoader, JsResolver, JsTypeScriptModule};
use crate::natives::SafePoints;
use crate::script_console::{JsConsoleLine, JsScriptConsole};
use crate::stock::StockLibrary;
use gnoblin_shell_host::supervisor::{Clock, SystemClock};

/// What a generation is built from. Cheap to clone, so a host builds every generation from one value.
#[derive(Debug, Clone)]
pub struct JsConfig {
    /// The config root. Every file module must resolve under it.
    pub root: PathBuf,
    /// Modules the host embeds, by import name (`gnoblin-core`, `gnoblin-std/layouts`), with their JavaScript.
    pub embedded: Arc<HashMap<String, String>>,
    /// Modules the host embeds as TypeScript (the stock library), stripped when imported. A name in `embedded` wins.
    pub typescript: Arc<HashMap<String, JsTypeScriptModule>>,
    /// The directories the stock library was looked for in. Only the error of a package that is not installed reads them.
    pub stock_dirs: Arc<Vec<PathBuf>>,
    /// Where compiled modules are kept between starts. Disabled by default; the runtime role turns it on.
    pub cache: JsBytecodeCache,
    /// The CPU-time budget [`JsGeneration::eval_module`] gets for the import and its jobs.
    pub stage_budget: Duration,
    /// The heap limit in bytes; 0 means unlimited.
    pub memory_limit: usize,
    /// The native stack limit in bytes, which bounds recursion.
    pub max_stack_size: usize,
    /// The clock the generation's `console` counts its rate limit on. The shell sets it to its own clock, so a test
    /// that moves that clock moves this one; the real clock otherwise.
    pub clock: Arc<dyn Clock>,
    /// The native objects the host library gives every generation, installed beside `__gnoblinHost`.
    pub natives: Vec<crate::natives::JsNative>,
    /// The stock library the generations of a shell choose from (see [`StockLibrary`]). `None` for a config whose `typescript` and
    /// `embedded` modules are all the stock library there is.
    pub stock: Option<Arc<StockLibrary>>,
    /// The stock library this config's generations run on. [`JsConfig::for_runtime`] sets it; a config that has no [`StockLibrary`]
    /// is the TypeScript one, which is what a library of its own is.
    pub runtime: StockRuntime,
}

impl JsConfig {
    /// The defaults: the staging budget of `crate::budget`, a 64 MiB heap and a 512 KiB stack.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            embedded: Arc::new(HashMap::new()),
            typescript: Arc::new(HashMap::new()),
            stock_dirs: Arc::new(Vec::new()),
            cache: JsBytecodeCache::disabled(),
            stage_budget: STAGE_BUDGET,
            memory_limit: 64 << 20,
            max_stack_size: 512 << 10,
            clock: Arc::new(SystemClock),
            natives: Vec::new(),
            stock: None,
            runtime: StockRuntime::Typescript,
        }
    }

    /// Registers an embedded module: `import ... from "<name>"` runs `source`, JavaScript.
    #[must_use]
    pub fn embed(mut self, name: impl Into<String>, source: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.embedded).insert(name.into(), source.into());
        self
    }

    /// Registers an embedded module as TypeScript: it is stripped and compiled when first imported, unless the
    /// bytecode cache holds it. `path` is what the stripper's diagnostics name.
    #[must_use]
    pub fn embed_typescript(mut self, name: impl Into<String>, module: JsTypeScriptModule) -> Self {
        Arc::make_mut(&mut self.typescript).insert(name.into(), module);
        self
    }

    /// Gives the generations this config makes a bytecode cache.
    #[must_use]
    pub fn with_cache(mut self, cache: JsBytecodeCache) -> Self {
        self.cache = cache;
        self
    }

    /// Gives the generations this config makes the stock library of the shell ([`StockLibrary`]), from which each generation takes the
    /// library its import graph needs.
    #[must_use]
    pub fn with_stock(mut self, stock: Arc<StockLibrary>) -> Self {
        self.stock = Some(stock);
        self
    }

    /// This config for a generation that runs on `runtime`: its stock modules are that library's, and a module embedded under a
    /// name stays what the caller made it.
    ///
    /// # Errors
    ///
    /// The TypeScript library has a file that cannot be read.
    pub fn for_runtime(&self, runtime: StockRuntime) -> Result<Self, JsError> {
        let mut config = self.clone();
        config.runtime = runtime;
        if let Some(stock) = &self.stock {
            let modules = stock.modules(runtime).map_err(|error| JsError {
                kind: JsErrorKind::Load,
                message: error.to_string(),
                location: None,
                stack: None,
            })?;
            config.typescript = if self.typescript.is_empty() {
                modules
            } else {
                let mut merged = (*modules).clone();
                merged.extend(
                    self.typescript
                        .iter()
                        .map(|(name, module)| (name.clone(), module.clone())),
                );
                Arc::new(merged)
            };
            config.stock_dirs = Arc::new(stock.dirs().to_vec());
            config.stock = None;
        }
        Ok(config)
    }
}

/// No deadline armed.
const NO_DEADLINE: u64 = u64::MAX;

/// A duration for a message: whole milliseconds, or one decimal under 10 ms so a budget of a few microseconds is not
/// reported as zero.
fn millis(duration: Duration) -> String {
    let ms = duration.as_secs_f64() * 1000.0;
    if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// The calling thread's CPU time in nanoseconds: what the script's own work has cost, and not the time the scheduler
/// kept the thread off the processor. A generation lives on one thread, so the reading the handler takes is on the
/// clock of the thread that armed the budget.
fn thread_cpu_ns() -> u64 {
    let at = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
    u64::try_from(at.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::try_from(at.tv_nsec).unwrap_or(0))
        .min(NO_DEADLINE - 1)
}

/// The deadlines the engine's interrupt handler polls: a call is stopped when its own CPU time reaches the budget, or
/// when the wall clock reaches [`WALL_BACKSTOP_FACTOR`] times the budget (`crate::budget` says why both).
///
/// It is atomics rather than cells so a watchdog thread could arm or trip it later without a change here.
#[derive(Debug)]
struct JsBudget {
    epoch: Instant,
    /// Nanoseconds since `epoch` at which the running call must stop, whatever it has computed.
    wall_deadline: AtomicU64,
    /// The thread's CPU time, in nanoseconds on its own clock, at which the running call must stop.
    cpu_deadline: AtomicU64,
    /// What the running call started with: wall nanoseconds since `epoch`, and the thread's CPU nanoseconds.
    wall_start: AtomicU64,
    cpu_start: AtomicU64,
    /// Set when the handler stopped a script, so the error can be classified after the unwind.
    tripped: AtomicBool,
    /// What the stopped call had used when the handler stopped it, in nanoseconds of wall time and of CPU time.
    wall_used: AtomicU64,
    cpu_used: AtomicU64,
}

impl JsBudget {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            wall_deadline: AtomicU64::new(NO_DEADLINE),
            cpu_deadline: AtomicU64::new(NO_DEADLINE),
            wall_start: AtomicU64::new(0),
            cpu_start: AtomicU64::new(0),
            tripped: AtomicBool::new(false),
            wall_used: AtomicU64::new(0),
            cpu_used: AtomicU64::new(0),
        }
    }

    fn now_ns(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(NO_DEADLINE - 1)
    }

    /// Starts a call that may use `budget` of CPU time, or [`WALL_BACKSTOP_FACTOR`] times that of wall time. The
    /// guard disarms the deadline when dropped, also on panic.
    fn arm(&self, budget: Duration) -> ArmGuard<'_> {
        self.tripped.store(false, Ordering::Relaxed);
        let cpu_ns = u64::try_from(budget.as_nanos()).unwrap_or(u64::MAX);
        let wall_ns = u64::try_from(budget.saturating_mul(WALL_BACKSTOP_FACTOR).as_nanos()).unwrap_or(u64::MAX);
        let (wall, cpu) = (self.now_ns(), thread_cpu_ns());
        self.wall_start.store(wall, Ordering::Relaxed);
        self.cpu_start.store(cpu, Ordering::Relaxed);
        self.wall_deadline
            .store(wall.saturating_add(wall_ns).min(NO_DEADLINE - 1), Ordering::Relaxed);
        self.cpu_deadline
            .store(cpu.saturating_add(cpu_ns).min(NO_DEADLINE - 1), Ordering::Relaxed);
        ArmGuard(self)
    }

    /// Whether the running call is out of time. Marks the call tripped, and keeps what it had used, when it is.
    fn expired(&self) -> bool {
        let wall_deadline = self.wall_deadline.load(Ordering::Relaxed);
        if wall_deadline == NO_DEADLINE {
            return false;
        }
        let (wall, cpu) = (self.now_ns(), thread_cpu_ns());
        if wall < wall_deadline && cpu < self.cpu_deadline.load(Ordering::Relaxed) {
            return false;
        }
        if !self.tripped.swap(true, Ordering::Relaxed) {
            self.wall_used.store(
                wall.saturating_sub(self.wall_start.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
            self.cpu_used.store(
                cpu.saturating_sub(self.cpu_start.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
        }
        true
    }

    fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }

    /// What the call that was stopped had used: CPU time, then wall time.
    fn used(&self) -> (Duration, Duration) {
        (
            Duration::from_nanos(self.cpu_used.load(Ordering::Relaxed)),
            Duration::from_nanos(self.wall_used.load(Ordering::Relaxed)),
        )
    }
}

/// Disarms the deadline of the call it was made for.
struct ArmGuard<'a>(&'a JsBudget);

impl Drop for ArmGuard<'_> {
    fn drop(&mut self) {
        self.0.wall_deadline.store(NO_DEADLINE, Ordering::Relaxed);
        self.0.cpu_deadline.store(NO_DEADLINE, Ordering::Relaxed);
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
    console: Rc<JsScriptConsole>,
    config: JsConfig,
    id: u64,
    /// What the generation's natives asked to run at the engine's safe points (`ScriptIds::on_safe_point`).
    safe_points: SafePoints,
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
        let resolver = JsResolver::new(
            &config.root,
            Arc::clone(&config.embedded),
            Arc::clone(&config.typescript),
            Arc::clone(&config.stock_dirs),
            Rc::clone(&load),
        )
        .map_err(engine)?;
        let loader = JsLoader::new(
            &config.root,
            Arc::clone(&config.embedded),
            Arc::clone(&config.typescript),
            config.cache.clone(),
            Rc::clone(&load),
        )
        .map_err(engine)?;
        runtime.set_loader(resolver, loader);

        // The `Eval` intrinsic is not optional: QuickJS compiles modules for the host through it, and
        // the host's own `eval_module` fails with "eval is not supported" without it. So scripts have
        // `eval` too. The config is the user's own code, and the boundary that matters is the process,
        // not the global object. `performance` is left out.
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

        let console = JsScriptConsole::new(config, Rc::clone(&load));
        context.with(|ctx| console.install(&ctx)).map_err(engine)?;

        Ok(Self {
            context,
            runtime,
            budget,
            load,
            console,
            config: config.clone(),
            id,
            safe_points: SafePoints::default(),
        })
    }

    /// The number the host gave this generation.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The stock library this generation runs on.
    pub fn runtime(&self) -> StockRuntime {
        self.config.runtime
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
            console: Rc::clone(&self.console),
            config: self.config.clone(),
            id: self.id,
            safe_points: Rc::clone(&self.safe_points),
        }
    }

    /// The functions the generation's natives run at the engine's safe points, for the natives to be installed with.
    pub(crate) fn safe_points(&self) -> &SafePoints {
        &self.safe_points
    }

    /// Runs what the natives asked to run when no script is running: the end of a frame, and the end of the generation. It runs no
    /// script, so it is not work and wakes no frame.
    pub(crate) fn run_safe_points(&self) {
        crate::natives::run_safe_points(&self.safe_points);
    }

    /// Bytes the engine currently has allocated for this generation.
    pub fn memory_used(&self) -> i64 {
        self.runtime.memory_usage().malloc_size
    }

    /// Whether promise jobs are waiting to run. The shell's pump runs a bounded number a frame, so a frame that cut it
    /// short has work no message will announce.
    pub fn jobs_pending(&self) -> bool {
        self.runtime.is_job_pending()
    }

    /// The canonical path of every file module this generation loaded, sorted: the entry and each
    /// module it imported, directly or not. Embedded modules are not files and are not listed.
    ///
    /// This is the set a file watcher observes for hot reload.
    pub fn module_paths(&self) -> Vec<PathBuf> {
        self.load.files()
    }

    /// The name of every embedded module this generation loaded, sorted: the stock library's import log
    /// (`gnoblin-core/host/client`, `gnoblin-std/layout`, ...) and any module the host registered by name. A test reads
    /// it to say which packages a shell pulled in.
    pub fn embedded_modules(&self) -> Vec<String> {
        self.load.embedded()
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
        self.eval_specifier(specifier)
    }

    /// Imports and evaluates the module `specifier` names, an absolute path or the name of an embedded module,
    /// under the same budget and with the same settling rule as [`eval_module`](Self::eval_module).
    pub fn eval_specifier(&self, specifier: String) -> Result<(), JsError> {
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

    /// Runs `f` with the generation's context, giving it at most `budget` of the thread's CPU time, and
    /// [`WALL_BACKSTOP_FACTOR`] times that of wall-clock time.
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

    /// The error for a call the handler stopped. The message names the limit, how much of each clock the call had
    /// used and the script's own innermost frames, so a trip in the log says which call, for how long and against
    /// what, whichever clock stopped it.
    fn budget_error(&self, budget: Duration, thrown: Option<JsError>) -> JsError {
        let (cpu, wall) = self.budget.used();
        let mut message = if cpu >= budget {
            format!(
                "the script used {} of CPU time ({} of wall time) against its {} CPU budget and was interrupted",
                millis(cpu),
                millis(wall),
                millis(budget)
            )
        } else {
            format!(
                "the script was interrupted after {} of wall time, the {} wall-clock backstop to its {} CPU budget \
                 (it used {} of CPU time)",
                millis(wall),
                millis(budget.saturating_mul(WALL_BACKSTOP_FACTOR)),
                millis(budget),
                millis(cpu)
            )
        };
        if let Some(stack) = thrown.as_ref().and_then(|thrown| thrown.stack.as_deref()) {
            let frames: Vec<&str> = stack
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(3)
                .collect();
            if !frames.is_empty() {
                message.push_str(&format!("; it was in {}", frames.join(", called from ")));
            }
        }
        let mut error = JsError::new(JsErrorKind::Budget, message);
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
                let (remapped, location) = remap_stack(&self.load, &text);
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

    /// `stack` with every frame of a loaded module rewritten to the file and position that wrote it: a `.ts` file's own lines.
    pub(crate) fn remap_stack(&self, stack: &str) -> String {
        remap_stack(&self.load, stack).0
    }

    /// Starts keeping the lines the script logs through `console`, until [`take_console`](Self::take_console).
    /// The console evaluates one expression at a time, so a capture never nests.
    pub(crate) fn capture_console(&self) {
        self.console.begin_capture();
    }

    /// Stops keeping console lines and returns the ones logged since [`capture_console`](Self::capture_console).
    pub(crate) fn take_console(&self) -> Vec<JsConsoleLine> {
        self.console.end_capture()
    }

    /// Reports the lines the console dropped in a second that is over, when the script has not called it since. The
    /// shell calls it once a frame, so a notice is not held back by a script that stopped logging.
    pub(crate) fn flush_console(&self) {
        self.console.roll();
    }
}

/// Rewrites every frame of `stack` whose file has a source map (a user's `.ts` file or a stock module) to the `.ts`
/// file and position that wrote it, and returns the location of the first frame that has a file and a line.
pub(crate) fn remap_stack(load: &JsLoadState, stack: &str) -> (String, Option<JsLocation>) {
    let mut out = String::with_capacity(stack.len());
    let mut first = None;
    for line in stack.lines() {
        let rewritten = match split_frame(line) {
            Some(frame) => {
                let (file, line_no, column) = match load.remap(frame.file, frame.line, frame.column) {
                    Some(at) => (at.file, at.line, at.column),
                    None => (frame.file.to_owned(), frame.line, frame.column),
                };
                let text = format!("{}{file}:{line_no}:{column}{}", frame.before, frame.after);
                first.get_or_insert_with(|| JsLocation {
                    file: PathBuf::from(file),
                    line: line_no,
                    column,
                });
                text
            }
            None => line.to_owned(),
        };
        out.push_str(&rewritten);
        out.push('\n');
    }
    (out, first)
}

/// One stack line split around its `file:line:column`.
pub(crate) struct JsFrame<'a> {
    before: &'a str,
    pub(crate) file: &'a str,
    pub(crate) line: u32,
    pub(crate) column: u32,
    after: &'a str,
}

/// Splits a QuickJS frame (`    at fn (file:1:2)` or `    at file:1:2`), or `None` for a line that is
/// not a frame with a position, such as a native frame.
pub(crate) fn split_frame(line: &str) -> Option<JsFrame<'_>> {
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
mod tests;
