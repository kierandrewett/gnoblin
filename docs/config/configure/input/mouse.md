# gnoblin.configure.input.mouse

Set only the mouse options you want to change. Gnoblin leaves omitted options
at their current GNOME/Mutter values, so a small override keeps the rest of
your pointer preferences intact.

For example, this sets a left-handed button layout and a custom acceleration
curve:

```lua
gnoblin.configure {
    input = {
        mouse = {
            speed = -0.2,
            left_handed = true,
            accel_profile = "custom",
            accel_curve = {step = 1.0, points = {0.0, 1.0, 2.0, 4.0}},
        },
    },
}
```

| Field            | Accepted values                                    | Default when omitted      | What it changes                                                                        |
| ---------------- | -------------------------------------------------- | ------------------------- | -------------------------------------------------------------------------------------- |
| `speed`          | Number from `-1` to `1`                            | Current device preference | `-1` is unaccelerated, `1` is fast, and `0` asks the system for its default speed.     |
| `left_handed`    | Boolean                                            | Current device preference | Swaps the primary mouse buttons.                                                       |
| `natural_scroll` | Boolean                                            | Current device preference | Reverses the scroll direction.                                                         |
| `accel_profile`  | `"default"`, `"flat"`, `"adaptive"`, or `"custom"` | Current profile           | Selects the system curve or a custom profile.                                          |
| `accel_curve`    | `{step = number, points = number[]}`               | Current system curve      | Sets pointer speed at evenly spaced input speeds. Requires `accel_profile = "custom"`. |

If a device does not support the selected acceleration profile, it uses its
default profile.

For a custom profile, `step` must be greater than zero. `points` must contain
at least two finite, non-negative numbers. The first point is the output speed
at input speed zero.

Each later point is spaced `step` device units per millisecond farther along
the input-speed axis. libinput linearly interpolates between points and
extrapolates beyond the last point. The maximum number of points is set by
libinput.

The curve changes pointer motion; it does not define a separate scroll curve.
When a custom curve is active, libinput ignores `speed` for pointer behavior.

Omitted values keep the current GNOME/Mutter preference for that device.
GNOME's [pointer-speed guide](https://help.gnome.org/gnome-help/mouse-sensitivity.html)
explains the user-facing speed setting. The [libinput guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
explains the units and interpolation used by custom curves.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field;
`|` separates accepted alternatives.

```lua
gnoblin.configure {
    input = {
        mouse = {
            speed = number?, -- -1 to 1
            left_handed = boolean?,
            natural_scroll = boolean?,
            accel_profile = "default" | "flat" | "adaptive" | "custom"?,
            accel_curve = {step = number, points = number[]}?,
        },
    },
}
```
