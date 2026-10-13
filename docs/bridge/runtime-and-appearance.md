# Runtime and Appearance

Inspect runtime state, appearance, animation policy, and capability changes.

Use the minimum API version stated for each operation or event.

### API version 1.34: desktop appearance changes

Subscribe to `gnoblin.appearance.color-scheme-changed` for desktop appearance
changes. It follows `org.gnome.desktop.interface/color-scheme`.

Each event contains:

- `color_scheme`: `default`, `prefer-dark`, or `prefer-light`.
- `sequence`: the event sequence number.
- `time`: monotonic time in microseconds.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 34 },
    "events": ["gnoblin.appearance.color-scheme-changed"]
}
```

### API version 1.66: orientation lock

API 1.66 adds `input.orientation_lock` and `input.set_orientation_lock`.
`input.orientation_lock` takes an empty `arguments` object and returns the
orientation-lock record directly:

```json
{
    "op": "api",
    "id": "orientation-lock",
    "api_version": { "major": 1, "minor": 66 },
    "method": "input.orientation_lock",
    "arguments": {}
}
```

The record has these fields:

- `available` and `locked` are booleans.
- `orientation` is `normal`, `bottom-up`, `left-up`, `right-up`, or
  `undefined`.
- `source` is `system`, `config`, or `runtime`.
- `revision` is an integer.

`input.set_orientation_lock` takes `{ "value": true }`,
`{ "value": false }`, or `{ "value": "inherit" }`. It completes
asynchronously with the resulting record. `inherit` clears the runtime
override, restores a configured boolean if present, or follows the system
setting otherwise. A runtime request does not write the config file.

Config reload reapplies a boolean `input.orientation_lock` value. If it is
omitted or set to `"inherit"`, reload clears the override and follows the
system setting.

Subscribe to `gnoblin.input.orientation-lock-changed` to receive changes. Its
top-level fields are:

- `available`, `locked`, `orientation`, `source`, and `revision` match the
  current state record.
- `event` names the event.
- `sequence` orders events.
- `time` is monotonic.

Request API 1.66 or newer:

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 66 },
    "events": ["gnoblin.input.orientation-lock-changed"]
}
```

### API version 1.67: Lua runtime health

Read worker health directly from Mutter with `runtime.status`:

```json
{
    "op": "api",
    "id": "runtime-status",
    "api_version": { "major": 1, "minor": 67 },
    "method": "runtime.status",
    "arguments": {}
}
```

`state` reports the worker's connection state:

- `starting` means the worker has not connected yet.
- `running` means the worker is serving requests.
- `restarting` means Mutter has suspended the worker for replacement.
- `unavailable` means the supervisor is disconnected or stopping.

`generation` identifies the runtime configuration accepted by Mutter. It
changes when Mutter accepts a new configuration. Worker recovery applies the
built-in defaults (see [worker recovery](/config/load#worker-recovery)), so the
generation rises by one after a worker restarts.

### API version 1.70: current appearance preference

Call `appearance.color_scheme` with no arguments. Its reply contains:

- `available: true` and `color_scheme` set to `default`, `prefer-dark`, or
  `prefer-light` when the setting is available.
- `available: false` with no `color_scheme` field when the schema or key is
  unavailable.

The current value follows `org.gnome.desktop.interface/color-scheme`.

```json
{
    "op": "api",
    "id": "appearance",
    "api_version": { "major": 1, "minor": 70 },
    "method": "appearance.color_scheme",
    "arguments": {}
}
```

### API version 1.72: runtime status changes

The `gnoblin.runtime.status-changed` event mirrors the state and generation
returned by `runtime.status()`.

Each event includes `sequence` and monotonic-clock `time`. Mutter sends it for
these transitions:

- `running` when a worker starts serving;
- `restarting` after Mutter suspends a worker for replacement;
- `running` after the replacement finishes restoring state; or
- `running` with a new generation after an accepted configuration change.

Rejected and rolled-back changes do not emit a generation update.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 72 },
    "events": ["gnoblin.runtime.status-changed"]
}
```

Events are not replayed. Subscribe, then call `runtime.status` to get the
current values; repeat that read after reconnecting. Mutter may close the
socket before it can send a final `unavailable` event when the compositor
stops.
