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
//!   replaced by a Rust matcher in [`gnoblin_lua_pattern`] that spends the same step budget, with
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
//! Binary Lua bytecode is never loaded. `load` is wrapped in both profiles so its mode is
//! always text, because crafted bytecode can crash the interpreter.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mlua::{Error, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState};

mod output;
mod strlib;

pub use output::{LogLevel, Sink};

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
pub fn create(profile: Profile, limits: &Limits, guard: Rc<Guard>, sink: Sink) -> Result<Lua, Error> {
    // Both profiles open the same libraries. `io`, `debug` and `package` are never opened,
    // so no script can reach them, even by name.
    let libraries = StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH | StdLib::COROUTINE | StdLib::OS;
    let lua = Lua::new_with(libraries, LuaOptions::default())?;
    lua.set_memory_limit(limits.memory_bytes)?;
    match profile {
        Profile::Config => clear_config(&lua)?,
        Profile::Layout => restrict_layout(&lua)?,
    }
    restrict_os(&lua, profile)?;
    seed_random(&lua)?;
    text_only_load(&lua)?;
    cap_unpack(&lua)?;
    output::install(&lua, sink)?;
    strlib::install(&lua, Rc::clone(&guard), limits.instructions)?;
    install_hook(&lua, limits, Rc::clone(&guard))?;
    guard_protected_calls(&lua, guard)?;
    Ok(lua)
}

/// Seed `math.random` with a fixed value, so draws are a pure function of the script.
///
/// Stock Lua seeds from the clock and a stack address when a state opens, so two identical
/// scripts would draw different numbers. Runs after the layout guard on `randomseed` is
/// installed, which is fine because a seed argument is passed through.
fn seed_random(lua: &Lua) -> Result<(), Error> {
    let math: Table = lua.globals().get("math")?;
    let randomseed: Function = math.get("randomseed")?;
    randomseed.call::<()>(CONFIG_RANDOM_SEED)
}

/// The seed the states give `math.random` at every open.
const CONFIG_RANDOM_SEED: i64 = 0x6f6d_7073;

/// The largest number of values one `table.unpack` call may return.
///
/// Stock Lua refuses only when the C stack cannot grow, which is a late and unclear error.
/// Refusing early keeps a single call from building a very large argument list.
const UNPACK_LIMIT: i128 = 1 << 20;

/// Wrap `table.unpack` so a huge explicit range is refused before any value is pushed.
///
/// Only explicit integer bounds are checked. Without an end index the length of the table
/// decides, and that is already bounded by the memory limit.
fn cap_unpack(lua: &Lua) -> Result<(), Error> {
    let table: Table = lua.globals().get("table")?;
    let original: Function = table.get("unpack")?;
    let unpack = lua.create_function(move |_, args: MultiValue| {
        let values: Vec<Value> = args.into_iter().collect();
        let start = match values.get(1) {
            None | Some(Value::Nil) => 1,
            Some(Value::Integer(start)) => *start,
            Some(_) => return original.call::<MultiValue>(values.into_iter().collect::<MultiValue>()),
        };
        if let Some(Value::Integer(end)) = values.get(2) {
            let count = i128::from(*end) - i128::from(start) + 1;
            if count > UNPACK_LIMIT {
                return Err(Error::runtime(format!(
                    "table.unpack would return {count} values, above the limit of {UNPACK_LIMIT}"
                )));
            }
        }
        original.call::<MultiValue>(values.into_iter().collect::<MultiValue>())
    })?;
    table.set("unpack", unpack)
}

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
    Ok(())
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

/// Cut the layout VM to the base library and the standard libraries that cannot reach the host.
///
/// Layouts run full Lua, so the base functions and the metatable machinery stay: `setmetatable`,
/// `getmetatable`, the raw accessors, `load` (text only, see [`text_only_load`]), `print`,
/// `warn`, `collectgarbage`, `_G` and `_VERSION`. `dofile`, `loadfile` and `require` read the
/// host filesystem, so they are not in the allowlist, and `io`, `debug` and `package` are never
/// opened. Two functions keep a guard. `string.format` refuses a width or precision above
/// `FORMAT_LIMIT`, and `math.randomseed` refuses a call without a seed, because that call
/// reads the clock and a layout must replay identically.
fn restrict_layout(lua: &Lua) -> Result<(), Error> {
    let globals = lua.globals();
    let keep = [
        "_G",
        "_VERSION",
        "assert",
        "collectgarbage",
        "coroutine",
        "error",
        "getmetatable",
        "ipairs",
        "load",
        "math",
        "next",
        "os",
        "pairs",
        "pcall",
        "print",
        "rawequal",
        "rawget",
        "rawlen",
        "rawset",
        "select",
        "setmetatable",
        "string",
        "table",
        "tonumber",
        "tostring",
        "type",
        "utf8",
        "warn",
        "xpcall",
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
    let original: Function = string.get("format")?;
    let format = lua.create_function(move |_, args: MultiValue| {
        if let Some(Value::String(template)) = args.front() {
            check_format(&template.to_str()?)?;
        }
        original.call::<MultiValue>(args)
    })?;
    string.set("format", format)?;

    // `__gc` is withheld in Layout. A finaliser runs at some later allocation, possibly inside a
    // different entry, so a looping one would be charged to whichever entry triggered the
    // collection. Refusing the key at `setmetatable` keeps every metamethod inside its own
    // entry's budget. Config keeps `__gc` because its budget is the host's own script budget.
    let set_metatable: Function = globals.get("setmetatable")?;
    let guarded = lua.create_function(move |_, args: MultiValue| {
        if let Some(Value::Table(metatable)) = args.get(1)
            && !metatable.get::<Value>("__gc")?.is_nil()
        {
            return Err(mlua::Error::RuntimeError(
                "__gc is not supported in the layout profile".into(),
            ));
        }
        set_metatable.call::<Value>(args)
    })?;
    globals.set("setmetatable", guarded)?;

    let math: Table = globals.get("math")?;
    let randomseed: Function = math.get("randomseed")?;
    let guarded_randomseed = lua.create_function(move |_, args: MultiValue| {
        if args.is_empty() {
            return Err(Error::runtime(
                "math.randomseed() without a seed reads the clock, which a layout cannot do",
            ));
        }
        randomseed.call::<()>(args)
    })?;
    math.set("randomseed", guarded_randomseed)?;
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

    /// A sink that drops every message, for tests that do not read the output.
    fn quiet() -> Sink {
        Rc::new(|_, _| {})
    }

    fn lua(profile: Profile) -> Lua {
        let limits = Limits::default();
        let guard = Rc::new(Guard::new());
        guard.begin(&limits);
        create(profile, &limits, guard, quiet()).expect("the sandbox builds")
    }

    #[test]
    fn layout_profile_drops_every_global_outside_the_allowlist() {
        // `load`, `print`, `warn` and `utf8` are deliberately kept (see `restrict_layout`).
        // Only the names that reach the host are absent.
        let lua = lua(Profile::Layout);
        for name in ["io", "debug", "package", "require", "dofile", "loadfile"] {
            let value: Value = lua.globals().get(name).expect("a global reads");
            assert!(
                matches!(value, Value::Nil),
                "{name} must be absent in the layout profile"
            );
        }
        for name in ["load", "print", "warn", "utf8"] {
            let value: Value = lua.globals().get(name).expect("a global reads");
            assert!(!matches!(value, Value::Nil), "{name} must stay in the layout profile");
        }
    }

    #[test]
    fn layout_profile_refuses_clock_reads_and_unseeded_randomness() {
        // Each refusal is one of the layout guards: the clock, an unseeded `randomseed`, and
        // a format width above the limit. Pattern functions are served by `strlib` and work.
        let lua = lua(Profile::Layout);
        for call in [
            "os.time()",
            "os.date()",
            "math.randomseed()",
            "string.format('%1000d', 1)",
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
        assert!(
            guard.aborted(),
            "the comparator loop must latch the budget ({} hook calls)",
            guard.hook_calls().len()
        );
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
        let layout = lua(Profile::Layout);
        assert!(layout.load("return os.clock == nil").eval::<bool>().expect("reads"));
        assert!(
            layout.load("return os.time()").exec().is_err(),
            "a layout must not read the clock"
        );
        assert!(
            layout.load("return os.date('%Y')").exec().is_err(),
            "a layout must not read the clock"
        );
        assert!(
            layout
                .load("return os.time({year = 2000, month = 1, day = 1, hour = 12})")
                .eval::<i64>()
                .is_ok()
        );
        assert!(layout.load("return os.date('!%Y', 0)").eval::<String>().is_ok());
        let config = lua(Profile::Config);
        assert!(
            config
                .load("return type(os.time()) == 'number' and type(os.clock()) == 'number'")
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
        let lua = create(Profile::Layout, &limits, Rc::clone(&guard), quiet()).expect("the sandbox builds");
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
        assert!(aborted, "the budget must latch");
    }

    /// Run `first` and then `second` on one state, re-arming the guard between them, on a
    /// worker thread with a five-second cap. Returns the second entry's string result (or
    /// its error text), whether the first entry aborted, and the guard's final abort flag.
    fn two_entries(profile: Profile, limits: Limits, first: &'static str, second: &'static str) -> (String, bool, u64) {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let guard = Rc::new(Guard::new());
            guard.begin(&limits);
            let lua = create(profile, &limits, Rc::clone(&guard), quiet()).expect("the sandbox builds");
            let _ = lua.load(first).exec();
            guard.begin(&limits);
            let result = match lua.load(second).eval::<String>() {
                Ok(value) => value,
                Err(error) => error.to_string(),
            };
            let calls: u64 = guard.hook_calls().iter().map(|(_, count)| count).sum();
            let _ = sender.send((result, guard.aborted(), calls));
        });
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the two entries must end within five seconds; a hang means the hook missed its thread")
    }

    #[test]
    fn layout_coroutine_created_before_exhaustion_is_bounded_when_resumed_after() {
        // Created in entry one, which then burns its budget through pcall. Entry two re-arms
        // and resumes the old thread, which must end with the budget error, not success.
        let (result, aborted, calls) = two_entries(
            Profile::Layout,
            small_budget(),
            "co = coroutine.create(function() while true do end end) while true do pcall(function() while true do end end) end",
            "local ok = coroutine.resume(co) if ok then error('resume succeeded') end return 'resume failed'",
        );
        assert_eq!(result, "resume failed", "{calls} hook calls");
        assert!(aborted, "{calls} hook calls");
    }

    #[test]
    fn layout_coroutine_resumed_in_a_later_entry_is_bounded_by_that_entry() {
        // Yields in entry one, then loops forever after the yield. Entry two's budget must
        // bound the inner loop, so the resume returns the budget error as its message.
        let (result, aborted, calls) = two_entries(
            Profile::Layout,
            small_budget(),
            "co = coroutine.create(function() coroutine.yield() while true do end end) coroutine.resume(co)",
            "local ok = coroutine.resume(co) if ok then error('resume succeeded') end return 'resume failed'",
        );
        assert_eq!(result, "resume failed", "{calls} hook calls");
        assert!(aborted, "{calls} hook calls");
    }

    #[test]
    fn layout_coroutine_nested_three_deep_inside_pcall_is_bounded() {
        let outcome = capped(
            layout_state,
            small_budget(),
            "local a = coroutine.wrap(function() pcall(coroutine.wrap(function() coroutine.wrap(function() while true do end end)() end)) end) a()",
        );
        assert!(
            outcome.error.contains("instruction budget") || outcome.error.contains("handler deadline"),
            "{} ({} hook calls on {} threads)",
            outcome.error,
            outcome.hook_calls,
            outcome.hook_threads
        );
        assert!(outcome.aborted, "{} hook calls", outcome.hook_calls);
    }

    #[test]
    fn layout_wrap_iterator_with_an_infinite_generator_is_bounded() {
        let outcome = capped(
            layout_state,
            small_budget(),
            "for v in coroutine.wrap(function() while true do coroutine.yield(1) end end) do end",
        );
        assert!(
            outcome.error.contains("instruction budget") || outcome.error.contains("handler deadline"),
            "{} ({} hook calls on {} threads)",
            outcome.error,
            outcome.hook_calls,
            outcome.hook_threads
        );
        assert!(outcome.aborted, "{} hook calls", outcome.hook_calls);
    }

    /// Runs the close-handler probe under `build`. The handler sets a global flag first and then
    /// loops forever. The entry must end with the budget error, and the flag must be set, which
    /// proves the handler ran and the budget cut it. A missing flag means the close path skipped
    /// the handler. Both profiles run it, because Layout now keeps `setmetatable`.
    fn close_handler_is_bounded(build: fn(&Limits) -> (Lua, Rc<Guard>)) {
        let script = "handler_started = false \
            local co = coroutine.create(function() \
                local c <close> = setmetatable({}, {__close = function() \
                    handler_started = true \
                    while true do end \
                end}) \
                coroutine.yield() \
            end) \
            coroutine.resume(co) \
            local ok, err = coroutine.close(co) return tostring(ok) .. '|' .. tostring(err)";
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let limits = small_budget();
            let (lua, guard) = build(&limits);
            let error = match lua.load(script).eval::<String>() {
                Ok(text) => text,
                Err(error) => error.to_string(),
            };
            let started = lua.globals().get::<bool>("handler_started").unwrap_or(false);
            let calls: u64 = guard.hook_calls().iter().map(|(_, count)| count).sum();
            let _ = sender.send((error, started, guard.aborted(), calls));
        });
        let (error, started, aborted, calls) = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the close script must end within five seconds");
        assert!(
            started,
            "coroutine.close did not run the pending __close handler (error: {error}, {calls} hook calls)"
        );
        assert!(
            error.contains("instruction budget") || error.contains("handler deadline"),
            "the entry must end with the budget error, got: {error} ({calls} hook calls)"
        );
        assert!(aborted, "the budget must latch ({calls} hook calls)");
    }

    #[test]
    fn coroutine_close_runs_an_endless_close_handler_and_the_budget_cuts_it() {
        // Config keeps the full base library, so this is the plain close path.
        close_handler_is_bounded(coroutine_state);
    }

    #[test]
    fn layout_coroutine_close_runs_an_endless_close_handler_and_the_budget_cuts_it() {
        close_handler_is_bounded(layout_state);
    }

    /// Runs `script` under the Layout profile with the small budget and asserts the budget cut it.
    /// Every hostile metamethod must end this way, and the hook counts go into the message.
    fn layout_metamethod_is_bounded(script: &'static str) {
        let outcome = capped(layout_state, small_budget(), script);
        assert!(
            outcome.error.contains("instruction budget") || outcome.error.contains("handler deadline"),
            "{} ({} hook calls on {} threads)",
            outcome.error,
            outcome.hook_calls,
            outcome.hook_threads
        );
        assert!(outcome.aborted, "{} hook calls", outcome.hook_calls);
    }

    #[test]
    fn layout_looping_add_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__add = function() while true do end end}) return o + 1",
        );
    }

    #[test]
    fn layout_looping_index_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__index = function() while true do end end}) return o.x",
        );
    }

    #[test]
    fn layout_looping_newindex_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__newindex = function() while true do end end}) o.x = 1",
        );
    }

    #[test]
    fn layout_looping_eq_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local mt = {__eq = function() while true do end end} local a = setmetatable({}, mt) local b = setmetatable({}, mt) return a == b",
        );
    }

    #[test]
    fn layout_looping_lt_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local mt = {__lt = function() while true do end end} local a = setmetatable({}, mt) local b = setmetatable({}, mt) return a < b",
        );
    }

    #[test]
    fn layout_looping_concat_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__concat = function() while true do end end}) return o .. ''",
        );
    }

    #[test]
    fn layout_looping_len_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__len = function() while true do end end}) return #o",
        );
    }

    #[test]
    fn layout_looping_call_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__call = function() while true do end end}) return o()",
        );
    }

    #[test]
    fn layout_looping_tostring_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "local o = setmetatable({}, {__tostring = function() while true do end end}) return tostring(o)",
        );
    }

    #[test]
    fn layout_looping_close_metamethod_is_bounded() {
        layout_metamethod_is_bounded(
            "do local c <close> = setmetatable({}, {__close = function() while true do end end}) end",
        );
    }

    #[test]
    fn layout_refuses_gc_so_a_looping_finaliser_cannot_run_later() {
        // Withheld on purpose: see `restrict_layout`. The entry must fail at `setmetatable`,
        // not finish, and must not reach the budget at all.
        let outcome = capped(
            layout_state,
            small_budget(),
            "setmetatable({}, {__gc = function() while true do end end})",
        );
        assert!(outcome.error.contains("__gc is not supported"), "{}", outcome.error);
        assert!(
            !outcome.aborted,
            "the refusal must not need the budget ({} hook calls)",
            outcome.hook_calls
        );
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
        let lua = create(profile, limits, Rc::clone(&guard), quiet()).expect("the sandbox builds");
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
        assert!(
            guard.aborted(),
            "the loop must latch the budget ({} hook calls)",
            guard.hook_calls().len()
        );
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
