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

| Field            | Accepted values                        | Default when omitted      | What it changes                                                                             |
| ---------------- | -------------------------------------- | ------------------------- | ------------------------------------------------------------------------------------------- |
| `speed`          | Number from `-1` to `1`                | Current device preference | `-1` is unaccelerated, `1` is fast, and `0` asks the system for its default speed.          |
| `left_handed`    | Boolean                                | Current device preference | Swaps the primary mouse buttons.                                                            |
| `natural_scroll` | Boolean                                | Current device preference | Reverses the scroll direction.                                                              |
| `accel_profile`  | `"default"`, `"flat"`, or `"adaptive"` | Current device profile    | Uses the device default, a constant pointer speed, or acceleration based on movement speed. |

If a device does not support the selected acceleration profile, it uses its
default profile.

Omitted values keep the current GNOME/Mutter preference for that device.
GNOME's [pointer-speed guide](https://help.gnome.org/gnome-help/mouse-sensitivity.html)
explains the user-facing speed setting. The [libinput guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
describes how `adaptive` and `flat` profiles affect pointer motion.

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
            accel_profile = "default" | "flat" | "adaptive"?,
        },
    },
}
```
