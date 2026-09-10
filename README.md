<div align="center">

# Gnoblin

GNOME with layer-shell support. Bring your own desktop shell.

[Install](docs/installation.md) · [Choose a shell](docs/bring-your-own-shell.md) · [Configure](docs/configuration.md) · [Contribute](CONTRIBUTING.md)

</div>

Gnoblin replaces GNOME's panel, dash and overview with a shell of your choice,
such as [Bingux](https://github.com/kierandrewett/bingux). You can also put
together your own setup with Waybar and other layer-shell clients.

Built on Mutter and GNOME Shell, it keeps GNOME's window management, hardware
integration, lock screen and desktop services. Select Gnoblin at login; the
regular GNOME session stays available.

## What's different from GNOME?

| | GNOME | Gnoblin |
| --- | --- | --- |
| Desktop | GNOME's panel, dash and Activities overview | Your choice of shell, panels and launchers |
| Bars and docks | Changes usually need Shell extensions | Separate applications using layer-shell |
| Window appearance | GNOME's built-in effects | Configurable blur, corners, shadows and animation rules |
| Configuration | Settings, GSettings and extension preferences | Those settings plus a live TOML config for Gnoblin features |
| Automation | GNOME's existing interfaces | Additional window-control APIs, commands and reloadable scripts |

Your applications still run on Mutter. GNOME's lock screen, password prompts,
keyring, network authentication and removable-drive handling remain. You can
keep using GNOME Settings for hardware and system preferences.

Gnoblin doesn't include a replacement desktop UI. Install a shell such as
Bingux, or choose the individual tools you want. Existing GNOME extensions
that depend on the panel or overview won't provide those features here.

## Choose your desktop

Use a complete shell or combine a bar, launcher and notification daemon from
different projects. Panels and docks run as separate applications through
layer-shell. They don't need to be GNOME Shell extensions.

Keep GNOME's screenshot tool and volume popups, or turn them off when your
chosen tools handle those jobs. GNOME's notification banners are hidden;
disable its notification service when using an external notification daemon.

## Supported protocols

Gnoblin adds these protocols to Mutter's existing Wayland support:

| Protocol | What it enables |
| --- | --- |
| `wlr-layer-shell` | Bars, docks, wallpapers and desktop overlays |
| `wlr-screencopy` | Capturing a screen or a selected region |
| `ext-foreign-toplevel-list` | Listing open windows, their titles and app IDs |
| `wlr-foreign-toplevel-management` | Focusing, minimising, maximising and closing windows from a dock or taskbar |
| `ext-data-control` | Clipboard managers, including primary selection |
| `ext-idle-notify` | Detecting inactivity and when you return |
| `wlr-gamma-control` | Colour-temperature tools that adjust display gamma |
| `wlr-output-power-management` | Switching displays on and off |

These are enabled by default in the Gnoblin session. Protocol settings take
effect at the next login. `ext-session-lock` and `wlr-output-management` are
not implemented; GNOME still handles locking and display configuration.

## Make it yours

Configure blur, rounded corners, shadows and animations. Apply window rules
per app, set your own shortcuts and choose which programs start at login.
Shell settings reload when you save `~/.config/gnoblin/gnoblin.toml`.

[Configuration](docs/configuration.md) · [Window effects](docs/window-effects.md)

## Go further

Use `gnoblinctl` to control windows and workspaces from scripts or keybindings.
Custom shells can use the D-Bus and socket APIs; reloadable JavaScript scripts
provide access to GNOME Shell and Mutter.

[Command-line tools](docs/gnoblinctl.md) · [Shell integration](docs/compositor-bridge.md)

## Try it

You can try Gnoblin in a nested window before changing your login session.
For everyday use, install it and select **Gnoblin** at the login screen, then
start your chosen shell.

Fedora RPMs, NixOS and source builds are covered in the installation guide.
The Fedora packages replace the Mutter and GNOME Shell builds used by both
sessions.

[Installation and package status](docs/installation.md) · [Documentation](docs/README.md)

Built from [Mutter](https://gitlab.gnome.org/GNOME/mutter) and
[GNOME Shell](https://gitlab.gnome.org/GNOME/gnome-shell).
