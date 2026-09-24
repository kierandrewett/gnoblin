# Older keybindings configuration

Existing configs can keep using `gnoblin.configure {keybindings = {...}}` to
override built-in Shell, window-manager, Mutter, and Wayland actions. New
shortcut settings belong in [`gnoblin.configure.shortcuts`](/config/configure/shortcuts),
where command shortcuts and built-in actions use one named map.

For example, change an older window-close override from:

```lua
gnoblin.configure {
    keybindings = {wm = {close = {"<Super>q"}}},
}
```

to:

```lua
gnoblin.configure {
    shortcuts = {
        close_window = {action = "wm.close", binding = {"<Super>q"}},
    },
}
```

The compatibility field remains supported. Do not configure the same built-in
action in both fields. Use `gsettings list-keys` with the schema names in the
[shortcut reference](/config/configure/shortcuts) to find action names.
