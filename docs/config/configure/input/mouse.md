# gnoblin.configure.input.mouse

Configure mouse behavior with `gnoblin.configure.input.mouse`:

```lua
gnoblin.configure {
    input = {
        mouse = {
            speed = 0.0,
            left_handed = false,
            natural_scroll = false,
            accel_profile = "adaptive",
        },
    },
}
```

| Field                           | Values                                 |
| ------------------------------- | -------------------------------------- |
| `speed`                         | Number from `-1` to `1`                |
| `left_handed`, `natural_scroll` | Boolean                                |
| `accel_profile`                 | `"default"`, `"flat"`, or `"adaptive"` |

Omitted fields keep their corresponding GNOME/Mutter setting. Changes apply on
config reload.
