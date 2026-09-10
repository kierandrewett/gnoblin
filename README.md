<div align="center">

# Gnoblin

A no-frills fork of GNOME Shell with layer-shell support.

[Install](docs/installation.md) · [Choose a shell](docs/bring-your-own-shell.md) · [Configure](docs/configuration.md) · [Contribute](CONTRIBUTING.md)

</div>

Gnoblin replaces GNOME's panel, dash and overview with a shell of your choice,
such as [Bingux](https://github.com/kierandrewett/bingux). You can also put
together your own setup with Waybar and other layer-shell clients.

Built on Mutter and GNOME Shell, it keeps GNOME's window management, hardware
integration, lock screen and desktop services. Select Gnoblin at login; the
regular GNOME session stays available.

## What's different?

- Use your own shell, bar, dock and launcher.
- User GNOME extensions are disabled; apps and shell plugins provide desktop customisation.
- Blur, corners, shadows and animations are configurable without extensions.
- Set window rules, shortcuts and autostart in a live TOML config.
- Control windows through `gnoblinctl`, D-Bus, sockets or reloadable JavaScript.

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
