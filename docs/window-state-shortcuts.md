# Restore or minimise

[Configuration reference](configuration-reference.md)

Bind Super+Down to restore a maximised/snapped window first, then minimise it
on a second press.

## 1. Release the existing binding

Add this after your includes and reload:

```lua
gnoblin.configure {
    keybindings = {
        wm = {
            minimize = {},
            unmaximize = {},
        },
    },
}
```

These empty lists release the built-in actions before the new command is bound.

## 2. Add the shortcut

Append this and reload again:

```lua
gnoblin.shortcut {
    name = "restore-or-minimize",
    binding = "<Super>Down",
    command = {"gnoblinctl", "window", "restore-or-minimize", "active"},
}
```

The command restores native maximisation/tiling or a saved custom snap frame.
If neither applies, it minimises the window.

## Square maximised windows

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 12, keep_maximized = false},
    borders = {inner_width = 1, inner_color = "#505050ff", keep_maximized = true},
}
```

Maximised borders become square. Sides touching the physical monitor edge are
omitted; sides beside reserved panels remain visible.

See [shortcuts](shortcuts.md) for conflicts and persistent bindings.
