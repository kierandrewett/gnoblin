---
layout: home
hero:
  name: Gnoblin
  text: A floating Wayland desktop
  tagline: Keep GNOME's window management. Choose the shell and desktop you want to build.
  actions:
    - theme: brand
      text: Get started
      link: /installation
    - theme: alt
      text: Build a desktop
      link: /build-a-desktop
features:
  - title: Choose your shell
    details: Use the bar, dock, launcher and desktop shell that fit your workflow.
    link: /bring-your-own-shell/
  - title: Shape every window
    details: Configure rules, titlebars, effects, cursors and animations.
    link: /configuration/
  - title: Build on open interfaces
    details: Connect shell tools through the compositor bridge and Wayland protocols.
    link: /shell-integration/
---

Gnoblin manages windows, workspaces, input and desktop services. Your chosen
shell supplies the visible desktop; a regular GNOME login remains available.

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
| Understand the planned compositor-owned lock | [Session locking design](session-lock-design.md) |
| Add a compositor-side integration | [Gnoblin scripts](user-scripts.md) |
| Choose which application gets an effect | [Window rules](window-rules.md) |

Gnoblin owns the compositor interfaces. Bingux is one independent project that
uses them; the same interfaces are available to your shell.

## Need help?

Start with [troubleshooting](troubleshooting.md). For custom automation, see
[user scripts](user-scripts.md). Shells use the built-in
[compositor bridge](compositor-bridge.md) and can use [gnoblinctl](gnoblinctl.md).
