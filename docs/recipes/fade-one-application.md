# Fade one application's windows

Animate only windows from the selected application. Replace the example app ID
with the GTK application ID or WM class used by your app:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = [[^org\.example\.Editor$]]},
    animation = {
        ["in"] = "fade",
        out = "fade",
        duration = 180,
        easing = "ease-out-cubic",
    },
}
```

The exact-match expression uses JavaScript regular-expression syntax. The
duration is in milliseconds. See [window rules](/guides/window_rules#match-text)
for finding and matching an app ID.
