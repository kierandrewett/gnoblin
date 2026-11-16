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
- **The script asked** through `entry.wake()` (`data/js/gnoblin-core/wake.ts`): the part of the library that has a clock to
  keep registers it, by name, where it holds its state. Timers, focus rests, launch deadlines, a window drag's limit, a
  window's close fade, a clock widget's minute, the display manager's dirty flag, queued listener calls, a look the
  not-responding alert skipped, and the sources that draw every frame while they hold (a lock screen that is up, an XR
  session, a toast). A source that throws is reported and asks for nothing, so one part's failure does not end the shell.
- **The last frame did work**, so one more looks for what it left for the pass after it.

`JsShell::next_wake` says how long until the next timer, and the role waits no longer than that for the compositor.

| Windows | Idle frame before | Idle frame after | Title change before | Title change after |
| ------- | ----------------- | ---------------- | ------------------- | ------------------ |
| 0       | 103 us            | 0.1 us           | not measured        | not measured       |
| 25      | 150 us            | 0.0 us           | 160 us              | 178 us             |
| 100     | 423 us            | 0.0 us           | 430 us              | 475 us             |
| 300     | 1213 us           | 0.0 us           | 1200 us             | 1330 us            |

Release, 4-CPU Agent37 instance, `tests/frame_cost.rs`, 120 idle frames and 60 frames with a title change, over the fake
link, which answers a poll at once. The compositor side already drew nothing when idle
(`an_unchanged_scene_draws_nothing`); its whole iteration of a 300-window scene costs 13 us idle (`idle_cost_tests.rs`),
so it is not gated further. `idle_frame_cost` asserts the idle frame at 300 windows stays under 300 us. The bytecode cache is on in the role (`JsBytecodeCache::from_environment`, `GNOBLIN_JS_CACHE=0` turns it off). A frame that does work costs what it did, plus one frame after it.

#### What proves it

A change that wakes no frame is never drawn. Each way in has a test that lets the shell go quiet, makes one thing
happen, and asserts that the very next frame ran and that the script heard it in that frame.

- **The tree link** (`tests/frame_gating.rs`): no frame runs at 0, 25 and 100 windows; a table of changes; one message
  of each kind the link carries, where the match that numbers the kinds has no wildcard, so a new kind stops the build
  until it has a row; a seeded random order of changes after random rests; a timer wakes at its deadline and not before;
  a window drag ends on its limit with the pointer still.
- **The links beside it**, with their fakes. A process that ends or outlives its timeout, a session call, a signal and
  a lost connection (`host_api.rs`). A dialog offer, the end of a request and a lost slot (`config_slots.rs`). An idle
  reading, a hold given back and a lost connection (`host_idle.rs`).
- **The parts that draw every frame**: the lock screen up and down, the lock's state and a password answer (`lock.rs`),
  and an XR session (`xr.rs`).
- **What has to register** (`tests/wake_sources.rs`): every field of `HostContext` is classified as state, as a part
  that wakes, or as driven by a message with the reason. The sources the stock shell registers are the expected ones, in
  both directions. Every file of the library that reads a clock registers a source or says why it needs none. The stock
  shell, with titlebars, the window menu and the alert, comes to rest.
- **The gate's own source** (`gate.rs`): a link handed to the shell that a quiet turn does not read, and a public way
  into script that does not mark the gate, each fail a unit test.
- **Real links** (`crates/gnoblin/tests/js_shell.rs`): a role over a real compositor session with five windows and the
  stock config runs no frame in two seconds of turns at the loop's pace.

#### What the proofs found

- A window drag past `REQUEST_DRAG_LIMIT_MS` recursed without end and reported a stack overflow every frame: the end
  asked for the last position, which asked the end for the limit again. The limit was also `>` where its wake was at the
  limit, so a frame woke and did nothing. Fixed in `window_state.ts`.
- The not-responding alert asked for a frame every 250 ms for ever, which is the polling gating exists to remove. It
  now asks only for a look that a frame skipped for being too soon after the last, so a burst of activity leaves one
  trailing frame 250 ms later and then the shell rests.
- A `Clock` widget redrew only on frames something else caused, so on a quiet desktop it stopped at the minute it was
  made. It now asks for a frame when its minute ends, or its second for a format that shows seconds.
- A wake source that threw ended the shell, because the shell asks at the end of every frame. A throw is now a report.

#### What a turn costs over the real links

| Windows | Idle connection | Frames run in 3 s | Turn, wall clock, before | Turn, wall clock, now | Turn, thread CPU, now |
| ------- | --------------- | ----------------- | ------------------------ | --------------------- | --------------------- |
| 0       | no              | 0                 | 2542 us                  | 10.1 us               | 10 us                 |
| 25      | no              | 0                 | 2489 us                  | 12.9 us               | 15 us                 |
| 100     | no              | 0                 | 2448 us                  | 12.6 us               | 15 us                 |
| 300     | no              | 0                 | 2537 us                  | 13.8 us               | 15 us                 |
| 100     | open            | 6 (4 before)      | 3676 us                  | 21.3 us               | 20 us                 |

Release, 4-CPU Agent37 instance, `idle_turn_cost_over_the_real_links` (ignored): a real compositor session with the
stock config, 3 s of turns at the loop's pace to count the frames, then the mean of 2000 back-to-back role ticks. "Before"
is the tip `87590e21c` and "now" is the same test with the change below, on the same instance. The open idle connection is
a script's `gnoblin.idle.watch`. The frames came with it in both runs: the same session without the watch ran none, and
what the watch adds to the shell is the idle readings the connection delivers, so the count follows the compositor's
idle clock and not this change.

The cost did not depend on the windows, so gating did what it says: no frame ran. What was left was not the shell but the
reads. The tree link, the slot link and an open idle connection each waited up to 1 ms for bytes on every poll
(`READ_WAIT` in `gnoblin-tree-client/src/tree_link.rs` and `slot_link.rs`, and in `host_idle.rs`), which is the 2.5 ms of
wall time, and the third millisecond when an idle watch is open.

#### One wait over the links

- **A read is made only when there is something to read.** `gnoblin_client::readable_now` asks the kernel with a zero
  timeout, so a quiet link costs one system call and no wait. The tree link asks before each read of its socket, and the
  links that hold a `Client` (slots, idle) ask `Client::has_input`, which also counts an event the client has already
  queued and the bytes its reader has already taken from the socket. `READ_WAIT` is now only the backstop for the rest
  of a frame that arrived in pieces: the bytes read so far are kept, as before, and the next turn goes on from them.
- **The loop sleeps in one `poll(2)`.** `JsRoleRunner::run` waits in `JsRole::wait_for_input`, which polls the tree
  link, the slot link, the idle link while it is open and the role's own connection to the compositor together. It
  ends when any of them has bytes, at the shell's next wake (`JsShell::next_wake`), or after `FRAME_WAIT` (16 ms), which
  is the longest a turn sleeps because four things are not watched by the wait: the config watcher, the console
  socket, the session thread's events and the guardian's stop flag. A frame that is due now still waits `MIN_WAIT`
  (1 ms) so a script that keeps asking for one is not spun on, and a bytes-ready link ends even that wait. A link that
  has bytes now wakes a turn at once, where before the turn saw them up to 16 ms late when the loop was sleeping on
  another connection.
- **A link with no descriptor is unchanged.** `wait_fd` defaults to `None`, so the in-memory fakes of the tests read as
  they did.

What proves it: `a_quiet_turn_over_the_real_links_does_not_wait_for_bytes` (the fastest of 300 quiet turns over real
links is under a millisecond, which the old wait made impossible; it is the minimum and not the mean so a loaded machine
cannot fail it, which three runs under eight busy loops on four CPUs showed a mean does),
`a_wait_for_input_ends_at_its_deadline_when_quiet_and_when_the_compositor_sends_something`, and the unit tests of the
readiness check (`gnoblin-client/src/ready.rs`, `unix.rs`) and of a quiet tree link
(`a_poll_of_a_quiet_link_returns_at_once_whatever_the_read_timeout`). `frame_gating`, `wake_sources` and `host_idle`
pass unchanged.

The thread-CPU column was an upper bound before: Agent37 runs gVisor, which charges the thread for part of a timed
read. With no timed read left it measures the turn itself.
