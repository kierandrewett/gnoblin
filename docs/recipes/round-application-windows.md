# Round application windows

> **Status:** not available: `gnoblin.window_rule` was removed (rules are `seat.data.window_rules` in a [shell file](/user-shell)), and the stock shell draws blur and shadow only, so the rule fields here do not work. The page describes the target, not current behaviour.

Set the corner radius and smoothing for application windows:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 14, smoothing = 0.6, mode = "auto"},
}
```

| Field       | Accepted values                     | Default  | Effect                                                              |
| ----------- | ----------------------------------- | -------- | ------------------------------------------------------------------- |
| `radius`    | Number from 0 to 200 logical pixels | `0`      | Sets the requested corner radius.                                   |
| `smoothing` | Number from 0 to 1                  | `0`      | Adjusts the corner curve.                                           |
| `mode`      | `"auto"`, `"force"`, or `"off"`     | `"auto"` | Preserve existing client corners, force a mask, or disable masking. |

The example uses `"auto"` to preserve existing client corners. See
[window effects](/guides/window_effects#rounded-window-corners) for state
exceptions and how to force a mask deliberately.
