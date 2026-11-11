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

- `set_activated`, with `set`, when the window manager makes a configured workspace set live. Every workspace the session holds raises it.

## Workspace sets

`gnoblin.workspaces.activate_set(name)` makes the configured workspace set `name` the live set. The sets are declared in the [window management](/config/configure/window_management) configuration. The call returns an operation that completes once the window manager shows the set's workspaces, including when the set is already live. An unknown name fails with `unknown-set`.

```lua
gnoblin.workspaces.activate_set("desk")
```

`gnoblinctl workspace activate-set desk` makes the same request from the command line.

## Not in the schema yet

These names appear in earlier design notes. The `workspace` object does not declare them, so they are not available
from Lua:

- `workspace:activate()`, `workspace:rename(name)` and `workspace:remove()`;
- `workspace:on(signal, fn)` with the `changed`, `activated` and `closed` signals, and `workspace:watch(field, fn)`;
- `gnoblin.workspaces.active()`, `create`, `next` and `previous`;
- the workspace fields `number`, `name`, `active`, `window_count`, `persistent` and `revision`;
- string workspace IDs, such as `"mail"`, and session IDs such as `@session-N`;
- selecting a workspace by its one-based number.
