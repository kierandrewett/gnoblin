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

| Field                                               | Values                                   |
| --------------------------------------------------- | ---------------------------------------- |
| `repeat`, `remember_numlock_state`, `numlock_state` | Boolean                                  |
| `delay`, `repeat_interval`                          | Integer from `1` to `10000` milliseconds |
| `xkb_options`                                       | Array of XKB option strings              |

`delay` is how long a key must be held before it starts repeating;
`repeat_interval` is the time between repeats. Set `repeat = false` to turn
repeat off.

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
