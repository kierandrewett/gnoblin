# Launch feedback

Use `gnoblin.launches` to read launch feedback and track a launch request.
`list()` returns the latest cached native snapshot as immutable `Launch`
records. `snapshot()` returns those records together with the collection
revision, including when the collection is empty.

The native controller seeds an empty snapshot during startup and refreshes it
before dispatching launch-change events. Both methods are unavailable before
native startup completes. `gnoblinctl lua` also exposes both reads.

| Method                            | Arguments                                     | Result                                          |
| --------------------------------- | --------------------------------------------- | ----------------------------------------------- |
| `gnoblin.launches.list()`         | None                                          | `Launch[]` snapshot                             |
| `gnoblin.launches.snapshot()`     | None                                          | `{launches, revision}` snapshot                 |
| `gnoblin.launches.begin(options)` | `token`, `application`; optional `timeout_ms` | An `Operation` whose value is a `Launch` record |
| `gnoblin.launches.finish(token)`  | Launch token string                           | An `Operation` whose value is `{ok, token}`     |

`gnoblin.launches.begin(options)` accepts these fields:

| Field         | Type and accepted value                              | Default | Meaning                             |
| ------------- | ---------------------------------------------------- | ------- | ----------------------------------- |
| `token`       | Non-empty string, at most 128 characters             | None    | Identifies this launch attempt.     |
| `application` | Non-empty string, at most 512 characters             | None    | Application hint used for matching. |
| `timeout_ms`  | Integer from 100 to 10000                             | `3000`  | Deadline before state becomes timed out. |

Call `finish(token)` with the same token when the launch is cancelled or the
application has started. The bracket form `gnoblin.launches["end"](token)`
remains as a compatibility alias because `end` is a Lua keyword. The returned
`Operation` completes asynchronously, as described near the start of this
page.

The operation-based `launch.status()`, `launch.begin(args)`, and
`launch.end(args)` names are low-level aliases. Prefer `gnoblin.launches` for
Lua code.

| Method                    | Arguments                                       | Successful result                                |
| ------------------------- | ----------------------------------------------- | ------------------------------------------------ |
| `launch.status()`         | None                                            | `{launches, revision}`                           |
| `launch.begin(args)`      | `token`, `application`; optional `milliseconds` | `Launch` record                                  |
| `launch.end(args)`        | `token`                                         | `{ok, token}`                                    |

The low-level `launch.begin(args)` alias accepts the same token and application
limits. It names the deadline field `milliseconds` and uses the same default
and accepted range.


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
