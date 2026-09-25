# Request a Gnoblin titlebar

Provide a server titlebar when an application requests Gnoblin to draw its
frame:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    frame = {mode = "auto", renderer = "native", extents = {36, 0, 0, 0}},
}
```

| Field      | Accepted values                                   | Default         | Effect                                                                      |
| ---------- | ------------------------------------------------- | --------------- | --------------------------------------------------------------------------- |
| `mode`     | `"off"`, `"auto"`, `"prefer-server"`, `"replace"` | `"off"`         | `"auto"` adds a Gnoblin frame when the app requests server-side decoration. |
| `renderer` | Registered renderer name or `"native"`            | `"native"`      | Selects the frame renderer.                                                 |
| `extents`  | Four integers from 0 to 256                       | `{32, 1, 1, 1}` | Top, right, bottom, and left frame sizes in logical pixels.                 |

Apps that draw their own titlebars keep them in `"auto"` mode.
See [window frames](/guides/window_frames) for other modes and negotiation.
