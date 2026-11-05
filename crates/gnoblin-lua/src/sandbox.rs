//! The Lua state and its limits.
//!
//! The runtime embeds a VM that runs user configuration and user handlers, so the
//! sandbox is built here once and stated in one place rather than visited by
//! whichever function happens to touch Lua next (design/rust-compositor.md, D1:
//! scripts run in a separate process with instruction and memory limits).
//!
//! Three limits bound every entry into Lua:
//!
//! - **Instructions.** A count hook stops an infinite loop. The hook is the only
//!   mechanism that can stop Lua from inside Lua, so the host arms it before every
//!   call and disarms it after: a handler that loops is aborted, and the host
//!   survives to report it.
//! - **Memory.** The VM allocates through `mlua`'s allocator, which refuses past
//!   the limit, so a script that builds a huge table fails with a memory error
//!   instead of growing the runtime process without bound.
//! - **A deadline.** The caller's clock is ticks, and a tick is a millisecond, so
//!   the deadline one entry gets is [`Limits::handler_ticks`] milliseconds of wall
//!   clock. The hook compares elapsed time with it, because a tick only advances
//!   between entries and a handler that loops would otherwise never see one. This
//!   is the deadline the plan asks for, sized by the clock the caller already
//!   keeps.
//!
//! The libraries are the safe subset only: `string`, `table`, `math`, `utf8` and
//! the base library. `io`, `os`, `debug` and `package` are not opened at all, and
//! the base functions that read files (`dofile`, `loadfile`) and the module loader
//! (`require`) are cleared, because a script's only way to a file is
//! [`crate::Host::load`], which confines it to the configuration directory.
//! `coroutine` is left out as well: the count hook is per thread, so a coroutine
//! would be a way around the instruction limit.
//!
//! Binary Lua bytecode is refused by the loader (`ChunkMode::Text`), because
//! running crafted bytecode can crash the interpreter it runs in.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gnoblin_model::Tick;
use mlua::{Error, HookTriggers, Lua, LuaOptions, StdLib, VmState};

/// How many instructions pass between two hook calls.
///
/// The hook is a C function called from the VM, so it must not run on every
/// instruction. A thousand is frequent enough that an aborted loop stops within
/// microseconds and rare enough that ordinary script code is not slowed down.
const HOOK_INSTRUCTIONS: u32 = 1000;

/// The budgets every entry into Lua runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Instructions one entry into Lua may execute.
    pub instructions: u64,
    /// The most memory the Lua allocator may hand out, in bytes.
    pub memory_bytes: usize,
    /// How long one entry into Lua may run, in ticks. A tick is a millisecond, so
    /// this is the wall-clock budget the hook enforces.
    pub handler_ticks: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            // About thirty million instructions per second on a current core, so
            // one entry gets a few milliseconds of interpreter time.
            instructions: 10_000_000,
            memory_bytes: 32 * 1024 * 1024,
            // Ticks are milliseconds and a handler should finish inside a frame.
            handler_ticks: 16,
        }
    }
}

/// Why an entry into Lua was aborted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Abort {
    /// Neither limit has been reached.
    #[default]
    None,
    /// The instruction budget ran out.
    Instructions,
    /// The tick deadline had already passed when the hook looked.
    Deadline,
}

/// The budgets one entry into Lua consumes, shared between the host and the hook.
///
/// The host owns the cells and the hook reads them, which is why this is a
/// separate type: the closure `mlua` stores cannot borrow the host, and the
/// numbers must be visible from inside the VM.
pub(super) struct Guard {
    remaining: Cell<u64>,
    started: Cell<Option<Instant>>,
    tick: Cell<Tick>,
    abort: Cell<Abort>,
}

impl Guard {
    /// A guard for a host that has not run anything yet.
    pub(super) fn new() -> Guard {
        Guard {
            remaining: Cell::new(0),
            started: Cell::new(None),
            tick: Cell::new(Tick::default()),
            abort: Cell::new(Abort::None),
        }
    }

    /// Arm the budgets for one entry into Lua.
    pub(super) fn begin(&self, limits: &Limits) {
        self.remaining.set(limits.instructions);
        self.abort.set(Abort::None);
        self.started.set(Some(Instant::now()));
    }

    /// The caller's clock, as of the last [`Guard::set_tick`].
    pub(super) fn tick(&self) -> Tick {
        self.tick.get()
    }

    /// Set the caller's clock, which the hook compares with the deadline.
    pub(super) fn set_tick(&self, tick: Tick) {
        self.tick.set(tick);
    }
}

/// Create the sandboxed Lua state for `limits`.
pub(super) fn create(limits: &Limits, guard: Rc<Guard>) -> Result<Lua, Error> {
    let libraries = StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
    let lua = Lua::new_with(libraries, LuaOptions::default())?;
    lua.set_memory_limit(limits.memory_bytes)?;

    // The base library is always open, and it holds the functions that reach the
    // filesystem. Clearing them by name, rather than trusting the library set, is
    // what makes the sandbox one list a reviewer can read.
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
        globals.set(name, mlua::Value::Nil)?;
    }

    // The closure outlives this call, so the two numbers it names are copied in
    // rather than borrowed from `limits`.
    let budget = limits.instructions;
    let deadline_ticks = limits.handler_ticks;
    let deadline = Duration::from_millis(deadline_ticks);
    let hook_guard = Rc::clone(&guard);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTIONS),
        move |_, _| {
            if hook_guard.abort.get() == Abort::Instructions {
                return Err(Error::runtime("the Lua instruction budget is exhausted"));
            }
            let remaining = hook_guard.remaining.get();
            if remaining <= u64::from(HOOK_INSTRUCTIONS) {
                hook_guard.remaining.set(0);
                hook_guard.abort.set(Abort::Instructions);
                return Err(Error::runtime(format!(
                    "the Lua instruction budget ({budget} instructions) is exhausted; a handler \
                     must finish inside its budget"
                )));
            }
            hook_guard.remaining.set(remaining - u64::from(HOOK_INSTRUCTIONS));
            if let Some(started) = hook_guard.started.get()
                && started.elapsed() >= deadline
            {
                hook_guard.abort.set(Abort::Deadline);
                return Err(Error::runtime(format!(
                    "the handler deadline ({deadline_ticks} ticks, one millisecond each) passed"
                )));
            }
            Ok(VmState::Continue)
        },
    )?;
    Ok(lua)
}
