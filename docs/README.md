# gnoblin documentation

gnoblin is patched GNOME: a pinned Mutter + GNOME Shell with a small patch
set that turns GNOME Shell into a compositor you draw your own chrome on top
of. Start with the root [README](../README.md) for the architecture overview
(what's patched, the overlay/patch split, the `org.gnoblin.Shell` control
protocol). The guides here cover the day-to-day mechanics.

## Guides

- **[Installation](installation.md)** — build dependencies, `just init` /
  `just dev`, installing the session, and picking Gnoblin at the login
  manager. Start here if you don't have a build yet.
- **[Devkit](devkit.md)** — `just gnome-devkit`, the fastest loop for
  iterating on your own chrome without logging out of your real session.
- **[Testing](testing.md)** — what each `just *-verify` recipe proves, what
  it needs (a build? a real GPU?), and when to run it.
- **[Configuration](configuration.md)** — `gnoblin.conf` (protocol gating),
  the `org.gnoblin.shell` GSettings schema (feature toggles), and the
  `gnoblinctl` command reference.
- **[Real-hardware verification](real-hardware-verification.md)** — the
  checklist for the things the headless suite can't cover: logging in at
  GDM, unattended screensharing, the gnoblin Settings panel.

## Source map

[`src/README.md`](../src/README.md) indexes `src/` by ownership boundary
(protocol overlays, the config parser, the control-protocol component,
runtime data) with pointers to the READMEs inside each protocol directory.
