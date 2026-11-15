# JS runtime: performance

What the runtime costs when it starts and when nothing happens: the bytecode cache and change gating. Part of the [JS/TS
runtime design](js-runtime.md).

## Performance

What the runtime costs when nothing is happening and when it starts. Numbers are release builds on a 4-CPU Agent37
instance (`crates/gnoblin-js/tests/boot_cost.rs`, ignored; read the numbers, not a pass).

### The bytecode cache

Starting the shell, and every reload, used to strip the types of the 52 stock modules with oxc and compile them with
QuickJS before the user's first line ran.

`src/bytecode.rs` keeps the QuickJS bytecode of each module under `$XDG_CACHE_HOME/gnoblin/js/qjs-<toolchain>/`
(`~/.cache` without the variable), one file per module named by the SHA-256 of the toolchain, the module's name, whether
it was stripped, and its text.

The stock modules and the user's `init.ts` and imports are cached the same way, so there is nothing to invalidate by
hand: changed text is a new file.

- **The key.** The toolchain is the `rquickjs-sys` (QuickJS) and oxc versions read from `Cargo.lock` by `build.rs`,
  and this crate's version. A directory is never read by a build that differs in any of them, and the other toolchains'
  directories are removed when a new one is first written. A build that cannot read its lock file does not cache.
- **Never trusted blindly.** A file carries its payload length and SHA-256, is written under a temporary name and
  renamed, and is created 0600 in a 0700 directory.

    A file that is short, damaged, from another format or refused by QuickJS is compiled from source, removed and written
    again; the cache can only make a start faster. QuickJS does not verify bytecode, so the checksum is what makes the one
    `unsafe` call (`engine::load`) sound, and the crate's `unsafe_code` is `deny` instead of `forbid` for that module
    alone.

- **Source maps.** A module loaded from bytecode was not stripped in this process, so its source map is made from its
  text the first time a stack names it (`MapSlot::Pending`); an error in a cached `init.ts` still reports its `.ts`
  line (`an_error_in_a_cached_config_is_reported_at_its_ts_line`).
- **Size.** At most `CACHE_MAX_ENTRIES` (512) files; the oldest go first.
- **Knobs.** `GNOBLIN_JS_CACHE=0` turns it off. Delete the directory to clear it. The library is embedded as TypeScript
  and stripped only for a module that misses, so a start that hits everything never runs oxc.

| Boot of the stock library and an `init.ts` that imports it | min     | median  |
| ---------------------------------------------------------- | ------- | ------- |
| Before the cache (rig boot)                                | 35.6 ms | 36.2 ms |
| No cache (this build)                                      | 36.9 ms | 39.3 ms |
| Cold: an empty cache, one file written per module          | 48.3 ms | 51.8 ms |
| Warm: every module from bytecode                           | 5.8 ms  | 6.4 ms  |

### Change gating

The runtime role takes a turn every 16 ms and each offered the shell a frame, which cost time proportional to the
windows whether or not anything had happened. `crates/gnoblin-js/src/shell/gate.rs` now runs a frame only when
something can have changed what it does:

- **The compositor sent something** on the tree link, whatever it was.
- **A link beside the tree has something** (a process ended, a session call or signal, a dialog, the idle state). A quiet
  turn reads these links and keeps what they said for the frame it then runs.
- **Script ran outside a frame** (`eval`, the console, a load, a reload, a link handed over).
- **The script asked** through `entry.wake()` (`data/js/gnoblin/wake.ts`): the part of the library that has a clock to
  keep registers it where it holds its state. Timers, focus rests, launch deadlines, the not-responding alert, a window
  drag's limit, a titlebar's close fade, the display manager's dirty flag, queued listener calls, and the sources that
  draw every frame while they hold (a lock screen that is up, an XR session, a toast).
- **The last frame did work**, so one more looks for what it left for the pass after it.

`JsShell::next_wake` says how long until the next timer, and the role waits no longer than that for the compositor.

`tests/frame_gating.rs` asserts that an idle shell runs no frame at 0, 25 and 100 windows. A table in it asserts that
each kind of change wakes the very next frame:

- a window moved, a title, a window created or destroyed;
- an output resized, a pointer edge, the end of an animation;
- the seat's focus, the lock's state;
- script outside a frame, a property written from script.

A timer is asserted to wake at its deadline and not before.

| Windows | Idle frame before | Idle frame after | Title change before | Title change after |
| ------- | ----------------- | ---------------- | ------------------- | ------------------ |
| 0       | 103 us            | 0.1 us           | not measured        | not measured       |
| 25      | 150 us            | 0.0 us           | 160 us              | 178 us             |
| 100     | 423 us            | 0.0 us           | 430 us              | 475 us             |
| 300     | 1213 us           | 0.0 us           | 1200 us             | 1330 us            |

Release, 4-CPU Agent37 instance, `tests/frame_cost.rs`, 120 idle frames and 60 frames with a title change, over the fake
link. An idle turn also reads the process, session, dialog and idle links (a few non-blocking reads), which the fake
link does not do.

The compositor side already drew nothing when idle (`an_unchanged_scene_draws_nothing`); its whole iteration of a
300-window scene costs 13 us idle (`idle_cost_tests.rs`), so it is not gated further. A frame that does work costs what
it did, plus one frame after it.
