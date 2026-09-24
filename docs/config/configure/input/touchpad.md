# gnoblin.configure.input.touchpad

Configure touchpad behavior with `gnoblin.configure.input.touchpad`:

```lua
gnoblin.configure {
    input = {
        touchpad = {
            speed = 0.0,
            tap_to_click = true,
            two_finger_scrolling_enabled = true,
            click_method = "default",
        },
    },
}
```

| Field                                                                                                                                                   | Values                                           |
| ------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `speed`                                                                                                                                                 | Number from `-1` to `1`                          |
| `left_handed`                                                                                                                                           | `"right"`, `"left"`, or `"mouse"`                |
| `natural_scroll`, `tap_to_click`, `tap_and_drag`, `tap_and_drag_lock`, `disable_while_typing`, `edge_scrolling_enabled`, `two_finger_scrolling_enabled` | Boolean                                          |
| `accel_profile`                                                                                                                                         | `"default"`, `"flat"`, or `"adaptive"`           |
| `tap_button_map`                                                                                                                                        | `"default"`, `"lrm"`, or `"lmr"`                 |
| `click_method`                                                                                                                                          | `"default"`, `"none"`, `"areas"`, or `"fingers"` |

Omitted fields keep their corresponding GNOME/Mutter setting. Changes apply on
config reload.
