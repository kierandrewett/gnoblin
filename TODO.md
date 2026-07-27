# gnoblin TODO

Active work for the patched-GNOME direction, currently pinned to Mutter 49.5
and GNOME Shell 49.6. Completed implementation history belongs in Git; checked
items stay here only when they still form part of an active delivery sequence.

## Validate on a real system

- [ ] Select Gnoblin in GDM and complete a full login, logout, lock, and unlock
  cycle with bring-your-own chrome running.
- [ ] Install the generated Mutter, GNOME Shell, and `gnoblin-session` RPMs,
  then verify both Gnoblin and stock GNOME sessions plus the documented
  downgrade/removal path.
- [ ] Approve, restore, narrow, and revoke persistent Screen Cast and Remote
  Desktop grants through a real portal session.
- [ ] Exercise the Gnoblin Settings panel interactively, including pending and
  error states when the Shell D-Bus service disappears.

## Product work

- [ ] Add the remaining `polkit` feature toggle without weakening the stock
  session's authentication-agent ownership.
- [ ] Add an explicit extension sideload workflow instead of requiring manual
  copies into `~/.local/share/gnome-shell/extensions/`.
- [ ] Validate Kobel Shell as the reference external chrome in a real Gnoblin
  login session; keep its implementation in the separate `kobel-shell` repo.

## Deferred protocols

- [ ] Implement `ext-session-lock-v1` only with the security and real-hardware
  checks in `src/protocols/session-lock/README.md`.
- [ ] Implement `wlr-output-management-unstable-v1` only with transactional
  apply, rollback, and real-display validation from
  `src/protocols/output-management/README.md`.

## Performance

Benchmark machine baseline (headless, llvmpipe, 2026-07-18): gnoblin mode
101 MB private dirty at boot, 95 MB after 90 s idle (no growth); stock user
mode 158 MB boot, 152 MB idle. Gnoblin mode is ~36% lighter and leak-free
at idle. The live-session lag on the benchmark machine traces to 17 enabled
third-party extensions in stock GNOME, a class gnoblin removes by design.

Idle CPU (headless, 60 s window, 2026-07-19): gnoblin mode 0.08% with ~0
wakeups/s; stock user mode 0.18% with ~4 wakeups/s (clock/indicator
timers). Boot to "GNOME Shell started" ~2.2 s on llvmpipe. No idle-CPU
defect in the core; remaining CPU investigation needs a real logged-in
session. Boot profile (strace): ~60 ms total real I/O, time split 0.2 s
mutter/device setup + 0.7 s JS load + 1.0 s UI construction on llvmpipe
— compute-bound, no sync-I/O stall to fix.

- [x] Measure and bound memory growth across repeated `org.gnoblin.Shell`
  soft reloads; fix any per-reload leak. Found ~3.8 MB leaked per
  `Main.loadTheme()` swap (upstream St/GJS: replaced StTheme wrapper
  survives GC at refcount 1). Mitigated in `softReload()` via a
  stylesheet-set digest; 15 reloads now hold private dirty flat.
- [x] Root-cause the upstream `loadTheme()` theme leak. NOT a leak:
  every StTheme finalizes (gdb-verified); the freed parsed CSS stays
  resident in glibc arenas. Fixed via `Shell.util_trim_memory()`
  (patch 31-memory-trim) called from `softReload()` after real theme
  swaps: 15 swaps now end 12 MB below baseline. Diagnosis notes:
  WeakRef liveness probing is unusable in GJS (kept-objects list never
  clears); GOBJECT_DEBUG instance counts need a debug GLib.
- [ ] Offer the trim helper + loadTheme call upstream to GNOME Shell
  (same mechanism fixes stock GNOME's "shell memory only ever grows"
  for theme/stylesheet churn).
- [x] Periodic idle trim: the control component now runs
  `Shell.util_trim_memory()` every 300 s (PRIORITY_LOW, <10 ms/pass).
  Validated end-to-end: heap inflated to 288 MB held until the timer
  fired at ~t+5 min, then dropped to 220 MB and stayed.
- [ ] Consider MALLOC_ARENA_MAX tuning in gnoblin-session once
  real-hardware numbers exist.
- [x] Bound extension hot-reload memory: reload cache-bust is now keyed
  by a content digest, so unchanged extensions reuse their cached module
  (50 no-op reloads: 42 MB retained -> ~7 MB, mostly reclaimable churn).
  Distinct code versions still pin ~1 MB each for the session — inherent
  to the eviction-less ES module registry.
- [x] Evaluate lazy-loading the overview module graph in Gnoblin mode.
  Not viable: shellDBus.js and screenShield.js (always loaded) import
  the overview graph statically, so the modules load regardless; the
  rewrite cost outweighs the few MB of module heap.
- [x] Window-churn leak check: 30 real client open/close cycles hold
  shell RSS flat (ends below start, ~0.2 s total CPU) — window
  lifecycle, foreign-toplevel and layer-shell paths are clean.
- [x] Codify the probes: `just perf-smoke` enforces the budgets (idle
  private dirty + growth, soft-reload bound, window-churn bound).
- [ ] Re-measure the baseline on real hardware in a logged-in Gnoblin
  session (llvmpipe keeps GPU buffers in RAM and skews headless numbers).
  Run `just perf-smoke` there too and tighten its thresholds.

### Boot time: where the 0.7 s JS load actually goes (2026-07-27)

Three measurements, two of them negative results worth not repeating.

- [x] **Trimming unused imports is worthless — GI is lazy.** `panel.js`
  statically imports ~20 `status/*.js` modules that gnoblin never
  instantiates (empty `panel` arrays mean `_ensureIndicator()` is never
  called), pulling in GWeather, GnomeDesktop, Gvc, UPowerGlib, IBus and
  Polkit. Measured cost of importing all six extra typelibs under `gjs`:
  **1.6 ms total** (GWeather 0.4, GnomeDesktop 0.4, Gvc 0.2, UPowerGlib
  0.3, IBus 0.3, Polkit 0.1). `imports.gi.X` only builds the namespace
  object; symbols resolve on use, so imported-but-unused is nearly free.
  Do not spend time pruning imports for boot time.
- [x] **The wellbeing machinery is not a boot cost either.** `breakManager`,
  `timeLimitsManager` and `screenTimeDBus` (main.js:259-263) are built
  unconditionally, ~2650 lines, and gnoblin has no chrome to show any of
  it. `TimeLimitsManager` does read and JSON-parse
  `session-active-history.json` at startup with `history-enabled` true by
  default, but that file is 16 KB here — about 1 ms. Gating it would be
  clean if wanted for other reasons (grep finds *zero* references to these
  globals outside main.js: construction plus one `shutdown()` at :273), but
  it is not where boot time is.
- [x] ~~**The real structural cause: GJS ships no bytecode cache.**~~
  **WRONG — retracted, see below.** gjs 1.86 genuinely has no bytecode
  cache (verified by `strings`/`nm -D` on `libgjs.so.0.0.0`: zero hits for
  `bytecode`, `startup_cache`, `JS::EncodeScript`, `DecodeScript`, `XDR`,
  and no `~/.cache/gjs`). The error was assuming that absence explained the
  0.7 s. It does not: parsing is cheap.

### Correction: parse is ~60 ms, not 700 ms (2026-07-27)

Measured directly, rather than inferred. Shell JS lives inside
`libshell-17.so` (153 files, 2.43 MiB, 78.8k lines) — extract it with
`gresource extract`. Compiling **all** of it with SpiderMonkey via
`new Function(src)`, imports stripped: **60.3 ms** (40 MiB/s). And
`GObject.registerClass` — 359 call sites across 99 files — benchmarks at
0.146 ms for a class with two properties and one signal, so **~52 ms**
for the lot.

| Component                          | Measured   |
|------------------------------------|------------|
| Typelib imports (6 extra)          | 1.6 ms     |
| JS syntax-check/compile (2.43 MiB) | 60 ms      |
| `GObject.registerClass` x359       | ~52 ms     |
| `Gio.Settings` x117 @ 0.026 ms     | ~3 ms      |
| `makeProxyWrapper` x31 @ 0.024 ms  | ~0.8 ms    |
| Wellbeing history read + parse     | ~1 ms      |
| **Accounted for**                  | **~118 ms** |
| **"JS load" budget in TODO**       | **~700 ms** |
| **Unexplained**                    | **~580 ms** |

Every JS-side primitive benchmarks cheap, from five independent angles now.
None of the 117 `Gio.Settings` sites are at module scope (they run on object
construction, not import) and the 31 `makeProxyWrapper` calls that *are* at
module scope only parse introspection XML — no bus round trip. Remaining
non-JS suspects, unmeasured: St parses **181 KB** of CSS at startup
(`gnome-shell-dark.css`, via bundled libcroco) and then resolves a theme node
per actor, which is O(actors x rules) and lands in the "UI construction"
bucket rather than this one.

So a bytecode cache would save at most ~60 ms of a ~700 ms window. **Not
worth pursuing**, and not worth taking upstream on gnoblin's account. The
0.7 s figure came from a coarse strace time-split, and roughly 85% of what
it attributed to "JS load" is still unidentified — it is neither parse nor
class registration. Most likely candidates are top-level module *evaluation*
(GSettings construction, D-Bus proxy setup, actor building), but that is a
hypothesis, not a measurement.

- [ ] **Get a real GJS profile.** `shell_profiler_init()` (gnome-shell
  `src/main.c:243`) starts the GJS profiler only when *both*
  `GJS_ENABLE_PROFILER` is set and `GJS_TRACE_FD` parses to an fd `> 2`,
  which it hands to `gjs_profiler_set_fd()`. So it needs a real open
  descriptor, not a path. `gnoblin-shell-service` now arms this from a
  one-shot marker file, because GDM starts the unit and there is no shell in
  which to export anything first:

  ```sh
  dnf install sysprof                       # to read the capture
  touch ~/.config/gnoblin/profile-boot      # then log in to Gnoblin
  # -> ~/.local/share/gnoblin/boot-profile-<stamp>.syscap
  ```

  The marker is consumed on read, so exactly one capture is taken and a
  failed boot never leaves the profiler armed.

  There is now a headless counterpart that needs no login — same mechanism,
  via `run-gnome-shell.sh`:

  ```sh
  GNOBLIN_PROFILE=/tmp/boot.syscap ./scripts/run-gnome-shell.sh
  ```

### ANSWERED: the boot spends ~750 ms idle in the startup animation (2026-07-27)

Ran the headless capture (`GNOBLIN_PROFILE=... ./scripts/run-gnome-shell.sh`)
and decoded it. sysprof itself is not installed; the capture format is
straightforward enough to parse directly (note the frame-type enum is
**1-based**: 2=SAMPLE, 7=JITMAP, 10=MARK — assuming 0-based silently yields
garbage). 1975 samples at 944 Hz, 515 JITMAP symbols.

Busy vs idle, 250 ms buckets, "busy" = leaf is not
`Meta.Context.run_main_loop`:

| Window        | Busy   |
|---------------|--------|
| 0.00–0.25 s   | 90.4%  |
| 0.25–0.50 s   | 100.0% |
| 0.50–0.75 s   | 79.0%  |
| **0.75–1.50 s** | **~1%** |
| 1.50–1.75 s   | 7.9%   |

The shell does all its real work in the first ~0.75 s, then **idles ~750 ms**
before `startup-complete` fires and "GNOME Shell started" is logged. It is not
compute-bound at all in that window — it is waiting on
`STARTUP_ANIMATION_TIME` (500 ms, `layout.js:19`) plus background loading.
The shell log agrees: gnoblin-control acquires its bus name at +0.57 s and
nothing else is logged until +1.61 s.

This also **corrects the old "compute-bound, no I/O stalls" note** at the top
of this section. That was true of the first 0.75 s and wrong about the boot as
a whole.

- [x] Skip the startup animation in Gnoblin mode
  (`patches/gnome-shell/51-startup-animation`). `_startupAnimationSession()`
  eases uiGroup from scale 0.75/opacity 0 to 1/255 over 500 ms — for gnoblin
  that group holds no panel content, dash or overview, so it animates nothing
  and delays login by half a second. `_prepareStartupAnimation()` is gated on
  the same condition so uiGroup is never put into the state the animation
  exists to undo. Expected saving ~500 ms of a ~1.6 s boot.
- [x] **Verified headlessly: 1.612 s -> 0.951 s, a 661 ms (41%) saving.**
  Built into `./install` and re-profiled. Mutter display name to
  "GNOME Shell started" was 1.612 s unpatched (`/usr`) and 0.951 s patched;
  `run-gnome-shell.sh` still reports `RESULT: PASS` with no fatal
  diagnostics. The profile confirms the mechanism rather than just the wall
  clock — the 0.75–1.50 s dead zone is gone (0.75–1.00 s went from ~1% busy
  to 23%, and everything after 1.0 s is ordinary post-startup idle). Saving
  is larger than the 500 ms animation alone because the background fade
  overlapped it.
- [x] Codified as a gate: `just test-boot-time` boots headless, measures
  compositor process start to "GNOME Shell started", takes the **best of 3**
  and fails past `BOOT_BUDGET_MS` (default 1350 ms). Calibrated against both
  builds on this machine: patched `./install` 913–1161 ms, unpatched `/usr`
  1541–1608 ms, so the gate genuinely separates them rather than just passing.
  Best-of-N because the run-to-run spread is ~25% and noise only ever makes a
  boot slower. (Two bugs the first run caught: a "median" that for an even
  count reported the *slower* half and failed a healthy build at 1655 ms, and
  a start marker 102 ms adrift from the one used in the numbers above.)
### Load perf: layer-shell chrome latency (2026-07-27)

First measurement of the number gnoblin's responsiveness actually rides on.
Boot happens once; a layer-shell surface appearing is paid every time a bar,
dock or popup shows. `just test-layer-latency` builds a minimal shm client
(`tests/layer-shell-latency-client.c`, no toolkit) and times it inside a
headless session:

| Mark      | run 1   | run 2   |
|-----------|---------|---------|
| connect   | 0.06 ms | —       |
| globals   | 0.28 ms | —       |
| configure | 1.76 ms | 2.33 ms |
| **frame** | 29.6 ms | 38.9 ms |

**The compositor-side layer-shell path costs ~2 ms.** Everything from there to
first pixel is frame scheduling — ~2 vblanks at 60 Hz — which is physics, not
overhead, and explains the run-to-run spread.

This confirms the old note that slow chrome was never layer-shell itself but
per-process client cold-start (EGL context ~33 ms, first render ~30 ms, icon
loading). So for chrome that feels instant, the lever is on the *client* side —
keep it resident rather than spawning per invocation — not in the compositor.
Deliberately reports rather than gates by default; set `LAYER_BUDGET_MS` to
turn it into a budget once there is a real-hardware baseline.

- [ ] Re-measure on **real hardware** in a logged-in session. Everything
  above is llvmpipe headless; the animation saving is wall-clock and should
  carry over intact, but confirm.
- [ ] Then look at the remaining idle: `_updateBackgrounds()` (2.4%
  inclusive) and `BACKGROUND_FADE_ANIMATION_TIME` (1000 ms, `layout.js:20`).
- [ ] Smaller leads from the same profile, in order: `Shell.get_default`
  7.2% self, input-source setup ~4% combined (`InputSourceManager` 1.6%
  inclusive, `IBus.Bus.list_engines_async_finish` 1.3% self,
  `GnomeDesktop.XkbInfo.get_layout_info` 1.2% self), `_createBackgroundManager`
  2.2% inclusive.

Note: `just dev` respects an exported `GNOBLIN_PREFIX`. It is `/usr` in at
least some shells here, which would install a dev build straight over the
RPM-owned files — always pass `GNOBLIN_PREFIX="$PWD/install"` explicitly when
building.

- [ ] Note: the isolated devkit bus (scripts/devkit_dbus.py) copies the
  SYSTEM D-Bus service files, so headless runs activate the stock
  /usr/libexec portal backend — the patched
  install/libexec/xdg-desktop-portal-gnome is only exercised via the
  manual `-r` replace flow. Decide whether the harness should rewrite
  the service Exec lines to point at the built backend.

## Packaging

- [ ] Implement and test the Debian/Ubuntu package split described in
  `packaging/deb/README.md`.
- [ ] Implement and test the Arch Linux package split described in
  `packaging/arch/README.md`.
