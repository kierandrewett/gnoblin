# CLI development

The user reference is [gnoblinctl](gnoblinctl.md).

## Command design

- Use singular groups and plain actions: `GROUP ACTION`.
- A bare group shows help without contacting the compositor.
- Define arguments once for help, validation and completion.
- Keep full values in JSON; shorten only terminal tables.
- Write results to stdout and errors to stderr.
- Exit 0 for success, 1 for runtime failure, 2 for invalid arguments.
- Do not retry a state-changing command after a timeout: the first request may
  already have taken effect.
- Keep configuration in Lua; do not add a second settings store.

## Transport

`gnoblinctl` sends session operations through the compositor socket. Shortcut
capture waits for a keypress in the terminal and returns a GTK accelerator.

Session API operations use the compositor socket with a canonical method name
and a JSON object of arguments:

```json
{ "op": "api", "id": "REQUEST_ID", "method": "workspace.list", "arguments": {} }
```

Lua calls the same methods with the same argument objects. The public session
methods are:

| Group         | Methods                                                                                          |
| ------------- | ------------------------------------------------------------------------------------------------ |
| `workspace`   | `list`, `create`, `rename`, `remove`, `switch`, `next`, `previous`, `move_active`, `move_window` |
| `window`      | `list`, `match`, `action`                                                                        |
| `layer`       | `list`                                                                                           |
| `monitor`     | `list`                                                                                           |
| `animation`   | `list`, `surfaces`, `inspect`, `preview`, `seek`, `step`, `play`, `pause`, `stop`                |
| `feature`     | `list`, `show`, `enable`, `disable`                                                              |
| `script`      | `list`                                                                                           |
| `input`       | `list`, `current`, `select`                                                                      |
| `privacy`     | `get`                                                                                            |
| `permissions` | `list`, `check`                                                                                  |
| `grant`       | `list`, `revoke`                                                                                 |
| `launch`      | `status`, `begin`, `end`                                                                         |
| `shell`       | `ping`, `version`, `status`, `reload`                                                            |
| `config`      | `reload`                                                                                         |
| `shortcut`    | `capture`                                                                                        |

`shortcut.capture` accepts `timeout`, an integer from 1 to 60 seconds. The
CLI defaults to 30 seconds and gives the socket request two extra seconds to
receive the result. Lua receives an operation ticket and handles completion
through `gnoblin.api.operation-completed`; see the [Lua runtime API](/config/runtime-api).

Methods use the `domain.method` form on the socket. Pass either
`{ "id": "code" }` or `{ "number": 2 }` to select a workspace. Replies
retain the request ID and return a structured result:

```json
{ "event": "reply", "id": "REQUEST_ID", "result": { "workspaces": [] } }
```

Errors use `event: "error"`, retain the request ID and include a `message`.
Do not retry a state-changing call after a timeout; check the current state
first.

The bridge also retains its lower-level `command` protocol for integrations
that need compositor records directly. For example, `windows` returns window
records:

```json
{ "op": "command", "id": "REQUEST_ID", "command": "windows" }
```

```json
{ "event": "reply", "id": "REQUEST_ID", "result": { "windows": [] } }
```

The request `id` is an opaque string used to match a reply. `command` selects
the operation:

| Command            | Purpose                               |
| ------------------ | ------------------------------------- |
| `windows`          | List managed windows.                 |
| `monitors`         | List monitors and work areas.         |
| `workspaces`       | List workspace positions and IDs.     |
| `workspace-switch` | Activate a workspace by ID or number. |
| `window`           | Apply an action to a listed window.   |

Errors use `event: "error"`, retain the request ID, and include a message.
Replies use `event: "reply"`; `result` contains operation-specific data.
`pending: true` acknowledges an asynchronous request, not its final state.
See the [bridge operation reference](/compositor-bridge#operation-index) for
payload fields and selector values.

## Change a window

The `window` command's `action` selects an operation such as `"move"`. See the
[`gnoblinctl` window reference](/gnoblinctl#window-actions) for all actions.

`window` is a stable string ID from a previous list. For `"move"`, `x` and
`y` are logical desktop-pixel coordinates:

```json
{ "op": "command", "id": "move-1", "command": "window", "action": "move", "window": "42", "x": 100, "y": 80 }
```

Reply:

```json
{ "event": "reply", "id": "move-1", "result": { "ok": true, "pending": true, "window": "42", "action": "move" } }
```

If the session is locked:

```json
{ "event": "error", "id": "move-1", "message": "window management is unavailable while the session is locked" }
```

Match replies to the request `id`. Other events can arrive between them.
`pending` acknowledges the request; read the next window snapshot for its result.
The CLI prints the inner `result`, not the socket envelope.

## Tests

Local CLI tests cover names, help, output, validation and transport failures.
`tests/test-gnoblinctl.py` checks the installed command in a private compositor.
