# Enable touchpad tap-to-click

Turn on tapping, two-finger scrolling and disable-while-typing:

```lua
gnoblin.configure {
    input = {
        touchpad = {
            tap_to_click = true,
            two_finger_scrolling_enabled = true,
            disable_while_typing = true,
        },
    },
}
```

Omitted touchpad options keep their current GNOME/Mutter values. Two-finger
scrolling takes effect only on devices that support it. See the
[touchpad reference](/config/configure/input/touchpad) for click and scroll
options.
