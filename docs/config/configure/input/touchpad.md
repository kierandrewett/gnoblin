# gnoblin.configure.input.touchpad

Set only the touchpad behavior you want to override. Unspecified fields keep
their current GNOME/Mutter values. Changes apply on configuration reload.

This example enables tap-to-click and two-finger scrolling:

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

- **`speed`** — Number from `-1` (unaccelerated) to `1` (fast). `0` uses the
  system default.
- **`scroll_speed`** — Number from `0` to `2`. `1` is the default; `0.5` is
  half speed and `2` is twice the speed.
- **`accel_profile`** — `"default"`, `"flat"`, or `"adaptive"`. These use the
  device default, constant pointer speed, or acceleration based on movement
  speed.
- **`left_handed`** — `"right"`, `"left"`, or `"mouse"`. The last choice follows
  the mouse button order.
- **`natural_scroll`** — Boolean. Reverses the scroll direction when `true`.

## Gestures

Each gesture field is a boolean. A gesture takes effect only when the device
supports it.

- **`tap_to_click`** — Click by tapping the touchpad.
- **`tap_and_drag`** — Start a drag by tapping, then moving a finger.
- **`tap_and_drag_lock`** — Keep a tap-and-drag active briefly after lifting
  your finger.
- **`disable_while_typing`** — Ignore touchpad input while typing. This can
  prevent pointer movement from a resting palm.
- **`edge_scrolling_enabled`** — Scroll along the touchpad edge.
- **`two_finger_scrolling_enabled`** — Scroll by moving two fingers.

## Click mapping

- **`tap_button_map`** — `"default"`, `"lrm"`, or `"lmr"`. `"lrm"` maps one-,
  two-, and three-finger taps to left, right, and middle click. `"lmr"` swaps
  the two- and three-finger mappings.
- **`click_method`** — `"default"`, `"none"`, `"areas"`, or `"fingers"`. Choose
  the device's hardware behavior, disable software-emulated clicks, or emulate
  clicks by touchpad area or finger count.

Omitted fields keep their current device preference. The `"default"` value asks
GNOME and libinput to choose the device behavior.

Use the Lua event API to change scrolling speed for the window under the
pointer. This also works when that window does not have keyboard focus:

```lua
gnoblin.on("pointer_window_changed", function(window)
    local speed = window.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

The event's `window` table includes `app_id`, `wm_class`, and `title`. See the
[Lua event API](/config/lua-events) for event names and payloads.

Available gestures depend on the hardware. GNOME's
[touchpad guide](https://help.gnome.org/gnome-help/mouse-touchpad-click.html)
explains tap, click, and scroll behavior. The
[libinput acceleration guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
describes the `adaptive` and `flat` profiles.
