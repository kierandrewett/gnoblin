# gnoblin.configure.input.touchpad

Set only the touchpad behavior you want to override. Unspecified fields keep
their current GNOME/Mutter values. Changes apply on configuration reload.
This example enables tap-to-click and
two-finger scrolling:

```lua
gnoblin.configure {
    input = {
        touchpad = {
            tap_to_click = true,
            middle_click_emulation = true,
            two_finger_scrolling_enabled = true,
            disable_while_typing_timeout = 500,
        },
    },
}
```

## Pointer behavior

| Field            | Accepted values                                    | What it changes                                                                                                      |
| ---------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `speed`          | Number from `-1` to `1`                            | `-1` is unaccelerated, `1` is fast, and `0` uses the system default.                                                 |
| `scroll_speed`   | Number from `0` to `2`                             | Scales two-finger and touchpad scrolling. `1` is the default speed, `0.5` is half speed, and `2` is twice the speed. |
| `accel_profile`  | `"default"`, `"flat"`, `"adaptive"`, or `"custom"` | Selects the system curve or a custom profile.                                                                        |
| `accel_curve`    | `{step = number, points = number[]}`               | Defines custom pointer acceleration. Requires `accel_profile = "custom"`; it affects pointer motion, not scrolling.  |
| `left_handed`    | `"right"`, `"left"`, or `"follow-mouse"`           | Selects the touchpad button order; `"follow-mouse"` uses the mouse setting. `"mouse"` still loads as the old name.   |
| `natural_scroll` | Boolean                                            | Reverses the scroll direction.                                                                                       |

## Gestures

| Field                          | Accepted values | Meaning                                                          |
| ------------------------------ | --------------- | ---------------------------------------------------------------- |
| `tap_to_click`                 | Boolean         | Click by tapping the touchpad.                                   |
| `tap_and_drag`                 | Boolean         | Drag by tapping, then moving a finger.                           |
| `tap_and_drag_lock`            | Boolean         | Keep a tap-and-drag active briefly after lifting your finger.    |
| `disable_while_typing`         | Boolean         | Helps prevent pointer movement from a resting palm while typing. |
| `disable_while_typing_timeout` | 100–5000 ms     | Wait this long after typing before enabling the touchpad again.  |
| `edge_scrolling_enabled`       | Boolean         | Scroll along the touchpad edge, if supported.                    |
| `two_finger_scrolling_enabled` | Boolean         | Scroll by moving two fingers, if supported.                      |

Set `disable_while_typing_timeout` to an integer from 100 to 5000
milliseconds. Omit it or set it to `"inherit"` to keep the system value when
available. An explicit Lua value works even when the installed settings schema
does not define this key. The touchpad must support disable-while-typing, and
the timeout takes effect only while that behavior is enabled.

## Click mapping

| Field                    | Accepted values                                  | Meaning                                                                                                                            |
| ------------------------ | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `tap_button_map`         | `"default"`, `"left-right-middle"`, or `"left-middle-right"` | `"left-right-middle"` maps one-, two-, and three-finger taps to left, right, and middle click. `"left-middle-right"` swaps the two- and three-finger mappings. `"lrm"` and `"lmr"` still load as the old names. |
| `click_method`           | `"default"`, `"none"`, `"areas"`, or `"fingers"` | Keep hardware behavior, disable software-emulated clicks, or emulate clicks using click areas or finger counts.                    |
| `middle_click_emulation` | Boolean                                          | Setting true enables middle-click when both buttons are pressed; omitted uses the current device preference, normally off.         |

Each field defaults to the current device preference when omitted. The
`"default"` enum value asks GNOME/libinput to choose the device behavior.
Every field also accepts `"inherit"` to restore the GNOME/Mutter preference
when an earlier config file supplied a value.

Set the initial scroll speed from the window under the pointer when the config
loads. Keyboard focus does not affect which window the event reports:

```lua
gnoblin.on("mutter.wayland.pointer-window-changed", function(event)
    local speed = event.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

The callback table includes `app_id`, `wm_class`, and `title`. This event runs
when the config loads; it does not track pointer movement continuously. See the
[Lua event API](/config/lua-events) for its payload and callback behavior.

For `accel_curve`, use a positive `step` and at least two finite,
non-negative `points`. The points define output speed at evenly spaced input
speeds in device units per millisecond. libinput interpolates between points
and extrapolates beyond the last one. Its implementation sets the maximum
number of points. When the custom curve is active, libinput ignores `speed`
for pointer behavior.

The available gestures depend on the touchpad hardware. GNOME's
[touchpad guide](https://help.gnome.org/gnome-help/mouse-touchpad-click.html)
explains tap, click, and scroll behavior. The [libinput acceleration guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
explains the curve units and behavior.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field;
`|` separates accepted alternatives.

```lua
gnoblin.configure {
    input = {
        touchpad = {
            speed = number?, -- -1 to 1
            scroll_speed = number?, -- 0 to 2
            accel_profile = "default" | "flat" | "adaptive" | "custom"?,
            accel_curve = {step = number, points = number[]}?,
            left_handed = "right" | "left" | "follow-mouse"?,
            natural_scroll = boolean?,
            tap_to_click = boolean?,
            middle_click_emulation = boolean?,
            tap_and_drag = boolean?,
            tap_and_drag_lock = boolean?,
            disable_while_typing = boolean?,
            disable_while_typing_timeout = integer?, -- 100 to 5000 milliseconds
            edge_scrolling_enabled = boolean?,
            two_finger_scrolling_enabled = boolean?,
            tap_button_map = "default" | "left-right-middle" | "left-middle-right"?,
            click_method = "default" | "none" | "areas" | "fingers"?,
        },
    },
}
```
