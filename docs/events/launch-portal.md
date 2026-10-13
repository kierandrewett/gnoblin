# Launch and Portal Events

Observe launch feedback and portal grant lifecycle events.

Events are emitted when the corresponding launch tracker or portal backend reports a change.

### Launch events

With `--gnoblin-config PATH`, Mutter sends window, workspace, monitor, input,
and launch-feedback events directly to Lua. `sequence` increases across this
event stream. Gesture events use their own sequence and have no state revision.

`gnoblin.launch.changed` wraps the record under `launch`. It contains:

- `token` and `application`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

States are `pending`, `started`, `ended`, and `timed_out`. Mutter reports a
launch when a matching mapped window appears or becomes focused. It cannot
report failures from an external process launcher. The revision is scoped to
launch records and does not advance the compositor `state_revision`.

Shortcut capture also completes through `gnoblin.operation.completed`:

- Success returns the accelerator in `value.accelerator`.
- Cancellation, timeout, a locked session, or an input grab returns an `Error` record.

The capture hook consumes key events while active, then cancels and releases
the hook if the session locks or another input grab starts.
