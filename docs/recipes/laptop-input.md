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

| Setting                                       | Type and accepted values                                                                    | Default                                                              | Effect                                                                                                                   |
| --------------------------------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `input_sources.sources`                       | Array of `{type, id}` records. `type` is `"xkb"` or `"ibus"`; `id` is a nonempty source ID. | The active GNOME sources remain in use when this setting is omitted. | Replaces the available keyboard sources. Choose IDs from the keyboard layouts or input methods installed on your system. |
| `input_sources.per_window`                    | Boolean                                                                                     | `false`                                                              | Remembers and restores each window's source. Unset windows inherit the source active at first focus.                     |
| `input.keyboard.xkb_options`                  | Array of XKB option strings                                                                 | The active GNOME options remain in use when omitted.                 | Replaces keyboard options such as `"caps:escape"` and `"grp:alt_shift_toggle"`.                                          |
| `input.touchpad.tap_to_click`                 | Boolean                                                                                     | The current GNOME touchpad setting remains in use when omitted.      | Enables or disables tapping to click.                                                                                    |
| `input.touchpad.tap_and_drag`                 | Boolean                                                                                     | The current GNOME touchpad setting remains in use when omitted.      | Enables or disables dragging by tapping and moving.                                                                      |
| `input.touchpad.disable_while_typing`         | Boolean                                                                                     | The current GNOME touchpad setting remains in use when omitted.      | Temporarily disables touchpad input while typing.                                                                        |
| `input.touchpad.two_finger_scrolling_enabled` | Boolean                                                                                     | The current GNOME touchpad setting remains in use when omitted.      | Enables or disables two-finger scrolling.                                                                                |

The source IDs and XKB option strings depend on installed layouts, input
methods, and XKB rules. Use the Keyboard panel in GNOME Settings to see the
available sources. `localectl list-x11-keymap-options` lists the XKB options
available from the system's installed rules.

With `per_window = true`, switching focus restores that window's last selected
layout. For example, select US in a terminal and UK in a document window, then
move the pointer between them with sloppy focus enabled.

Alt+Shift cycles layouts in the focused window. This follows focus; it does not
choose a layout automatically from the app ID.

If tap-and-drag feels too easy to trigger, remove that line and retain tap to
click. Unsupported touchpad gestures are ignored by devices that do not offer
them. Apply the change with `gnoblinctl config reload`, then test typing,
clicking and switching layouts in two windows.

References: [input sources](/config/configure/input_sources),
[keyboard](/config/configure/input/keyboard), and
[touchpad](/config/configure/input/touchpad).
