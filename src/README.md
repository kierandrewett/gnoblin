# Source map

Gnoblin's session runs a Lua supervisor alongside its Mutter compositor.
Gnoblin owns the runtime, compositor plugin, protocols, configuration, and
session services in this tree. It does not launch GNOME Shell or require GJS.

## Top-level areas

- `compositor/` — the Gnoblin Mutter plugin entry point.
- `config/` — the Lua configuration loader, parser, shared schema, and pattern
  matching used by the runtime.
- `native-control/` — the private runtime protocol, cached configuration
  snapshots, and native input routing shared with the compositor.
- `permissions/` — Gnoblin portal policy and permission helpers.
- `protocols/` — Mutter overlays for Gnoblin's Wayland protocols. The
  directory manifests describe the files copied into the pinned Mutter tree.
- `session/` — the Lua runtime supervisor, compositor startup, and idle
  service.
- `data/` — the session desktop entry, user service units, schema defaults,
  shader example, and initial Lua configuration.
- `tools/` — the `gnoblin` login launcher, environment setup, configuration
  seeder, and `gnoblinctl` command.

## Common changes

- Add or update a Wayland protocol in `protocols/`, then update the protocol
  aggregator and regenerate the Mutter patch series.
- Change configuration loading or Lua patterns in `config/` and keep the
  standalone runtime behavior aligned with the compositor snapshot interface.
- Change startup or child process handling in `session/`; the parent runtime
  owns Lua and supervises the Mutter process.
- Change session units or first-login configuration in `data/` and `tools/`.
  `scripts/install-session.sh` installs the session into the selected prefix.
