# Highlight the focused window

Draw an accent-coloured inner border around the active application window:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = true},
    borders = {
        inner_width = 2,
        inner_color = "#66c7ce",
    },
}
```

The width uses logical pixels and accepts values from 0 to 40. The colour
accepts `#RRGGBB` or `#RRGGBBAA`. See the [window-rule reference](/config/window_rule#rule-fields).
