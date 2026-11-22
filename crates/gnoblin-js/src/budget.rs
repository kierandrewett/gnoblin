//! The limits on a call into a script, in one place.
//!
//! Each is a bound on a script that hangs the shell, and not a promise about latency: a call that runs past its limit
//! is interrupted, reported and counted as a strike (`JsShell::strikes`), and three strikes in 30 seconds stop the
//! generation (`StrikeSupervisor`).
//!
//! The limit is on the call's own CPU time (the calling thread's `CLOCK_THREAD_CPUTIME_ID`), so a machine that does
//! not schedule the thread, a build running beside the desktop or a loaded CI box, does not trip it: time the thread
//! spends off the processor is not charged to the script. A call that never computes but is held up for
//! [`WALL_BACKSTOP_FACTOR`] times its limit in wall time is stopped all the same, so a stall cannot run on for ever.
//! A trip says which clock stopped the call, the limit, what the call had used of each clock and the script frames it
//! was in (`JsGeneration::call_budgeted`).
//!
//! # Where the numbers come from
//!
//! `tests/budgets.rs` boots the stock desktop with 40 windows and churns 300 frames, on a 4-CPU Agent37 instance in
//! release (design/js/runtime-budgets.md, "Product budgets", has the table). Idle, a frame took 9 ms at the median and 71 ms
//! at worst. With two busy threads per CPU, which is a build running beside the desktop, the previous budgets (5 s to
//! load, 100 ms per listener call, 500 ms per frame) were exceeded 7 times in 300 frames, more than the three strikes in
//! 30 s that stop a generation, because the worst frame took 1.25 s. With the values below no budget fired at two busy
//! threads per CPU, in release or debug, or at four in release, where the worst frame took 1.5 s, loading a config
//! 2.7 s and installing the library 12.7 s.
//!
//! Those numbers are wall time, which is at least the CPU time, so the limits below hold with at least the same
//! margin on the CPU clock.
//!
//! The listener-call limit stays below the compositor's two-second lease on the lock screen (`LOCK_UI_LEASE_MS`), so
//! a listener that spins is cut off before the compositor takes the lock screen back. One that is only not scheduled
//! can outlast the lease, and the compositor takes the lock screen back then, as it does for any shell that stops
//! painting it.

use std::time::Duration;

/// Loading a module and everything it imports (`JsConfig::stage_budget`): the user's `shell.ts`, and the stock library
/// when a shell builds one. Compiling and evaluating a tree of modules is the one place a call's work grows with what
/// the user wrote, so it has the most room. Measured at four busy threads per CPU: 2.7 s to load the starter.
pub const STAGE_BUDGET: Duration = Duration::from_secs(30);

/// One listener, handler, binding or timer call (`JsShellConfig::entry_budget`). Loading the stock default config is a
/// module load and has [`STAGE_BUDGET`].
pub const ENTRY_BUDGET: Duration = Duration::from_secs(1);

/// One frame's dispatch (`JsShellConfig::frame_budget`): each of the shell's own calls in a frame (`plan`, `pointer`,
/// `input`, `apiFrame`, `layout`, `ui`) may use this much, and the queued listener calls stop starting once the frame
/// as a whole has used it, to run in the next frame. Measured at four busy threads per CPU: 1.5 s for the worst frame.
pub const FRAME_BUDGET: Duration = Duration::from_secs(5);

/// A call is also stopped when its wall-clock time reaches this many times its CPU limit, however little it has
/// computed. It is a backstop for a call that is held up and not for one that spins: a spinning call reaches the CPU
/// limit first, and a descheduled one is not charged until the machine runs it again. Eight times the call limits
/// above is 8 s for a listener call and 40 s for a frame, which is more than the worst the measurements saw
/// (1.5 s for a whole frame at four busy threads per CPU).
pub const WALL_BACKSTOP_FACTOR: u32 = 8;
