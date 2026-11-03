# Launch feedback

Use `gnoblin.launches` to read launch feedback and track a launch request.
`list()` returns the cached native snapshot as immutable `Launch`
records. `snapshot()` returns those records together with the collection
revision, including when the collection is empty.

The native controller seeds an empty snapshot during startup and refreshes it
before dispatching launch-change events. Both methods are unavailable before
native startup completes. `gnoblinctl lua` also exposes both reads.

| Method                                                 | Arguments                               | Result                                          |
| ------------------------------------------------------ | --------------------------------------- | ----------------------------------------------- |
| `gnoblin.launches.list()`                              | None                                    | `Launch[]` snapshot                             |
| `gnoblin.launches.snapshot()`                          | None                                    | `{launches, revision}` snapshot                 |
| `gnoblin.launches.begin(token, application, options?)` | Token, application; option `timeout_ms` | An `Operation` whose value is a `Launch` record |
| `gnoblin.launches.finish(token)`                       | Launch token string                     | An `Operation` whose value is `{ok, token}`     |

`gnoblin.launches.begin` takes these arguments:

| Argument             | Type and accepted value                  | Default | Meaning                                  |
| -------------------- | ---------------------------------------- | ------- | ---------------------------------------- |
| `token`              | Non-empty string, at most 128 characters | None    | Identifies this launch attempt.          |
| `application`        | Non-empty string, at most 512 characters | None    | Application hint used for matching.      |
| `options.timeout_ms` | Integer from 100 to 10000                | `3000`  | Deadline before state becomes timed out. |

Call `finish(token)` with the same token when the launch is cancelled or the
application has started. The returned `Operation` completes asynchronously, as
described near the start of this page.

Socket clients use the `launch.status`, `launch.begin` and `launch.end`
methods; see the [compositor bridge](/bridge/).

The native runtime retains at most 64 records. It evicts the oldest completed
record when needed and rejects a new launch if all retained records are pending.

Each immutable `Launch` record contains:

- `token` and `application`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

The state begins as `pending`. Mutter reports `started` when a matching mapped
window appears or becomes focused. `launch.end` changes a pending record to
`ended`; the deadline changes it to `timed_out`.

Matching uses GTK application ID, WM class, or WM class instance. Values are
lowercased and a `.desktop` suffix is removed. The native controller cannot report failures from an external process
launcher.

Subscribe to `gnoblin.launch.changed` for updates. End a request with the same
token when it is cancelled or the application has started.
