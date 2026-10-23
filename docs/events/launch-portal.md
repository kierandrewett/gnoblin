# Launch Events

Observe launch feedback events.

Events are emitted when the launch tracker reports a change.

### Launch events

With `--gnoblin-config PATH`, Mutter sends window, workspace, monitor, input,
and launch-feedback events directly to Lua. `sequence` increases across this
event stream. Gesture events use their own sequence and have no state revision.

`gnoblin.launch.changed` wraps the record under `launch`. Each record has these fields:

| Field         | Type or accepted values                       | Meaning                                                                        |
| ------------- | --------------------------------------------- | ------------------------------------------------------------------------------ |
| `token`       | Non-empty string, up to 128 characters        | Caller-supplied launch token.                                                  |
| `application` | Non-empty string, up to 512 characters        | Application ID, WM class, or WM class instance to observe.                     |
| `started_at`  | Integer                                       | Start time as Unix time in milliseconds.                                       |
| `timeout_ms`  | Integer from 100 to 10000                     | Observation deadline in milliseconds.                                          |
| `state`       | `pending`, `started`, `ended`, or `timed_out` | Current launch state, described below.                                         |
| `revision`    | Integer                                       | Launch snapshot revision; it does not advance the compositor `state_revision`. |

The `state` values mean:

- `pending`: Mutter is watching for the application window.
- `started`: a matching mapped window appears or becomes focused.
- `timed_out`: the deadline passes before a window is observed.
- `ended`: `finish()` runs while the launch is still `pending`.

Mutter cannot report failures from an external process launcher.

Set these fields in `gnoblin.launches.begin`:

| Field         | Type or accepted values                | Default  |
| ------------- | -------------------------------------- | -------- |
| `token`       | Non-empty string, up to 128 characters | Required |
| `application` | Non-empty string, up to 512 characters | Required |
| `timeout_ms`  | Integer from 100 to 10000              | 3000     |

```lua
gnoblin.launches.begin {
    token = "launcher-42",
    application = "org.example.Editor",
    timeout_ms = 5000,
}
```

Shortcut capture also completes through `gnoblin.operation.completed`:

- Success returns the accelerator in `value.accelerator`.
- Cancellation, timeout, a locked session, or an input grab returns an `Error` record.

The capture hook consumes key events while active, then cancels and releases
the hook if the session locks or another input grab starts.
