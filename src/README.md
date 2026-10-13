# Source map

Gnoblin's session runs a Lua supervisor alongside its Mutter compositor.
Gnoblin owns the runtime, compositor plugin, protocols, configuration, and
session services in this tree. It does not launch GNOME Shell or require GJS.

## Entry points

Each program starts in one file. Start reading there.

| Program               | Entry file                           | What it does                                                                                                |
| --------------------- | ------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `gnoblin`             | `main.c`                             | Runs the Lua supervisor from `session/`. With `--gnoblin-runtime-fd` it runs the Mutter compositor instead. |
| `gnoblin-idle`        | `idle.c`                             | The idle service that the user session target starts.                                                       |
| `gnoblinctl`          | `cli/gnoblinctl.c`                   | The command line client for the control socket.                                                             |
| compositor plugin     | `compositor/plugin/gnoblin-plugin.c` | The Mutter plugin loaded into the compositor.                                                               |
| frame renderer client | `tools/frame-renderer/client.c`      | The example client for the window frame protocol.                                                           |

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
    - `gnoblin-native-control.c` — control object setup, the socket server start and stop, and policy checks.
    - `gnoblin-native-control-private.h` — types and declarations that the files below share.
    - `gnoblin-control-internal.h` — shared state struct and helper declarations for the operation files.
    - `gnoblin-control-focus.c` — focus and menu contexts, pointer buttons and trusted shortcuts.
    - `gnoblin-control-input-config.c` — native input configuration, tablet and orientation lock, and monitor privacy.
    - `gnoblin-control-wire.c` — client buffers and JSON or variant encoding.
    - `gnoblin-control-input-routing.c` — capability snapshots, input devices, and input decisions.
    - `gnoblin-control-dispatch.c` — operation dispatch, shortcut capture and session lock.
    - `gnoblin-control-publish.c` — window, appearance, privacy and location snapshots and publication.
    - `gnoblin-control-events.c` — touchpad actions, config and native events, and socket focus.
    - `gnoblin-control-text-snap.c` — socket text insertion and snap commits.
    - `gnoblin-control-launches.c` — launch tracking.
    - `gnoblin-control-monitors.c` — monitor snapshots, the on-screen display and window properties.
    - `gnoblin-control-queue.c` — runtime queues, animation and read APIs, and UI sessions.
    - `gnoblin-control-requests.c` — the socket request handler.
    - `gnoblin-control-signals.c` — response writing and Mutter signal watches.
    - `gnoblin-control-connection.c` — client connections and request reading.
    - `gnoblin-control-runtime-channel.c` — the channel to the runtime worker: dispatch, flush and republish.
    - `gnoblin-control-api.c` — the public `gnoblin_native_control_*` functions.
    - `gnoblin-control-runtime.c` — runtime worker protocol helpers: reload document comparison, resume matching, config results, queue flushing and request cancel.
    - `gnoblin-control-command.c` — command spawning: the single spawn worker, child-exit watches, and `command.run` argument parsing.
    - `gnoblin-control-capture.c` — the `command.capture` operation.
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
- `session/` — the Lua runtime supervisor, compositor startup, autostart, and
  scheduling priority.
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
