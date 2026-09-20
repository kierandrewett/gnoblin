# CLI development

The user reference is [gnoblinctl](gnoblinctl.md).

## Command design

- Use singular groups and plain actions: `GROUP ACTION`.
- A bare group shows help without contacting the compositor.
- Define arguments once for help, validation and completion.
- Keep full values in JSON; shorten only terminal tables.
- Write results to stdout and errors to stderr.
- Exit 0 for success, 1 for runtime failure, 2 for invalid arguments.
- Do not retry an uncertain mutation.
- Keep configuration in Lua; do not add a second settings store.

## Transport

Settings use D-Bus through `busctl`. Input-source reads can use the dedicated
service when the Shell interface is absent. Failed mutations do not fall back.

Window commands use the private bridge socket:

```json
{ "op": "command", "id": "REQUEST_ID", "command": "windows" }
```

```json
{ "event": "reply", "id": "REQUEST_ID", "result": { "windows": [] } }
```

Errors keep the request ID and use `event: "error"` with a message.
Other command names include `monitors`, `workspaces`,
`workspace-switch` and `window`.

## Change a window

Request (the window ID comes from a previous list):

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
