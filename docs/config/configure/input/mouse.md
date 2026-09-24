# gnoblin.configure.input.mouse

Set only the mouse options you want to change. Gnoblin leaves omitted options
at their current GNOME/Mutter values, so a small override keeps the rest of
your pointer preferences intact.

For example, this sets a left-handed button layout and a slightly slower,
adaptive pointer:

```lua
gnoblin.configure {
    input = {
        mouse = {
            speed = -0.2,
            left_handed = true,
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

`left_handed` swaps the primary mouse buttons. `speed` ranges from `-1`
(unaccelerated) to `1` (fast); `0` uses the system default. `natural_scroll`
reverses the scroll direction. `"default"` uses the device's default
acceleration profile, `"flat"` uses a constant factor, and `"adaptive"` adjusts
acceleration to movement speed. A device without the selected profile falls
back to its default.
