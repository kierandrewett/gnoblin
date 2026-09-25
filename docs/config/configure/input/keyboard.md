# gnoblin.configure.input.keyboard

Use this group for key repeat and XKB options. This example turns Caps Lock
into Escape and selects that as the active XKB option:

```lua
gnoblin.configure {
    input = {
        keyboard = {
            xkb_options = {"caps:escape"},
        },
    },
}
```

| Field                    | Values                      | Default                    | Effect                                                               |
| ------------------------ | --------------------------- | -------------------------- | -------------------------------------------------------------------- |
| `repeat`                 | Boolean                     | Current GNOME/Mutter value | Enables or disables key repeat.                                      |
| `delay`                  | Integer, 1–10000 ms         | Current GNOME/Mutter value | Time a key must be held before repeating starts.                     |
| `repeat_interval`        | Integer, 1–10000 ms         | Current GNOME/Mutter value | Time between repeated key presses.                                   |
| `remember_numlock_state` | Boolean                     | Current GNOME/Mutter value | Remembers the Num Lock LED state between sessions.                   |
| `numlock_state`          | Boolean                     | Current GNOME/Mutter value | Temporarily sets the Num Lock LED state while this config is active. |
| `xkb_options`            | Array of XKB option strings | Current option list        | Replaces the active option list.                                     |

Changes apply on configuration reload. Set `repeat = false` to turn repeat
off; the delay fields then have no effect.

`xkb_options` replaces the active option list, so include every option you want
to keep. Common values include:

- `"caps:escape"` — make Caps Lock an additional Escape key.
- `"compose:ralt"` — use Right Alt as the Compose key.
- `"grp:alt_shift_toggle"` — switch layouts with Alt+Shift.

You can combine options in the array. The upstream
[XKB configuration guide](https://xkeyboard-config.freedesktop.org/doc/config/)
explains how they work; `/usr/share/X11/xkb/rules/evdev.lst` lists the options
available on your system.

`remember_numlock_state` controls whether GNOME remembers the Num Lock LED
state between sessions. `numlock_state` temporarily overrides that state while
Gnoblin's config is active. Omitted fields keep their current GNOME/Mutter
values.
