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
{
    "op": "api",
    "id": "REQUEST_ID",
    "api_version": { "major": 1, "minor": 37 },
    "method": "workspaces.list",
    "arguments": {}
}
```

Lua and CLI calls share the method registry. The CLI maps its actions and
arguments to canonical methods before dispatch. Mutter handles typed window
operations, which require the stable ID printed by `gnoblinctl window list`.

`gnoblinctl window list` reads the API 1.37 `windows.list` snapshot and keeps
the CLI's `windows` JSON wrapper. The list below is a method-name index. See
the [runtime API reference](/config/runtime-api) for method arguments,
accepted values, results, and compatibility limits.

`gnoblinctl workspace list` reads `workspaces.list` and keeps its `workspaces`
JSON wrapper. It maps each snapshot's `window_count` to the CLI's `windows`
field.

Raw socket clients can continue using the older `workspace.list` method.
API 1.52 and newer serve that compatibility method from the Lua snapshot while
keeping its original wrapper and `windows` count; earlier API versions use the
native route.

Raw socket clients can also use `window.list`. API 1.53 and newer serve it from
the Lua snapshot while keeping its original wrapper and field names; earlier
API versions use the native route.

The monitor and layer list commands also use their API 1.37 snapshot methods,
`monitors.list` and `layers.list`; the CLI wraps each returned array under its
existing `monitors` or `layers` key.

The input commands use `input.sources`, `input.current_source`, and
`input.select_source` at API 1.6. Source selection returns an operation that
completes after Mutter confirms the selected keyboard layout.

`gnoblinctl shortcut actions` reads `shortcuts.actions` from the shared Lua
runtime at API 1.41. Its optional group filter and action records match
`gnoblin.shortcuts.actions(group?)`.

| Group         | Methods                                                                                                                                                                                                                              |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `workspace`   | `list`, `create`, `rename`, `remove`, `switch`, `next`, `previous`, `move_active`, `move_window`                                                                                                                                     |
| `window`      | `list`, `match`, `action`, `close`, `minimize`, `toggle_minimize`, `restore`, `restore_or_minimize`, `set_maximized`, `set_fullscreen`, `set_above`, `set_sticky`, `move`, `resize`, `move_to_workspace`, `move_to_monitor`, `focus` |
| `layer`       | `list`                                                                                                                                                                                                                               |
| `monitor`     | `list`                                                                                                                                                                                                                               |
| `animation`   | `list`, `surfaces`, `inspect`, `preview`, `seek`, `step`, `play`, `pause`, `stop`                                                                                                                                                    |
| `feature`     | `list`, `show`, `enable`, `disable`                                                                                                                                                                                                  |
| `script`      | `list`                                                                                                                                                                                                                               |
| `input`       | `list`, `current`, `select`                                                                                                                                                                                                          |
| `privacy`     | `get`                                                                                                                                                                                                                                |
| `permissions` | `list`, `check`                                                                                                                                                                                                                      |
| `grant`       | `list`, `revoke`                                                                                                                                                                                                                     |
| `launch`      | `status`, `begin`, `end`                                                                                                                                                                                                             |
| `shell`       | `ping`, `version`, `status`, `reload`                                                                                                                                                                                                |
| `config`      | `reload`                                                                                                                                                                                                                             |
| `shortcut`    | `list`, `actions`, `capture`                                                                                                                                                                                                         |

`shortcut.capture` options:

| Argument  | Type or accepted values      | Default    | Effect                                                                                    |
| --------- | ---------------------------- | ---------- | ----------------------------------------------------------------------------------------- |
| `timeout` | Integer from 1 to 60 seconds | 30 seconds | Maximum wait for a shortcut press; the CLI allows two extra seconds for the socket reply. |

Lua receives an operation ticket and handles completion through
`gnoblin.operation.completed` on native API 1.11. The legacy
`gnoblin.api.operation-completed` event remains available during migration; see
the [Lua runtime API](/config/runtime-api).

Methods use the `domain.method` form on the socket. A workspace selector has
exactly one of these fields:

| Field    | Type or accepted values                           | Meaning                                                   |
| -------- | ------------------------------------------------- | --------------------------------------------------------- |
| `id`     | String from `workspaces.list` or `workspace.list` | Selects that stable workspace.                            |
| `number` | Integer from 1 to 1024                            | Selects the workspace at that current one-based position. |

Replies retain the request ID and return a structured result:

```json
{ "event": "reply", "id": "REQUEST_ID", "result": [] }
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

For a typed native operation, call its canonical method with a stable window
ID. The CLI maps each supported action to its typed method. The
`restore-or-minimize` action is handled natively and restores a saved snap frame
when one exists.

Setters take an `enabled` boolean. Move takes `x` and `y`; resize takes `width`
and `height`.

Workspace moves take a `workspace` selector with either an `id` or `number`.
Typed monitor moves take the connector ID printed by `gnoblinctl monitor list`.

For `active`, the CLI reads `windows.list` from the shared Lua runtime and
resolves the focused window's stable ID before sending the typed request. This
read requires compositor API 1.37.

For monitor moves, the CLI accepts the numeric monitor index and maps it to a
connector ID from the shared `monitors.list` snapshot. This read uses
compositor API 1.37.

Raw socket clients can use `window.action` for the actions documented in the
runtime API. Its `window` argument is a stable ID or `"active"`.

API 1.61 adds `action: "resize"` with integer `width` and `height` values from
1 to 32768.
API 1.62 adds `action: "move"` with integer `x` and `y` coordinates from
−100000 to 100000 logical pixels.

API 1.60 routes basic actions through the Lua runtime; API 1.61 also routes
resize through `window.resize`, and API 1.62 routes move through
`window.move`. These preserve the legacy response shape.
Earlier versions use the native compatibility route for basic actions. Focus,
menu, and interactive move or resize use their dedicated methods because they
require verified activation or menu context.

`toggle-minimize` is available through its typed method, which takes a stable
window ID.
See the [`gnoblinctl` window reference](/gnoblinctl#window-actions) for CLI
arguments and ranges.

`window` is a stable string ID from a previous list. For `window.move`, `x` and
`y` are logical desktop-pixel coordinates:

```json
{ "op": "api", "id": "move-1", "method": "window.move", "arguments": { "id": "42", "x": 100, "y": 80 } }
```

Reply:

```json
{ "event": "reply", "id": "move-1", "result": { "id": "42" } }
```

If the session is locked:

```json
{ "event": "error", "id": "move-1", "message": "window management is unavailable while the session is locked" }
```

Match replies to the request `id`. Other events can arrive between them.
The API reply contains the affected stable ID. The CLI prints the inner
`result`, not the socket envelope.

## Tests

Local CLI tests cover names, help, output, validation and transport failures.
`tests/test-gnoblinctl.py` checks the installed command in a private compositor.
