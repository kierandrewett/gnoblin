# Set up a writing workspace

Give a writing workspace a stable ID, send new editor windows there, and use
`gnoblinctl` to switch between workspaces or move an open window. Add this to
`~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines:

```lua
gnoblin.configure {
    workspaces = {
        {id = "write", name = "Write"},
        {id = "research", name = "Research"},
        {id = "review", name = "Review"},
        {id = "other", name = "Other"},
    },
    window_management = {
        center_new_windows = true,
        attach_modal_dialogs = true,
    },
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org%.gnome%.TextEditor$]]},
    workspace = {id = "write"},
    corners = {radius = 10, smoothing = 0.5},
}

gnoblin.window_rule {
    match = {
        type = "window",
        app_id = [[^org%.gnome%.TextEditor$]],
        workspace_id = "write",
        focused = true,
    },
    borders = {inner_width = 2, inner_color = "#72c7ce"},
}
```

Each object declares a persistent workspace. Its ID stays attached when the
workspace position changes; the list order sets the initial positions. Declare
every ID used by a placement effect or workspace matcher here.

| Field  | Accepted value                                                       | Meaning                                                         |
| ------ | -------------------------------------------------------------------- | --------------------------------------------------------------- |
| `id`   | Required, unique string matching `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$` | Stable workspace identity used by rules, Lua, and `gnoblinctl`. |
| `name` | Required, nonempty string up to 80 characters                        | Label shown for the workspace.                                  |

The list must contain at least one entry. Each declared workspace stays
available when empty and across sessions. If `workspaces` is omitted, Gnoblin
starts with four unconfigured workspaces.

The editor's `workspace` effect runs once when it opens. Modal dialogs stay
with their parent. The border rule follows the editor on the `write` workspace
and adds a border only while it is focused.

## Create a temporary workspace from Lua

This callback creates a session-only `Review session` workspace the first time
you activate `review`. It checks the current workspace snapshot first, so
switching away and back does not create duplicates. Add it to the same Lua file:

```lua
local creating = false
local requests = {}

gnoblin.on("gnoblin.workspace.activated", function(event)
    if event.id ~= "review" or creating then return end
    for _, workspace in ipairs(gnoblin.workspaces.list()) do
        if workspace.id == "review-session" then return end
    end

    creating = true
    local operation = gnoblin.workspaces.create {
        id = "review-session",
        name = "Review session",
    }
    requests[operation.id] = true
end)

gnoblin.on("gnoblin.operation.completed", function(event)
    if not requests[event.operation_id] then return end
    requests[event.operation_id] = nil
    creating = false

    if not event.ok then
        print("Workspace action failed: " .. event.error.message)
    end
end)
```

`gnoblin.workspaces.list()` reads the current snapshot immediately. Creating a
workspace returns an operation handle; Gnoblin reports its result through
`gnoblin.operation.completed`. See [Lua events](/config/lua-events) for event
payloads and callback rules.

Runtime-created workspaces are temporary even when you provide an ID.
Config-declared workspaces cannot be removed, and Gnoblin rejects removal of
the active or occupied workspace. The [Lua runtime API](/config/runtime-api)
lists available methods and result fields.

| Call or field                  | Accepted value                                | Meaning                                         |
| ------------------------------ | --------------------------------------------- | ----------------------------------------------- |
| `workspaces.list()`            | No arguments                                  | Returns an immediate immutable snapshot.        |
| `workspaces.create` `name`     | Required, nonempty string up to 80 characters | Display name for the new workspace.             |
| `workspaces.create` `id`       | Optional unique ID for this session           | Gives the temporary workspace a predictable ID. |
| `workspaces.create` `activate` | Boolean; default `false`                      | Switches to the new workspace when `true`.      |

Only workspace creation returns an operation handle and completion event.

Save the file, then reload and inspect the workspace IDs and positions:

```sh
gnoblinctl config reload
gnoblinctl workspace list
```

Switch using the stable ID or the current one-based position. Move the focused
window to a workspace and switch there with `--follow`:

```sh
gnoblinctl workspace switch --id research
gnoblinctl workspace switch --number 2
gnoblinctl workspace next
gnoblinctl workspace previous
gnoblinctl workspace move-active --id review --follow
```

To move a particular open window without changing the active workspace, get
its window ID from `gnoblinctl window list` and run:

```sh
gnoblinctl window workspace 42 --id write
gnoblinctl window workspace 42 --number 1
```

Replace `42` with the ID shown on your system. Workspace numbers can change
when workspaces are removed or reordered; use configured IDs in saved rules
and scripts. See [workspace settings](/config/configure/window_management),
the [window rules guide](/guides/window_rules#workspaces), [Lua events](/config/lua-events), and the
[`gnoblinctl` reference](/gnoblinctl#workspaces-and-monitors).
