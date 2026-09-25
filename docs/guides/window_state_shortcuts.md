# window_state_shortcuts

[Configuration reference](/config/configure)

Bind Super+Down to restore a maximised/snapped window first, then minimise it
on a second press.

## Configure the binding

`keybindings.wm` selects Mutter's window-manager action group. `minimize` and
`unmaximize` are GSettings action names. Empty lists clear their bindings.
Action names depend on the installed GNOME version; list them with
`gsettings list-keys org.gnome.desktop.wm.keybindings`. See the
[keybinding reference](/config/configure/keybindings#find-an-action).

Add this to `~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines:

```lua
gnoblin.configure {
    keybindings = {
        wm = {minimize = {}, unmaximize = {}},
    },
    shortcuts = {
        restore_or_minimize = {
            binding = "<Super>Down",
            command = {"gnoblinctl", "window", "restore-or-minimize", "active"},
        },
    },
}
```

Run `gnoblinctl config reload` to apply the change.

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
