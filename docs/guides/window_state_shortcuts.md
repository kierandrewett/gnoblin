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

## Disable compositor corner effects on maximised windows

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {
        radius = 12,
        border_width = 1,
        border_color = "#505050ff",
        keep_maximized = false,
    },
}
```

When a window is maximised, Gnoblin disables its configured rounding and
outline. This does not change rounded corners drawn by the application itself.

See [shortcuts](/guides/shortcuts) for conflicts and persistent bindings.
