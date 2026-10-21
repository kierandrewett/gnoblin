# Privacy

Inspect privacy state or request supported privacy actions.

The bridge exposes these operations in the standalone session at the minimum API versions stated below.

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

### API version 1.31: privacy stop methods

API 1.31 adds `privacy.stop_sharing` and `privacy.stop_recording`. Each method
takes no arguments and calls `stop()` on matching tracked Mutter handles. The
methods match handles by their `is-recording` property.

The socket returns an operation descriptor. Wait for its matching
`gnoblin.operation.completed` event. The result's integer `requested` field
counts handles passed to `meta_remote_access_handle_stop()`. The request is
synchronous, but the count does not confirm session closure.

Mutter updates the privacy snapshot and `gnoblin.privacy.changed` after a
handle stops.

```json
{
    "op": "api",
    "id": "stop-sharing",
    "api_version": { "major": 1, "minor": 31 },
    "method": "privacy.stop_sharing",
    "arguments": {}
}
```

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
