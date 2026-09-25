# Set up a writing workspace

Give a writing workspace a stable ID, send new editor windows there, and use
`gnoblinctl` to switch between workspaces or move an open window. Add this to
`~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines:

```lua
gnoblin.configure {
    window_management = {
        dynamic_workspaces = false,
        num_workspaces = 4,
        workspace_names = {"Write", "Research", "Review", "Other"},
        workspace_ids = {"write", "research", "review", "other"},
        center_new_windows = true,
        attach_modal_dialogs = true,
    },
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org\.gnome\.TextEditor$]]},
    workspace = {id = "write"},
    corners = {radius = 10, smoothing = 0.5},
}

gnoblin.window_rule {
    match = {
        type = "window",
        app_id = [[^org\.gnome\.TextEditor$]],
        workspace_id = "write",
        focused = true,
    },
    borders = {inner_width = 2, inner_color = "#72c7ce"},
}
```

The ordered `workspace_ids` array assigns IDs by initial position. IDs follow
their workspaces if order changes; display names are separate. Declare every
ID used by a placement effect or workspace matcher in this array.

The editor's `workspace` effect runs once when it opens. Modal dialogs stay
with their parent. The border rule follows the editor on the `write` workspace
and adds a border only while it is focused.

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
when dynamic workspaces are removed or reordered; use configured IDs in saved
rules and scripts. See [workspace settings](/config/configure/window_management),
the [window rules guide](/guides/window_rules#workspaces), and the
[`gnoblinctl` reference](/gnoblinctl#workspaces-and-monitors).
