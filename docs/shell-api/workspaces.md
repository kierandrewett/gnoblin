# Workspaces

For window-level workspace moves, see [Windows](/shell-api/windows). For
displays, see [Outputs](/shell-api/outputs). Event subscriptions are listed in
[Lua events](/events/).

## Workspaces

Select a workspace with exactly one selector field:

| Field    | Type   | Meaning                                                      |
| -------- | ------ | ------------------------------------------------------------ |
| `id`     | String | Select a stable workspace ID, such as `"code"`.              |
| `number` | 1–1024 | Select its current one-based position in the workspace list. |

IDs declared in `gnoblin.configure.workspaces` persist across sessions. IDs
supplied to runtime `create` are temporary and last only for the session.
Gnoblin-generated `@session-N` IDs can be selected during that session but
should not be saved in configuration.
See the [workspace configuration reference](/config/configure/window_management)
and the [writing workspace recipe](/recipes/writing-workspace).

## Read workspaces

These methods return immediately:

| Method                         | Arguments           | Result                          |
| ------------------------------ | ------------------- | ------------------------------- |
| `gnoblin.workspaces.list()`    | None                | All current `Workspace` objects |
| `gnoblin.workspaces.active()`  | None                | Active workspace or `nil`       |
| `gnoblin.workspaces.by_id(id)` | String workspace ID | Matching workspace or `nil`     |

To select workspaces by field, call `gnoblin.find("Workspace", filter)`. A table filter matches
declared scalar fields such as `id`, `number`, `name` and `active`, and a predicate function
also works. See [find](/shell-api/find).

```lua
local active = gnoblin.find("Workspace", {active = true})[1]
```

Like [window objects](/shell-api/windows#live-window-objects), every read of
one workspace returns the same object, updated with each change:

- `workspace.alive` turns `false` when the workspace is removed.
- `workspace:on(signal, fn)` takes `"changed"`, `"activated"` or `"closed"`.
- `workspace:watch(field, fn)` calls `fn(workspace, value, old)` when an
  observable field, `number`, `name`, `active` or `window_count`, changes.
- `workspace.revision` is the state revision of the snapshot. It goes up with
  every change of the workspace list, so reading it again after a rename gives a
  higher number.

Gnoblin assigns session workspace IDs once the config is committed at startup.
A workspace object read before then ends, and the same workspace appears again
under its session ID.

## Create and change workspaces

| Method                                      | Arguments                      | Successful result     |
| ------------------------------------------- | ------------------------------ | --------------------- |
| `gnoblin.workspaces.create(name, options?)` | Name; options `id`, `activate` | New `Workspace`       |
| `gnoblin.workspaces.next()`                 | None                           | Activated `Workspace` |
| `gnoblin.workspaces.previous()`             | None                           | Activated `Workspace` |

Each result is the same `Workspace` object that `gnoblin.workspaces.by_id(id)` returns, so the two compare equal.

Change one workspace through its object, and move a window with
`window:move_to_workspace(workspace, options?)`. A workspace argument there is a
`Workspace` object, its ID string or a one-based workspace number from 1 to 1024:

```lua
gnoblin.workspaces.list()[2]:activate()
gnoblin.workspaces.by_id("mail"):rename("Mail")
gnoblin.windows.focused():move_to_workspace(3, {follow = true})
gnoblin.windows.focused():move_to_workspace(gnoblin.workspaces.by_id("mail"))
```

Mutations return operation handles and complete asynchronously. For socket
operation names and fields, see the [compositor bridge](/bridge/).

Names for `create` and `rename` are nonempty and at most 80 characters. The
options follow these rules:

- `id` must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. Without it, Gnoblin
  generates a session-only ID.
- `activate` defaults to `false`.
- `follow` defaults to `false`. When true, moving a window also switches to the
  destination workspace.

A runtime rename changes the live workspace only. It does not edit your Lua
configuration or change workspace names saved for GNOME. A config reload restores
the declared name; update `gnoblin.configure {workspaces = {...}}` to keep a name
across reloads and sessions.

## Workspace records

A `Workspace` record has `id`, `number`, `name`, `active`, `window_count`, and
`persistent` fields. Gnoblin does not remove a declared workspace, the active
workspace, or a workspace that still contains windows.

Workspace objects also provide these methods. Call them with colon syntax
from a runtime event callback; each returns an `Operation` handle.

| Method                   | Arguments                         | Effect                              |
| ------------------------ | --------------------------------- | ----------------------------------- |
| `workspace:activate()`   | None                              | Activate this workspace.            |
| `workspace:rename(name)` | Nonempty name up to 80 characters | Rename this workspace.              |
| `workspace:remove()`     | None                              | Remove this workspace when allowed. |
