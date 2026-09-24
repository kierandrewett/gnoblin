# Name fixed workspaces

Use four stable workspaces and give each a name:

```lua
gnoblin.configure {
    window_management = {
        dynamic_workspaces = false,
        num_workspaces = 4,
        workspace_names = {"Main", "Web", "Work", "Chat"},
    },
}
```

`num_workspaces` is used while dynamic workspaces are off. Workspace names are
ordered to match their positions; each name can contain up to 80 characters.
See the [window-management reference](/config/configure/window_management).
