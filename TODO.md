# gnoblin — TODO

Working tracker for outstanding work + polish. Kept in the repo (not Claude's
task tool). Newest asks bubble to the top of **To do**.

Ethos: everything customisable (config / process-command); chrome follows macOS
HIG; animations buttery + customisable (easing/length/scale).

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
