# Set up a focused writing workspace

Use a fixed set of named workspaces, center new windows and keep modal dialogs
with their parent. Give editor windows rounded corners and a restrained focus
border. The example matches GNOME Text Editor; replace its app ID after checking
your editor with `gnoblinctl window list --json` or the
[window-rule guide](/guides/window_rules#match-text).

```lua
gnoblin.configure {
    window_management = {
        dynamic_workspaces = false,
        num_workspaces = 4,
        workspace_names = {"Write", "Research", "Review", "Other"},
        center_new_windows = true,
        attach_modal_dialogs = true,
    },
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org.gnome.TextEditor$]]},
    corners = {radius = 10, smoothing = 0.5},
}

gnoblin.window_rule {
    match = {
        type = "window",
        app_id = [[^org.gnome.TextEditor$]],
        focused = true,
    },
    borders = {inner_width = 2, inner_color = "#72c7ce"},
}
```

The editor rule changes only matching windows. The later focused rule adds a
border while retaining the corner settings. It avoids lowering whole-window
opacity, which can make text harder to read. Window names are positional; keep
them in the same order as your workspace shortcuts expect. `num_workspaces` is
used while dynamic workspaces are off.

Reload the config, open the editor and a modal dialog, then check placement and
focus styling. If the editor does not match, use its actual GTK application ID
or WM class in the regular expression. See
[window management](/config/configure/window_management) and
[window effects](/guides/window_effects).
