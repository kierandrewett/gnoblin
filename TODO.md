# gnoblin — TODO

Working tracker for outstanding work + polish. Kept in the repo (not Claude's
task tool). Newest asks bubble to the top of **To do**.

## In progress
- [ ] **MPRIS media card (#6)** — 4x1 "now playing" tile (`layout: "media"`):
  art chip + title/artist + prev/play-pause/next. Heroicons-solid media glyphs
  added (`play/pause/backward/forward.svg`); `ShellMediaCard` component + grid
  branch + mpris provider `span:4 layout:"media"` still to do.

## To do
- [ ] **Blur smearing + shadow artifacts/flicker** — content-behind blur is
  heavily SMEARED and flickers with artifacts where the shadow is. Investigate
  `gnoblin-blur.cpp` (Gaussian sample bounds / smear direction) + the shadow
  second-pass in `gnoblin-shell-plugin.cpp` (`shadow_clip_region`). Note:
  llvmpipe devkit may not reproduce GPU artifacts.

## Done (recent)
- [x] **Topbar flush + flat** — no rounding/shadow/frost on the edge-to-edge
  bar (the blur was smearing a rounded halo into the corners). (`<topbar-flush>`)
- [x] Topbar status cluster: more icon spacing (gap 5→10) + cleaner hover pill
  (padding 4→10). (`ae0dc7f`)
- [x] Submenu slides as a carousel — grid exits left as the page enters right,
  220ms ease-out (was sliding over). (`ae0dc7f`)
- [x] Cap control centre to screen height + scrollable body (Flickable);
  harness `scroll()` added. (`841ca43`)
- [x] Slide-out submenu for tile chevrons (Firefox-app-menu style) + re-plumbed
  plugin row events. (`78331fb`)
- [x] Unified the control centre into one plugin tile grid (span 1–4, no special
  sections). (`8c3e0f0`)
- [x] Minimal single-row Output/Mic sliders with device-picker chevron. (`5a0ce4c`)
