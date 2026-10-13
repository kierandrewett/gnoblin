# Permissions and Privacy

Inspect permission policy, portal grants, and privacy state or request supported privacy actions.

The bridge exposes these operations in the standalone session at the minimum API versions stated below.

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

### API version 1.17: privacy state

API 1.17 adds `privacy.state`, which returns a privacy snapshot and revision.
Each source has an availability flag. Unavailable activity fields are omitted.

Mutter tracks screen-sharing and recording through remote-access handles.
Microphone monitoring requires remote-desktop support and a PipeWire connection.
It reports running audio-capture streams, excluding the GNOME Volume Control
and pavucontrol meter streams. It does not check for audible samples. Camera
and location monitoring are unavailable.

```json
{
    "op": "api",
    "id": "privacy",
    "api_version": { "major": 1, "minor": 17 },
    "method": "privacy.state",
    "arguments": {}
}
```

Subscribe to `gnoblin.privacy.changed` at API 1.17 to receive updates. Each
event has a `state` snapshot with its `revision`, plus top-level `sequence` and
monotonic `time` metadata.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 17 },
    "events": ["gnoblin.privacy.changed"]
}
```

The socket and its parent directory are restricted to the current user. Any
same-user process can connect, so treat connected clients as trusted shell
components.

Tokens belong to the connection that received them. Do not forward a token to
another client.

Call `launch.status` to read current records and enable launch-change events on
the connection.

Launch feedback uses Gnoblin's native controller.

API 1.50 routes `launch.begin` and `launch.end` through the Lua runtime. These
calls return an operation descriptor and complete through
`gnoblin.operation.completed`; subscribe to that event to receive the result.
Earlier API versions keep the synchronous native route.

API 1.54 adds the Lua-backed `launch.status` route. Every supported client
version now reads through `gnoblin.launches.snapshot()`. The request still
enables launch-change events for the connection.

Clients with API 1.29 or newer read `session.status` directly from Mutter
without the Lua supervisor. The response contains `state`, `lock_available`,
`revision`, and `lock_state` when locking is available. The revision matches
the lock-state event and remains unchanged between lock transitions.

API 1.52 and newer serve `workspace.list` from the Lua workspace snapshot.
The reply keeps the `{ "workspaces": [...] }` wrapper and `windows` count.
Earlier API versions use the native route. Lua callers use
`gnoblin.workspaces.list()`.

API 1.53 and newer serve `window.list` from the Lua window snapshot. The reply
keeps its `{ "windows": [...] }` wrapper and fields such as `appId` and
`geometry`. Older API versions use Mutter's native route. Lua callers use
`gnoblin.windows.list()`.

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

Use the API-version tables above to choose a socket method and its minimum
version. For Lua calls, see the [runtime API reference](/shell-api/).

### API version 1.31: privacy stop methods and layer animation policy

API 1.31 adds `privacy.stop_sharing` and `privacy.stop_recording`. Each method
takes no arguments and calls `stop()` on matching tracked Mutter handles. The
methods match handles by their `is-recording` property.

The socket returns an operation descriptor. Wait for its matching
`gnoblin.operation.completed` event. The result's integer `requested` field
counts handles passed to `meta_remote_access_handle_stop()`. The request is
synchronous, but the count does not confirm session closure.

Mutter updates the privacy snapshot and `gnoblin.privacy.changed` after a
handle stops. These methods do not revoke persistent grants.

```json
{
    "op": "api",
    "id": "stop-sharing",
    "api_version": { "major": 1, "minor": 31 },
    "method": "privacy.stop_sharing",
    "arguments": {}
}
```

Call `layer.animation_policy` with a `namespace` string of 1–128 UTF-8 bytes
to read its enter and exit phases and `window_shadow`. It requires API 1.31.

Without a matching animation rule, each phase uses its first enabled event
registration, or `none` if no event is registered.

`window_shadow` defaults to `false` unless a matching default-window rule
supplies a shadow value.

### API version 1.33: capability availability changes

`capabilities.list` includes `microphone-monitor`. It is available when this
Mutter build includes remote-desktop support and its PipeWire monitor is
connected. When unavailable, the record includes `reason`:

- `remote_desktop_disabled`: this Mutter build has remote-desktop support
  disabled.
- `pipewire_unavailable`: the PipeWire microphone monitor is disconnected or
  could not connect.

Subscribe to `gnoblin.capability.changed` at API 1.33 to receive the updated
record when availability changes. Each event contains:

- `capability`: the updated capability record.
- `revision`: the current compositor state revision.
- `sequence`: the event sequence number.
- `time`: monotonic event time.

Microphone activity changes remain part of `gnoblin.privacy.changed`.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 33 },
    "events": ["gnoblin.capability.changed"]
}
```

### API version 1.74: monitor privacy screens

API 1.74 adds `monitors.privacy_screen` and the `monitors.set_privacy_screen`
operation. The read returns the effective requested state, its source, a
revision, and per-monitor support and state:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 74 },
    "id": "read-privacy-screen",
    "method": "monitors.privacy_screen",
    "arguments": {}
}
```

Each monitor record has:

- `id`: the active connector name;
- `available`: whether the output supports a privacy screen;
- `enabled` and `locked`: present only when `available` is true.

The snapshot's `source` value is one of:

- `runtime`: an explicit runtime override supplies the request;
- `config`: the config sets `monitors.privacy_screen`;
- `system`: neither the runtime nor config sets a value.

Runtime overrides apply for the current session and do not write the system
preference.

Set a session-only override with `true` or `false`. Pass `"inherit"` to clear
the runtime override, exposing the configured value or the system preference:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 74 },
    "id": "set-privacy-screen",
    "method": "monitors.set_privacy_screen",
    "arguments": { "value": true }
}
```

The setter returns an operation descriptor. Its completion value is the
updated privacy-screen snapshot. Subscribe to
`gnoblin.monitor.privacy-screen-changed` at API 1.74 to receive the same
snapshot fields with `sequence` and `time` when the effective request, source,
or per-monitor state changes.
