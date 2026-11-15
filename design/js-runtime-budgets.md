# JS runtime: product budgets

The wall-clock limits on one call into a script, and the measurements that set them. Part of the [JS/TS runtime
design](js-runtime.md).

## Product budgets

Lane SmallSafety. The call budgets of `crates/gnoblin-js/src/budget.rs` bound a script that hangs the shell. They are
wall-clock limits, so a machine that does not schedule the thread trips them as a loop does, and three strikes in 30 s
stop the generation (`JsStrikePolicy`).

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
  test shortens `stage_budget`), `DOCUMENT_BUDGET` (the 1 s of the `gnoblinctl check` dry run, which is a command and not
  the session; under load a check can run out of it) and the honest-test budgets above.
