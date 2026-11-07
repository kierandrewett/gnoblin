# Workspaces

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

| Field    | Type    | Meaning                                                                                                                     |
| -------- | ------- | --------------------------------------------------------------------------------------------------------------------------- |
| `id`     | Integer | The workspace's identity, stable for its lifetime.                                                                          |
| `layout` | String  | The layout the workspace runs now: its own override when one was set, otherwise its output's default, otherwise `floating`. |

## Layouts

`workspace:set_layout(root, module, hash, options)` sets the workspace's own layout to a layout module. The reference
names the root it resolves under (`shipped` or `user`), the module name, the source hash and the options. The hash pins
the source, so an edit made after the configuration was checked is refused. An unknown module fails with
`unknown-layout`.

`workspace:layout_command(name, params)` sends a command to the layout the workspace runs. `params` is a JSON object,
`{}` when the command takes none. The layout answers with the placements the command produces. `refused` means the
layout does not know the command, and `unknown-layout` means the workspace has no layout to ask.

The workspace object signals:

- `layout_changed`, with `layout`, when the workspace's layout changes;
- `layout_failed`, with `layout` and `reason`, when a layout run fails. The workspace then runs floating until a run
  succeeds. `reason` is one of `deadline`, `memory`, `error`, `refused-reply`, `vm-wedged`, `vm-down`,
  `missing-module` or `invalid-params`.

## Not in the schema yet

These names appear in earlier design notes. The `workspace` object does not declare them, so they are not available
from Lua:

- `workspace:activate()`, `workspace:rename(name)`, `workspace:remove()` and `window:move_to_workspace(workspace, options?)`;
- `workspace:on(signal, fn)` with the `changed`, `activated` and `closed` signals, and `workspace:watch(field, fn)`;
- `gnoblin.workspaces.active()`, `create`, `next` and `previous`;
- the workspace fields `number`, `name`, `active`, `window_count`, `persistent` and `revision`;
- string workspace IDs, such as `"mail"`, and session IDs such as `@session-N`;
- selecting a workspace by its one-based number.
