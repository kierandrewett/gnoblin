# Connection and Framing

Connect to the local compositor socket and exchange newline-delimited JSON requests.

The bridge API is versioned. The server greeting reports the API version it supports. These connection rules apply to API 1.x.

## Connect

The socket path is set by one of these, in order:

- `GNOBLIN_COMPOSITOR_SOCKET`, when set. Use it on both ends for a private test path.
- `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`, the default. The `gnoblinctl` CLI falls back to `/run/user/<uid>/gnoblin/compositor-v1.sock` when `XDG_RUNTIME_DIR` is unset.

The server rejects a socket directory that is not owned by your user or that is accessible to other users.

After the connection opens, the server sends one greeting line with `"event": "hello"`. It has these fields:

| Field                               | Type             | Meaning                                                                                      |
| ----------------------------------- | ---------------- | -------------------------------------------------------------------------------------------- |
| `version`, `api_major`              | integer          | Major API version. Both fields hold the same value.                                          |
| `api_minor`                         | integer          | Highest minor version this server accepts.                                                   |
| `state_revision`                    | integer          | Revision of the state named in `revision_scope`.                                             |
| `revision_scope`                    | string           | The exact string `window-workspace-monitor-layer-input-device-source-capability-state`.      |
| `revision_semantics`                | string           | `monotonic-per-session`. The revision only increases for the life of the compositor session. |
| `methods`, `events`, `capabilities` | array of strings | Names this server supports. Read them from the greeting; they change with the API version.   |
| `features`                          | array            | Always empty.                                                                                |

`state_revision` advances when any state in its scope changes. That scope covers windows, workspaces, monitors, layer surfaces, input devices, and input sources. Launch feedback uses a separate revision on each launch record and on the `gnoblin.launch.changed` event.

Send one UTF-8 JSON object per line, followed by a newline. Keep the connection open for later requests and events.

## Request fields

| Field                    | Type    | Meaning                                                                                         |
| ------------------------ | ------- | ----------------------------------------------------------------------------------------------- |
| `op`                     | string  | Operation to run. See [top-level operations](#top-level-operations).                            |
| `id`                     | string  | Request ID, 1 to 64 bytes. The reply carries the same ID.                                       |
| `api_version`            | object  | Requested protocol version: `{ "major": <integer>, "minor": <integer> }`.                       |
| `api_major`, `api_minor` | integer | Top-level alternative to `api_version`. Use both together or use `api_version`, not both forms. |

The version rules:

- `major` must equal the server's major version, `1`.
- `minor` must be from `0` through the server's `api_minor`.
- If you omit the version, the connection uses API 1.0 compatibility behavior.
- A malformed or unsupported version gets an error reply, and the server then closes the connection.

```json
{ "api_version": { "major": 1, "minor": 86 } }
```

`ping` does not need an API version. It accepts only `op` and `id`:

```json
{ "op": "ping", "id": "health-check" }
```

The reply has the same `id` and contains `{"pong":"pong"}` in `result`.

## Top-level operations

| `op`       | Request fields                             | Response                                       |
| ---------- | ------------------------------------------ | ---------------------------------------------- |
| `ping`     | `id` only                                  | `{"pong":"pong"}` in `result`.                 |
| `events`   | API version and event names                | Subscription acknowledgement and later events. |
| `windows`  | API version, when subscribing              | Initial snapshot and later window changes.     |
| `monitors` | API version, when subscribing              | Initial snapshot and later monitor changes.    |
| `api`      | API version, request ID, method, arguments | One result or error.                           |

## Limits and disconnects

| Limit                               | Value    | What happens when it is exceeded                                |
| ----------------------------------- | -------- | --------------------------------------------------------------- |
| Request line                        | 64 KiB   | Error reply `request is too large`, then the connection closes. |
| Queued output per connection        | 1 MiB    | The connection closes.                                          |
| Event names in one `events` request | 64       | Error reply; the connection stays open.                         |
| Request `id`                        | 64 bytes | Error reply; the connection stays open.                         |

Malformed JSON and oversized input disconnect the client. Other invalid requests return an error and keep the connection open. Disconnect releases the client's bindings and other resources.

### API additions

| Minimum version | Added methods or events                                                                               |
| --------------- | ----------------------------------------------------------------------------------------------------- |
| 1.2             | `layer.list`                                                                                          |
| 1.3             | `input.devices` read                                                                                  |
| 1.4             | Input-device lifecycle events                                                                         |
| 1.5             | `shortcut.actions`                                                                                    |
| 1.6             | XKB input-source methods                                                                              |
| 1.7             | `launch.status`, `launch.begin`, `launch.end`, and `gnoblin.launch.changed`                           |
| 1.9             | `shortcut.list`, generic event subscriptions, and `gnoblin.input.gesture`                             |
| 1.10            | `gnoblin.shortcut.activated` focus grants and `window.focus`                                          |
| 1.11            | Connection-owned `shortcut.bind` and `shortcut.unbind`; canonical `gnoblin.operation.completed` event |
| 1.12            | Trusted interactive window grabs and their capability                                                 |
| 1.13            | `gnoblin.focus.policy-changed`                                                                        |
| 1.17            | Privacy state reads and `gnoblin.privacy.changed`                                                     |
| 1.18            | Animation registry, preview operations, and animation lifecycle events                                |
| 1.19            | Shared Lua reads for version, capabilities, focus, and settings                                       |
| 1.20            | Native `runtime.reload_config` for supported runtime settings                                         |
| 1.21            | `session.lock` and compositor-confirmed lock-state events                                             |
| 1.22            | Held and modal shortcut sessions, bare-Super capture, and forwarded key events                        |
| 1.23            | `window.thumbnail` and the `window-thumbnails` capability                                             |
| 1.24            | `session.activity`, activity-change events, and the `session-activity` capability                     |
| 1.26            | Pointer-drag lifecycle events and capability-bound `window.snap.offer`                                |
| 1.27            | `gnoblin.window.menu-requested` and `gnoblin.osd.requested`                                           |
| 1.28            | Connection-owned text targets and keyboard snap contexts                                              |
| 1.29            | `session.status` live-state read                                                                      |
| 1.30            | One-use WM menu capabilities for target-bound move and resize grabs                                   |
| 1.31            | Privacy session stop methods and the `layer.animation_policy` shared read                             |
| 1.32            | XDG Activation token focus and `session.logout`                                                       |
| 1.33            | `microphone-monitor` availability and `gnoblin.capability.changed`                                    |
| 1.34            | `gnoblin.appearance.color-scheme-changed`                                                             |
| 1.35            | Snake_case aliases for window snapshot fields                                                         |
| 1.36            | `gnoblin.shortcut.binding-deactivated` for press-triggered shortcuts                                  |

### API additions from 1.37

| Minimum version | Added methods or events                                                                                           |
| --------------- | ----------------------------------------------------------------------------------------------------------------- |
| 1.37            | Lua snapshot reads for windows, workspaces, monitors, layers, and launches                                        |
| 1.38            | Adds `window.restore_or_minimize`; all supported clients use the Lua operation                                    |
| 1.39            | `launches.snapshot` with collection revision                                                                      |
| 1.40            | Shared `shortcuts.list` snapshot read                                                                             |
| 1.41            | Shared `shortcuts.actions` read backed by the installed Lua runtime                                               |
| 1.46            | Adds shared Lua input reads; all client versions now use them                                                     |
| 1.47            | Adds Lua-backed `privacy.state`; all client versions now use it                                                   |
| 1.48            | Adds Lua operation for `window.restore_or_minimize`; all clients now use it                                       |
| 1.49            | Lua runtime operation for `session.lock`                                                                          |
| 1.50            | Lua operations for `launch.begin` and `launch.end`                                                                |
| 1.51            | Added a Lua-backed response adapter; current builds answer directly from Mutter                                   |
| 1.52            | Adds Lua-backed `workspace.list`; all client versions now use it                                                  |
| 1.53            | Adds Lua-backed `window.list`; all client versions now use it                                                     |
| 1.54            | Adds Lua-backed `launch.status`; all client versions now use it                                                   |
| 1.55            | Lua-backed `shortcut.list` read                                                                                   |
| 1.56            | Lua-backed `shortcut.actions` read                                                                                |
| 1.57            | Adds Lua-backed `layer.list`; all client versions now use it                                                      |
| 1.58            | Adds Lua-backed `monitor.list`; all client versions now use it                                                    |
| 1.59            | Adds the Lua-backed `window.match` read; all client versions now use it                                           |
| 1.60            | Basic legacy `window.action` requests use typed Lua operations for all supported client versions                  |
| 1.61            | Legacy `window.action` adds a resize request routed through `window.resize`                                       |
| 1.62            | Legacy `window.action` maps move to `window.move`                                                                 |
| 1.63            | Adds `workspace` and `monitor` actions to `window.action`                                                         |
| 1.64            | `shortcut.session.end` ends an owned active session without removing its binding                                  |
| 1.65            | `location.authorize_app` grants location access for a verified application identity                               |
| 1.66            | `input.orientation_lock` read and `input.set_orientation_lock` update                                             |
| 1.67            | Native status reads remain available while a Lua worker restarts                                                  |
| 1.68            | `gnoblin.shortcut.session.key` may carry one-use focus authority                                                  |
| 1.69            | Adds `gnoblin.window.activation-denied` for strict focus-policy denials                                           |
| 1.70            | Adds the shared `appearance.color_scheme` read                                                                    |
| 1.71            | Layer lifecycle events                                                                                            |
| 1.72            | `gnoblin.runtime.status-changed`                                                                                  |
| 1.73            | `gnoblin.input.pad-help-requested`                                                                                |
| 1.74            | `monitors.privacy_screen`, `monitors.set_privacy_screen`, and `gnoblin.monitor.privacy-screen-changed`            |
| 1.75            | Adds the `ui-sessions` capability for state and command messages between shell processes                          |
| 1.76            | Adds the `gnoblin.session.state-changed` event                                                                    |
| 1.78            | Adds the `auth-agent` capability, `auth.begin`, `auth.respond`, `auth.cancel`, and `gnoblin.auth.*` events        |
| 1.79            | Adds the `prompt-broker` capability, `prompt.respond`, `prompt.cancel`, and `gnoblin.prompt.*` events             |
| 1.80            | Adds the `gnoblin.pointer.locate-requested` event                                                                 |
| 1.81            | Adds `work_area` to monitor records and to the monitor `changed` list                                             |
| 1.84            | Reserved; no API changes                                                                                          |
| 1.85            | Replaces `window_management.center_new_windows` with `new_window_placement` (`free_space`, `center` or `cascade`) |
| 1.86            | Removes the permission and portal grant API and its events; see `CHANGELOG.md`                                    |
