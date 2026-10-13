# Errors and Results

This page explains the result conventions used by Gnoblin's Lua runtime API.

The Lua runtime exposes API version `1.81` in the current source. A shell using the compositor bridge should negotiate the methods and events advertised by that connection's greeting.

## Operation results

An asynchronous Lua operation exposes an identifier, method, status, value, error, and completion callback. The callback is registered with `on_complete(callback)`.

Status values are `pending`, `succeeded`, and `failed`. On success, `value` contains the result and `error` is `nil`; on failure, `error` contains a stable code and message. Completion callbacks receive `(value, error)`.

Invalid arguments and calls outside an allowed runtime event fail before queueing. A queued operation can fail later when applied by Mutter or the session service.

| Error code         | Meaning                                                          |
| ------------------ | ---------------------------------------------------------------- |
| `invalid_argument` | An operation argument or event-produced config value is invalid. |
| `not_found`        | The requested object or resource no longer exists.               |
| `unsupported`      | The current compositor or session cannot perform the operation.  |
| `denied`           | Session policy or security state rejects the operation.          |
| `busy`             | An exclusive input operation is already active.                  |
| `cancelled`        | The user cancelled the operation.                                |
| `timed_out`        | The operation reached its deadline.                              |
| `unavailable`      | The compositor or session service is not ready.                  |
| `internal`         | An unexpected implementation error occurred.                     |

```lua
-- Put this in a supervised runtime event callback.
local operation = gnoblin.workspaces.next()
operation:on_complete(function(workspace, err)
    if err then
        print(err.code, err.message)
        return
    end
    print("Active workspace:", workspace.name)
end)
```

The callback prints the activated workspace name or the operation error.

Related: [runtime API overview](/shell-api/), [events](/events/), and [compositor bridge errors](/bridge/connection).
