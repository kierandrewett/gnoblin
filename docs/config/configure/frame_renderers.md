# gnoblin.configure.frame_renderers

Configure this part of `gnoblin.configure` with the `frame_renderers` key.

Register renderer commands with `frame_renderers`. Each value is an argument
list of 1–32 strings. Its first item can be an absolute executable path or a
command name resolved through the compositor's `PATH`. Remaining items are
passed as separate arguments, without shell expansion. The name `native` is
reserved. Select the registered name in a `frame` field on
[`gnoblin.window_rule`](/config/window_rule#frame-fields).

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
