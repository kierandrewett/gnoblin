# gnoblin — TODO

Working tracker for outstanding work + polish. Kept in the repo (not Claude's
task tool). Newest asks bubble to the top of **To do**.

Ethos: everything customisable (config / process-command); chrome follows macOS
HIG; animations buttery + customisable (easing/length/scale).

## Inspector huge pass — DONE + 1 bug found ("check the inspector.. let's do a huge pass")
The scene inspector dumped geometry + pre-composite textures, but every rendering
bug this session lived in the FINAL composited frame (cropped + sampled by hand
each time). Turned it into a tool that finds bugs mechanically, then dogfooded it.
- [x] Harness `audit OPS [OUT]` — rendering-invariant checks on the screenshot,
  exit non-zero on FAIL: corner-black (the libadwaita popup bug), runaway/zero
  frame, HiDPI 2x. Validated FAIL on pre-fix menu shot, PASS post-fix. (`446ba95`)
- [x] Harness `probe OPS [OUT]` — auto-crop+zoom every window's 4 corners into one
  contact sheet (the by-hand corner review, mechanised). (`446ba95`)
- [x] Harness `annotate OPS [OUT]` — overlay buffer/frame/corner-probe boxes. (`446ba95`)
- [x] Shared op-runner (spawn/sleep/click/rclick/dispatch); `rclick:WxH` added.
- [x] Compositor InspectScene now reports the live effect's applied corner_fill /
  adaptive / focused / FBO size (was hidden — had to read source). (`9d60144`)
- [x] Fixed latent JSON bug the dogfooding caught: an unallocated actor's box is
  ±inf / NaN → printf emits bare inf/-nan → invalid JSON → whole scene parse
  silently broke (foot+calc). `json_fvec` emits null + harness sanitiser. (`9d60144`)
- [x] Inner ring now drawn over the CSD corner-fill (Kieran feedback). (`8e5aff1`)
- [x] **BUG (probe-found): SSD windows rendered SQUARE corners — FIXED.** The
  rounding rounded the offscreen TEXTURE inset only by the CSD shadow margin
  (content_inset); for SSD the texture = frame + titlebar/shadow + effect margin
  but content_inset = 0, so the arc landed ~6px outside the frame → square. Fix:
  derive the per-side inset from the actor paint_box vs the window frame in
  paint_target (`inset = enlarge + (frame_edge − paintbox_edge)·scale`),
  asymmetric-aware + uniform across CSD/SSD/layer, guarded with a content_inset
  fallback. Added `paint_box` to InspectScene (revealed it). Verified: foot rounds
  all 4 corners, calc/menu/maximized-foot unchanged, audit green. (`cac5199`,
  `62cd6ff`). NB still worth an eyeball on real HW (llvmpipe SSD shadow may differ).

## GTK app menus: black popup corners — FIXED ("context menus in gtk apps are bugged")
Right-click/hamburger menus of libadwaita apps (e.g. gnome-calculator) showed
opaque BLACK right-triangles in their top corners. Root cause: the menu/popover
shares its app's pid, so `window_is_self_rounding()` reported TRUE and CSD
corner-fill was enabled on it — but a GtkPopover is self-shaped (rounded body +
tail/beak + wide transparent margin), so near its frame corner the nearest
straight edge is empty margin → sample (0,0,0,0) → fill painted black.
- [x] Gate corner-fill to real app TOPLEVELs (`window_is_app_toplevel`:
  NORMAL/DIALOG/MODAL_DIALOG) in `maybe_round_corners`. Menus still get rounding
  + the adaptive ring, just no fill. (`9effb90`)
- [x] Harden the shader regardless: `framevalid = smoothstep(0.02,0.25,fs.a)`
  fades corner-fill out as the sampled frame pixel loses opacity, so a transparent
  sample can never paint black wherever fill runs. (`9effb90`)
- [x] Verified headlessly: calc menu top corners were (0,0,0) → now menu-body
  white / wallpaper-through; calc TOPLEVEL corner-fill unchanged (no regression).
- [x] Harness `click:WxH` inspect op to open popups headlessly. (`723c01c`)
- foot/firefox menus: non-libadwaita → never self-rounding → never had the fill,
  so never black; they already get gnoblin rounding + ring.

## HiDPI 2× rendering — FIXED ("hidpi is important")
Long-running "Slint clients render 2× too big at HiDPI" bug. Root cause was NOT
the Slint client (proven: its GL buffer is pixel-correct) nor EGL/dmabuf/layer
(intermediate hypotheses, disproven) — it was **`gnoblin-blur`** (a
ClutterOffscreenEffect): its `paint_target` override composited the offscreen
actor texture using PHYSICAL (resource-scaled) quad coords into a LOGICAL-coord
framebuffer, so on a scale-2 output the actor was drawn 2× too big (magnified from
top-left). Coincidentally fine at scale 1 (physical==logical).
- [x] Fix: divide the composite quad by `clutter_actor_get_resource_scale` in
  `gnoblin_blur_paint_target` (matches the base OffscreenEffect; no-op at scale 1;
  correct for fractional scales). Only blurred surfaces (chrome) were affected.
- [x] Verified at HIDPI=2: topbar bar 68px (was 134); HiDPI÷2 vs scale-1 whole-frame
  mean diff 2.4 (was ~26); dock frost + blur still render correctly; scale-1 unchanged.
- [x] Inspector tooling that cracked it (kept): client `GNOBLIN_DUMP_BUFFER`
  (glReadPixels) + compositor `GNOBLIN_DUMP_TEXTURE` (cogl plane readback) + scene
  fields stage/scale/content/alloc/txsize/redirect/rscale. `HIDPI=N` harness env.

## QS DE-HARDCODING — DONE (Kieran's directive)
"there shouldnt be built in plugins.. dont hardcode... define in config... they
poll async" + "prefix commands with gnoblin" + "no graceful fallback".
- [x] Default `gnoblin-qs-*` plugin scripts shipped (src/data/plugins/), NO
  fallbacks: wifi(nmcli)/bluetooth(bluetoothctl)/output+mic(wpctl slider)/
  nightlight+dnd(runtime flag file)/darkstyle($XDG_RUNTIME_DIR/gnoblin-theme)/
  powermode(powerprofilesctl)/backgroundapps(flatpak). (`96cebd2`)
- [x] Installed to bindir on PATH. (`33b27cb`)
- [x] Default config layer (`src/data/gnoblin.defaults.conf`) declaring them in
  order; qsplugin::load_configs parses it as a BASE and overlays the user's
  gnoblin.conf (override by id, `enabled=off` to disable, `[quicksettings] order`
  to reorder). config.rs from_text un-cfg-test'd. TileSpec gained `value` (sliders).
- [x] build_qs_tiles renders PURELY from the plugin snapshot (no builtin_tile);
  dispatch forwards every tap/slide to the host; removed set_volume + the
  vestigial [commands] wired/wifi/bluetooth/background_apps.
- [x] Verified: cargo check + clippy(-D warnings) + 49 shell-ui tests green; devkit
  CC renders entirely from plugins (wifi/bluetooth sliding submenus, sliders,
  toggles, media card) with NO hardcoded built-ins. Theme/dnd/nightlight changes
  propagate via the existing client theme-follow poll. (`a4c8a34`)
- [x] codex gpt-5.5 review of the pivot — one P2 (plugin slider value unclamped →
  over-100% PipeWire volume overdrew the track); fixed with a 0..1 clamp. (`48641b0`)

## Blur — FIXED (was never GPU-broken; harness uses the real GPU /dev/dri/renderD128)
- [x] SMEAR: 9-tap Gaussian ran at radius*0.5 with fixed half-res → ~90 texels
  between taps → under-sampled streaks. Fixed: downsample ∝ radius, tight kernel.
  Verified smooth at radius 100. (`6ebeb16`)
- [x] NO FROST ON TOPBAR/QS: self-inflicted — I'd disabled blur on gnoblin-topbar
  to kill a halo, but the QS popout shares that surface. Halo was the ROUNDING.
  Topbar now flat+frosted; QS+dock frosted. (`6ebeb16`)
- [x] FLICKER ("blurring shadows / flickers"): isolated to the blur (not the
  shadow — A/B'd via no-blur/no-shadow rules + an 8-frame capture diff). Root
  cause: read-back blur feeds back through double-buffering → frost TOGGLES
  between two states. Fixed with temporal smoothing (2-frame average converges
  both buffers). Flicker dropped ~90% (dock max 38→14, many frame-pairs now
  identical). Verified with the multi-frame capture harness.
- Tooling added: scratchpad/flicker.py (multi-frame flicker capture + region
  diff) and GNOBLIN_EXTRA_CONF harness hook for A/B config testing.

## Other in progress
- (nothing actively in flight — blocked items are below)

## Code sweep (docs/code-sweep-2026-06.md) — autonomous-loop cleanup
Applying the report's verified "safe" findings one batch per loop tick; each
batch builds/tests/fmts clean and lands as its own commit. mutter gitlink stays
pristine (86e92a2); only working-tree edits, never the submodule pointer.
- [x] spec-util cursor tokenizer dedup (1.1, d9c3653); config empty-key C↔Rust
  parity (9ca0bc6); inspector finite-float JSON guards (203274a)
- [x] dead-code/comment cleanups across control/rules/switcher/overview/shell
  (203274a, efa3214, 76d9606, 7379e66, e71240f)
- [x] night-light gamma re-create→re-fail loop fix (b45d9f4)
- [x] FileFlag dedup for dnd/nightlight runtime flags (2.3, 3946eeb)
- [x] notifcenter legacy flag → clear_legacy_flag(), dead toggle() removed (8.7, 740d9e9)
- [x] ControlCentrePopout 6 dead colour props + 3 dead animates (7.1, c7364be)
- [x] Panel.icon-bluetooth dead in-property (7.5, 44ad95d)
- [x] Theme 7.4 dead tokens — highlight-*-top ×4 + 2 re-exports, panel-corner-radius,
  motion-overlay-duration, motion-overlay-curve alias (8731cab)
- [x] Theme 7.2 DatetimePopout.clock-text inert — decl + BOTH feeds (Compositor +
  topbar; report missed the topbar one, compiler caught it) + header (87113e4)
- [x] Theme 6 comments (this loop): 6.14 entries_with_prefix doc + drop no-op seen
  (cf33118); 6.6 qsplugin misattached doc + 6.11 WindowChrome 18px-not-14px (8d759b6);
  6.5 shell.rs close_app_windows doc move (HEAD). Earlier: 6.1/6.2/6.9/6.10/rounded.h/
  lock.cpp.
- [x] Theme 6 comments (this loop, cont.): 6.4 lib.rs load_backdrop doc + 6.15 config.h
  header docs (c19d31d); 6.12 Tokens overlay-motion rewrite (f1a513c). 6.7 was already
  done (76d9606).
- [ ] Theme 6 leftovers (low ROI): 6.3 topbar 4× misattached + dead #[allow(too_many_
  args)] — the allow needs clippy to confirm removable; lower-value comment batch
  (shadow.h, blur.cpp:38-60 Gaussian, anim.h, overview.cpp:205, prefs.rs dead_code,
  Dock.slint/ContextMenu.slint/IconButton.slint headers, launcher/main.rs:64/82).
  SKIP 6.8 (already clean). DEFER 6.13 (premise shaky — needs data-flow trace).
- [ ] NEXT: spec-util dedup (1.3 nonneg_int, 1.5 hex colour) into gnoblin-spec-util;
  residual small dead code (8.x)
- NEEDS-KIERAN / careful pass (not done blind in the headless loop):
  - Theme 7.3 Dock backdrop — WIDER than the report: dead chain spans Dock.slint
    props (304-307) → dock/ui/dock.slint wrapper (2 instantiations) → dock/src/
    main.rs:81-84 (load_backdrop + 4 setters, load_backdrop shared w/ topbar) +
    Compositor feed. Dock component provably never renders it (only icon Image),
    but this touches frost/backdrop behaviour the memory says needs REAL-HW verify
    (llvmpipe can't repro). Don't rip the plumbing headless.
  - lib.rs 3422-line split (4.6); shared SDF GLSL extract blur↔rounded (4.1);
    PopoutChrome + overlay grab/teardown dedup (4.5/4.3); control-center GSettings
    ↔ gnoblin.conf source-of-truth question

## To do (open-ended — "whatever gnoblin needs")
- [ ] More launcher providers if wanted (dictionary define, ssh hosts, window
  switcher) — trivial to add. (convert + color shipped.)
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
- [x] Full regression sweep after the blur + QS-pivot work: test-clients (fmt +
  clippy -D warnings + 55 unit tests), test-logic (C parsers/SDF/PAM), and the
  blur/shadow/topbar devkit tests (blur, content-behind-blur, chrome-blur,
  blur-alpha-threshold, shadow-not-blurred, topbar-live-commands) ALL green. Found
  + fixed 2 pre-existing failures (rustfmt drift; a stale maximize/unmaximize
  expectation failing since the initial commit) + updated topbar-live-commands for
  the plugin pivot. (`3d0ea4a`..`566dbda`)
- [x] Launcher: unit-conversion + colour-converter providers (`c 10 km to mi`,
  `# ff8800` → hex/rgb/hsl, ⏎ copies). Offline, awk-based, ⏎-copyable results;
  registered in the harness + documented in conf.example. codex gpt-5.5 caught +
  I fixed a P2 ("in" source unit mis-parsed as separator); re-review clean.
  (`23ada93`, `7dfab31`)
- [x] Topbar truly flush — re-disabled its frost (the re-frost had reintroduced
  the rounded screen-edge halo; blur can't frost a screen edge). (`1c60bbb`)
- [x] Launcher: clipboard-history provider example (cliphist, prefix "v ").
  Launcher now ships web/files/emoji/kill/clipboard providers + calc + web
  fallback. (`9cf841d`)
- [x] Launcher: opt-in Spotlight-style web-search fallback (`[launcher]
  web-search = <url %s>`) when nothing else matches. (`afd23b6`)
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
