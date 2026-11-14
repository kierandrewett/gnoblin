# Mouse settings

Set only the mouse options you want to change. Gnoblin leaves omitted options at their current GNOME/Mutter values, so
a small override keeps the rest of your pointer preferences intact.

These settings are accepted and checked, and no device reads them: the compositor logs `not applied: ...` for the
`input.mouse` group.

For example, this sets a left-handed button layout, a custom acceleration curve, and a double-click time of 350
milliseconds:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        mouse: {
            speed: -0.2,
            doubleClickTime: 350,
            middleClickEmulation: true,
            leftHanded: true,
            accelProfile: "custom",
            accelCurve: { step: 1.0, points: [0.0, 1.0, 2.0, 4.0] },
        },
    },
});
```

| Field                  | Accepted values                                    | Default when omitted                       | What it changes                                                                    |
| ---------------------- | -------------------------------------------------- | ------------------------------------------ | ---------------------------------------------------------------------------------- |
| `speed`                | Number from `-1` to `1`                            | Current device preference                  | `-1` is unaccelerated, `1` is fast, and `0` asks the system for its default speed. |
| `doubleClickTime`      | Integer from `0` to `2147483647` milliseconds      | Current system preference (normally `400`) | Time between presses for them to count as a double-click.                          |
| `dragThreshold`        | Integer from `1` to `2147483647`                   | Current system preference (normally `8`)   | Pointer distance in pixels before movement counts as a drag.                       |
| `middleClickEmulation` | Boolean                                            | Current device preference (normally off)   | Enables middle-click when the left and right buttons are pressed together.         |
| `leftHanded`           | Boolean                                            | Current device preference                  | Swaps the primary mouse buttons.                                                   |
| `naturalScroll`        | Boolean                                            | Current device preference                  | Reverses the scroll direction.                                                     |
| `accelProfile`         | `"default"`, `"flat"`, `"adaptive"`, or `"custom"` | Current profile                            | Selects the system curve or a custom profile.                                      |
| `accelCurve`           | `{ step: number, points: number[] }`               | Current system curve                       | Sets pointer speed at evenly spaced input speeds. Needs `accelProfile: "custom"`.  |

If a device does not support the selected acceleration profile, it uses its default profile.

For a custom profile, `step` must be greater than zero. `points` must contain at least two finite, non-negative
numbers. The first point is the output speed at input speed zero.

Each later point is spaced `step` device units per millisecond farther along the input-speed axis. libinput linearly
interpolates between points and extrapolates beyond the last point. The maximum number of points is set by libinput.

The curve changes pointer motion; it does not define a separate scroll curve. When a custom curve is active, libinput
ignores `speed` for pointer behaviour.

Omitted values keep the current GNOME/Mutter preference for that device. Every field also accepts `"inherit"` to say
that you want that preference.

GNOME's [pointer-speed guide](https://help.gnome.org/gnome-help/mouse-sensitivity.html) explains the user-facing
speed setting. The [libinput guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
explains the units and interpolation used by custom curves.
