<div align="center">

# Gnoblin

A no-frills fork of GNOME Shell with layer-shell support.

[Install](docs/installation.md) · [Choose a shell](docs/bring-your-own-shell.md) · [Configure](docs/configuration.md) · [Protocols](#supported-protocols) · [Contribute](CONTRIBUTING.md)

</div>

Gnoblin replaces GNOME's panel, dash and overview with a shell of your choice,
such as [Bingux](https://github.com/kierandrewett/bingux). You can also put
together your own setup with Waybar and other layer-shell clients.

Built on Mutter and GNOME Shell, it keeps GNOME's window management, hardware
integration, lock screen and desktop services. Select Gnoblin at login; the
regular GNOME session stays available.

## Features

- **Bring your own chrome.** Use [Bingux](https://github.com/kierandrewett/bingux),
  [Waybar](https://github.com/Alexays/Waybar) or another layer-shell client
  for the bar, dock, launcher and notifications. Gnoblin does not impose a
  replacement desktop shell.
- **Layer-shell first.** `zwlr_layer_shell_v1` version 5 supports panels,
  docks, wallpapers, launchers and overlays, including layer popups and
  exclusive zones. See [bring-your-own-shell](docs/bring-your-own-shell.md).
- **Hyprcursor support.** [Mutter loads Hyprcursor themes](docs/cursors.md)
  before falling back to Xcursor. Vector and animated frames scale to the
  requested cursor size and monitor scale while preserving hotspots and
  timing; the same themed wait cursor is used for launch feedback.
- **Compositor effects.** Configure [blur, opacity, rounded corners, borders,
  shadows, custom shaders and layer animations](docs/window-effects.md) with
  window rules.
- **Live configuration.** Edit [one Lua file](docs/configuration.md) for window rules, shortcuts,
  autostart, protocol gates, animation and feature ownership. Valid changes
  reload without restarting applications.
- **Window control and scripting.** [`gnoblinctl`](docs/gnoblinctl.md) exposes windows, workspaces,
  monitors, input sources, feature toggles, reloads and a user-private
  compositor bridge for desktop shells.
- **External desktop controls.** The external shell owns OSDs, capture controls
  and workspace feedback. Gnoblin forwards OSD requests without
  creating GNOME widgets. Native notifications and the keyboard-layout popup
  are disabled by default.
- **Desktop recovery.** Right-click the desktop to open a terminal or Settings.
  If no layer surface is visible for eight seconds, a native recovery panel
  appears. These tools work independently of the external shell.
- **Portal permissions.** The [optional portal backend](docs/permissions.md) supports persistent,
  identity-checked rules for Screen Cast, Remote Desktop, input capture,
  screenshots and Access. Stock GNOME keeps its normal portal behaviour.
- **Scriptable session.** Reload Gnoblin user scripts without replacing the
  compositor or disconnecting applications. GNOME Shell extensions and their
  management tools are removed from the Gnoblin session.

## Supported protocols

Added to Mutter's existing Wayland support, enabled by default:

| Protocol | Used for |
| --- | --- |
| `wlr-layer-shell` | Bars, docks, wallpapers and overlays |
| `wlr-screencopy` | Screen and region capture |
| `ext-foreign-toplevel-list` | Window lists, titles and app IDs |
| `wlr-foreign-toplevel-management` | Dock and taskbar window controls |
| `ext-data-control` | Clipboard and primary-selection managers |
| `ext-idle-notify` | Idle detection |
| `wlr-gamma-control` | Display gamma and colour temperature |
| `wlr-output-power-management` | Display power control |

GNOME still handles locking and display configuration. `ext-session-lock`
and `wlr-output-management` aren't implemented.

## Get started

[Install](docs/installation.md) on Fedora, NixOS or from source, or
[try a nested session](docs/devkit.md) without logging out.
Every install method adds Gnoblin alongside GNOME. Your existing GNOME binaries
and login session stay in place; choose either session at login.

[Configuration](docs/configuration.md) · [Window effects](docs/window-effects.md) · [Scripting](docs/gnoblinctl.md) · [All docs](docs/README.md)

Built from [Mutter](https://gitlab.gnome.org/GNOME/mutter) and
[GNOME Shell](https://gitlab.gnome.org/GNOME/gnome-shell).
