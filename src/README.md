# Source map

Gnoblin's session runs a Lua supervisor alongside its Mutter compositor.
Gnoblin owns the runtime, compositor plugin, protocols, configuration, and
session services in this tree. It does not launch GNOME Shell or require GJS.

## Layout

Each directory holds one area of the product. Directories that hold overlay
sources have a `manifest` that names the files copied into a pinned upstream
tree (Mutter or the GNOME portal backend).

- `agents/` — session agents the compositor hosts: the polkit authentication
  agent, the location agent, the PipeWire monitor, the keyring and GPG
  passphrase prompter, and the privacy snapshot behind the `auth.*`,
  `prompt.*` and privacy methods.
- `cli/` — the `gnoblinctl` command.
- `compositor/` — code loaded into the Mutter process.
    - `plugin/` — the Gnoblin Mutter plugin entry point and its console and
      recovery panels.
    - `effects/` — window effects drawn by the compositor.
- `config/` — configuration loading and the shared schema.
    - `lua/` — the Lua configuration loader, parser, and pattern matching used
      by the runtime and the portal backend.
- `control/` — the private runtime protocol: the native control socket, cached
  configuration snapshots, and the operation files that serve it.
    - `gnoblin-native-control.c` — the socket server and method dispatch.
    - `gnoblin-control-internal.h` — shared state struct and helper declarations for the split files.
    - `gnoblin-control-command.c` — command spawning: the single spawn worker, child-exit watches, and `command.run` argument parsing.
    - `gnoblin-control-capture.c` — the `command.capture` operation.
    - `gnoblin-control-runtime.c` — runtime worker protocol helpers: reload document comparison, resume matching, config results, queue flushing and request cancel.
    - `windows/` — window rules, window drag and snap offers, workspace and
      monitor snapshots, and layer snapshots with their events.
- `cursor/` — cursor theme and Hyprcursor support.
- `data/` — the session desktop entry, user service units, schema defaults,
  shader example, and initial Lua configuration.
- `input/` — native input configuration and routing: input device settings,
  the input disposition, the touchpad router, the keybinding catalog, XKB and
  IBus input sources, and dynamic shortcut bindings.
    - `method/` — the IBus input method that gives Wayland text-input clients
      preedit and commit, built when `libibus` is available.
- `portal/` — portal support.
    - `interfaces/` — the portal D-Bus interface XML.
    - `backend/` — Gnoblin portal policy, identity, file chooser, email and
      inhibit helpers, with the portal service units in `units/`.
    - `grants/` — the portal grant cache and `grant.*` operations.
- `protocols/` — Mutter overlays for Gnoblin's Wayland protocols. The
  directory manifests describe the files copied into the pinned Mutter tree.
- `session/` — the Lua runtime supervisor, compositor startup, and idle
  service.
- `tools/` — the `gnoblin` login launcher, environment setup, configuration
  seeder, and the frame renderer client.

## Common changes

- Add or update a Wayland protocol in `protocols/`, then update the protocol
  aggregator and regenerate the Mutter patch series.
- Change configuration loading or Lua patterns in `config/` and keep the
  standalone runtime behavior aligned with the compositor snapshot interface.
- Change startup or child process handling in `session/`; the parent runtime
  owns Lua and supervises the Mutter process.
- Change session units or first-login configuration in `data/` and `tools/`.
  `cmake/install-session.cmake` installs the session into the selected prefix.
