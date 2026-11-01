# Errors and Results

Gnoblin's Lua runtime API reports the outcome of an asynchronous operation through a result object.

## Operation results

An asynchronous Lua operation has these parts:

| Part          | What it holds                                                                           |
| ------------- | --------------------------------------------------------------------------------------- |
| `id`          | An identifier for the operation                                                         |
| `method`      | The name of the operation                                                               |
| `status`      | `pending`, `succeeded` or `failed`                                                      |
| `value`       | The result when the status is `succeeded`; `nil` otherwise                              |
| `error`       | A table with a stable `code` and a `message` when `status` is `failed`; `nil` otherwise |
| `on_complete` | Registers a callback that receives `(value, error)`                                     |

Invalid arguments and calls outside an allowed runtime event fail before the operation is queued. A queued operation
can fail later, when Mutter or the session service applies it.

## Error codes

| Error code         | Meaning                                                          |
| ------------------ | ---------------------------------------------------------------- |
| `invalid_argument` | An operation argument or event-produced config value is invalid. |
| `not_found`        | The requested object or resource does not exist.                 |
| `unsupported`      | The compositor or session cannot perform the operation.          |
| `denied`           | Session policy or security state rejects the operation.          |
| `busy`             | An exclusive input operation is already active.                  |
| `cancelled`        | The user cancelled the operation.                                |
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
