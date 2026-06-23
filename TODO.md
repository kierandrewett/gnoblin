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
- [ ] **App launcher → macOS Spotlight.** Redesign appearance (Spotlight-esque),
  and add smart modes: file search, calculator, and invent fun providers
  (unit/currency convert, define, web-search handoff, emoji, clipboard, kill
  process, etc.). Drive results via the process/command approach so each search
  source is a configurable plugin (like the QS plugins). Keep everything
  customisable.
- [ ] **Dock redesign** — still looks ugly; the borders hurt it. Follow macOS HIG
  for borders/chrome (hairline/none, proper translucency + blur, separators,
  running-dot, hover magnification?). Improve the whole look.
- [ ] **Context menu redesign** — currently chunky + ugly. Tighten padding, type,
  row height, separators, radius; macOS-style refined menu chrome.
- [ ] **codex CLI (GPT 5.5) validation pass** — once the above land, have codex
  review/validate the changes and act on findings.
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
