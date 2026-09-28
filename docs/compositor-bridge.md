# Compositor bridge

Use this socket API to build a dock, launcher or window switcher. Set persistent
shortcuts in the [Lua config](/config/configure/shortcuts); use bridge bindings
for interactive UI while your client is connected. The bridge also lets your
shell read windows and request window actions.

For terminal commands and scripts, [gnoblinctl](gnoblinctl.md) handles the
connection for you.

## Availability

The bridge is a built-in Gnoblin compositor service. It starts with the
Gnoblin Shell component, stays available across `gnoblinctl reload`, and does
not need to be installed under `~/.config/gnoblin/scripts/`.

The native compositor preview started with `mutter --gnoblin-config PATH` has
a limited endpoint at the same socket path. It sends `hello` with an empty
`features` list and accepts multiple API requests on one connection. It
supports ping, window, monitor, layer, input-device, and XKB input-source
listing, workspace management by ID or number, and basic window actions.

The greeting lists the methods, events, and capabilities available in the
preview. `monitor.list` returns active connector names as IDs and the current
numeric `index` used by compatibility actions. For a logical monitor that
combines cloned outputs, Gnoblin uses the lexicographically first active
connector.

`layer.list` returns layer-shell surfaces with stable IDs, layer placement,
anchor, exclusive zone, keyboard-interactivity mode, geometry, and their
optional title and namespace. Each record includes `revision`; layer surfaces
do not have application IDs in the layer-shell protocol.

Send `{"op":"windows"}` to receive an initial snapshot and later snapshots as
windows change. Each snapshot has a monotonically increasing `revision` shared
by window, workspace, monitor, layer, and input-device state.

Send `{"op":"monitors"}` to receive the current monitor snapshot and later
snapshots when monitor state changes. A monitor snapshot has
`event: "monitors"`, a `monitors` array, and the current state `revision`.

Native monitor records include geometry, primary status, scale, index, and a
record revision. Only active logical monitors are listed. For cloned outputs,
the connector ID uses the first active connector alphabetically. An ID remains
usable while its monitor is listed; request another snapshot after outputs
change.

API version 1.1 subscriptions receive window, workspace, and monitor
lifecycle messages. `hello.events` lists available messages, and
`hello.capabilities` includes `window-lifecycle-events`,
`workspace-lifecycle-events`, and `monitor-lifecycle-events`.

API version 1.2 adds `layer.list` and the `layer-list` capability. Its optional
`monitor_id`, `namespace`, and `layer` arguments are exact string filters. Layer
snapshots use the same state revision as window, workspace, and monitor
snapshots. The preview does not send layer lifecycle events.
Request API version 1.2 when calling `layer.list`; older or versionless
requests receive an unsupported-version error. For example:

```json
{ "op": "api", "id": "layers", "api_version": { "major": 1, "minor": 2 }, "method": "layer.list", "arguments": {} }
```

### API versions 1.3 and 1.4: input devices

API version 1.3 adds `input.devices` and the `input-device-list` capability.
The method takes an empty `arguments` object and returns `{devices, revision}`.
Each device record also has a `revision`.

API 1.3 returns a one-time snapshot.

At API 1.4, `input.devices` also subscribes the connection after returning its
initial snapshot. The greeting advertises `input-device-lifecycle-events` and
the two event names. The subscription lasts until the connection closes.

Each event has `revision`, `sequence`, and monotonic `time`. Added events
include `device`; removed events include `device_id` and the final `last`
record.

Device IDs are stable only for the current compositor session. Device records
do not expose device paths or `enabled`; Mutter has no safe enabled-state
getter. Input-device changes advance the shared state revision. Request API
version 1.4 to receive lifecycle events:

```json
{
    "op": "api",
    "id": "input-devices",
    "api_version": { "major": 1, "minor": 4 },
    "method": "input.devices",
    "arguments": {}
}
```

### API version 1.5: shortcut actions

API version 1.5 adds `shortcut.actions` to list built-in Mutter and window
manager shortcut actions without GNOME Shell. Its optional `group` argument
accepts `wm`, `mutter`, or `wayland`. Omit it to list all installed groups.

Each result record contains `id`, `group`, `key`, and `default_bindings`. A
record also contains `description` when the schema provides one. Bindings come
from installed GSettings schema defaults and do not reflect user-overridden
bindings. Request API version 1.5:

```json
{
    "op": "api",
    "id": "shortcut-actions",
    "api_version": { "major": 1, "minor": 5 },
    "method": "shortcut.actions",
    "arguments": { "group": "mutter" }
}
```

### API version 1.9: configured shortcuts

API version 1.9 adds `shortcut.list`. It takes no arguments and returns named
shortcuts configured for the native compositor. Each record contains `name`,
`binding`, `enabled`, `trigger`, and `revision`.

A record also contains a `command` argument array or an `action` identifier.
One binding is returned as a string; multiple bindings are returned as an
array. A built-in action with no bindings has `enabled: false`. Disabled
declarations and shell integration shortcuts are not included.

```json
{
    "op": "api",
    "id": "shortcut-list",
    "api_version": { "major": 1, "minor": 9 },
    "method": "shortcut.list",
    "arguments": {}
}
```

### API version 1.6: XKB input sources

API version 1.6 adds methods for XKB input sources:

- `input.sources` returns configured layouts and variants as
  `{sources, revision}`.
- `input.current_source` returns `{available, source?, revision}`. `available`
  is false when the active keymap was not installed by Gnoblin.
- `input.select` selects a listed source.

A source read or selection subscribes the connection to input-source lifecycle
events.

`input.select` takes `{type, id}` from a listed source and completes
asynchronously. The connection receives `gnoblin.api.operation-completed` with
the request ID, method, and result or error. API 1.11 clients also receive
`gnoblin.operation.completed` with the operation ID and either `value` or a
structured error. Success means Mutter confirmed the keymap change.

Native IBus selection returns an unsupported error. Mutter loads at most four
layouts into one keymap. Gnoblin changes the active group when you select a
source outside it. The greeting advertises the input-source capabilities and
event names.

Source events include `revision`, `sequence`, and monotonic `time`.
`gnoblin.input.sources-changed` contains the updated `sources` array.
`gnoblin.input.source-changed` contains `available` and, when available, the
confirmed `source` record. Request API version 1.6:

```json
{
    "op": "api",
    "id": "input-sources",
    "api_version": { "major": 1, "minor": 6 },
    "method": "input.sources",
    "arguments": {}
}
```

Send `{"op":"windows","api_version":{"major":1,"minor":1}}` to subscribe.
Use `{"op":"monitors","api_version":{"major":1,"minor":1}}` for an
initial monitor snapshot and lifecycle events.

Monitor events carry `event`,
`revision`, `sequence`, and monotonic `time` fields. Added and changed messages
include a `monitor` record; changed messages also include `changed`. Removed
messages include `monitor_id` and the final `last` record.

The monitor `changed` array can contain `id`, `index`, `x`, `y`, `width`,
`height`, `primary`, `scale`, `enabled`, `name`, `make`, `model`, `serial`,
`refresh_rate`, or `transform`.

The server sends an initial window snapshot, then streams window and workspace
events on that connection. Window and workspace events carry `revision`,
`sequence`, and monotonic-clock `time` in microseconds. Event payload fields
match native Lua events, with `event` as the socket message name instead of
Lua's `name`.

| Event                                                                                   | Additional fields                                                                  |
| --------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `gnoblin.workspace.created`, `gnoblin.workspace.renamed`, `gnoblin.workspace.activated` | `workspace` record                                                                 |
| `gnoblin.workspace.changed`                                                             | `workspace` record and `changed` array (`number`, `window_count`, or `persistent`) |
| `gnoblin.workspace.removed`                                                             | `workspace_id` and `last` record                                                   |
| `gnoblin.workspace.window-moved`                                                        | `window_id`, `from_id`, and `to_id`                                                |

Native workspace records use `window_count`. Event payload fields match native
Lua workspace events.

| Event                                                | Additional fields                                     |
| ---------------------------------------------------- | ----------------------------------------------------- |
| `gnoblin.window.created`                             | `window` record                                       |
| `gnoblin.window.changed`                             | `window_id`, `changed` array, and `window` record     |
| `gnoblin.window.focused`, `gnoblin.window.unfocused` | `window_id` and `window` record                       |
| `gnoblin.window.attention-changed`                   | `window_id`, `window` record, and `demands_attention` |
| `gnoblin.window.closed`                              | `window_id` and `last` record                         |

The initial snapshot establishes the window-event baseline, so existing
windows do not trigger `created` events when a client subscribes. Workspace
state is not included in that snapshot; request `workspace.list` to establish
a workspace baseline.

The stream reports `unfocused` before `focused` when focus moves between
windows. Focus changes do not also emit `changed` solely because the focus
field changed. Versionless and 1.0 subscriptions receive snapshots only.

The preview does not provide other subscription types, bindings, or Shell commands.
Its workspace IDs belong to Mutter and follow their workspace objects. Its
window records use the GTK app ID or
WM class because Shell's application tracker is not active in this preview.

`gnoblinctl window list` uses the same socket. Check `gnoblinctl status` before
debugging a client connection.

If the socket pathname disappears or stops accepting connections while Gnoblin
is running, the bridge restores it within a few seconds. Clients should retry
their connection and register their bindings again after receiving `hello`.

Bingux is a separate shell project that uses this interface. A custom shell can
connect to it without installing Bingux.

![Bingux dock showing Files, Firefox and Foot in a Gnoblin session](images/gnoblin-bingux-firefox.png)

_Bingux is one shell example built on Gnoblin's compositor interfaces._

## Connect

Socket: `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Use `GNOBLIN_COMPOSITOR_SOCKET` on both ends for a private test path.

The server sends a greeting, including available features:

```json
{ "event": "hello", "version": 1, "features": ["ui-sessions", "switcher-fallback", "overlay-shortcut"] }
```

Additional features depend on the running build. Send one UTF-8 JSON object
per line, followed by a newline. Keep the connection open.

The native compositor preview greeting reports its API version and lists
supported methods, events, and capabilities. `state_revision` is the latest
revision for the preview state.

`revision_scope` names the window, workspace, monitor, layer, input-device,
and input-source state covered by that revision. It advances when any of those
states change. Launch feedback uses its own revision on each launch record and
`gnoblin.launch.changed` event.
The full shell bridge may advertise additional features.

Native preview clients may request an API version with `api_version`, which
contains `major` and `minor` fields. The equivalent `api_major` and `api_minor`
fields are also accepted.

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

### API version 1.9: event subscriptions

Send a request with `op: "events"` and an `events` array to subscribe this
connection to selected native events. Event names must appear in `hello.events`.

Use the existing subscription operations for the `windows` and `monitors`
snapshot names. The greeting lists `generic-event-subscriptions` and
`input-gesture-events` in `hello.capabilities`.

The list may contain up to 64 unique event names. A valid request replaces the
connection's previous event list. Send an empty list to unsubscribe from these
events. Invalid lists leave the previous subscription in place.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 9 },
    "events": ["gnoblin.input.gesture", "gnoblin.window.focused"]
}
```

The server acknowledges a valid request with `event: "subscribed"` and the
accepted event names. Requests without API version 1.9 receive an error.
Subscriptions last until replaced or the connection closes.

The stable `gnoblin.input.gesture` event comes from Mutter's
`mutter.touchpad.gesture` source. Its fields are:

- `gesture`, `phase`, `fingers`, `sequence`, `time`, and `input_time`.
- `dx` and `dy` for swipe events; `scale` and `angle_delta` for pinch events.

`input_time` is Mutter's original timestamp. Socket `sequence` values increase
across native events and may have gaps when a connection filters other events.
`time` is monotonic-clock microseconds. Device names and private focus contexts
are not sent.

### API version 1.10: shortcut focus grants

Subscribe to `gnoblin.shortcut.activated` with an API 1.10 event request. The
event is sent only to subscribed connections and includes the configured
command shortcut name, `trigger: "press"`, and a connection-specific
`focus_context` token. Built-in action shortcuts are not included.

The token expires five seconds after the key press and is bound to the
connection that received it. It can authorize one focus request or, with API
1.12, one interactive move or resize request.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 10 }, "events": ["gnoblin.shortcut.activated"] }
```

Use the received token with `window.focus` when the user selects a listed
window:

```json
{
    "op": "api",
    "id": "focus",
    "api_version": { "major": 1, "minor": 10 },
    "method": "window.focus",
    "arguments": { "id": "42", "focus_context": "TOKEN_FROM_SHORTCUT_EVENT" }
}
```

`window.focus` accepts exactly the string fields `id` and `focus_context`.
Every attempt with a valid connection token consumes it, including requests
with an invalid window ID or unknown argument. API 1.12 also accepts this token
for one interactive move or resize operation. The request fails if the token
expired, was already used, belongs to another connection, or was revoked.

Config reloads, session locks, subscription changes, and disconnects revoke
socket tokens. Native Lua `Window:focus(context)` uses its protected Lua
context instead of a socket token.

### API version 1.11: dynamic shortcut bindings

#### Register a binding

Use `shortcut.bind` to register a connection-owned global accelerator. The
`arguments` object accepts only `id` and `accelerator`.

- IDs contain 1 to 64 letters, digits, underscores, or hyphens.
- Accelerators use Mutter's GTK accelerator syntax and are limited to 128 bytes.
- Mutter rejects invalid accelerators and bindings already claimed by another
  owner.
- Bare `Super` is the overlay key and is not accepted by this method.
- Bindings activate on key press and ignore autorepeat.
- Each connection can register up to 32 bindings. The compositor accepts 128
  dynamic bindings in total.

The `hold`, `modal`, `trigger`, and `captureInput` options are rejected. This
API does not provide held-modifier sessions, modal input handling, or type-ahead
handoff.

```json
{
    "op": "api",
    "id": "bind-search",
    "api_version": { "major": 1, "minor": 11 },
    "method": "shortcut.bind",
    "arguments": { "id": "search", "accelerator": "<Super>space" }
}
```

Use `shortcut.unbind` with the same ID to release a binding. A client can remove
only its own IDs. Disconnecting the client releases all its bindings.

#### Receive activations

Subscribe to `gnoblin.shortcut.binding-activated` with API 1.11 to receive
activations for bindings owned by that connection. Each event includes the ID,
accelerator, `trigger: "press"`, and Mutter's input timestamp. It also includes
a monotonic timestamp and a connection-specific focus token. The token can
authorize one `window.focus`, `window.begin_move`, or `window.begin_resize`
request and expires after five seconds.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 11 }, "events": ["gnoblin.shortcut.binding-activated"] }
```

API 1.11 adds `gnoblin.operation.completed`, which carries an operation ID and
either a success value or a structured error. The legacy event remains for
older clients. Subscribe with `op: "events"`; clients tracking input-source or
shortcut-capture APIs also receive completion notifications. Match them by
operation ID.

Dynamic bindings do not replace the Shell `op: "bind"` protocol. Use that
protocol for held-modifier sessions, modal input, or type-ahead handoff.

### API version 1.12: interactive window grabs

API 1.12 adds `window.begin_move` and `window.begin_resize`. Both require the
connection-bound `focus_context` token from a trusted shortcut activation and
consume it on every attempt, including invalid arguments. A token can authorize
only one focus or interactive-grab operation.

Use `window.begin_move` to start Mutter's keyboard move grab for a listed
window:

```json
{
    "op": "api",
    "id": "move",
    "api_version": { "major": 1, "minor": 12 },
    "method": "window.begin_move",
    "arguments": { "id": "42", "focus_context": "TOKEN_FROM_SHORTCUT_EVENT" }
}
```

Use `window.begin_resize` with one of `north`, `south`, `east`, `west`,
`north_east`, `north_west`, `south_east`, or `south_west`:

```json
{
    "op": "api",
    "id": "resize",
    "api_version": { "major": 1, "minor": 12 },
    "method": "window.begin_resize",
    "arguments": { "id": "42", "edge": "south_east", "focus_context": "TOKEN_FROM_SHORTCUT_EVENT" }
}
```

The greeting advertises `window-interactive-grabs` when these methods are
available. Mutter starts the keyboard grab with the trusted shortcut timestamp
and current pointer sprite. Calls fail if the token is expired, already used,
revoked, or belongs to another connection; if the session is locked; or if the
window cannot be moved or resized.

### API version 1.13: focus policy events

Subscribe to `gnoblin.focus.policy-changed` with an API 1.13 event request.
Gnoblin sends the event after a successful config commit when the effective
focus policy changes. Failed or rejected config changes do not emit it.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 13 }, "events": ["gnoblin.focus.policy-changed"] }
```

The event contains `policy`, `revision`, `sequence`, and monotonic `time`.
`policy` is the committed focus-policy snapshot. `revision` matches
`policy.revision`; both identify the committed settings snapshot.

### API version 1.14: portal grants

API 1.14 adds the `grant.list` and `grant.revoke` methods. They use the portal
backend that owns the persistent grants. Each request returns an operation
descriptor; the same connection receives its `gnoblin.operation.completed`
event when the backend finishes. Match the event's `operation_id` to the
descriptor's `request_id`.

```json
{
    "op": "api",
    "id": "list-grants",
    "api_version": { "major": 1, "minor": 14 },
    "method": "grant.list",
    "arguments": {}
}
```

Each item in `value.grants` contains:

| Field           | Meaning                                               |
| --------------- | ----------------------------------------------------- |
| `id`            | Opaque value returned by the listing method.          |
| `kind`          | `screen-cast` or `remote-desktop`.                    |
| `requester`     | Verified portal identity.                             |
| `devices`       | Bitmask: keyboard `1`, pointer `2`, touchscreen `4`.  |
| `clipboard`     | Boolean permission.                                   |
| `screenStreams` | Boolean indicating whether screen streams are stored. |

Revoke a record using its listed `kind` and `id`:

```json
{
    "op": "api",
    "id": "revoke-grant",
    "api_version": { "major": 1, "minor": 14 },
    "method": "grant.revoke",
    "arguments": { "kind": "screen-cast", "id": "OPAQUE_ID_FROM_LIST" }
}
```

The successful value is `{ "ok": true, "id": "..." }`. The operation fails
if the grant no longer exists, its stored record is invalid, or the portal
backend is unavailable. `gnoblinctl grant list` and `gnoblinctl grant revoke`
wait for this completion before returning.

### API version 1.15: portal grant snapshots and events

API 1.15 adds a native snapshot of persistent portal grants. The optional
filter selects one portal kind. Each record contains its opaque ID, verified
requester, permission scope, creation timestamp, and snapshot revision:

| Field                | Meaning                                            |
| -------------------- | -------------------------------------------------- |
| `id`                 | Opaque portal grant ID.                            |
| `kind`               | `screen-cast` or `remote-desktop`.                 |
| `requester`          | Verified portal identity.                          |
| `devices`            | Array of `keyboard`, `pointer`, or `touchscreen`.  |
| `clipboard`          | Whether remote-desktop clipboard access is stored. |
| `has_screen_streams` | Whether a screen-stream selection is stored.       |
| `created_at`         | Unix time in milliseconds.                         |
| `revision`           | Revision of the current grant snapshot.            |

```json
{
    "op": "api",
    "id": "grants",
    "api_version": { "major": 1, "minor": 15 },
    "method": "portals.grants",
    "arguments": { "kind": "remote-desktop" }
}
```

Use `kind`, `id`, and `created_at` from a record when revoking it. API 1.15
rechecks the creation time in the portal backend, so a stale record cannot
revoke a later grant that reused the same ID. The older API 1.14 request shape
without `created_at` remains available for compatibility.

The portal backend stores creation time for new records. Older stored records
use their file modification time rounded to whole seconds; this is inferred
metadata, not a verified consent time.

Subscribe to the portal grant events with API 1.15. Both include the current
snapshot revision, event sequence, and monotonic time.

- `gnoblin.portal.grant-added` carries a `grant` record.
- `gnoblin.portal.grant-removed` carries the grant ID and portal kind.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 15 },
    "events": ["gnoblin.portal.grant-added", "gnoblin.portal.grant-removed"]
}
```

### API version 1.16: permission policy

API 1.16 adds `permissions.policy`. It returns the committed default level,
ordered rules, and revision.

The compatibility method `permissions.list` keeps its existing response shape.

```json
{
    "op": "api",
    "id": "policy",
    "api_version": { "major": 1, "minor": 16 },
    "method": "permissions.policy",
    "arguments": {}
}
```

`gnoblin.permission.changed` reports a policy change after a successful
configuration commit. Its payload contains the committed policy and standard
revision, sequence, and monotonic-time metadata.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 16 }, "events": ["gnoblin.permission.changed"] }
```

The socket and its parent directory are restricted to the current user. Any
same-user process can connect, so treat connected clients as trusted shell
components.

Tokens belong to the connection that received them. Do not forward a token to
another client.

Call `launch.status` to read current records and enable launch-change events on
the connection.

`launch.begin` requests cursor feedback for an application hint. It does not
start a process. Mutter reports `started` when a matching mapped window appears
or becomes focused.

The deadline changes a pending record to `timed_out`. `launch.end` changes it
to `ended`.

Launch records and events use their own `revision`. Launch feedback does not
advance the compositor `state_revision`.

Versionless requests remain supported for API 1.0 methods. Newer methods require
their documented minimum version. An unsupported or malformed requested version
returns an error and closes that connection.

The `hello.version` field is the socket protocol version. This build always
advertises `ui-sessions`, `switcher-fallback` and `overlay-shortcut`. It may
also advertise `blur-regions` and `layer-animation-policy` when the required
compositor support is available.

A validation error has an `error` event. If a valid `command` request fails,
the response also carries its request `id`. Malformed JSON or excessive input
closes the socket; ordinary validation errors leave it open.

## Operation index

| `op`                             | Required fields                                                          | Reply or stream                                     |
| -------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------- |
| `command`                        | `id`, `command`; command-specific fields                                 | One `reply` with matching `id`, or `error`          |
| `windows`                        | None                                                                     | Current `windows` snapshot, then changes            |
| `privacy`                        | None                                                                     | Current `privacy` state, then changes               |
| `status`                         | None                                                                     | One `status` with binding IDs and active session ID |
| `bind`                           | `id`, `accelerator`, `hold`; optional `trigger`, `modal`, `captureInput` | `bound`, then activation and input events           |
| `activate`                       | `window`; optional `session`                                             | Focus a window; no success reply                    |
| `preview`                        | `window`, `width`, `height`                                              | One `preview` event                                 |
| `shortcut-input`                 | `name`, `state`                                                          | Input handoff; no success reply                     |
| `ui-session`                     | `action`; other fields depend on action                                  | `ui-state` or `ui-command` events                   |
| `layer-animation-policy`         | `namespace`                                                              | One policy event for that layer namespace           |
| `blur-region`                    | `namespace`, `screen`, `region`                                          | No success reply                                    |
| `window-drag`                    | None                                                                     | Current `window-drag` state, then changes           |
| `snap-offer`                     | `serial`, `regions`                                                      | No success reply; may later get `snap-completed`    |
| `snap-context`                   | None                                                                     | One `snap-context` event                            |
| `snap-window`                    | `window`, `monitor`, `target`                                            | Applies a region; no success reply                  |
| `stop-sharing`, `stop-recording` | None                                                                     | Requests stop; no success reply                     |
| `end`                            | Optional `session` for fallback switcher                                 | Ends this client's input session                    |
| `clear`                          | None                                                                     | Removes this client's bindings and session          |

`command` accepts `windows`, `capture-windows`, `workspaces`, `workspace-list`,
`workspace-switch`, `workspace-next`, `workspace-previous`,
`workspace-move-active`, `monitors`, `layers` and `window`. `layers` returns
the current layer-shell surfaces in a `surfaces` array. `window` needs an
`action` and a stable window ID or `"active"`. The [CLI reference](gnoblinctl.md)
lists window actions and arguments.

### Workspace commands

Use `workspace-list` to get the workspace ID, current one-based number, display
name, active state and eligible window count:

```json
{ "op": "command", "id": "request-1", "command": "workspace-list" }
```

The reply's `result` is shaped like this:

```json
{
    "workspaces": [
        { "id": "code", "number": 1, "name": "Code", "active": true, "windows": 2 },
        { "id": "web", "number": 2, "name": "Web", "active": false, "windows": 0 }
    ]
}
```

The request `id` correlates its reply. A configured workspace ID is assigned by
its initial position and stays with that `MetaWorkspace` if workspaces are
reordered during the session. An unconfigured workspace receives a session-only
ID such as `@session-N`.

Each item from `workspaces` has these fields:

| Field     | Meaning                                                             |
| --------- | ------------------------------------------------------------------- |
| `id`      | Configured ID, or a generated session-only ID such as `@session-1`. |
| `number`  | Current one-based position.                                         |
| `name`    | Display label.                                                      |
| `active`  | Whether this workspace is selected.                                 |
| `windows` | Number of eligible windows on the workspace.                        |

The older `workspaces` command keeps its legacy response: its numeric `id` is
the current one-based position, and it does not include the display name.

Switch by stable ID or current number. Send exactly one selector:

```json
{"op":"command","id":"request-2","command":"workspace-switch","workspaceId":"code"}
{"op":"command","id":"request-3","command":"workspace-switch","workspaceNumber":2}
```

`workspace-switch` also accepts a numeric `workspace` selector.
`workspace-next` and `workspace-previous` take no selector and wrap at the
ends of the current workspace list.

Switch replies contain `ok`, `pending`, and the one-based `workspace` number.
They also include the resolved workspace's `id`, `number`, `name`, `active`,
and `windows` fields.

Move the focused window with `workspace-move-active`. It accepts the same
`workspaceId` or `workspaceNumber` selector, plus an optional `follow` boolean.
The default is `false`; set it to `true` to activate the destination after
moving.

For an explicit window, use `command: "window"`, `action: "workspace"`, and
the `window` ID with `workspaceId` or `workspaceNumber`. The one-based numeric
`workspace` selector is also accepted.

Successful move replies include the resolved workspace fields, `follow`, the
stable window `window` ID, and the selected `workspace` number, `workspaceId`,
and `workspaceNumber`.

Numbers can change when dynamic workspaces are removed. Use IDs in shell
integrations that need to keep addressing a configured workspace. The
[workspace CLI examples](gnoblinctl.md#workspaces-and-monitors) show equivalent
terminal commands.

Only `command` supplies a correlation ID: match `reply` or `error` by that ID
because other events can arrive first. A reply with `pending: true` means the
action was accepted; observe later state to confirm completion.

Animations are registered in Lua with `gnoblin.animation` and are previewed
through `gnoblinctl animation`. Shell clients should continue to own their
surface content motion; use a matching `animation = "none"` layer rule to
avoid applying compositor motion twice.

The bridge's `layer-animation-policy` operation lets a shell read the
configured enter/exit policy for a namespace. See the [animation guide](/guides/animations)
for layer-shell lifecycle events and target selection.

### Capture records

`capture-windows` returns visible, non-minimised windows in stacking order.

| Field                         | Meaning                                                          |
| ----------------------------- | ---------------------------------------------------------------- |
| `id`                          | Mutter window ID, used for capture.                              |
| `title`, `appId`, `appName`   | Window title, desktop-entry ID and application name.             |
| `x`, `y`, `width`, `height`   | Frame rectangle in logical pixels.                               |
| `bufferWidth`, `bufferHeight` | Paint-box size; may include decoration shadows beyond the frame. |

### Window records

The `windows` snapshot has these fields:

| Field                                         | Meaning                                                              |
| --------------------------------------------- | -------------------------------------------------------------------- |
| `id`                                          | Stable window sequence ID for this session.                          |
| `title`, `appId`                              | Window title and desktop-entry ID (or WM class if unavailable).      |
| `gtkAppId`, `wmClass`, `ruleAppId`            | Raw GTK ID, WM class and the ID used by window rules.                |
| `focused`, `minimized`                        | Whether the window is focused or minimised.                          |
| `workspace`, `workspaceId`, `workspaceNumber` | Current workspace number, stable ID and current one-based position.  |
| `monitorId`                                   | Active connector name for the current monitor, when available.       |
| `monitorIndex`, `monitor`                     | Zero-based monitor index and its logical origin `{x, y}`.            |
| `maximized`, `fullscreen`                     | Current window state.                                                |
| `above`, `sticky`, `demandsAttention`         | Stacking, workspace visibility, and attention state.                 |
| `closable`, `minimizable`, `maximizable`      | Whether the corresponding window operation is supported.             |
| `movable`, `resizable`                        | Whether the window can be moved or resized.                          |
| `role`, `type`                                | Optional window role and Mutter `MetaWindowType` integer.            |
| `geometry`                                    | Frame rectangle `{x, y, width, height}` in logical pixels.           |
| `lastUserTime`                                | Mutter's timestamp for the last user interaction with the window.    |
| `parent`                                      | Stable ID of its transient parent, or `null`/omitted if unavailable. |

Mutter omits optional parent and monitor values when unavailable. Shell may
return `null` for these fields and may omit native-only fields. See the [Lua event
reference](/config/lua-events#gnoblin) for `MetaWindowType` values.

Capture IDs come from Mutter; `windows` snapshots use stable sequence IDs.
Use an ID from the `windows` snapshot or `gnoblinctl window list` for a
`window` action. The CLI reference lists the supported actions and arguments.

## Example: watch the window list

Run this Python program inside your Gnoblin session:

```python
import json
import os
import socket

path = os.environ.get("GNOBLIN_COMPOSITOR_SOCKET") or os.path.join(
    os.environ["XDG_RUNTIME_DIR"], "gnoblin/compositor-v1.sock"
)
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.connect(path)
    with connection.makefile("r", encoding="utf-8") as messages:
        print("Greeting:", json.loads(messages.readline()))
        connection.sendall(b'{"op":"windows","api_version":{"major":1,"minor":1}}\n')
        for line in messages:
            event = json.loads(line)
            if event.get("event") == "windows":
                print(event["windows"])
            elif event.get("event", "").startswith("gnoblin.window."):
                print(event)
            elif event.get("event", "").startswith("gnoblin.workspace."):
                print(event)
```

It prints the current list and subsequent snapshots. The native preview also
prints individual window lifecycle events for created, changed, focused, and
closed windows. Stop it with Ctrl+C. Each snapshot replaces the previous list;
it is not a list of changes.

## Temporary UI bindings

Use `bind` for an interactive UI such as a switcher. These bindings belong to
the client connection and disappear when it disconnects.

Optional `trigger` is `"press"` (the default) or `"release"`; it chooses whether `activated` is sent when the accelerator is pressed or its main key is released. This works with chords using Alt, Control, Shift or Super. Bare `"Super"` is release-only because Mutter first checks whether another key joins the chord.

```json
{ "op": "bind", "id": "example", "accelerator": "<Alt>F8", "hold": 8, "trigger": "release" }
```

The server acknowledges:

```json
{ "event": "bound", "id": "example" }
```

| Field          | Accepted values                                              | Default                                         |
| -------------- | ------------------------------------------------------------ | ----------------------------------------------- |
| `id`           | 1–64 letters, numbers, `_` or `-`; unique on this connection | Required                                        |
| `accelerator`  | GTK accelerator string up to 128 characters, or `"Super"`    | Required                                        |
| `hold`         | `0`, Alt `8`, Control `4`, or Super `67108864`               | Required; `0` fires once                        |
| `trigger`      | `"press"` or `"release"`                                     | `"press"`; `"Super"` uses `"release"`           |
| `modal`        | Boolean                                                      | `true`; `false` leaves app input ungrabbed      |
| `captureInput` | Boolean                                                      | `false`; enables the popup typing handoff below |

For a modal held shortcut, Gnoblin captures key and pointer events before your
popup becomes visible. An Alt-held switcher can finish when Alt is released.

Set `modal = false` for a held shortcut that should leave focus and input with
the current app. Gnoblin reports the activation and modifier release without
opening an input grab. For example, this reports Alt+F8 and the later Alt
release while the app keeps receiving input:

```json
{ "op": "bind", "id": "cycle-mode", "accelerator": "<Alt>F8", "hold": 8, "modal": false }
```

Events sent during a binding session:

| Event                   | Fields                                        | Meaning                                                      |
| ----------------------- | --------------------------------------------- | ------------------------------------------------------------ |
| `activated`             | `id`, `first`, `session`, `modifiers`, `time` | The accelerator fired.                                       |
| `key`                   | `key`, `modifiers`                            | A captured key press.                                        |
| `pointer`               | `x`, `y`, `button`                            | A captured button press in global logical pixels; 1 is left. |
| `released`, `cancelled` | `session`                                     | The modifier was released or the session ended early.        |

`first` is true when no binding session was active. `session` is zero for
ordinary bindings and nonzero for the built-in switcher fallback. `modifiers`
is a Clutter modifier mask, and `time` is the event timestamp in milliseconds.

Hide the UI on release or cancellation. Sending `{"op":"end"}`, locking,
disconnecting or reaching the ten-second timeout also releases the captured
input.

## Bare Super and buffered typing

```json
{ "op": "bind", "id": "search", "accelerator": "Super", "hold": 0, "captureInput": true }
```

Requires the advertised `overlay-shortcut` feature. Only one bridge client can
own bare Super; remove a duplicate Lua command binding first.

Super activates on release, excluding chords. With capture enabled, report the
popup's focus handoff:

```jsonl
{ "op": "shortcut-input", "name": "search", "state": "prepared" }
{ "op": "shortcut-input", "name": "search", "state": "ready" }
```

Send `prepared` once the popup is mapped. Gnoblin releases its keyboard grab
but keeps buffering keys. Send `ready` when the text field has focus; Gnoblin
replays the buffered keys in order. Send `closed` when the popup is dismissed.
The buffer expires after three seconds if the popup never becomes ready.

The binding ID is the handoff name.

## Windows and controls

| Request                            | Result                                    |
| ---------------------------------- | ----------------------------------------- |
| `{"op":"windows"}`                 | Subscribe to complete snapshots           |
| `{"op":"activate","window":"123"}` | End this client's grab and focus a window |
| `{"op":"status"}`                  | Registered IDs and active session         |
| `{"op":"end"}`                     | Cancel this client's input session        |
| `{"op":"clear"}`                   | Remove this client's bindings and session |

Snapshots exclude skip-taskbar and override-redirect windows. IDs last for the
window's session lifetime. The client chooses grouping and ordering; stale IDs
return an error. See the field list below for the complete record shape.

A subscription with no open windows produces:

```json
{ "event": "windows", "windows": [] }
```

When windows exist, the array uses the [window record example](gnoblinctl.md#output-for-scripts).
See [CLI development](cli-development.md#change-a-window) for a complete
request, successful reply and error reply.

## Previews

```json
{ "op": "preview", "window": "123", "width": 224, "height": 126 }
```

The response contains a PNG data URI in `source`, dimensions and window ID.
On failure it returns an empty source and a message; retain the last valid image.

Maximum size: 480 × 320, preserving aspect ratio. One capture may be pending
per client. No continuous stream is started; clients choose update frequency.

Capture is unavailable while locked. Readback runs at low priority, PNG encoding
uses a worker, and minimised windows use their retained buffer.

## Bingux text-entry integration

Text entry is provided by a Bingux-owned integration. Bingux installs a Gnoblin
script that registers the namespaced
`bingux.input-anchor` and `bingux.type-text` operations. Other shells can omit
the integration script and implement their own text-entry behavior.

`{"op":"bingux.input-anchor"}` returns x, y, pid, window, frame and caret.
Coordinates are logical desktop pixels. Caret may be null; it clears on focus
changes and lock, and adjusts when the window moves.

`{"op":"bingux.type-text","window":"123","text":"…"}` accepts up to 64 UTF-16
units, without control characters. The target must be focused and unlocked,
with Control, Alt and Super released.

Hide the picker and restore the target before sending. A `typed` reply
acknowledges delivery, not proof that the app consumed the text.

For XWayland, a GTK helper temporarily offers the text and sends Ctrl+V.
It preserves clipboard formats and restores them after 600 ms unless a new copy
takes priority. Clipboard managers may record the temporary text.

Clipboard preparation is limited to 64 formats, 64 MiB and three seconds.
Failed preparation leaves it unchanged. Native Wayland text input does not
use the clipboard.

On XWayland, Bingux uses its `bingux-clipboard-paste` helper, which needs
Python, PyGObject and GTK 3. Native Wayland text input does not use the helper.
The helper and GJS integration are shipped by Bingux. The integration script
is loaded from the XDG script search path; see [writing scripts](user-scripts.md).

## Recording and camera activity

`{"op":"privacy"}` subscribes to screenSharing, recording, recordingCount,
recordingElapsed, cameraInUse and locationCaptures.

Location captures list authorised GeoClue desktop IDs. Elapsed time is whole
seconds; clients can advance it between updates.

`stop-sharing` stops non-recording remote sessions.
`stop-recording` stops recording sessions. An encoder owner should use its
normal finalisation path to save output. These requests do not grant access.

## Coordinate shell processes

`ui-session` shares named state between UI processes. Names match
`^[a-z][a-z0-9-]{0,63}$`; one client owns each name.

| Action    | Fields                 | Event                                       |
| --------- | ---------------------- | ------------------------------------------- |
| `watch`   | None                   | `ui-state` for each owner and later changes |
| `state`   | `name`, `state` object | Publishes `ui-state`                        |
| `command` | `name`, `command`      | Sends `ui-command` to the owner             |

`watch` immediately sends current owners, then updates. A `state` request claims
or updates a name; another client cannot claim it. Owner disconnect removes its
state and publishes `state: null`. `command` is forwarded to the current owner
as-is; it is ignored when no owner exists and is never queued.

These requests show the message shapes; each goes over the relevant client's
own bridge connection:

```jsonl
{"op":"ui-session","action":"watch"}
{"op":"ui-session","action":"state","name":"search","state":{"visible":true}}
{"op":"ui-session","action":"command","name":"search","command":{"action":"close"}}
```

The owner defines the state and command objects. To coordinate layer stacking,
include `surface` (the owner's namespace) and `companions` (up to 16 namespace
strings) in state. Gnoblin applies the request while `visible` or
`revealCompanions` is true; set `companionsAbove: true` to place companions
above the owner. The state is advisory and does not change how a client draws.

| Operation                | Fields                                                                                    | Limit or effect                                                                                                                            |
| ------------------------ | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `blur-region`            | `namespace`, monitor origin `screen: [x,y]`, local `region: [x,y,width,height]` or `null` | Up to 64 per client; requires a matching blur window rule; cleared on disconnect. See [effect rendering](effects-rendering.md#blur-cache). |
| `layer-animation-policy` | `namespace`                                                                               | Returns `enter`, `exit`, `windowShadow`; namespace at most 128 characters                                                                  |

For `blur-region`:

- `screen`: two finite coordinates within ±65,536 for the monitor origin.
- `region`: `null` clears it. Otherwise use four finite values
  `[x, y, width, height]`; coordinates must be within ±65,536, and dimensions
  cannot be negative.
- Scope: Gnoblin applies the rectangle only to a matching layer surface owned
  by the sending process. A matching window rule must also enable blur.

For drag layouts, subscribe with `window-drag`, offer hit and target rectangles
using `snap-offer`, and apply a keyboard-chosen rectangle with `snap-window`.
The [snapping guide](/guides/window_snapping#shell-integration) gives the request
shapes and work-area checks.

## Limits and disconnects

The socket directory is private to the user.
Limits: 32 clients, 32 bindings per client, 64 queued records and 4 MiB queued
output per connection. An input buffer over 16 KiB or malformed JSON closes the
connection. A slow reader can also be disconnected when its output queue fills.

Invalid requests return errors. Malformed JSON or excessive input disconnects
the client. Disconnect releases its bindings and other resources.
