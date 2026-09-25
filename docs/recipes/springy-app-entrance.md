# Give an app a springy entrance

Animate one application's windows with a small overshoot when they open, then
shrink and fade them when they close. Replace the example app ID with the value
reported by `gnoblinctl window list --json`.

```lua
gnoblin.animation {
    name = "spring-open",
    event = "open",
    duration = 340,
    ease = "ease-out-cubic",
    keyframes = {
        {at = 0, y = 22, scale = 0.9, opacity = 0},
        {at = 0.72, y = -4, scale = 1.025, opacity = 1},
        {at = 1, y = 0, scale = 1, opacity = 1},
    },
}

gnoblin.animation {
    name = "quick-close",
    event = "close",
    duration = 140,
    ease = "ease-in-quad",
    from = {scale = 1, opacity = 1},
    to = {scale = 0.88, opacity = 0},
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org.gnome.TextEditor$]]},
    animation = {open = "spring-open", close = "quick-close"},
}
```

The middle keyframe grows slightly past full size, then settles. Only windows
matching the rule use these animations; all other windows keep their current
profiles. Change `app_id` to target another application.

Keyframe positions (`at`) run from `0` to `1`; `scale = 1` is the original
size. See the [animation guide](/guides/animations#properties-and-keyframes)
and [window-rule animation fields](/config/window_rule#animation-fields).
