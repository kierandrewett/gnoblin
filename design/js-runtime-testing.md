# JS runtime: tests against a real compositor and test timing

How the runtime is tested against a real compositor session, and the rule that no assertion rests on how fast a machine
is. Part of the [JS/TS runtime design](js-runtime.md).

## Real-compositor tests

`crates/gnoblin/tests/js_shell.rs` runs the shell against a headless compositor in the test's own process, and
`crates/gnoblin/tests/support/mod.rs` holds what that file and `js_guardian.rs` share.

The tests of a file run in parallel, one compositor each, so nothing is shared between them: every session, config
directory and file lives in a temporary directory of the test's own, and no test reads the machine's runtime directory
or config.

- **`Session`** is `Compositor::run` on a thread, with its IPC, Wayland and EIS sockets in a private directory and an
  input tap that feeds the seat what a device would send. Its liveness pings are off, because a test thread that a busy
  machine did not schedule is not a hung application. The one test about the not-responding alert turns them on.
- **`Shell`** is a `JsRole` ticking in a thread of its own, over a config directory of the test's. `Shell::start`
  returns once the first frame is over.

    What `generation()`, `diagnostics()`, `view()` and `probe()` read is the snapshot the thread published when its last
    frame ended.

    `probe()` is what `globalThis.probe()` returned in the test's `init.ts`, for a state that the compositor's tree does
    not show.

- **The scene** is read back through an observer connection (`scene`, `rect_of`, and `Watcher`, which stays attached and
  follows the changes), so a test asserts what the compositor holds and not what the shell believes.
- **`Mapper`** is a real Wayland client that maps toplevels with a buffer and answers pings. `Mapper::pointer()` says
  what its pointer was told: whether the compositor has the pointer over one of its windows, and how many presses it was
  given.

    A press that a claimed chord took for the shell is never given to the client, which is how a test tells a claimed
    press from one that came before the claim. `Mapper::keyboard()` says the same of the keyboard: the key presses the
    compositor gave the client.

    It withholds the last key of a claimed chord, so a key the client heard is one nobody holds, which is how a test tells
    a released claim from one that is still held.

    `Session::start_logged()` keeps what the compositor logs (every event, so only a test that reads the log asks for it)
    and `Session::finish()` returns it, for a test that asserts a refused claim never logged.

### Wait on state, with a deadline

A test never sleeps for a time it guessed, and never counts frames to decide that something happened. It waits for the
state it needs with `wait_for` (the shell and the scene) or `wait_for_scene` (a `Watcher`), bounded by `PATIENCE`.

A run that passes never waits near it: it is the margin a loaded machine gets, so that a failure is a state that never
came and not a slow turn.

The failure names what it waited for and the last state it saw: the shell's diagnostics and view, the compositor's scene
and what the shell logged.

| The test needs                         | It waits for                                                            |
| -------------------------------------- | ----------------------------------------------------------------------- |
| A window placed                        | the window's rectangle in the scene                                     |
| A reload                               | `generation()` above the one the shell had, and the effect, in one wait |
| A refusal or an error                  | `diagnostics()` to hold the text                                        |
| A press the shell must hear            | `Mapper::pointer().over()`, then one click                              |
| A claimed pointer chord                | the window to follow, or the press to reach the client (`begin_drag`)   |
| The compositor to take a shell's close | the fallback policy's placements                                        |
| A state only the script knows          | `Shell::probe()`                                                        |
| A dialog request judged                | the compositor to answer it with something other than `busy`            |

The rules behind the table:

- **A snapshot is behind the compositor.** The effects of a frame reach the compositor while the frame runs, and the
  shell publishes its snapshot when the frame is over. A test that has seen an effect in the scene and then reads
  `generation()` may read the snapshot from before the frame that made it, so the generation is waited for with the effect.
- **Budgets are wall-clock.** The shell stops a call, a frame or a staging that outlives its budget, and a loaded machine
  that does not schedule the thread spends it on an honest script. `Shell::start` gives every budget `HONEST_BUDGET`. Only
  a test about a runaway script sets its own (`RUNAWAY_CALL`, `RUNAWAY_STAGE`), and keeps them far above what an honest
  staging takes.
- **Save a config file the way an editor does.** `save` writes under a name the watcher ignores and renames over the file,
  so a reload reads the old text or all of the new. A save of two files is two events: when a reload between them must
  not read a broken `init.ts` with new places, save `init.ts` first.
- **Do not hammer a booting compositor.** The shell waits a few seconds for its attach, and a compositor that is busy with
  a test's requests answers it late. A test asks nothing of the dialogs before `Shell::start` has returned.
- **A check that something did not happen waits for what must have happened first**, such as the next window placed or the
  reload's generation, and then looks. Where there is no such state it lets the client answer for a short span (`idle`).

A change to these tests is proven with `cargo test -p gnoblin` run several times in a row, once while two busy loops per
CPU run beside it, and once with `--test-threads=1`.

## Unit test timing

The tests of `crates/gnoblin-js` (`tests/**` and the `#[cfg(test)]` modules of `src/`) run on the same busy machines,
and they follow the rule of the section above: no assertion may rest on a wall-clock assumption under one second, and
nothing sleeps for a time it guessed.

A test that passes alone and fails one run in twenty hides the regression it would have caught, so this was measured,
not assumed: `cargo test -p gnoblin-js` ten times in a row passes, and four times while two busy loops per CPU run
beside it passes. Both are the proof a change to these tests needs.

- **Time is injected.** `JsShellConfig::clock` is the one clock the scripts read: `gnoblin.timers.now`, and through it
  every timer, `after`, fade, slide, alert arming window, launch deadline and focus delay.

    It is a `Clock` (`gnoblin-shell-host`): `SystemClock` in the product, and in a test a `ManualClock`, which stands at
    zero until the test calls `advance`.

    The rigs of `tests/common` build one (`timed_config`, `Rig::advance`), so a test of a slide or an arming window says
    how much time passed, and no real time does.

- **An honest script gets `HONEST_BUDGET`, 5 s,** for each call and 10 s for a frame (`honest_config`, and
  `honest_role_config` for a role). The product's budgets (`src/budget.rs`: 1 s per listener call, 5 s per frame, 30 s
  per module load) are a limit on a script that hangs the shell, and are set above what a loaded machine needs; see
  ["Product budgets"](js-runtime-budgets.md#product-budgets) for the measurements.
- **A test about a budget runs a script that never ends,** under a short budget (`RUNAWAY_BUDGET`, 500 ms: not shorter,
  because the honest calls the same budget covers must not be able to trip it), and asserts that the script was
  interrupted, never when.

    A script that must outlast a frame budget outlasts it by a factor of two, and the frame budget is a few hundred
    milliseconds so that no other frame of the test comes near it. Where a loop could hang the run if the interrupt
    failed, the loop ends by itself after 30 s.

- **The shell installs its bundled library under at least 5 s** (`HOST_INSTALL_BUDGET`), whatever `stage_budget` says, so
  a stage budget shortened to trip a runaway entry does not fail the boot.
- **Real time stays only where the code under test reads the real clock:** a file watcher's debounce (one second in the
  tests: the burst is written in microseconds, and a stall longer than the debounce would split it) and a spawned
  process's own timeout. Those tests wait for a state, with a deadline of seconds.
