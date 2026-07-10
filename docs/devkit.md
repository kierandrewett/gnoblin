# Devkit

The devkit is the fast loop for iterating on your own chrome (Quickshell,
waybar, a custom layer-shell client) against a real gnoblin compositor,
without touching your login session. It boots a **visible nested gnoblin
session** — a window inside your current Wayland session — plus a terminal
already wired to it.

For testing things that only make sense in a real login session (GDM, an
actual seat, unattended screensharing), see
[Real-hardware verification](real-hardware-verification.md) instead.

## Prerequisites

A build: `just dev`. You also need to be in a Wayland session yourself (the
devkit renders as a window in it) — see [Headless mode](#headless--scripting-mode)
if you're not.

## Running it

```sh
just gnome-devkit            # auto-detects foot/kitty/alacritty/wezterm/…
just gnome-devkit kitty      # or name a terminal explicitly
```

This boots gnome-shell in the `gnoblin` session mode using Mutter's
development-kit viewer (`--devkit`) — a window in your current session that
does **not** take the seat, so it coexists with your real desktop. Once
`org.gnoblin.Shell` comes up, a terminal opens. It renders as a window on
your host session, but its shell exports `WAYLAND_DISPLAY` so anything you
launch from it draws into the nested session, not your real desktop; see
[Isolation](#isolation) below for what else is (and isn't) separated from
your host session.

Environment variables:

| Variable | Default | Effect |
|---|---|---|
| `MONITOR` | `1600x900` | Nested virtual monitor resolution |
| `GNOME_DEVKIT_HEADLESS` | unset | `1` boots with no visible window (see below) |
| `GNOME_DEVKIT_EXEC` | unset | Run this command instead of opening a terminal, with children pointed at the nested display |

## Run your own chrome

From the terminal the devkit opens:

```sh
qs -p ~/dev/kobel-shell     # Quickshell
# or: waybar
# or: your own layer-shell client
```

Your bar/dock appears inside the nested gnoblin window, anchored via
`zwlr_layer_shell_v1`.

> If Quickshell warns *"built against Qt X but system has Qt Y … must be
> rebuilt"*, rebuild the Quickshell package first — a stale build crashes
> before it maps a surface.

## Drive gnoblin

`gnoblinctl` is on `PATH` inside the devkit terminal:

```sh
gnoblinctl ping
gnoblinctl version
gnoblinctl reload
gnoblinctl features
gnoblinctl disable osd            # let your bar's OSD own volume/brightness popups
```

Full command + feature reference in [Configuration](configuration.md).

## Isolation

The devkit is a sandbox, not a preview window into your real session:

- `DISPLAY` is unset and `GDK_BACKEND`/`QT_QPA_PLATFORM`/`CLUTTER_BACKEND`
  are forced to `wayland`, so anything you launch connects to the nested
  compositor, not your host's Xwayland.
- It runs on its own D-Bus session bus, shared by the nested shell, the
  terminal, and anything you launch from it — not your host session bus.
  `devkit_dbus.py` copies in only the services the devkit needs (portal
  bits, dconf for cross-process GSettings notification); a host service
  it doesn't list isn't reachable from inside the devkit.
- No host accessibility bus, no gvfs mounts.
- `HOME` and `XDG_RUNTIME_DIR` are left as your real ones (not scrubbed),
  so config that reads them directly (not over D-Bus) behaves normally.

This is also what `run-gnome-devkit.sh` isolates for the headless regression
test (`just gnome-devkit-verify`) — see [Testing](testing.md).

## Ending the session

Close the terminal. The script tears the nested session down on exit (or on
Ctrl-C/TERM).

## Headless / scripting mode

```sh
GNOME_DEVKIT_HEADLESS=1 GNOME_DEVKIT_EXEC='gnoblinctl ping' just gnome-devkit
```

Boots with no visible window and runs `GNOME_DEVKIT_EXEC` with its children
pointed at the nested display instead of opening a terminal, then exits.
This is exactly what `just gnome-devkit-verify` does to regression-test the
devkit's env plumbing (isolated bus + `gnoblinctl` reaching
`org.gnoblin.Shell`) without needing a Wayland session or a terminal
emulator — useful in CI or over SSH.

## Troubleshooting

- **`no host WAYLAND_DISPLAY`** — the devkit renders into your current
  Wayland session; log into one, or use `GNOME_DEVKIT_HEADLESS=1`.
- **`no gnome-shell in ./install`** — run `just dev` first.
- **`Failed to take control of the session: EBUSY`** — you're hitting this
  from the native/KMS backend, not `--devkit`; `run-gnome-devkit.sh` always
  uses `--devkit` precisely to avoid fighting your real session for the
  seat, so this should only come up if you're invoking `gnome-shell`
  directly.
- **no terminal found** — install foot, kitty, or alacritty, or pass one
  explicitly (`just gnome-devkit <terminal>`).
