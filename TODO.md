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
session.

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
- [ ] Consider MALLOC_ARENA_MAX tuning in gnoblin-session once
  real-hardware numbers exist.
- [ ] Bound extension hot-reload memory: the cache-busting import URLs
  (34-extension-hot-reload) pin every previous module version in the ES
  module registry for the session; measure growth under heavy reload
  cycles and cap if it matters in practice.
- [x] Evaluate lazy-loading the overview module graph in Gnoblin mode.
  Not viable: shellDBus.js and screenShield.js (always loaded) import
  the overview graph statically, so the modules load regardless; the
  rewrite cost outweighs the few MB of module heap.
- [ ] Re-measure the baseline on real hardware in a logged-in Gnoblin
  session (llvmpipe keeps GPU buffers in RAM and skews headless numbers).

## Packaging

- [ ] Implement and test the Debian/Ubuntu package split described in
  `packaging/deb/README.md`.
- [ ] Implement and test the Arch Linux package split described in
  `packaging/arch/README.md`.
