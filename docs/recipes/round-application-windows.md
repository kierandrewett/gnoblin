# Round application windows

Set the corner radius and smoothing for application windows:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 14, smoothing = 0.6, mode = "auto"},
}
```

Radius uses logical pixels; smoothing is a 0–1 shape parameter. Automatic mode
preserves existing client corners. See [window effects](/guides/window_effects#rounded-window-corners)
for state exceptions and how to force a mask deliberately.
