# Gnoblin documentation

Gnoblin provides a Wayland session built from Mutter and GNOME Shell.
It manages application windows and desktop services while a separate
layer-shell client provides the bar, dock and other desktop controls. The
session is intentionally usable with no shell running: log out or start your
own chrome when you are ready. Start with the [README](../README.md).

- [Installation](installation.md): build and install the session.
- [Bring your own shell](bring-your-own-shell.md): start Bingux, Waybar or a
  custom layer-shell client.
- [Distribution](distribution.md): package boundaries and repository publication.
- [Devkit](devkit.md): test in a nested session without logging out.
- [Configuration](configuration.md): `gnoblin.toml` and `gnoblinctl`.
- [Testing](testing.md): automated checks and release requirements.
- [Real-hardware verification](real-hardware-verification.md): login, graphics,
  screen sharing and other checks that need a physical session.
- [Source map](../src/README.md): owned source, patches and upstream dependencies.
