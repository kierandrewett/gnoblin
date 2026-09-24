# Blur one application's background

Apply blur to a matching app. Replace the example app ID with the one used by
your application:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = [[^org\.example\.Terminal$]]},
    blur = 24,
    blur_ignore_shadows = true,
}
```

Blur is visible only through translucent parts of the app, so enable background
transparency in the terminal too. `blur_ignore_shadows` keeps the blur from
sampling translucent black shadow pixels. See [window effects](/guides/window_effects#blur-and-opacity).
