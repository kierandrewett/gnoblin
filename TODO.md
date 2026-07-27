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
| Wellbeing history read + parse     | ~1 ms      |
| **Accounted for**                  | **~115 ms** |
| **"JS load" budget in TODO**       | **~700 ms** |
| **Unexplained**                    | **~585 ms** |

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
  failed boot never leaves the profiler armed. This is the next real step
  for boot time; everything measurable without logging in is now done.

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
