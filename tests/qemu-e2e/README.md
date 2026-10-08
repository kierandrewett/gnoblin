# QEMU end-to-end checks

These scripts check a built Gnoblin prefix inside the QEMU guest described in
`design/qemu-e2e-validation.md`. They need a running guest and a prefix that was
copied into it.

1. Build: `./build.sh --prefix "$PWD/build/validation-install"`.
2. Copy it into the guest: `scripts/qemu-e2e sync-prefix RUN_DIRECTORY "$PWD/build/validation-install"`.
3. Restart the guest session: `scripts/qemu-e2e guest RUN_DIRECTORY -- sudo systemctl restart gdm`.
4. Run a check from the repository root.

| Script                              | What it checks                                                                 |
| ----------------------------------- | ------------------------------------------------------------------------------ |
| `tests/qemu-e2e/run-regression.sh`  | Capabilities, privacy, grants, `commands.capture`, polkit prompts, keyring and GPG prompts through the prompt broker, input sources, `gnoblinctl` queries, compositor stability. Prints PASS or FAIL for each check. |
| `tests/qemu-e2e/run-shortcut.sh`    | A Lua dynamic shortcut fires when its key combination reaches the guest.       |
| `tests/qemu-e2e/run-config-shortcut.sh` | A shortcut declared in `gnoblin.configure {shortcuts = ...}` runs its command, moves when the binding changes on reload, and stops when disabled. |
| `tests/qemu-e2e/run-config-fallback.sh` | A config whose root `init.lua` cannot be loaded at login (a Lua syntax error) keeps the session running, and the fallback choice and error reach the marker file and the session log. Restarts the guest session twice. |
| `tests/qemu-e2e/run-partial-recovery.sh` | One invalid setting, or one included file with a Lua syntax error, at login is ignored and named in the log and the notice, while the valid settings stay active and nothing falls back. Restarts the guest session twice. |
| `tests/qemu-e2e/run-titlebar-actions.sh` | Titlebar actions change what a click does: `action_double_click_titlebar` as `toggle-maximize`, `minimize` or `none`, and `action_middle_click_titlebar` as `lower` or `none` (the window behind or on top, read from the pixel where two windows overlap). The right-click `menu` action is not covered. |
| `tests/qemu-e2e/run-frame-buttons.sh` | A window rule's `frame.button_layout` changes the buttons on Gnoblin's native frame: the default three, close only, and `gnoblin.array {}` for none (counted as ink in the titlebar's right end of a Wayland GTK4 window with `GTK_CSD=0`). |
| `tests/qemu-e2e/run-preserve-active-window.sh` | `layer_shell.preserve_active_window`: with `true` a focused window stays focused when a layer-shell launcher (fuzzel) opens; with `false` the launcher takes focus. |
| `tests/qemu-e2e/run-unfocused.sh`   | The documented rule that dims unfocused windows follows focus when you click.  |
| `tests/qemu-e2e/run-animations.sh`  | A registered `open` animation fades a normal window in, and `dialog-open` fades a GTK modal dialog in. |
| `tests/qemu-e2e/run-blur.sh`        | A `blur` rule smooths a striped wallpaper behind a translucent terminal.       |
| `tests/qemu-e2e/run-opacity.sh`     | Prints a window pixel with and without an `opacity` rule. `run-regression.sh` checks the same thing. |
| `tests/qemu-e2e/run-location.sh`    | An experiment: asks GeoClue for location with and without its demo agent. It does not pass or fail yet. |

Both scripts read the run directory from `GNOBLIN_QEMU_RUN`, or from `/tmp/gnoblin-run`.
`GNOBLIN_PREFIX` selects the prefix inside the guest and defaults to
`build/validation-install`. Output is saved in the run directory.

The guest needs `pipewire`, `gnome-keyring`, `gpg`, `pinentry-gnome3`, `secret-tool`,
`swaybg`, `foot` and `ibus`. The prompt broker checks also need a default keyring that holds
a secret for `app prompt-e2e` and a GPG key `gnoblin-e2e@example.com` in `~/.gnupg-e2e`
whose passphrase is `gnoblin-keyring-test`. Create both once with
`tests/qemu-e2e/run-prompt-setup.sh`. It is safe to run again.

The scripts leave the test Lua configs in `~/.config/gnoblin/config` as disabled files.

## Reload and pointer checks

- `run-resize-drag.sh`: Super + right-button drag resizes from the nearest corner.
- `run-bell.sh`: `compositor.visual_bell` flashes the whole screen (`fullscreen-flash`) or only the window that rang the bell (`frame-flash`), and stays quiet when off.
- `run-audible-bell.sh`: `compositor.audible_bell` plays the bell sound (counts new PipeWire streams) and stays silent when off.
- `run-cursor-themes.sh`: cursor themes and sizes work with no hyprcursor installed (custom Xcursor theme, missing-theme fallback, size limits).
- `run-window-switching.sh`: Alt+Tab, Alt+Shift+Tab, Super+Tab and Alt+Esc move focus between windows with no shell running.
- `run-x11-fractional.sh`: a GTK X11 app keeps its logical size and gets scale factor 2 at a fractional monitor scale (1.25), and the layout is restored.
- `run-reload-effects.sh`: autostart entries start and stop on reload, `window_management.focus_mode` click versus hover changes focus on pointer entry, and `center_new_windows` centres or does not centre a new window, `raise_on_click` raises or does not raise a clicked window, `edge_tiling` tiles or does not tile a window dragged to the screen edge, `auto_maximize` maximizes or does not maximize a nearly monitor-sized new window, and `workspaces_only_on_primary` makes a second-monitor window follow the active workspace or stay put, `auto_raise` raises a window under a resting pointer under sloppy focus, and `focus_new_windows` strict or smart decides whether a new window takes focus.
- `run-work-area.sh`: monitor records carry `work_area` (whole monitor, minus a panel's exclusive zone) and `gnoblin.monitor.changed` reports it.
- `run-drag-constraint.sh`: with Waybar running, `window_management.constrain_drag_to_work_area` true keeps a window dragged downwards inside the work area, and false lets Mutter's 75 px rule apply.
- `run-input-method.sh`: a GTK4 Wayland text field gets Latin text with no `ibus-daemon`, and `привет` from `privet` with the Russian transliteration engine (needs `ibus`, `ibus-m17n` and `m17n-db` in the guest).
- `run-layer-focus.sh`: `layer_shell.preserve_active_window` keeps or moves focus when a layer surface opens.
- `run-logout-cycles.sh`: repeated `gnoblinctl logout` ends cleanly (no core dump, no compositor errors). Set `GNOBLIN_LOGOUT_CYCLES` to change the count.
- `guest-regression.sh` also covers protocol reload timing and Xwayland option reloads.
