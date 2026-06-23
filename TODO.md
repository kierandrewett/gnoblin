# gnoblin — TODO

Working tracker for outstanding work + polish. Kept in the repo (not Claude's
task tool). Newest asks bubble to the top of **To do**.

Ethos: everything customisable (config / process-command); chrome follows macOS
HIG; animations buttery + customisable (easing/length/scale).

## In progress
- [ ] **MPRIS media card (#6)** — 4x1 "now playing" tile (`layout: "media"`):
  art chip + title/artist + prev/play-pause/next. Heroicons-solid media glyphs
  added (`play/pause/backward/forward.svg`); `ShellMediaCard` component + grid
  branch + mpris provider `span:4 layout:"media"` + transport→plugin-row wiring
  still to do.

## To do (open-ended — "whatever gnoblin needs")
- [ ] More launcher providers if wanted (unit/currency convert, dictionary
  define, clipboard history, ssh hosts, window switcher) — trivial to add.
- [ ] Polish driven by Kieran's real-hardware feel feedback (blur, animation
  timing, dock, menu, launcher).
- [ ] Then: whatever gnoblin needs next, following the ethos.

## Real bug found (pre-existing, needs a focused fix)
- [ ] **Alt+Tab keybind never fires the switcher** — diagnosed: `switcher_open`
  is never called on Alt+Tab (instrumented + confirmed). NOT a regression and NOT
  the switcher logic (which is correct: opens on index 1, commits on Alt release).
  Root cause: gnoblin registers every config keybind via
  `meta_display_grab_accelerator` (gnoblin-control.cpp), and mutter does **not**
  emit `accelerator-activated` for switch-application-style accelerators like
  `<Alt>Tab` — `Super+Space` fires fine through the same path
  (run-keybind-launcher passes), so it's Alt+Tab-specific. Fix: route Tab-based
  switcher binds through mutter's `switch-applications` /
  `switch-applications-backward` custom-handler
  (`meta_keybindings_set_custom_handler`) instead of grab_accelerator. Careful
  keybind-system change → needs real-hardware testing (the headless harness also
  can't fully validate the held-modifier release-commit). Confirm on real
  hardware whether Alt+Tab is dead there too (very likely yes).

- [ ] **Blur** — rewrote `gnoblin-blur.cpp` (padded capture + downsample-then-
  blur). Smooth on llvmpipe + all blur tests green; confirm the smear/flicker is
  gone on real hardware. If shadow still flickers, dig into the shadow
  second-pass (`shadow_clip_region`). (`4b178f6`)
- [ ] **Animation feel** — confirm the QS carousel now feels buttery and the
  window open/close is "a little more obvious" but not too much; tune
  `[animations] slint-page` / `open` / `close` to taste. (`00c31b0`)

## Done (recent)
- [x] Hardening sweep — ran the compositor-effects + UI devkit tests touched by
  the blur/rounding/shadow/anim changes: blur, content-behind-blur, chrome-blur,
  blur-alpha-threshold, shadow-not-blurred, effects-shadow, effects-rules-visual,
  maximize-effects/animation, window-rules, overview — all green. (One pre-
  existing held-Alt+Tab harness failure isolated as NOT a regression.)
- [x] codex re-review (gpt-5.5) of the new code — clean (build + tests pass); its
  one P1 was a false positive (misread the locally-patched mutter submodule
  working tree as a committed change; the committed pointer is unchanged — left
  alone). Untracked stale `__pycache__` .pyc noise. (`8a45ffe`)
- [x] Launcher: emoji + kill-process providers; fixed the provider query
  interface ($GNOBLIN_QUERY — $1 didn't reach script-path commands). (`5409f16`)
- [x] Submenu rows refresh live while open (last codex P2 cleared). (`e9243cd`)
- [x] codex gpt-5.5 validation pass — caught a P1 build-breaker (untracked
  transport SVGs) + 2 P2 UX bugs (launcher scroll math, empty-submenu chevron),
  all fixed. (`27946a8`, transport-glyphs commit)
- [x] Launcher: process/command provider host — `[launcher-provider.NAME]`
  commands emit TSV results (title/subtitle/icon/action), prefix-gated; web +
  file-search examples shipped. Search anything, config-driven. (`82b36e5`)
- [x] Launcher: macOS Spotlight redesign (frosted upper-third panel, big search
  field, rich rows) + inline calculator (type maths → answer, ⏎ copies). The
  Row/Action model is provider-ready. (`191231b`)
- [x] QS: MPRIS now-playing media card (4x1 "media" tile, transport → plugin rows). (`a801b21`)
- [x] Dock: frosted-glass macOS chrome — more translucent, soft rim instead of
  the boxy border, no focused-slot border, radius 23. (`4b7ba65`)
- [x] Context menu: compact macOS rows (34→26px), tighter insets, less chunky.
- [x] Buttery QS carousel (`Motion.page`, ease-out-expo 420ms) + more noticeable
  window open/close; both customisable. (`00c31b0`)
- [x] Blur smear/halo fix (downsample + padded capture); topbar re-frosted but
  still flush/flat. (`4b178f6`)
- [x] Topbar flush + flat (no rounding/shadow); roomier status cluster; tests
  adapted. (`d98c435`)
- [x] Submenu carousel slide + TODO.md. (`ae0dc7f`)
- [x] Cap CC to screen + scrollable; harness `scroll()`. (`841ca43`)
- [x] Slide-out submenu for tile chevrons + plugin row events. (`78331fb`)
- [x] Unified CC plugin tile grid (span 1–4, no special sections). (`8c3e0f0`)
- [x] Minimal Output/Mic sliders with device-picker chevron. (`5a0ce4c`)
