# gnoblin.configure.input.touchpad

Set only the touchpad behavior you want to override. This example enables
tap-to-click and two-finger scrolling while leaving GNOME's other touchpad
choices unchanged:

```lua
gnoblin.configure {
    input = {
        touchpad = {
            tap_to_click = true,
            two_finger_scrolling_enabled = true,
        },
    },
}
```

`speed` ranges from `-1` (unaccelerated) to `1` (fast); `0` uses the system
default. `accel_profile` accepts `"default"`, `"flat"`, or `"adaptive"`.
`left_handed` accepts `"right"`, `"left"`, or `"mouse"` to follow the mouse
setting. `natural_scroll` reverses the scroll direction.

The gesture controls are booleans: `tap_to_click`, `tap_and_drag`,
`tap_and_drag_lock`, `disable_while_typing`, `edge_scrolling_enabled`, and
`two_finger_scrolling_enabled`. A drag lock keeps a tap-and-drag active briefly
after lifting your finger; the scrolling options enable the matching gesture
when the touchpad supports it. Disable-while-typing can help avoid pointer
movement from a resting palm.

`tap_button_map` accepts `"default"`, `"lrm"`, or `"lmr"`. `"lrm"` maps one-,
two-, and three-finger taps to left, right, and middle click; `"lmr"` swaps the
two- and three-finger mappings. `click_method` accepts `"default"`, `"none"`,
`"areas"`, or `"fingers"`: keep the device's hardware behavior, disable
software-emulated clicks, or emulate them using click areas or finger counts.
