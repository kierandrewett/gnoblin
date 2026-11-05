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
//!   `math`, `utf8` and the base library. `io`, `os`, `debug`, `package` and
//!   `coroutine` are not opened, and the base functions that read files (`dofile`,
//!   `loadfile`) and the loader (`require`) are cleared.
//! - [`Profile::Layout`] is the pure subset a layout runs under (D17). The base
//!   library is cut to an allowlist, `string` to `len`, `sub`, `byte`, `char` and a
//!   `format` that refuses a width or precision above 99, `table` without `sort`,
//!   and `math` without `random`. `string.find`, `match`, `gmatch`, `gsub` and `rep`
//!   are absent because they can run without bound inside C, where the hook cannot
//!   reach them. `pcall`, `coroutine`, `load` and `collectgarbage` are absent so the
//!   limits cannot be caught or bypassed.
//!
//! Binary Lua bytecode is never loaded. In the configuration profile `load` is
//! wrapped so its mode is always text, because crafted bytecode can crash the
//! interpreter; the layout profile has no `load` at all.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Error, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState};

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
}

/// Create a sandboxed Lua state for `profile` under `limits`.
///
/// The returned state has no file, process or clock access for either profile.
/// The caller arms `guard` with [`Guard::begin`] before each entry.
pub fn create(profile: Profile, limits: &Limits, guard: Rc<Guard>) -> Result<Lua, Error> {
    let libraries = match profile {
        Profile::Config => StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH,
        Profile::Layout => StdLib::TABLE | StdLib::STRING | StdLib::MATH,
    };
    let lua = Lua::new_with(libraries, LuaOptions::default())?;
    lua.set_memory_limit(limits.memory_bytes)?;
    match profile {
        Profile::Config => clear_config(&lua)?,
        Profile::Layout => restrict_layout(&lua)?,
    }
    install_hook(&lua, limits, guard)?;
    Ok(lua)
}

/// Clear the configuration VM's dangerous globals by name.
///
/// The base library is always open, and it holds the functions that reach the
/// filesystem. Clearing them by name is what makes the sandbox one list a reviewer
/// can read.
fn clear_config(lua: &Lua) -> Result<(), Error> {
    let globals = lua.globals();
    for name in [
        "dofile",
        "loadfile",
        "require",
        "os",
        "io",
        "debug",
        "package",
        "coroutine",
    ] {
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
        "assert", "error", "ipairs", "next", "pairs", "select", "tonumber", "tostring", "type", "math", "string",
        "table",
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
    table.set("sort", Value::Nil)?;
    let math: Table = globals.get("math")?;
    math.set("random", Value::Nil)?;
    math.set("randomseed", Value::Nil)?;
    Ok(())
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

/// Install the count hook that enforces the instruction and deadline budgets.
fn install_hook(lua: &Lua, limits: &Limits, guard: Rc<Guard>) -> Result<(), Error> {
    // The closure outlives this call, so the numbers it names are copied in.
    let budget = limits.instructions;
    let deadline_ms = limits.handler_ticks;
    let deadline = Duration::from_millis(deadline_ms);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTIONS),
        move |_, _| {
            if guard.aborted.get() {
                return Err(Error::runtime(
                    "the Lua handler was already aborted by a limit in this entry",
                ));
            }
            let remaining = guard.remaining.get();
            if remaining <= u64::from(HOOK_INSTRUCTIONS) {
                guard.remaining.set(0);
                guard.aborted.set(true);
                return Err(Error::runtime(format!(
                    "the Lua instruction budget ({budget} instructions) is exhausted; a handler must \
                     finish inside its budget"
                )));
            }
            guard.remaining.set(remaining - u64::from(HOOK_INSTRUCTIONS));
            if let Some(started) = guard.started.get()
                && started.elapsed() >= deadline
            {
                guard.aborted.set(true);
                return Err(Error::runtime(format!(
                    "the handler deadline ({deadline_ms} ms) passed"
                )));
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
        for name in [
            "os", "io", "debug", "package", "require", "load", "dofile", "pcall", "print", "utf8",
        ] {
            let value: Value = lua.globals().get(name).expect("a global reads");
            assert!(
                matches!(value, Value::Nil),
                "{name} must be absent in the layout profile"
            );
        }
    }

    #[test]
    fn layout_profile_refuses_pattern_functions_and_sort() {
        let lua = lua(Profile::Layout);
        for call in [
            "string.find('abc', 'b')",
            "string.match('abc', 'b')",
            "string.gsub('abc', 'b', 'c')",
            "string.rep('a', 10)",
            "table.sort({2, 1})",
            "math.random()",
            "string.dump(print)",
        ] {
            assert!(lua.load(call).exec().is_err(), "{call} must be refused");
        }
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
        for name in ["os", "io", "require", "dofile", "coroutine", "package"] {
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
