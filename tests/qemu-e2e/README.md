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
- `run-layer-focus.sh`: `layer_shell.preserve_active_window` keeps or moves focus when a layer surface opens.
- `run-logout-cycles.sh`: repeated `gnoblinctl logout` ends cleanly (no core dump, no compositor errors). Set `GNOBLIN_LOGOUT_CYCLES` to change the count.
- `guest-regression.sh` also covers protocol reload timing and Xwayland option reloads.
