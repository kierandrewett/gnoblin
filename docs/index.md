---
layout: home
hero:
  name: Gnoblin
  text: Windows, workspaces, input.
  tagline: Gnoblin provides the compositor and desktop services. You choose the shell, bar, dock and launcher.
---

<nav class="home-paths" aria-label="Start here">
  <a href="/gnoblin/installation/">
    <span class="home-paths__step">01 / Set up</span>
    <strong>Install Gnoblin</strong>
    <span>Fedora, Debian, Ubuntu, NixOS or from source</span>
  </a>
  <a href="/gnoblin/bring-your-own-shell/">
    <span class="home-paths__step">02 / Pick a shell</span>
    <strong>Bring your own desktop</strong>
    <span>Keep GNOME Shell or connect another shell</span>
  </a>
  <a href="/gnoblin/config">
    <span class="home-paths__step">03 / Make it yours</span>
    <strong>Configure Gnoblin</strong>
    <span>Set shortcuts, window rules and effects</span>
  </a>
</nav>

## Configuration reference

[All settings](/config/reference) · [Shortcuts](/config/shortcuts) · [Window rules](/config/window_rules) · [Effects](/config/window_effects) · [Titlebars](/config/window_frames) · [Animations](/config/animations)

## Shell and compositor APIs

Shells and tools can use:

- [Control windows with `gnoblinctl`](gnoblinctl.md)
- [Connect through the compositor bridge](compositor-bridge.md) · [Examples](bridge-examples.md)
- [Use a Wayland protocol](wayland-protocols.md)
- [Write compositor-side scripts](user-scripts.md)

## Troubleshooting

[Troubleshooting guide](troubleshooting.md)
