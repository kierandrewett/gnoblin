# Source map

Gnoblin's session runs a Lua supervisor alongside its Mutter compositor.
Gnoblin owns the runtime, compositor plugin, protocols, configuration, and
session services in this tree. It does not launch GNOME Shell or require GJS.

## Top-level areas

- `compositor/` — the Gnoblin Mutter plugin entry point.
- `config/` — the Lua configuration loader, parser, shared schema, and pattern
  matching used by the runtime.
- `input-method/` — the IBus input method that gives Wayland text-input clients
  preedit and commit, built when `libibus` is available.
- `native-control/` — the private runtime protocol, cached configuration
  snapshots, and native input routing shared with the compositor.
  - `gnoblin-control-internal.h` — shared state struct and helper declarations for the split files.
  - `gnoblin-control-auth.c` — polkit agent glue behind the `auth.*` methods.
  - `gnoblin-control-prompts.c` — the keyring and GPG passphrase prompter behind the `prompt.*` methods.
  - `gnoblin-control-capture.c` — the `command.capture` operation.
  - `gnoblin-control-privacy.c` — the privacy snapshot, screen-sharing handle tracking, and location authorization requests.
  - `gnoblin-control-grants.c` — the portal grant cache and `grant.*` operations, and the session activity query.
  - `gnoblin-control-input-sources.c` — the XKB and IBus input sources, per-window source memory, and the input source snapshot and events.
  - `gnoblin-control-command.c` — command spawning: the single spawn worker, child-exit watches, and `command.run` argument parsing.
  - `gnoblin-control-workspaces.c` — workspace and monitor snapshots, their created, changed, removed and activated events, and the signal watches on the workspace and monitor managers.
  - `gnoblin-control-layers.c` — layer snapshots and layer events, and the input-device added and removed events.
  - `gnoblin-control-shortcuts.c` — dynamic shortcut bindings (shortcut.bind, unbind, session end), bare-Super arming and the lookup and cleanup of bound shortcuts.
  - `gnoblin-control-window-rules.c` — window rule matching and application, frame style and corner values, the corner toolkit probe, window shadows, shader files and rule effects.
  - `gnoblin-control-window-drag.c` — window drag state, `gnoblin.window.drag.*` events and snap offers.
  - `gnoblin-control-runtime.c` — runtime worker protocol helpers: reload document comparison, resume matching, config results, queue flushing and request cancel.
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
