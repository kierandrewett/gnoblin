# Combine window rules

Use one rule for shared defaults, then a later rule to override selected fields. Put this in
`~/.config/gnoblin/shell.lua`:

```lua
seat.data.window_rules = {
    { match = {}, opacity = 1, maximised = false },
    { match = { focused = false }, opacity = 0.95 },
}
```

The first rule applies to every window. The second changes only opacity when a window loses focus; its `maximised`
value from the first rule remains in effect.
