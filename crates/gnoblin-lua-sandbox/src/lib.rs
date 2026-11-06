//! The sandboxed Lua state shared by the runtime and the layout host.
//!
//! Every Lua state in the workspace is built here, so the limits are stated once
//! (design/rust-compositor.md, D1 and D17). Three limits bound every entry into Lua:
//!
//! - **Instructions.** A count hook stops a loop that never returns. The host arms
//!   it before every entry with [`Guard::begin`], so each entry gets a budget.
//! - **Memory.** The allocator refuses past [`Limits::memory_bytes`], so a script
//!   that builds a huge table fails with a memory error.
//! - **Deadline.** The hook also compares wall-clock time with
//!   [`Limits::handler_ticks`] milliseconds, because a loop that never yields to the
//!   caller would otherwise never see its budget run out.
//!
//! Two profiles choose the stdlib each state loads:
//!
//! - [`Profile::Config`] is the runtime's configuration VM: `string`, `table`,
//!   `math`, `utf8`, `coroutine` and the base library, plus the time half of `os`.
//!   `io`, `debug` and `package` are not opened, and the base functions that read
//!   files (`dofile`, `loadfile`) and the loader (`require`) are cleared. `os` is
//!   replaced by a table with `time`, `date`, `clock` and `difftime`. The clock
//!   functions read the host's clock, which is the only way a config learns the time.
//! - [`Profile::Layout`] is the pure subset a layout runs under (D17). The base
//!   library is cut to an allowlist that keeps `pcall`, `xpcall` and `coroutine`; the
//!   hook bounds all three. `string` keeps `len`, `sub`, `byte`, `char` and a
//!   `format` that refuses a width or precision above 99. `table` keeps `sort`, whose
//!   comparator is Lua code and so runs under the instruction hook. `math` loses
//!   `random`. `string.rep` is absent because the stock C version cannot be stopped by
//!   the count hook. `os` keeps only the pure parts: `time(table)`, `date(format,
//!   time)` and `difftime`. Calls that would read the clock are refused.
//! - In the configuration profile `string.find`, `match`, `gmatch` and `gsub` are
//!   replaced by a Rust matcher in [`pattern`] that spends the same step budget, with
//!   output identical to stock Lua 5.5 (`tests/pattern_parity.rs`). `pcall` and
//!   `xpcall` are wrapped in both profiles so a limit cannot be caught. A coroutine
//!   here is only control flow inside one entry, so the same hook bounds it.
//!
//! Coroutines run inside one `arrange` call or one config entry. Scheduling a layout
//! across frames is a separate design, and is not what this crate provides.
//!
//! The sandbox keeps `io`, `os.execute`, `os.remove`, `os.rename`, `os.getenv`,
//! `os.tmpname`, `os.exit`, `package`, `debug`, `dofile` and `loadfile` out of both
//! profiles for good. They reach the host system, which is the sandbox boundary. This
//! is the one deliberate exception to "full Lua": every other standard library
//! function a script can name is either present or refused by a documented rule.
//!
//! Binary Lua bytecode is never loaded. In the configuration profile `load` is
//! wrapped so its mode is always text, because crafted bytecode can crash the
//! interpreter; the layout profile has no `load` at all.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mlua::{Error, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState};

mod pattern;
mod strlib;

/// How many instructions pass between two hook calls.
///
/// The hook is a C function called from the VM, so it must not run on every
/// instruction. A thousand stops an aborted loop within microseconds and does not
/// slow ordinary code.
const HOOK_INSTRUCTIONS: u32 = 1000;

/// The largest width or precision `string.format` accepts in the layout profile.
const FORMAT_LIMIT: usize = 99;

/// The budgets one entry into Lua runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Instructions one entry into Lua may execute.
    pub instructions: u64,
    /// The most memory the Lua allocator may hand out, in bytes.
    pub memory_bytes: usize,
    /// How long one entry into Lua may run, in milliseconds (the name is the
    /// caller's clock unit: a tick is a millisecond).
    pub handler_ticks: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            // About thirty million instructions per second on a current core, so one
            // entry gets a few milliseconds of interpreter time.
            instructions: 10_000_000,
            memory_bytes: 32 * 1024 * 1024,
            // A handler should finish inside a frame.
            handler_ticks: 16,
        }
    }
}

/// Which stdlib subset a state loads. See the crate documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// The runtime's configuration VM.
    Config,
    /// The layout VM: a pure subset with no file, clock or spawn access.
    Layout,
}

/// The budgets one entry consumes, shared between the host and the hook.
///
/// The host owns the cells and the hook reads them, so this is a separate type: the
/// closure `mlua` stores cannot borrow the host.
#[derive(Debug)]
pub struct Guard {
    remaining: Cell<u64>,
    started: Cell<Option<Instant>>,
    aborted: Cell<bool>,
    /// The message of the abort that latched `aborted`. Every later raise repeats it, so a
    /// caught error still names the budget that ran out.
    abort_message: std::cell::RefCell<String>,
    /// Hook calls per Lua thread as (thread pointer, count). Tests read it to see which
    /// threads the hook reached.
    #[cfg(test)]
    hook_calls: std::cell::RefCell<Vec<(usize, u64)>>,
}

impl Default for Guard {
    fn default() -> Guard {
        Guard::new()
    }
}

impl Guard {
    /// A guard for a host that has not run anything yet.
    pub fn new() -> Guard {
        Guard {
            remaining: Cell::new(0),
            started: Cell::new(None),
            aborted: Cell::new(false),
            abort_message: Default::default(),
            #[cfg(test)]
            hook_calls: Default::default(),
        }
    }

    /// Arm the budgets for one entry into Lua.
    pub fn begin(&self, limits: &Limits) {
        self.remaining.set(limits.instructions);
        self.aborted.set(false);
        self.started.set(Some(Instant::now()));
    }

    /// Whether the hook stopped the last entry for exhausting a budget.
    pub fn aborted(&self) -> bool {
        self.aborted.get()
    }

    /// Latch the abort so every later hook call raises again, and return the error to raise.
    fn latch(&self, message: String) -> Error {
        self.aborted.set(true);
        self.abort_message.replace(message.clone());
        Error::runtime(message)
    }

    /// The message of the abort that latched this entry, or empty when none has.
    fn abort_message(&self) -> String {
        self.abort_message.borrow().clone()
    }

    /// Count one hook call against the Lua thread that made it.
    #[cfg(test)]
    fn record_hook_call(&self, lua: &Lua) {
        let thread = lua.current_thread().to_pointer() as usize;
        let mut calls = self.hook_calls.borrow_mut();
        match calls.iter_mut().find(|(seen, _)| *seen == thread) {
            Some((_, count)) => *count += 1,
            None => calls.push((thread, 1)),
        }
    }

    /// The per-thread hook counts recorded so far.
    #[cfg(test)]
    fn hook_calls(&self) -> Vec<(usize, u64)> {
        self.hook_calls.borrow().clone()
    }
}

/// Create a sandboxed Lua state for `profile` under `limits`.
///
/// The returned state has no file, process or environment access in either profile. The
/// configuration profile reads the host clock through `os`; the layout profile does not.
/// The caller arms `guard` with [`Guard::begin`] before each entry.
pub fn create(profile: Profile, limits: &Limits, guard: Rc<Guard>) -> Result<Lua, Error> {
    let libraries = match profile {
        Profile::Config => {
            StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH | StdLib::COROUTINE | StdLib::OS
        }
        Profile::Layout => StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::COROUTINE | StdLib::OS,
    };
    let lua = Lua::new_with(libraries, LuaOptions::default())?;
    lua.set_memory_limit(limits.memory_bytes)?;
    match profile {
        Profile::Config => clear_config(&lua)?,
        Profile::Layout => restrict_layout(&lua)?,
    }
    restrict_os(&lua, profile)?;
    if profile == Profile::Config {
        // Stock Lua seeds `math.random` from the clock and a stack address when a state
        // opens, so two identical configs would draw different numbers. A fixed seed
        // makes the draws a pure function of the script.
        let math: Table = lua.globals().get("math")?;
        let randomseed: Function = math.get("randomseed")?;
        randomseed.call::<()>(CONFIG_RANDOM_SEED)?;
        strlib::install(&lua, Rc::clone(&guard), limits.instructions)?;
    }

    install_hook(&lua, limits, Rc::clone(&guard))?;
    guard_protected_calls(&lua, guard)?;
    Ok(lua)
}

/// The seed the configuration profile gives `math.random` at every state open.
const CONFIG_RANDOM_SEED: i64 = 0x6f6d_7073;

/// Wrap `pcall` and `xpcall` so a limit abort cannot be caught.
///
/// The hook latches [`Guard::aborted`] when it stops an entry, and every later hook
/// call raises again. Without this wrapper a loop around `pcall` would catch each
/// raise and keep running, so the entry would never end. The wrapper rethrows the
/// caught message whenever the latch is set.
fn guard_protected_calls(lua: &Lua, guard: Rc<Guard>) -> Result<(), Error> {
    let globals = lua.globals();
    let pcall: Function = globals.get("pcall")?;
    let guarded_pcall = {
        let guard = Rc::clone(&guard);
        lua.create_function(move |_, args: MultiValue| {
            let results = pcall.call::<MultiValue>(args).map_err(|e| abort_or(e, &guard))?;
            rethrow_if_aborted(results, &guard)
        })?
    };
    globals.set("pcall", guarded_pcall)?;
    let xpcall: Function = globals.get("xpcall")?;
    let guarded_xpcall = lua.create_function(move |_, args: MultiValue| {
        let results = xpcall.call::<MultiValue>(args).map_err(|e| abort_or(e, &guard))?;
        rethrow_if_aborted(results, &guard)
    })?;
    globals.set("xpcall", guarded_xpcall)
}

/// The error to raise for a protected call that failed outside its own results. Once a limit
/// has aborted the entry, the stored budget message replaces the raw mlua error, which may
/// carry no usable text.
fn abort_or(error: Error, guard: &Guard) -> Error {
    if guard.aborted() {
        Error::runtime(guard.abort_message())
    } else {
        error
    }
}

/// Pass a protected call's results through, or rethrow them if a limit aborted.
///
/// Once the latch is set the caught value is ignored: a message handler may return nil,
/// which Lua turns into the text '<no error object>', so the stored budget message is the
/// only reliable thing to rethrow.
fn rethrow_if_aborted(results: MultiValue, guard: &Guard) -> Result<MultiValue, Error> {
    let failed = matches!(results.front(), Some(Value::Boolean(false)));
    if failed && guard.aborted() {
        return Err(Error::runtime(guard.abort_message()));
    }
    Ok(results)
}

/// Clear the configuration VM's dangerous globals by name.
///
/// The base library is always open, and it holds the functions that reach the
/// filesystem. Clearing them by name is what makes the sandbox one list a reviewer
/// can read.
fn clear_config(lua: &Lua) -> Result<(), Error> {
    let globals = lua.globals();
    for name in ["dofile", "loadfile", "require", "io", "debug", "package"] {
        globals.set(name, Value::Nil)?;
    }
    text_only_load(lua)
}

/// Wrap `load` so its mode is always text.
///
/// Crafted bytecode can crash the interpreter, so `load` never accepts a binary
/// chunk. The caller's mode argument is replaced, and the argument count is kept,
/// because an explicit `nil` environment is not the same as an absent one.
fn text_only_load(lua: &Lua) -> Result<(), Error> {
    let original: Function = lua.globals().get("load")?;
    let load = lua.create_function(move |lua, args: MultiValue| {
        let mut values: Vec<Value> = args.into_iter().collect();
        while values.len() < 2 {
            values.push(Value::Nil);
        }
        let mode = Value::String(lua.create_string("t")?);
        if values.len() == 2 {
            values.push(mode);
        } else {
            values[2] = mode;
        }
        original.call::<MultiValue>(values.into_iter().collect::<MultiValue>())
    })?;
    lua.globals().set("load", load)
}

/// Cut the layout VM to the pure subset (D17, "Stdlib subset").
fn restrict_layout(lua: &Lua) -> Result<(), Error> {
    let globals = lua.globals();
    let keep = [
        "assert",
        "error",
        "ipairs",
        "next",
        "pairs",
        "pcall",
        "select",
        "tonumber",
        "tostring",
        "type",
        "xpcall",
        "math",
        "string",
        "table",
        "coroutine",
        "os",
    ];
    let names: Vec<Value> = globals
        .pairs::<Value, Value>()
        .map(|pair| pair.map(|(key, _)| key))
        .collect::<Result<_, _>>()?;
    for key in names {
        let allowed = match &key {
            Value::String(name) => keep.contains(&&*name.to_str()?),
            _ => false,
        };
        if !allowed {
            globals.set(key, Value::Nil)?;
        }
    }

    let string: Table = globals.get("string")?;
    let kept_strings = ["len", "sub", "byte", "char", "format"];
    let strings: Vec<Value> = string
        .pairs::<Value, Value>()
        .map(|pair| pair.map(|(key, _)| key))
        .collect::<Result<_, _>>()?;
    for key in strings {
        let allowed = match &key {
            Value::String(name) => kept_strings.contains(&&*name.to_str()?),
            _ => false,
        };
        if !allowed {
            string.set(key, Value::Nil)?;
        }
    }
    let original: Function = string.get("format")?;
    let format = lua.create_function(move |_, args: MultiValue| {
        if let Some(Value::String(template)) = args.front() {
            check_format(&template.to_str()?)?;
        }
        original.call::<MultiValue>(args)
    })?;
    string.set("format", format)?;

    let table: Table = globals.get("table")?;
    let math: Table = globals.get("math")?;
    math.set("random", Value::Nil)?;
    math.set("randomseed", Value::Nil)?;
    Ok(())
}

/// Replace `os` with a table that holds only the time functions.
///
/// Stock `os` reaches the host (files, processes and environment variables), so the table
/// is rebuilt from the functions that do not. The layout profile keeps the pure parts:
/// `time` with a table, `date` with an explicit time, and `difftime`. Layouts must replay
/// identically, so a call that would read the clock is refused. The configuration profile
/// reads the host clock through its own functions, so `time()`, `date(format)` and
/// `clock()` work there. The stock functions are captured here and dropped with the old
/// table.
fn restrict_os(lua: &Lua, profile: Profile) -> Result<(), Error> {
    let stock: Table = lua.globals().get("os")?;
    let stock_time: Function = stock.get("time")?;
    let stock_date: Function = stock.get("date")?;
    let os = lua.create_table()?;
    os.set("difftime", stock.get::<Function>("difftime")?)?;
    match profile {
        Profile::Layout => {
            os.set(
                "time",
                lua.create_function(move |_, args: MultiValue| {
                    let values: Vec<Value> = args.into_iter().collect();
                    if !matches!(values.first(), Some(Value::Table(_))) {
                        return Err(Error::runtime(
                            "os.time() without a table reads the clock, which a layout cannot do",
                        ));
                    }
                    stock_time.call::<MultiValue>(values.into_iter().collect::<MultiValue>())
                })?,
            )?;
            os.set(
                "date",
                lua.create_function(move |_, args: MultiValue| {
                    let values: Vec<Value> = args.into_iter().collect();
                    if !matches!(values.get(1), Some(Value::Integer(_) | Value::Number(_))) {
                        return Err(Error::runtime(
                            "os.date() without an explicit time reads the clock, which a layout cannot do",
                        ));
                    }
                    stock_date.call::<MultiValue>(values.into_iter().collect::<MultiValue>())
                })?,
            )?;
        }
        Profile::Config => {
            // The clock is read when the state opens, not at each call, so `clock()` counts
            // from the state's start and never from the process's start.
            let started = Instant::now();
            os.set(
                "time",
                lua.create_function(move |_, args: MultiValue| {
                    let values: Vec<Value> = args.into_iter().collect();
                    if matches!(values.first(), None | Some(Value::Nil)) {
                        return Ok(vec![Value::Integer(host_now_secs())]
                            .into_iter()
                            .collect::<MultiValue>());
                    }
                    stock_time.call::<MultiValue>(values.into_iter().collect::<MultiValue>())
                })?,
            )?;
            os.set(
                "date",
                lua.create_function(move |_, args: MultiValue| {
                    let mut values: Vec<Value> = args.into_iter().collect();
                    if values.len() < 2 {
                        values.resize(2, Value::Nil);
                    }
                    if matches!(values[1], Value::Nil) {
                        values[1] = Value::Integer(host_now_secs());
                    }
                    stock_date.call::<MultiValue>(values.into_iter().collect::<MultiValue>())
                })?,
            )?;
            os.set(
                "clock",
                lua.create_function(move |_, ()| Ok(started.elapsed().as_secs_f64()))?,
            )?;
        }
    }
    lua.globals().set("os", os)
}

/// The host's wall clock as whole seconds since the Unix epoch.
///
/// A clock set before 1970 reads as zero rather than a negative time, which keeps the
/// value a plain Lua integer.
fn host_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

/// Refuse a `string.format` template whose width or precision exceeds the limit.
fn check_format(template: &str) -> Result<(), Error> {
    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        index += 1;
        if bytes.get(index) == Some(&b'%') {
            index += 1;
            continue;
        }
        while bytes.get(index).is_some_and(|byte| b"-+ #0".contains(byte)) {
            index += 1;
        }
        let (width, next) = digits(bytes, index);
        index = next;
        if width > FORMAT_LIMIT {
            return Err(format_refused());
        }
        if bytes.get(index) == Some(&b'.') {
            let (precision, next) = digits(bytes, index + 1);
            index = next;
            if precision > FORMAT_LIMIT {
                return Err(format_refused());
            }
        }
        index += 1;
    }
    Ok(())
}

fn digits(bytes: &[u8], mut index: usize) -> (usize, usize) {
    let mut value = 0usize;
    while let Some(byte) = bytes.get(index).filter(|byte| byte.is_ascii_digit()) {
        value = value.saturating_mul(10).saturating_add(usize::from(byte - b'0'));
        index += 1;
    }
    (value, index)
}

fn format_refused() -> Error {
    Error::runtime(format!(
        "string.format width or precision above {FORMAT_LIMIT} is refused in a layout"
    ))
}

/// The message the budget abort raises. The budget is the instruction count the entry
/// was armed with, so the text names the same number whichever path ran out.
pub(crate) fn budget_message(budget: u64) -> String {
    format!(
        "the Lua instruction budget ({budget} instructions) is exhausted; a handler must \
         finish inside its budget"
    )
}

/// Install the count hook that enforces the instruction and deadline budgets.
fn install_hook(lua: &Lua, limits: &Limits, guard: Rc<Guard>) -> Result<(), Error> {
    // The closure outlives this call, so the numbers it names are copied in.
    let budget = limits.instructions;
    let deadline_ms = limits.handler_ticks;
    let deadline = Duration::from_millis(deadline_ms);
    // A global hook, not a thread hook. mlua's thread hook answers only for the thread it
    // was set on, so a coroutine would run unbounded. A global hook is copied into every
    // coroutine that `lua_newthread` creates, so no thread escapes the budgets.
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTIONS),
        move |_lua, _| {
            #[cfg(test)]
            guard.record_hook_call(_lua);
            if guard.aborted.get() {
                return Err(Error::runtime(guard.abort_message()));
            }
            let remaining = guard.remaining.get();
            if remaining <= u64::from(HOOK_INSTRUCTIONS) {
                guard.remaining.set(0);
                return Err(guard.latch(budget_message(budget)));
            }
            guard.remaining.set(remaining - u64::from(HOOK_INSTRUCTIONS));
            if let Some(started) = guard.started.get()
                && started.elapsed() >= deadline
            {
                return Err(guard.latch(format!("the handler deadline ({deadline_ms} ms) passed")));
            }
            Ok(VmState::Continue)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua(profile: Profile) -> Lua {
        let limits = Limits::default();
        let guard = Rc::new(Guard::new());
        guard.begin(&limits);
        create(profile, &limits, guard).expect("the sandbox builds")
    }

    #[test]
    fn layout_profile_drops_every_global_outside_the_allowlist() {
        let lua = lua(Profile::Layout);
        for name in ["io", "debug", "package", "require", "load", "dofile", "print", "utf8"] {
            let value: Value = lua.globals().get(name).expect("a global reads");
            assert!(
                matches!(value, Value::Nil),
                "{name} must be absent in the layout profile"
            );
        }
    }

    #[test]
    fn layout_profile_refuses_pattern_functions_and_random() {
        let lua = lua(Profile::Layout);
        for call in [
            "string.find('abc', 'b')",
            "string.match('abc', 'b')",
            "string.gsub('abc', 'b', 'c')",
            "string.rep('a', 10)",
            "math.random()",
            "string.dump(print)",
        ] {
            assert!(lua.load(call).exec().is_err(), "{call} must be refused");
        }
    }

    #[test]
    fn layout_sort_comparator_is_bounded_by_the_budget() {
        let limits = Limits {
            instructions: 50_000,
            ..Limits::default()
        };
        let (lua, guard) = armed(Profile::Layout, &limits);
        let result = lua
            .load("local t = {3, 1, 2}; table.sort(t, function(a, b) while true do end end)")
            .exec();
        assert!(result.is_err(), "a looping comparator must hit the budget");
        assert!(guard.aborted());
        guard.begin(&limits);
        let sorted: String = lua
            .load("local t = {3, 1, 2}; table.sort(t, function(a, b) return a > b end); return table.concat(t, ',')")
            .eval()
            .expect("a fresh entry sorts");
        assert_eq!(sorted, "3,2,1");
    }

    #[test]
    fn layout_format_refuses_width_above_ninety_nine() {
        let lua = lua(Profile::Layout);
        assert!(lua.load("return string.format('%5d', 7)").eval::<String>().is_ok());
        assert!(lua.load("return string.format('%100d', 7)").exec().is_err());
        assert!(lua.load("return string.format('%.200f', 1.0)").exec().is_err());
        assert!(lua.load("return string.format('%%100d')").eval::<String>().is_ok());
    }

    #[test]
    fn config_profile_keeps_its_stdlib_and_clears_file_access() {
        let lua = lua(Profile::Config);
        for name in ["io", "require", "dofile", "package", "debug"] {
            let value: Value = lua.globals().get(name).expect("a global reads");
            assert!(
                matches!(value, Value::Nil),
                "{name} must be absent in the config profile"
            );
        }
        assert!(lua.load("return utf8.char(65)").eval::<String>().is_ok());
        assert!(lua.load("return string.find('abc', 'b')").eval::<i64>().is_ok());
    }

    #[test]
    fn os_keeps_only_host_clock_functions_in_both_profiles() {
        for profile in [Profile::Config, Profile::Layout] {
            let lua = lua(profile);
            let ok: bool = lua
                .load("return os.execute == nil and os.remove == nil and os.rename == nil and os.getenv == nil and os.tmpname == nil and os.exit == nil and os.difftime ~= nil")
                .eval()
                .expect("the os table reads");
            assert!(ok, "{profile:?} keeps a host-reaching os function");
        }
        let lua = lua(Profile::Layout);
        assert!(lua.load("return os.clock == nil").eval::<bool>().expect("reads"));
        assert!(
            lua.load("return os.time()").exec().is_err(),
            "a layout must not read the clock"
        );
        assert!(
            lua.load("return os.date('%Y')").exec().is_err(),
            "a layout must not read the clock"
        );
        assert!(
            lua.load("return os.time({year = 2000, month = 1, day = 1, hour = 12})")
                .eval::<i64>()
                .is_ok()
        );
        assert!(lua.load("return os.date('!%Y', 0)").eval::<String>().is_ok());
        let lua = lua(Profile::Config);
        assert!(
            lua.load("return type(os.time()) == 'number' and type(os.clock()) == 'number'")
                .eval::<bool>()
                .expect("reads")
        );
    }

    #[test]
    fn the_instruction_budget_aborts_a_loop() {
        let limits = Limits {
            instructions: 50_000,
            ..Limits::default()
        };
        let guard = Rc::new(Guard::new());
        guard.begin(&limits);
        let lua = create(Profile::Layout, &limits, Rc::clone(&guard)).expect("the sandbox builds");
        let error = lua.load("while true do end").exec().expect_err("the loop must abort");
        assert!(error.to_string().contains("instruction budget"), "{error}");
        assert!(guard.aborted());
    }

    /// What one capped run of a script reports back to the test thread. A `Lua` state is
    /// not `Send`, so the state is built and dropped on the worker thread.
    struct Outcome {
        error: String,
        aborted: bool,
        /// How many distinct Lua threads the hook fired on.
        hook_threads: usize,
        /// Total hook calls across every thread.
        hook_calls: u64,
    }

    /// Run `script` on a worker thread and wait at most five seconds. A hang then fails
    /// fast instead of stalling the test run. The worker is leaked if it is still spinning,
    /// which only happens on a failing test, and the process ends after the test run.
    fn capped(build: fn(&Limits) -> (Lua, Rc<Guard>), limits: Limits, script: &'static str) -> Outcome {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (lua, guard) = build(&limits);
            let error = match lua.load(script).exec() {
                Ok(()) => String::from("the script finished without an error"),
                Err(error) => error.to_string(),
            };
            let calls = guard.hook_calls();
            let outcome = Outcome {
                error,
                aborted: guard.aborted(),
                hook_threads: calls.len(),
                hook_calls: calls.iter().map(|(_, count)| count).sum(),
            };
            let _ = sender.send(outcome);
        });
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the script must end within five seconds; a hang means the hook missed its thread")
    }

    fn config_state(limits: &Limits) -> (Lua, Rc<Guard>) {
        armed(Profile::Config, limits)
    }

    fn coroutine_state(limits: &Limits) -> (Lua, Rc<Guard>) {
        armed(Profile::Config, limits)
    }

    fn layout_state(limits: &Limits) -> (Lua, Rc<Guard>) {
        armed(Profile::Layout, limits)
    }

    fn small_budget() -> Limits {
        Limits {
            instructions: 50_000,
            ..Limits::default()
        }
    }

    #[test]
    fn a_bare_loop_hits_the_budget_in_the_config_profile() {
        let outcome = capped(config_state, small_budget(), "while true do end");
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn pcall_cannot_catch_an_instruction_abort_in_the_config_profile() {
        let outcome = capped(
            config_state,
            small_budget(),
            "while true do pcall(function() while true do end end) end",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn a_pcall_loop_of_raises_cannot_outrun_the_budget() {
        let outcome = capped(config_state, small_budget(), "while true do pcall(error, 'x') end");
        assert!(
            outcome.error.contains("instruction budget") || outcome.error.contains("handler deadline"),
            "{}",
            outcome.error
        );
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn a_repeated_pcall_of_a_loop_ends_after_the_latch() {
        let outcome = capped(
            config_state,
            small_budget(),
            "repeat local ok = pcall(function() while true do end end) until false",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn xpcall_cannot_catch_an_instruction_abort_in_the_config_profile() {
        let outcome = capped(
            config_state,
            small_budget(),
            "while true do xpcall(function() while true do end end, print) end",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn a_coroutine_wrap_loop_is_bounded_by_the_budget() {
        let outcome = capped(
            coroutine_state,
            small_budget(),
            "local co = coroutine.wrap(function() while true do end end) co()",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
        // Hook counts are reported per thread the hook saw. mlua's current-thread query can
        // report only the main state, so this asserts the bound (`aborted`), not the count.
    }

    #[test]
    fn layout_coroutine_wrap_loop_is_bounded_by_the_budget() {
        let outcome = capped(
            layout_state,
            small_budget(),
            "local co = coroutine.wrap(function() while true do end end) co()",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn layout_pcall_around_a_coroutine_cannot_swallow_the_budget() {
        let outcome = capped(
            layout_state,
            small_budget(),
            "while true do pcall(coroutine.wrap(function() while true do end end)) end",
        );
        assert!(
            outcome.error.contains("instruction budget") || outcome.error.contains("handler deadline"),
            "{}",
            outcome.error
        );
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn pcall_around_a_coroutine_cannot_swallow_the_budget() {
        let outcome = capped(
            coroutine_state,
            small_budget(),
            "while true do pcall(coroutine.wrap(function() while true do end end)) end",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn a_resume_loop_cannot_outrun_the_budget() {
        let outcome = capped(
            coroutine_state,
            small_budget(),
            "while true do coroutine.resume(coroutine.create(function() while true do end end)) end",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn a_coroutine_loop_hits_the_deadline() {
        let limits = Limits {
            instructions: 1 << 50,
            handler_ticks: 20,
            ..Limits::default()
        };
        let outcome = capped(
            coroutine_state,
            limits,
            "while true do coroutine.resume(coroutine.create(function() while true do end end)) end",
        );
        assert!(outcome.error.contains("deadline"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn xpcall_with_a_handler_that_returns_nothing_cannot_swallow_the_budget() {
        // A handler that returns no value makes xpcall report `nil`, which the rethrow path
        // must not treat as a recovered error. The latch decides, so the loop still stops.
        let outcome = capped(
            config_state,
            small_budget(),
            "while true do xpcall(function() while true do end end, function(e) end) end",
        );
        assert!(outcome.error.contains("instruction budget"), "{}", outcome.error);
        assert!(
            outcome.aborted,
            "{} hook calls on {} threads",
            outcome.hook_calls, outcome.hook_threads
        );
    }

    #[test]
    fn a_coroutine_created_before_exhaustion_stays_bounded_when_resumed_after() {
        // The thread is created in one entry and resumed in a later, re-armed entry after the
        // first entry's budget ran out. The hook is global, so the old thread must still be
        // counted and stopped rather than run free.
        let limits = small_budget();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (lua, guard) = coroutine_state(&limits);
            lua.load("co = coroutine.create(function() while true do coroutine.yield() end end)")
                .exec()
                .expect("the thread is created inside the first budget");
            let exhausted = lua.load("while true do end").exec().is_err();
            let rearmed = {
                guard.begin(&limits);
                lua.load("return coroutine.resume(co)").eval::<bool>()
            };
            let error = lua
                .load("while true do coroutine.resume(co) end")
                .exec()
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            let _ = sender.send((exhausted, rearmed.ok(), error, guard.aborted()));
        });
        let (exhausted, resumed, error, aborted) = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the resumed thread must end within five seconds");
        assert!(exhausted, "the first entry must exhaust its budget");
        assert_eq!(
            resumed,
            Some(true),
            "the pre-exhaustion thread must resume after re-arming"
        );
        assert!(error.contains("instruction budget"), "{error}");
        assert!(aborted);
    }

    #[test]
    fn config_profile_opens_the_coroutine_library() {
        let lua = lua(Profile::Config);
        let value: String = lua
            .load("return type(coroutine.wrap) .. type(coroutine.yield)")
            .eval()
            .expect("the coroutine library reads");
        assert_eq!(value, "functionfunction");
    }

    fn armed(profile: Profile, limits: &Limits) -> (Lua, Rc<Guard>) {
        let guard = Rc::new(Guard::new());
        guard.begin(limits);
        let lua = create(profile, limits, Rc::clone(&guard)).expect("the sandbox builds");
        (lua, guard)
    }

    #[test]
    fn a_rearmed_guard_gives_the_next_entry_a_fresh_budget() {
        let limits = Limits {
            instructions: 50_000,
            ..Limits::default()
        };
        let (lua, guard) = armed(Profile::Layout, &limits);
        assert!(lua.load("while true do end").exec().is_err());
        assert!(guard.aborted());
        guard.begin(&limits);
        assert!(!guard.aborted());
        assert_eq!(lua.load("return 1 + 1").eval::<i64>().expect("a fresh entry runs"), 2);
    }

    #[test]
    fn the_deadline_aborts_a_loop_that_outlives_its_wall_clock_budget() {
        let limits = Limits {
            instructions: u64::MAX,
            handler_ticks: 5,
            ..Limits::default()
        };
        let (lua, guard) = armed(Profile::Layout, &limits);
        let error = lua.load("while true do end").exec().expect_err("the loop must abort");
        assert!(error.to_string().contains("handler deadline (5 ms) passed"), "{error}");
        assert!(guard.aborted());
    }

    #[test]
    fn the_memory_limit_refuses_a_table_that_grows_past_it() {
        let limits = Limits {
            instructions: u64::MAX,
            memory_bytes: 1024 * 1024,
            handler_ticks: 60_000,
        };
        let (lua, _guard) = armed(Profile::Layout, &limits);
        let error = lua
            .load("local t = {} for i = 1, 100000000 do t[i] = i end")
            .exec()
            .expect_err("the table must not fit");
        assert!(error.to_string().to_lowercase().contains("memory"), "{error}");
    }

    #[test]
    fn config_string_rep_is_bounded_by_the_memory_limit() {
        let limits = Limits {
            instructions: u64::MAX,
            memory_bytes: 1024 * 1024,
            handler_ticks: 60_000,
        };
        let (lua, _guard) = armed(Profile::Config, &limits);
        let error = lua
            .load("return string.rep('a', 1e12)")
            .exec()
            .expect_err("a terabyte string must not be built");
        assert!(error.to_string().to_lowercase().contains("memory"), "{error}");
    }

    #[test]
    fn config_random_is_seeded_identically_in_every_state() {
        let draw = || -> String {
            let lua = lua(Profile::Config);
            lua.load("return table.concat({math.random(1, 1000), math.random(1, 1000)}, ',')")
                .eval()
                .expect("random draws")
        };
        assert_eq!(draw(), draw(), "two fresh states must draw the same numbers");
    }

    #[test]
    fn config_collectgarbage_is_bounded_by_the_budget() {
        let limits = Limits {
            instructions: 50_000,
            ..Limits::default()
        };
        let (lua, guard) = armed(Profile::Config, &limits);
        let count: f64 = lua.load("return collectgarbage('count')").eval().expect("count reads");
        assert!(count > 0.0);
        assert!(lua.load("collectgarbage('step') return true").exec().is_ok());
        let result = lua.load("while true do collectgarbage('step') end").exec();
        assert!(result.is_err(), "a step loop must hit the budget");
        assert!(guard.aborted());
    }

    #[test]
    fn config_profile_refuses_bytecode_through_load() {
        let limits = Limits::default();
        let (lua, _guard) = armed(Profile::Config, &limits);
        let refused: bool = lua
            .load("return load(string.dump(function() return 1 end)) == nil")
            .eval()
            .expect("load reports the refusal as nil");
        assert!(refused, "binary chunks must not load in the config profile");
        let value: i64 = lua
            .load("return load('return 7')()")
            .eval()
            .expect("text chunks still load");
        assert_eq!(value, 7);
    }

    #[test]
    fn layout_profile_is_deterministic_across_states() {
        let script = "local acc = {} \
            for i = 1, 200 do acc[#acc + 1] = string.format('%03d:%.3f', i, i / 7) end \
            return table.concat(acc, ',')";
        let limits = Limits::default();
        let (first, _) = armed(Profile::Layout, &limits);
        let (second, _) = armed(Profile::Layout, &limits);
        let a: String = first.load(script).eval().expect("the script runs");
        let b: String = second.load(script).eval().expect("the script runs");
        assert_eq!(a, b);
        assert!(a.starts_with("001:0.143,002:0.286,"), "{a}");
    }
}
