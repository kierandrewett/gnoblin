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

- [x] Measure and bound memory growth across repeated `org.gnoblin.Shell`
  soft reloads; fix any per-reload leak. Found ~3.8 MB leaked per
  `Main.loadTheme()` swap (upstream St/GJS: replaced StTheme wrapper
  survives GC at refcount 1). Mitigated in `softReload()` via a
  stylesheet-set digest; 15 reloads now hold private dirty flat.
- [ ] Root-cause the upstream `loadTheme()` theme leak so stylesheet
  changes stop paying it; upstream a fix. Probes live in the session
  scratchpad (theme-leak-probe.js: RSS sampling; WeakRef is unusable in
  GJS — kept-objects list never clears).
- [ ] Evaluate lazy-loading the overview module graph in Gnoblin mode
  (Overview is a dummy but its ES module imports still load).
- [ ] Re-measure the baseline on real hardware in a logged-in Gnoblin
  session (llvmpipe keeps GPU buffers in RAM and skews headless numbers).

## Packaging

- [ ] Implement and test the Debian/Ubuntu package split described in
  `packaging/deb/README.md`.
- [ ] Implement and test the Arch Linux package split described in
  `packaging/arch/README.md`.
