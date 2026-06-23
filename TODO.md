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

## To do (from the loop brief, roughly in order)
- [ ] **More launcher providers** (optional polish) — the host is DONE
  (`82b36e5`) with web + file-search examples. Easy wins to add as shipped
  example scripts: unit/currency convert, dictionary define, emoji search,
  clipboard history, ssh hosts, kill-process, window switcher.
- [ ] Then: whatever gnoblin needs next, following the ethos.

## Needs Kieran (real-GPU verification)
- [ ] **Blur** — rewrote `gnoblin-blur.cpp` (padded capture + downsample-then-
  blur). Smooth on llvmpipe + all blur tests green; confirm the smear/flicker is
  gone on real hardware. If shadow still flickers, dig into the shadow
  second-pass (`shadow_clip_region`). (`4b178f6`)
- [ ] **Animation feel** — confirm the QS carousel now feels buttery and the
  window open/close is "a little more obvious" but not too much; tune
  `[animations] slint-page` / `open` / `close` to taste. (`00c31b0`)

## Done (recent)
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
