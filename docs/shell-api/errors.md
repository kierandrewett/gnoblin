# Operations, Errors and Timers

Gnoblin's Lua runtime API reports the outcome of an asynchronous operation through an `Operation` object.

## Operation results

An asynchronous Lua operation has these fields:

| Field    | What it holds                                                                           |
| -------- | --------------------------------------------------------------------------------------- |
| `id`     | An identifier for the operation                                                         |
| `method` | The name of the operation                                                               |
| `status` | `pending`, `succeeded` or `failed`                                                      |
| `value`  | The result when the status is `succeeded`; `nil` otherwise                              |
| `error`  | A table with a stable `code` and a `message` when `status` is `failed`; `nil` otherwise |

Invalid arguments and calls outside an allowed runtime event fail before the operation is queued. A queued operation
can fail later, when Mutter or the session service applies it.

## Handler errors

An error raised in a handler is local to that handler. This applies to event handlers (`gnoblin.on`), `on` and `watch`
callbacks of objects, `Process` and `Request` signals, timers, `Operation` callbacks, tween callbacks and input
handlers.

- Gnoblin logs the error with the event or signal name and the Lua stack traceback.
- The handler stays registered and runs again on the next event. Gnoblin never disables a handler.
- Other handlers of the same event still run. Each handler gets its own instruction budget (one million
  instructions), so a loop that never ends in one handler cannot stop the handlers after it.
- An input handler that throws or returns an invalid value decides nothing: its queued operations are dropped and
  the handlers after it still decide.
- Gnoblin shows a notice for the first failure of a handler. A success resets the count. To keep the log short, a
  handler that fails again and again is logged for its first three failures in a row and then once for every 1000.
- When an event handler fails, Gnoblin rolls back all configuration changes made while it dispatched that event,
  including changes from handlers that did not fail. Operations that other handlers queued still run.

## Operation methods

| Method             | What it does                                                                              |
| ------------------ | ----------------------------------------------------------------------------------------- |
| `on_complete(fn)`  | Calls `fn(value, error)` once when the operation finishes. Returns a subscription.        |
| `map(fn)`          | Returns a new operation that succeeds with `fn(value)`, or fails with the same error.     |
| `timeout(seconds)` | Returns a new operation that fails with `timed_out` if this one has not finished in time. |
| `cancel()`         | Stops waiting. Returns `false` when the operation had already finished.                   |

A callback added after the operation finished runs once, after the current handler returns. Call `unsubscribe()` on
the subscription to stop a callback before it runs. An error raised in one callback is logged and does not stop the
other callbacks. An error raised by the `map` function fails the new operation with the code `internal`.

`cancel()` fails the operation with the code `cancelled` and runs its callbacks. It stops a timer. An action that was
already sent to the compositor, such as a workspace switch or a command, still runs; its result is ignored. `timeout`
does not cancel the operation it waits on.

## Timers

`gnoblin.timers.after(seconds)` returns an operation that succeeds with `nil` after the given time, from 0 to 86400
seconds. Call it from an event handler or an operation callback; it raises an error while the config file loads. A
runtime can hold at most 1024 pending timers. Pending timers end when the config reloads and do not delay the reload.

`gnoblin.timers.now()` returns seconds on a monotonic clock. Only the difference between two readings is meaningful.

This handler waits two seconds, then switches to the next workspace and gives up after one more second:

```lua
gnoblin.on("gnoblin.workspace.activated", function()
    gnoblin.timers.after(2):on_complete(function()
        gnoblin.workspaces.next():timeout(1):on_complete(function(workspace, err)
            print(err and err.code or workspace.name)
        end)
    end)
end)
```

## Error codes

| Error code         | Meaning                                                          |
| ------------------ | ---------------------------------------------------------------- |
| `invalid_argument` | An operation argument or event-produced config value is invalid. |
| `not_found`        | The requested object or resource does not exist.                 |
| `unsupported`      | The compositor or session cannot perform the operation.          |
| `denied`           | Session policy or security state rejects the operation.          |
| `busy`             | An exclusive input operation is already active.                  |
| `cancelled`        | The user or the caller cancelled the operation.                  |
| `timed_out`        | The operation reached its deadline.                              |
| `unavailable`      | The compositor or session service is not ready.                  |
| `internal`         | An unexpected implementation error occurred.                     |

## Example

Put this in a supervised runtime event callback. It prints the name of the activated workspace, or the error:

```lua
local operation = gnoblin.workspaces.next()
operation:on_complete(function(workspace, err)
    if err then
        print(err.code, err.message)
        return
    end
    print("Active workspace:", workspace.name)
end)
```

## See also

- [Runtime API overview](/shell-api/)
- [Events](/events/)
- [Compositor bridge errors](/bridge/connection)
- A shell that uses the compositor bridge negotiates the methods and events that the connection's greeting advertises.
  `gnoblin.version()` returns the API version.
