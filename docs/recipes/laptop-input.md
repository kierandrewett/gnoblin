# Set up a laptop for two keyboard layouts

This setup keeps US and UK layouts available per window, maps Caps Lock to Escape, and enables touchpad tapping and
two-finger scrolling. Change the IDs to the layouts you actually use.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    inputSources: {
        sources: [
            { type: "xkb", id: "us" },
            { type: "xkb", id: "gb" },
        ],
        perWindow: true,
    },
    input: {
        keyboard: {
            xkbOptions: ["caps:escape", "grp:alt_shift_toggle"],
        },
        touchpad: {
            tapToClick: true,
            tapAndDrag: true,
            disableWhileTyping: true,
            twoFingerScrollingEnabled: true,
        },
    },
});
```

The source list replaces the active layouts, and the XKB option list replaces the active keyboard options. Include
every layout and option you want to keep. The keyboard options and the source list apply; the
`touchpad` group is accepted and checked and nothing reads it, so the compositor logs `not applied: ...` for it and the
touchpad keeps its system settings.

| Setting                                    | Type and accepted values                                                                     | Default                                                 | Effect                                                                                                                   |
| ------------------------------------------ | -------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `inputSources.sources`                     | List of `{ type, id }` records. `type` is `"xkb"` or `"ibus"`; `id` is a nonempty source ID. | Empty; Gnoblin leaves the current keymap in place.      | Replaces the available keyboard sources. Choose IDs from the keyboard layouts or input methods installed on your system. |
| `inputSources.perWindow`                   | Boolean                                                                                      | `false`                                                 | Remembers and restores each window's source. Unset windows inherit the source active at first focus.                     |
| `input.keyboard.xkbOptions`                | List of XKB option strings                                                                   | Session keyboard options stay in use when omitted.      | Replaces keyboard options such as `"caps:escape"` and `"grp:alt_shift_toggle"`.                                          |
| `input.touchpad.tapToClick`                | Boolean                                                                                      | The session touchpad setting stays in use when omitted. | Enables or disables tapping to click.                                                                                    |
| `input.touchpad.tapAndDrag`                | Boolean                                                                                      | The session touchpad setting stays in use when omitted. | Enables or disables dragging by tapping and moving.                                                                      |
| `input.touchpad.disableWhileTyping`        | Boolean                                                                                      | The session touchpad setting stays in use when omitted. | Temporarily disables touchpad input while typing.                                                                        |
| `input.touchpad.twoFingerScrollingEnabled` | Boolean                                                                                      | The session touchpad setting stays in use when omitted. | Enables or disables two-finger scrolling.                                                                                |

The source IDs and XKB option strings depend on installed layouts, input methods, and XKB rules. Use the Keyboard panel
in GNOME Settings to see the available sources. `localectl list-x11-keymap-options` lists the XKB options available
from the system's installed rules.

With `perWindow: true`, switching focus restores that window's last selected layout. For example, select US in a
terminal and UK in a document window, then move the pointer between them with sloppy focus enabled.

Alt+Shift cycles layouts in the focused window, through the `grp:alt_shift_toggle` option. This follows focus; it does
not choose a layout automatically from the app ID.

References: [input sources](/config/configure/input_sources), [keyboard](/config/configure/input/keyboard), and
[touchpad](/config/configure/input/touchpad).
