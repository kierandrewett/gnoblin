# gnoblin.configure.frame_renderers

Register a frame renderer by name, then select it in a window rule. Names can
contain up to 64 letters, numbers, `_` or `-`; `native` is reserved. Each value
is an array of 1–32 strings: the first item is the executable, and later items
are passed to it unchanged. Gnoblin accepts an absolute executable path or a
command found on the compositor's `PATH`.

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"gnoblin-frame-cairo"},
    },
}

gnoblin.window_rule {
    match = {type = "window"},
    frame = {mode = "auto", renderer = "cairo"},
}
```

Use the [window rule reference](/config/window_rule#frame-fields) to choose
which windows get a frame. Renderer-specific arguments are passed through;
check the renderer's documentation for supported options.
