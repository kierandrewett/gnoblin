# Combine window rules

Use one rule for shared defaults, then a later rule to override selected
properties:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 12, smoothing = 0.5},
    opacity = 1,
}

gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

The first rule applies to application windows. The second changes only opacity
when a window loses focus; its corner settings remain in effect.
