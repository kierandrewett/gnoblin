# Launch feedback

> **Status:** not available yet: `:finish`, `gnoblin.launch.changed`, `gnoblin.launches`, `gnoblin.launches.begin`, `gnoblin.launches.list`. The sections that use them describe the target, not current behaviour.

Use `gnoblin.launches` to read launch feedback and track a launch request.
`list()` returns the cached native snapshot as `Launch` objects, one for each
token. The Lua API has no `snapshot()`; each `Launch` has its own `revision`.

The native controller seeds an empty snapshot during startup and refreshes it
before dispatching launch-change events. `list()` is unavailable before native
startup completes. `gnoblinctl lua` also exposes the read.

| Method                                                 | Arguments                               | Result                          |
| ------------------------------------------------------ | --------------------------------------- | ------------------------------- |
| `gnoblin.launches.list()`                              | None                                    | `Launch[]`                      |
| `gnoblin.launches.begin(token, application, options?)` | Token, application; option `timeout_ms` | The `Launch`, pending at once   |
| `launch:finish()`                                      | None                                    | An `Operation` of `{ok, token}` |

`begin` returns the `Launch` before the compositor confirms it, with the values
you asked for and `revision` 0. The confirmation and each later change update
the same object. Beginning a token that is already in use ends the earlier
`Launch`.

`gnoblin.launches.begin` takes these arguments:

| Argument             | Type and accepted value                  | Default | Meaning                                  |
| -------------------- | ---------------------------------------- | ------- | ---------------------------------------- |
| `token`              | Non-empty string, at most 128 characters | None    | Identifies this launch attempt.          |
| `application`        | Non-empty string, at most 512 characters | None    | Application hint used for matching.      |
| `options.timeout_ms` | Integer from 100 to 10000                | `3000`  | Deadline before state becomes timed out. |

Call `launch:finish()` when the launch is cancelled or the application has
started. It returns an `Operation` that completes asynchronously, as described
near the start of this page. A launch that is not pending is unchanged.

Socket clients use the `launch.status`, `launch.begin` and `launch.end`
methods; see the [compositor bridge](/bridge/).

The native runtime retains at most 64 records. It evicts the oldest completed
record when needed and rejects a new launch if all retained records are pending.

Each `Launch` is a live object. It is the same object for as long as the launch
exists: every read of one token returns it, and each change from the compositor
updates it. It has these fields, which `launch:watch(field, fn)` can follow:

- `token` and `application`.
- `alive`, which is true while `state` is `pending`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

The state begins as `pending`. Mutter reports `started` when a matching mapped
window appears or becomes focused. `launch:finish()` changes a pending record to
`ended`; the deadline changes it to `timed_out`. A launch ends in one of these
three states, and it never leaves them.

`launch:on(signal, fn)` calls `fn(launch, ...)`:

| Signal      | When                                                                |
| ----------- | ------------------------------------------------------------------- |
| `changed`   | A field changed. The second argument lists the changed field names. |
| `started`   | A matching window appeared or took focus.                           |
| `timed_out` | The deadline passed first.                                          |
| `closed`    | The launch left `pending` for any reason.                           |

`closed` runs last. A launch that a second `begin` with the same token replaced
passes the reason `replaced`. A launch that the compositor refused passes the
error message, and its `state` becomes `ended`. Listening ends after `closed`.

Gnoblin does not link a `Launch` to the window or `Process` that answered it. The
compositor matches a window by application ID and does not record which window
did, so match the window yourself, for example by `app_id`.

Matching uses GTK application ID, WM class, or WM class instance. Values are
lowercased and a `.desktop` suffix is removed. The native controller cannot report failures from an external process
launcher.

Subscribe to `gnoblin.launch.changed` for every launch at once, or call
`launch:on` for one. `launch:finish()` ends a request when it is cancelled or
the application has started.
