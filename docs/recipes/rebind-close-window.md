# Rebind close window

Override the window-manager close action with Super+Q:

```lua
gnoblin.configure {
    keybindings = {
        wm = {close = {"<Super>q"}},
    },
}
```

This changes the built-in close action; it does not launch a command. Remove
the `wm.close` override and reload to restore the built-in binding. Use
[`gnoblin.configure.shortcuts`](/config/configure/shortcuts) when a key should
launch a program instead.
