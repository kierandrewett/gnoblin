# JS runtime: product budgets

The wall-clock limits on one call into a script, and the measurements that set them. Part of the [JS/TS runtime
design](runtime.md).

## Product budgets

Lane SmallSafety. The call budgets of `crates/gnoblin-js/src/budget.rs` bound a script that hangs the shell. They are
wall-clock limits, so a machine that does not schedule the thread trips them as a loop does, and three strikes in 30 s
stop the generation (`StrikePolicy` of `gnoblin-shell-host`).

The product used to carry 5 s to load a module, 100 ms per listener call and 500 ms per frame; they are now named
constants, `STAGE_BUDGET` (30 s), `ENTRY_BUDGET` (1 s) and `FRAME_BUDGET` (5 s), and `JsConfig::new` and
`JsShellConfig::new` take their defaults from them.

What they guard was measured by `crates/gnoblin-js/tests/budgets.rs` (ignored; read the numbers, not a pass), release
build, the stock desktop with 40 windows and 300 frames of title changes and new windows, on a 4-CPU Agent37 instance.
"2 busy" and "4 busy" are busy-loop threads per CPU, which is a build running beside the desktop.

| Run                     | Library install | Load the starter | Load the stock default | Frame p50 / p99 / max   |
| ----------------------- | --------------- | ---------------- | ---------------------- | ----------------------- |
| Idle                    | 0.12 s          | 12 ms            | 14 ms                  | 9 / 44 / 71 ms          |
| 2 busy, the old budgets | 7.6 s           | 1.7 s            | not measured           | 39 ms / 0.92 s / 1.25 s |
| 2 busy, the new budgets | 7.2 s           | 2.0 s            | 1.2 s                  | 31 ms / 0.84 s / 1.15 s |
| 4 busy, the new budgets | 12.7 s          | 2.2 s            | 2.7 s                  | 61 ms / 1.3 s / 1.5 s   |

- **With the old budgets, two busy threads per CPU tripped the frame budget 7 times in 300 frames** (the UI pass was
  interrupted at 500 ms), which is more than the three strikes in 30 s that stop a generation. A debug build tripped it
  26 times. Idle, nothing came near a limit: the work of a frame is milliseconds, and only the scheduler stretches it.
- **Loading the stock default config was under the listener-call budget by mistake.** It evaluates the whole starter,
  as loading a user's `init.ts` does, and took 1.2 to 2.7 s under load against a 100 ms limit. It now runs under the stage
  budget (`load_default_config`; `the_stock_default_config_is_a_load_and_runs_under_the_stage_budget`).
- **The new values leave room.** With them no budget fired at two busy threads per CPU in release or in debug, or at
  four in release, so no listener call reached 1 s and no frame call reached 5 s.

    The worst whole frame was 1.5 s at four busy threads per CPU, so 5 s is more than three times that, and 30 s is more
    than twice the 12.7 s the library took to install at four.

    The listener-call limit is below the 2 s lease the lock screen holds on the shell (`LOCK_UI_LEASE_MS`), so the shell
    is not stuck in one listener while the compositor takes the lock screen back.

- **Not changed.** `HOST_INSTALL_BUDGET` (the 5 s floor on installing the bundled library, which only matters when a
  test shortens `stage_budget`), the dry run of `gnoblinctl check` (the launcher's `check` role, which runs the shell under the runtime's own stage budget: it is a command and
  not the session; under load a check can run out of it) and the honest-test budgets above.

## CPU time, and a wall-clock backstop

Lane Hardening. The numbers above were wall time, and the measurements show what that costs: two busy threads per CPU
stretched a frame from 71 ms to 1.25 s, and a limit on wall time trips on a script that did nothing wrong. A limit
that was meant to catch a hang should not count the time the scheduler took away.

`JsBudget` (`crates/gnoblin-js/src/generation/mod.rs`) therefore arms two deadlines for each call, and the engine's
interrupt handler stops the call at whichever comes first:

- the thread's own CPU time (`clock_gettime(CLOCK_THREAD_CPUTIME_ID)`, through `rustix`) reaching the call's budget;
- the wall clock reaching `WALL_BACKSTOP_FACTOR` (8) times the budget, so a call that is held up and never computes
  still ends. A spinning script reaches the CPU limit first.

A generation lives on one thread, so the handler reads the clock of the thread that armed the deadline.

The stop is deterministic where the old one was not. `time_the_script_spends_waiting_for_the_scheduler_is_not_charged_to_its_budget`
runs a script that stalls for 1.2 s of wall time with a few milliseconds of work against a 500 ms budget and expects it
to finish (it tripped before the change); `a_script_stalled_for_longer_than_the_wall_clock_backstop_is_still_stopped`
shows the backstop; `a_trip_reports_the_limit_the_time_used_and_where_the_script_was` pins what a trip says: the limit,
the CPU and wall time the call used, and the innermost script frames. No test asserts how fast the machine is.

The values of `STAGE_BUDGET`, `ENTRY_BUDGET` and `FRAME_BUDGET` are unchanged: they were set from wall measurements, and
CPU time is never more than wall time. They stay settable per generation (`JsConfig::stage_budget`,
`JsShellConfig::entry_budget` and `frame_budget`). The "frame budget is used up" diagnostic of the queued listener
calls still compares wall time, because it is about how long the frame took, not about a hang.
