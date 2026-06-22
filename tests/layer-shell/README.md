# Devkit harness + behavioural regression tests

Headless, no-human-in-the-loop tests that drive a real `gnoblin-shell` and assert
how it behaves — the "playwright for the devkit". They need a dev build in
`./install` (`just dev`); run them all with **`just test-devkit`**.

## The harness — `scripts/devkit-harness.py`

A long-lived Python (gi/Gio) process that owns its own private `dbus-daemon` + XDG
sandbox (so it never leaks state into your session) and boots `gnoblin-shell
--headless`. As a CLI: `just harness <sub>` —

| sub | what it does |
|-----|--------------|
| `smoke` | boot + settle + crash-check |
| `shot OUT.png` | boot, settle, screenshot (grim) |
| `late OUT.png` | boot with **no** monitor, then add one at runtime via ScreenCast `RecordVirtual` + a PipeWire consumer (reproduces the devkit's late-monitor flow) |
| `storm` | add a late monitor then renegotiate its size in a loop (configure storm) |
| `keys 'Super+Space:calc' OUT` | inject a key chord (+ optional typed text) via mutter RemoteDesktop, then screenshot |
| `wm 'spawn:foot,maximize' OUT` | drive the window manager over `dev.gnoblin.Shell` (spawn / maximize / snap / minimize / fullscreen / lock / close …) |
| `run CLIENT` | boot a bare compositor, run an arbitrary layer-shell client, report COMPOSITOR SURVIVED/CRASHED |
| `boot` | boot and stay up (prints WAYLAND_DISPLAY), Ctrl-C to stop |

As a library, `Devkit` also offers: `boot(monitors=[...])` (multi-output),
`dispatch/list_windows/workspace_state`, `extra_appearance` (inject `[appearance]`
config, e.g. a wallpaper). Keyboard injection works; pointer injection is
available through a RemoteDesktop session linked to a ScreenCast monitor stream,
and `Devkit.click()` is used by the pointer/input regressions below.

Crash detection = process rc + a scan for fatal signatures in the compositor log
(`assertion failed`, `Bail out`, GLib `*-CRITICAL`, runtime-check failures,
segfault/status-139 text, panics, frame-callback aborts). The latest log is saved
to `/tmp/gnoblin-harness-last.log`.

## The regression tests (`run-*.sh`, all in `just test-devkit`)

| test | guards |
|------|--------|
| `run-devkit-dbus.sh` | isolated devkit DBus config activates the Documents stub even with spaced paths |
| `run-frame-callback-crash.sh` | a layer-shell client committing a buffer before ack_configure gets a protocol error, **not** a compositor abort (the original devkit crash) |
| `run-layer-shell-protocol.sh` | valid map/remap + popup/stale-configure paths work, state-only layer-shell commits get a fresh configure, null-buffer unmap resets state, and invalid requests get protocol errors without killing the compositor |
| `run-output-destroyed-closes.sh` | requested-output layer surfaces receive `closed` when that output disappears, and post-`closed` requests are ignored |
| `run-configure-storm.sh` | late virtual monitor creation plus repeated size renegotiation does not kill the compositor or autostart layer-shell clients |
| `run-autostart-retry.sh` | failed global and per-output startup entries are retried after config reload instead of being permanently skipped |
| `run-autostart-output-removal.sh` | per-output autostart clients are not respawned for removed outputs |
| `run-keybind-launcher.sh` | Super+Space → launcher spawns; Escape → it closes (full input path) |
| `run-launcher-activates-app.sh` | Super+Space → type app → Return launches the selected desktop app, and a real list-row click does the same |
| `run-role-spawn-reap.sh` | short-lived on-demand role clients are reaped instead of accumulating zombies |
| `run-explicit-command-reap.sh` | short-lived explicit `gnoblin-shell -- COMMAND` children are reaped instead of accumulating zombies |
| `run-pointer-input.sh` | synthetic pointer clicks reach Slint layer-shell callbacks |
| `run-dock-launch.sh` | dock launch resolves nested XDG desktop IDs, handles Exec fallbacks, and launches from a real dock icon click |
| `run-dock-live-favorites.sh` | running dock reloads `[dock] favorites` after config edits |
| `run-osd-passthrough.sh` | full-screen OSD overlay commits an empty input region and passes clicks through to the topbar |
| `run-dock-menu-input-region.sh` | shrinking a layer-surface input region under an idle stationary pointer refreshes compositor focus |
| `run-window-menu-input.sh` | modal window-menu overlay catches a stationary outside click, exits, and releases that pointer position to the app below |
| `run-layer-move-focus.sh` | moving a layer-shell surface under a stationary pointer refreshes compositor focus |
| `run-layer-keyboard-focus.sh` | `keyboard_interactivity=on_demand` layer surfaces receive keyboard focus after click, and `exclusive` surfaces receive it on map |
| `run-topbar-live-commands.sh` | running topbar reloads `[topbar]` action commands after config edits |
| `run-topbar-layout-live.sh` | running topbar reloads `[topbar]` widget layout and moves the status widget's input/callback geometry |
| `run-topbar-focused-app-menu.sh` | the topbar focused-app widget opens the shared app context menu and its Quit row affects the focused app |
| `run-notification-center-input.sh` | notification popups, quick-settings history and topbar popouts do not block each other's input regions, including after idle gaps |
| `run-maximize-strut.sh` | maximize does **not** overlap the topbar or dock exclusive zones |
| `run-fullscreen-cover.sh` | a fullscreen window **does** cover the topbar/dock |
| `run-lock-engage.sh` | the lock overlay obscures the whole desktop (security) |
| `run-snap-regions.sh` | every built-in snap region lands in the work-area fraction and stays out of the dock band |
| `run-notifications.sh` | notifyd popup + quick-settings notification history render |
| `run-region-lifetime.sh` | Slint layer-shell input-region updates destroy temporary `wl_region` objects |
| `run-slint-animation-frames.sh` | Slint animations advance on frame callbacks without requiring pointer motion |
| `run-topbar-live-motion.sh` | running topbar applies live `[animations] enabled` changes to Slint motion scale |
| `run-topbar-live-backdrop.sh` | running topbar reloads its popout wallpaper backdrop after config edits |
| `run-night-light-hotplug.sh` | `gnoblin-night-light` binds gamma controls for outputs hotplugged while Night Light is already on |
| `run-multimonitor.sh` | two outputs → per-monitor panels; maximize stays on its monitor |
| `run-power-menu-output-size.sh` | full-screen shell-ui clients center using the configured output size, not a hard-coded 800px fallback |
| `run-power-menu-resize.sh` | full-screen shell-ui clients recompute geometry after monitor resize |
| `run-wallpaper.sh` | `gnoblin-wallpaper` renders below normal windows and does not reserve app work area |
| `run-wallpaper-output.sh` | `gnoblin-wallpaper --output` pins distinct wallpapers to distinct monitors |
| `run-background-layer-input.sh` | background wallpaper stays below apps and does not intercept pointer input |
| `run-protocols.sh` | the compositor advertises gnoblin's full wlr-/ext- protocol set (9 globals) |
| `run-kde-appmenu-backend.sh` | KDE appmenu Wayland addresses are exposed/hidden according to `[topbar] appmenu-backend` |
| `run-topbar-dbusmenu.sh` | the topbar renders, opens, and activates a DBusMenu global menu |
| `run-effects-shadow.sh` | the effects layer draws a drop shadow around windows |
| `run-maximize-animation.sh` | `maximize`/`unmaximize` use configurable frame-lerp animations between restored and maximized rects |
| `run-foreign-toplevel.sh` | `ext-foreign-toplevel-list-v1` streams the window list (a probe client sees the open app) |
| `run-window-rules.sh` | `[window-rules]` place/state windows at map time; new WM actions dispatch |
| `run-tiling.sh` | keyboard `tile` cycling (half/two-thirds/one-third), `[appearance] gaps`, and topbar/dock work-area bounds |
| `run-overview.sh` | `overview` opens an opaque live-thumbnail grid, toggles shut, leaves windows intact |
| `run-switcher.sh` | visual `switcher`/Alt+Tab draws a thumbnail panel; held Alt+Tab commits a focus change |
| `run-grid.sh` | `gnoblin-launcher --grid` draws the app grid (square selection tile, not a row), closes on Escape, and launches a clicked tile |
| `run-input-config.sh` | `[input]` config maps onto the org.gnome.desktop GSettings mutter's input backend reads |
| `run-output-config.sh` | `[output]` config rotates/places the monitor via DisplayConfig, exactly once (no reconfigure loop) |
| `run-workspaces.sh` | named workspaces (WorkspaceNames D-Bus) + scratchpad stash/show/hide |
| `run-blur.sh` | `[appearance] blur` composites a Gaussian-blurred desktop behind the Overview tint |
| `run-gestures.sh` | touchpad gestures resolve via `[gestures]` override / built-in default / `none` |
| `run-maximize-effects.sh` | rounded corners + shadow are suppressed while a window is maximized/fullscreen |

The pixel-checking tests use PIL on grim screenshots; grim PNGs are lossless, so low
diff thresholds are safe. Each `run-*.sh` SKIPs cleanly if a dependency (grim, foot,
PIL, notify-send) is missing.

## Visible Devkit Smoke

`just test-devkit-visible` runs `scripts/run-devkit.sh visible` under `timeout`
several times and checks the path that `just devkit` uses directly: Mutter Devkit
viewer startup, portal negotiation, layer-shell autostart, optional command launch
after the virtual monitor exists, `/tmp/gnoblin-shell-last.log` publication, and
cleanup after interruption. It is intentionally not part of
`just test-devkit` because it needs a host graphical session and opens a window.

Tuning:

- `VISIBLE_DEVKIT_RUNS=1` for a quick single pass (default: `3`)
- `VISIBLE_DEVKIT_TIMEOUT=45s` if the local PipeWire/portal path is slow (default: `45s`)
- `VISIBLE_INPUT_CYCLES=N` and `VISIBLE_INPUT_IDLE_SECONDS=S` to stress repeated
  launcher keyboard input after idle gaps in the visible devkit path (defaults:
  `4` cycles, `6` seconds)

## Adding a test

1. Write `tests/layer-shell/<name>-test.py` importing the harness `Devkit` (see any
   existing test for the boilerplate), assert via `dispatch`/`list_windows` and/or a
   grim screenshot + PIL.
2. Add `tests/layer-shell/run-<name>.sh` (copy an existing one — same SKIP guards).
3. Append it to the `test-devkit` recipe in the `Justfile`.

Gotcha: do not add host-wide `pgrep`/`pkill` cleanup to these tests. It can match
the developer's real session (for example `foot`, `gnoblin-launcher`, or
`gnoblin-notifyd`). Prefer `Devkit.processes("name")` for assertions and rely on
the harness teardown for nested clients.
