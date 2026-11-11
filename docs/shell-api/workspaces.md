# Workspaces

> **Status:** not available yet: `:activate`, `:rename`, `gnoblin.workspaces.active`. The sections that use them describe the target, not current behaviour.

For window-level workspace moves, see [Windows](/shell-api/windows). For displays, see [Outputs](/shell-api/outputs).

## Read workspaces

These methods return immediately:

| Method                         | Arguments            | Result                          |
| ------------------------------ | -------------------- | ------------------------------- |
| `gnoblin.workspaces.list()`    | None                 | All current `Workspace` objects |
| `gnoblin.workspaces.by_id(id)` | Integer workspace ID | Matching workspace or `nil`     |

`gnoblin.find("Workspace", filter?)` filters the same objects by their fields. The fields are `id` and `layout`. See
[find](/shell-api/find).

```lua
local first = gnoblin.find("Workspace", {id = 1})[1]
```

Every read of one workspace returns the same object. `workspace.alive` turns `false` once the workspace is gone, and
the object keeps its last state.

## Workspace records

| Field | Type    | Meaning                                            |
| ----- | ------- | -------------------------------------------------- |
| `id`  | Integer | The workspace's identity, stable for its lifetime. |

The workspace object signals:

- `created`, with `workspace`, when a workspace is created. Its `id` is already set when the handler runs.
- `removed`, with `workspace_id` and `last`, when a workspace is removed. The handler can still read `last`.

## Workspace sets

Workspace sets are not part of this API. They are a Lua table in your shell files, and `activate_set(name)` is a function
of the stock shell. See [Shell files](/user-shell#workspace-sets).

## Not in the schema yet

These names appear in earlier design notes. The `workspace` object does not declare them, so they are not available
from Lua:

- `workspace:activate()`, `workspace:rename(name)` and `workspace:remove()`;
- `workspace:on(signal, fn)` with the `changed`, `activated` and `closed` signals, and `workspace:watch(field, fn)`;
- `gnoblin.workspaces.active()`, `create`, `next` and `previous`;
- the workspace fields `number`, `name`, `active`, `window_count`, `persistent` and `revision`;
- string workspace IDs, such as `"mail"`, and session IDs such as `@session-N`;
- selecting a workspace by its one-based number.
