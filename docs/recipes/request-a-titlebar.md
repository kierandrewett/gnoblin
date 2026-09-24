# Request a Gnoblin titlebar

Provide a server titlebar when an application requests Gnoblin to draw its
frame:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    frame = {mode = "auto", renderer = "native", extents = {36, 0, 0, 0}},
}
```

This draws a 36-pixel titlebar for apps that request a server frame. Apps that
draw their own titlebars keep them. `extents` gives the top, right, bottom and
left sizes in logical pixels. See [window frames](/guides/window_frames) for
the available modes.
