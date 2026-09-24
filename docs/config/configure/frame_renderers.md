# gnoblin.configure.frame_renderers

Configure this part of `gnoblin.configure` with the `frame_renderers` key.

Register named renderer commands with `frame_renderers`. Each value is an argument list; use an absolute executable path. `native` is reserved. Select the registered name in a `frame` field on [`gnoblin.window_rule`](/config/window_rule#frame-fields).

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"/absolute/path/gnoblin-frame-cairo"},
    },
}
```
