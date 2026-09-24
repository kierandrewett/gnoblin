# Set up a laptop for two keyboard layouts

This setup keeps US and UK layouts available per window, maps Caps Lock to
Escape, and enables touchpad tapping and two-finger scrolling. Change the IDs
to the layouts you actually use.

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
        },
        per_window = true,
    },
    input = {
        keyboard = {
            xkb_options = {"caps:escape", "grp:alt_shift_toggle"},
        },
        touchpad = {
            tap_to_click = true,
            tap_and_drag = true,
            disable_while_typing = true,
            two_finger_scrolling_enabled = true,
        },
    },
}
```

The source list replaces the active layouts, and the XKB option list replaces
the active keyboard options. Include every layout and option you want to keep.
With `per_window = true`, switching to another window restores that window's
last selected layout. Alt+Shift cycles the listed layouts.

If tap-and-drag feels too easy to trigger, remove that line and retain tap to
click. Unsupported touchpad gestures are ignored by devices that do not offer
them. Apply the change with `gnoblinctl config reload`, then test typing,
clicking and switching layouts in two windows.

References: [input sources](/config/configure/input_sources),
[keyboard](/config/configure/input/keyboard), and
[touchpad](/config/configure/input/touchpad).
