# Dim unfocused windows

Set the opacity of windows that are not focused. Put this in `~/.config/gnoblin/shell.lua`:

```lua
seat.data.window_rules = {
    { match = { focused = false }, opacity = 0.95 },
}
```

`focused = false` matches windows that do not have focus. `opacity` is a number from `0` (transparent) to `1`
(opaque); the example uses `0.95`. The rule applies to windows, not layer surfaces.

Opacity affects text and controls too. For a translucent panel with opaque text, use transparency in the client's
background instead. Remove the rule and restart the session to go back to full opacity; a window whose rule stops
matching returns to opacity `1`.

See the [window rule reference](/config/window_rule) for all match keys and field ranges.
