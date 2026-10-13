# Connection and Framing

Connect to the local compositor socket and exchange newline-delimited JSON requests.

The bridge API is versioned; the server greeting reports its supported version. These connection rules apply to API 1.x.

## Connect

Socket: `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Use `GNOBLIN_COMPOSITOR_SOCKET` on both ends for a private test path.

The server sends a greeting with its API version, current state revision,
methods, events, and capabilities. The `features` array is empty.

Send one UTF-8 JSON object per line, followed by a newline, and keep the
connection open.

`state_revision` is the latest revision for compositor state. `revision_scope`
names the kinds of state covered by that revision:

- windows, workspaces, monitors, and layer surfaces;
- input devices and input sources.

The revision advances when any covered state changes. Launch feedback uses a
separate revision on each launch record and `gnoblin.launch.changed` event.

Clients can request a protocol version with `api_version`, an object containing
integer `major` and `minor` fields. The equivalent top-level `api_major` and
`api_minor` integer fields are also accepted.

The major version must match the server. The requested minor version must be
zero or higher and no greater than the server’s advertised minor version. A
malformed or unsupported version closes the connection after the error reply.

Both fields are required together. If you omit the version, the connection uses
API 1.0 compatibility behavior. For this server, request major `1` and a minor
integer from `0` through `81`.

```json
{ "api_version": { "major": 1, "minor": 81 } }
```

Send a transport ping to confirm that the socket responds; it does not require
an API version:

```json
{ "op": "ping", "id": "health-check" }
```

The reply uses the matching ID and contains `{"pong":"pong"}` in `result`.

## Top-level operations

| `op`       | Request fields                             | Response                                       |
| ---------- | ------------------------------------------ | ---------------------------------------------- |
| `events`   | API version and event names                | Subscription acknowledgement and later events. |
| `windows`  | API version, when subscribing              | Initial snapshot and later window changes.     |
| `monitors` | API version, when subscribing              | Initial snapshot and later monitor changes.    |
| `api`      | API version, request ID, method, arguments | One result or error.                           |

The greeting advertises the available methods, events, capabilities, and API
version. Its `features` array is empty.

## Limits and disconnects

The socket directory is private to the user. Gnoblin supports up to 32 clients
and 32 bindings per client. Each connection can queue 64 records and 4 MiB of
output; requests are limited to 16 KiB.

Oversized input, malformed JSON, or a full output queue disconnects the client.
Other invalid requests return an error and keep the connection open. Disconnect
releases the client’s bindings and other resources.

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
| 1.14            | Portal grant listing and revocation                                                                   |
| 1.15            | Portal grant snapshots and lifecycle events                                                           |
| 1.16            | Permission policy snapshots and `gnoblin.permission.changed`                                          |
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

| Minimum version | Added methods or events                                                                                    |
| --------------- | ---------------------------------------------------------------------------------------------------------- |
| 1.37            | Lua snapshot reads for windows, workspaces, monitors, layers, and launches                                 |
| 1.38            | Adds `window.restore_or_minimize`; all supported clients use the Lua operation                             |
| 1.39            | `launches.snapshot` with collection revision                                                               |
| 1.40            | Shared `shortcuts.list` snapshot read                                                                      |
| 1.41            | Shared `shortcuts.actions` read backed by the installed Lua runtime                                        |
| 1.42            | Adds Lua-backed `permissions.list`; all client versions now use it                                         |
| 1.43            | Adds Lua-backed `permissions.check`; all client versions now use it                                        |
| 1.44            | Adds Lua-backed `permissions.policy`; all client versions now use it                                       |
| 1.45            | Adds the Lua-backed `portals.grants` read; all client versions now use it                                  |
| 1.46            | Adds shared Lua input reads; all client versions now use them                                              |
| 1.47            | Adds Lua-backed `privacy.state`; all client versions now use it                                            |
| 1.48            | Adds Lua operation for `window.restore_or_minimize`; all clients now use it                                |
| 1.49            | Lua runtime operation for `session.lock`                                                                   |
| 1.50            | Lua operations for `launch.begin` and `launch.end`                                                         |
| 1.51            | Added a Lua-backed response adapter; current builds answer directly from Mutter                            |
| 1.52            | Adds Lua-backed `workspace.list`; all client versions now use it                                           |
| 1.53            | Adds Lua-backed `window.list`; all client versions now use it                                              |
| 1.54            | Adds Lua-backed `launch.status`; all client versions now use it                                            |
| 1.55            | Lua-backed `shortcut.list` read                                                                            |
| 1.56            | Lua-backed `shortcut.actions` read                                                                         |
| 1.57            | Adds Lua-backed `layer.list`; all client versions now use it                                               |
| 1.58            | Adds Lua-backed `monitor.list`; all client versions now use it                                             |
| 1.59            | Adds the Lua-backed `window.match` read; all client versions now use it                                    |
| 1.60            | Basic legacy `window.action` requests use typed Lua operations for all supported client versions           |
| 1.61            | Legacy `window.action` adds a resize request routed through `window.resize`                                |
| 1.62            | Legacy `window.action` maps move to `window.move`                                                          |
| 1.63            | Adds `workspace` and `monitor` actions to `window.action`                                                  |
| 1.64            | `shortcut.session.end` ends an owned active session without removing its binding                           |
| 1.65            | `location.authorize_app` grants location access for a verified application identity                        |
| 1.66            | `input.orientation_lock` read and `input.set_orientation_lock` update                                      |
| 1.67            | Native status reads remain available while a Lua worker restarts                                           |
| 1.68            | `gnoblin.shortcut.session.key` may carry one-use focus authority                                           |
| 1.69            | Adds `gnoblin.window.activation-denied` for strict focus-policy denials                                    |
| 1.70            | Adds the shared `appearance.color_scheme` read                                                             |
| 1.71            | Layer lifecycle events                                                                                     |
| 1.72            | `gnoblin.runtime.status-changed`                                                                           |
| 1.73            | `gnoblin.input.pad-help-requested`                                                                         |
| 1.74            | `monitors.privacy_screen`, `monitors.set_privacy_screen`, and `gnoblin.monitor.privacy-screen-changed`     |
| 1.75            | Adds the `ui-sessions` capability for state and command messages between shell processes                   |
| 1.76            | Adds the `gnoblin.session.state-changed` event                                                             |
| 1.78            | Adds the `auth-agent` capability, `auth.begin`, `auth.respond`, `auth.cancel`, and `gnoblin.auth.*` events |
| 1.79            | Adds the `prompt-broker` capability, `prompt.respond`, `prompt.cancel`, and `gnoblin.prompt.*` events      |
| 1.80            | Adds the `gnoblin.pointer.locate-requested` event                                                          |
| 1.81            | Adds `work_area` to monitor records and to the monitor `changed` list                                      |
