# Gnoblin

Gnoblin is the floating window manager and desktop tooling you can build a
Wayland desktop on. It manages windows, workspaces, input and desktop services;
you choose the bar, dock, launcher and other visible shell components. A regular
GNOME login remains available separately.

## Get started

1. [Install Gnoblin](installation.md).
2. [Choose a shell](bring-your-own-shell.md).
3. [Configure your desktop](configuration.md).
4. [Build a desktop from Gnoblin's primitives](build-a-desktop.md).

## Make it yours

- [Shortcuts](shortcuts.md) — launch commands and change keys.
- [Window rules](window-rules.md) — choose which windows a setting affects.
- [Effects](window-effects.md) — blur, corners, borders and shadows.
- [Titlebars](window-frames.md) — client and server decorations.
- [Animations](animations.md) — timing and motion.
- [All settings](configuration-reference.md) — find an option.

## Build a shell or integration

| Need | Start here |
| --- | --- |
| Read and control windows from a program | [gnoblinctl](gnoblinctl.md) or the [compositor bridge](compositor-bridge.md) |
| Wire up a bar, dock or launcher | [Shell integration](shell-integration.md) and [bridge examples](bridge-examples.md) |
| Use a Wayland client protocol | [Protocol catalog](wayland-protocols.md) |
| React inside GNOME Shell | [User scripts](user-scripts.md) |
| Choose which application gets an effect | [Window rules](window-rules.md) |

Gnoblin owns the compositor interfaces. Bingux is one independent project that
uses them; the same interfaces are available to your shell.

## Need help?

Start with [troubleshooting](troubleshooting.md). For custom automation, see
[user scripts](user-scripts.md). Shells use the built-in
[compositor bridge](compositor-bridge.md) and can use [gnoblinctl](gnoblinctl.md).
