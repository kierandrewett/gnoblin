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
            two_finger_scrolling_enabled = true,
        },
    },
}
```

## Pointer behavior

| Field            | Accepted values                        | What it changes                                                                                                      |
| ---------------- | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `speed`          | Number from `-1` to `1`                | `-1` is unaccelerated, `1` is fast, and `0` uses the system default.                                                 |
| `scroll_speed`   | Number from `0` to `2`                 | Scales two-finger and touchpad scrolling. `1` is the default speed, `0.5` is half speed, and `2` is twice the speed. |
| `accel_profile`  | `"default"`, `"flat"`, or `"adaptive"` | Uses the device default, a constant pointer speed, or acceleration based on movement speed.                          |
| `left_handed`    | `"right"`, `"left"`, or `"mouse"`      | Selects the touchpad button order; `"mouse"` follows the mouse setting.                                              |
| `natural_scroll` | Boolean                                | Reverses the scroll direction.                                                                                       |

## Gestures

| Field                          | Accepted values | Meaning                                                          |
| ------------------------------ | --------------- | ---------------------------------------------------------------- |
| `tap_to_click`                 | Boolean         | Click by tapping the touchpad.                                   |
| `tap_and_drag`                 | Boolean         | Drag by tapping, then moving a finger.                           |
| `tap_and_drag_lock`            | Boolean         | Keep a tap-and-drag active briefly after lifting your finger.    |
| `disable_while_typing`         | Boolean         | Helps prevent pointer movement from a resting palm while typing. |
| `edge_scrolling_enabled`       | Boolean         | Scroll along the touchpad edge, if supported.                    |
| `two_finger_scrolling_enabled` | Boolean         | Scroll by moving two fingers, if supported.                      |

## Click mapping

| Field            | Accepted values                                  | Meaning                                                                                                                            |
| ---------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `tap_button_map` | `"default"`, `"lrm"`, or `"lmr"`                 | `"lrm"` maps one-, two-, and three-finger taps to left, right, and middle click. `"lmr"` swaps the two- and three-finger mappings. |
| `click_method`   | `"default"`, `"none"`, `"areas"`, or `"fingers"` | Keep hardware behavior, disable software-emulated clicks, or emulate clicks using click areas or finger counts.                    |

Each field defaults to the current device preference when omitted. The
`"default"` enum value asks GNOME/libinput to choose the device behavior.

Use the Lua event API to change speed for the window under the pointer. This
also works when the pointer window has not taken keyboard focus:

```lua
gnoblin.on("pointer_window_changed", function(window)
    local speed = window.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

The event's `window` table includes `app_id`, `wm_class`, and `title`. See the
[Lua event API](/config/lua-events) for other event names and payloads.

The available gestures depend on the touchpad hardware. GNOME's
[touchpad guide](https://help.gnome.org/gnome-help/mouse-touchpad-click.html)
explains tap, click, and scroll behavior. The [libinput acceleration guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
describes the `adaptive` and `flat` profiles.
