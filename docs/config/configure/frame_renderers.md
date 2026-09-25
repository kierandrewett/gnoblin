# gnoblin.configure.frame_renderers

Register a frame renderer by name, then select it in a window rule.

| Field         | Accepted values                                                 | Default and effect                                                                                                                    |
| ------------- | --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Renderer name | 1–64 letters, numbers, `_` or `-`; unique; `native` is reserved | User-defined name referenced by a window rule.                                                                                        |
| Command       | Array of 1–32 strings                                           | Required. First item is an absolute executable path or a command on the compositor's `PATH`; remaining items are passed as arguments. |

Gnoblin starts the configured renderer when the config loads. Updating a
renderer restarts it during config reload.

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
