# gnoblin — TODO

Working tracker for outstanding work + polish. Kept in the repo (not Claude's
task tool). Newest asks bubble to the top of **To do**.

## In progress
- [ ] **MPRIS media card (#6)** — 4x1 "now playing" tile (`layout: "media"`):
  art chip + title/artist + prev/play-pause/next. Heroicons-solid media glyphs
  added (`play/pause/backward/forward.svg`); `ShellMediaCard` component + grid
  branch + mpris provider `span:4 layout:"media"` still to do.

## To do
- [ ] **Blur — confirm on real GPU.** Rewrote `gnoblin-blur.cpp`: padded capture
  (samples real backdrop past the actor edge → no clamp halo) + downsample-then-
  blur (half-res bilinear average → the 9-tap Gaussian covers the radius smoothly
  → no undersampling smear). Looks smooth on llvmpipe + all blur tests green, but
  **Kieran to verify the smear/flicker is gone on real hardware.** If the shadow
  still flickers, dig into the shadow second-pass (`shadow_clip_region`).

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
