# gnoblin.configure.input.keyboard

Configure keyboard behavior with `gnoblin.configure.input.keyboard`:

```lua
gnoblin.configure {
    input = {
        keyboard = {
            ["repeat"] = true,
            delay = 500,
            repeat_interval = 30,
            remember_numlock_state = true,
            xkb_options = {"caps:escape"},
        },
    },
}
```

| Field                                               | Values                                                   |
| --------------------------------------------------- | -------------------------------------------------------- |
| `repeat`, `remember_numlock_state`, `numlock_state` | Boolean                                                  |
| `delay`, `repeat_interval`                          | Integer from `1` to `10000` milliseconds                 |
| `xkb_options`                                       | Array of XKB option strings; replaces the active options |

`numlock_state` is kept in memory while Gnoblin's config is active. Omitted
fields keep their corresponding GNOME/Mutter setting. Changes apply on config
reload.
