<div align="center">

# Gnoblin

A standalone Mutter-based compositor and session runtime.

[Documentation](https://gnoblin.org/) · [Install](docs/installation.md) · [Choose a shell](docs/bring-your-own-shell.md) · [Configure](docs/config.md) · [Protocols](#supported-protocols) · [Contribute](CONTRIBUTING.md)

</div>

Gnoblin provides window management and Wayland interfaces for an external
desktop shell such as [Bingux](https://github.com/kierandrewett/bingux), or a
setup built from Waybar and other layer-shell clients.

Gnoblin is built on Mutter. It does not ship GNOME Shell or a desktop shell UI.
Select Gnoblin at login; the regular GNOME session stays available.

## Features

- **Bring your own chrome.** Use [Bingux](https://github.com/kierandrewett/bingux),
  [Waybar](https://github.com/Alexays/Waybar) or another layer-shell client
  for the bar, dock, launcher and notifications. Gnoblin does not impose a
  replacement desktop shell.
- **Layer-shell first.** `zwlr_layer_shell_v1` version 5 supports panels,
  docks, wallpapers, launchers and overlays, including layer popups and
  exclusive zones. See [bring-your-own-shell](docs/bring-your-own-shell.md).
- **Cursor themes.** [Configure compositor cursors](docs/guides/cursors.md) with
  the generic `cursor.theme` and `cursor.size` settings in Gnoblin's live
  config. Xcursor themes work on every install, and an uninstalled theme falls
  back to the default cursor. Builds with Hyprcursor support also read
  Hyprcursor themes, with animated frames that preserve hotspots and timing.
- **Compositor effects.** Configure [blur, opacity, rounded corners, borders,
  shadows, custom shaders and layer animations](docs/guides/window_effects.md) with
  window rules.
- **Live configuration.** Edit [one Lua file](docs/config.md) for window rules, shortcuts,
  autostart, protocol gates and animation behavior. Reload applies the settings
  supported by the runtime; startup-only changes need a new session.
- **Window control and scripting.** [`gnoblinctl`](docs/gnoblinctl.md) exposes windows, workspaces,
  monitors and input sources. Lua callbacks and the user-private compositor
  bridge expose supported runtime state and actions.
- **External desktop controls.** A separate shell or desktop clients provide
  bars, docks, launchers, notifications and other visible controls.
- **Prompts for your shell.** Gnoblin can act as the polkit agent and can
  serve keyring and GPG passphrase prompts. Each request reaches Lua as an
  event, so your shell draws the dialog and sends the answer back. Both are
  opt-in. See [authentication](docs/shell-api/authentication.md) and
  [passphrase prompts](docs/shell-api/passphrase-prompts.md).
- **Portal permissions.** The optional [Gnoblin portal backend](docs/guides/permissions.md) supports persistent,
  identity-checked rules for Screen Cast, Remote Desktop, input capture,
  screenshots and Access. Stock GNOME keeps its normal portal behaviour.
- **Lua runtime.** Configure the compositor and handle supported runtime events
  with Lua. See [how Lua configuration works](docs/user-scripts.md).

## Supported protocols

Added to Mutter's existing Wayland support, enabled by default:

| Protocol                          | Used for                                                       |
| --------------------------------- | -------------------------------------------------------------- |
| `wlr-layer-shell`                 | Bars, docks, wallpapers and overlays                           |
| `ext-background-effect-v1`        | [Client-requested background blur](docs/background-effects.md) |
| `wlr-screencopy`                  | Screen and region capture                                      |
| `ext-foreign-toplevel-list`       | Window lists, titles and app IDs                               |
| `wlr-foreign-toplevel-management` | Dock and taskbar window controls                               |
| `ext-data-control`                | Clipboard and primary-selection managers                       |
| `ext-idle-notify`                 | Idle detection                                                 |
| `wlr-gamma-control`               | Display gamma and colour temperature                           |
| `wlr-output-power-management`     | Display power control                                          |
| `ext-session-lock`                | Compositor-enforced session locking                            |

Gnoblin exposes standard `ext-session-lock-v1` in its own session. It does not
ship a built-in lock screen or choose a locker. Bingux supplies its lock client
independently; hyprlock, swaylock, gtklock, waylock and other conforming
clients can use the same protocol. A normal GNOME session continues to use
GNOME's own lock screen. See [session locking](docs/session-lock.md).

Existing authorised portal monitor streams remain subject to their established
portal permission. While locked, those streams show the lock scene. Remote
input is available only after `locked` and the lock
scene has presented, and is routed to the active lock surface; it is refused
during transitions and failsafe. No lock-specific portal setting or opt-in is
required.

## Get started

Download the source tarball from a [Gnoblin release](https://github.com/kierandrewett/gnoblin/releases),
extract it, and run `./build.sh` from the extracted directory. A Git checkout
uses the same command. The build uses compatible development libraries from
your distribution and keeps the compositor and native runtime in a private
prefix. Add `--with-portal` to build Gnoblin's GTK-based portal backend too.

After building, run `./build.sh --register-session` to add the lean login to
the session picker. The [source instructions](docs/install-source.md) cover
prerequisites and login setup.

[Install](docs/installation.md) on Fedora, Arch, openSUSE, NixOS or from source, or
[try a nested session](docs/devkit.md) without logging out.
On a minimal installation, [add GNOME application services](docs/gnome-apps.md)
only if you need them.

Every install method adds Gnoblin alongside GNOME. Your existing GNOME binaries
and login session stay in place; choose either session at login.

[Configuration](docs/config.md) · [Window effects](docs/guides/window_effects.md) · [Lua config](docs/user-scripts.md) · [All docs](docs/index.md)

Built from the [Gnoblin Mutter fork](https://github.com/kierandrewett/gnoblin-mutter),
based on [upstream Mutter](https://gitlab.gnome.org/GNOME/mutter), and the GNOME
desktop portal backend.
