# window_state_shortcuts

[Configuration reference](/config/configure)

Bind Super+Down to restore a maximised/snapped window first, then minimise it
on a second press.

## 1. Release the existing binding

Add this to `~/.config/gnoblin/init.lua`, after any `gnoblin.load(...)` lines,
then run `gnoblinctl config reload`:

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
gnoblin.configure {
    shortcuts = {
        restore_or_minimize = {
            binding = "<Super>Down",
            command = {"gnoblinctl", "window", "restore-or-minimize", "active"},
        },
    },
}
```

Press Super+Down on a maximised or snapped window to return it to its previous
size. Press it again to minimise. On an ordinary floating window, the first
press minimises immediately.

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

See [shortcuts](/guides/shortcuts) for conflicts and persistent bindings.
