# gnoblin.configure.input.keyboard

Use this group for key repeat, XKB options, and keyboard accessibility. This
example turns Caps Lock into Escape and selects that as the active XKB option:

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

## Accessibility

Set `accessibility` to configure Mutter's keyboard accessibility features.
Omitted fields keep their current system setting. On a system using the schema
defaults, booleans are `false`, delay values are `300` ms, and Mouse Keys max
speed is `10` pixels per second. A system preference can change those fallback
values. Use `"inherit"` on a field or feature group to remove a Gnoblin
override.

`shortcuts_enabled` controls keyboard shortcuts that toggle accessibility
features. It does not turn all the features on or off. Each feature's
`enabled` field controls that feature.

| Field                             | Accepted values                    | Effect                                                          |
| --------------------------------- | ---------------------------------- | --------------------------------------------------------------- |
| `shortcuts_enabled`               | Boolean                            | Enables keyboard shortcuts for toggling accessibility features. |
| `beep_on_feature_state_change`    | Boolean                            | Beeps when an accessibility feature changes state.              |
| `bounce_keys.enabled`             | Boolean                            | Ignores repeated presses of a key during the bounce delay.      |
| `bounce_keys.delay_ms`            | Integer, 0–10000 ms                | Minimum interval between accepted presses of the same key.      |
| `bounce_keys.beep_on_reject`      | Boolean                            | Beeps when a repeated key press is rejected.                    |
| `mouse_keys.enabled`              | Boolean                            | Moves the pointer with the keyboard.                            |
| `mouse_keys.max_speed`            | Integer, 1–10000 pixels per second | Maximum pointer speed when using Mouse Keys.                    |
| `mouse_keys.acceleration_time_ms` | Integer, 1–10000 ms                | Time to accelerate from rest to maximum pointer speed.          |
| `mouse_keys.initial_delay_ms`     | Integer, 0–10000 ms                | Delay before Mouse Keys starts moving the pointer.              |
| `slow_keys.enabled`               | Boolean                            | Accepts a key press only after the key is held for the delay.   |
| `slow_keys.delay_ms`              | Integer, 0–10000 ms                | Time a key must be held before Mutter accepts the press.        |
| `slow_keys.beep_on_press`         | Boolean                            | Beeps when a key is first pressed.                              |
| `slow_keys.beep_on_accept`        | Boolean                            | Beeps when a held key press is accepted.                        |
| `slow_keys.beep_on_reject`        | Boolean                            | Beeps when a key press is rejected.                             |
| `sticky_keys.enabled`             | Boolean                            | Lets modifier keys apply to one key press at a time.            |
| `sticky_keys.two_key_off`         | Boolean                            | Disables Sticky Keys when two keys are pressed together.        |
| `sticky_keys.beep_on_modifier`    | Boolean                            | Beeps when a modifier key is pressed.                           |
| `toggle_keys.enabled`             | Boolean                            | Beeps when a lock key such as Caps Lock changes state.          |

For example, this enables Slow Keys with a 400 ms hold and leaves the other
accessibility settings at their system values:

```lua
gnoblin.configure {
    input = {
        keyboard = {
            accessibility = {
                slow_keys = {
                    enabled = true,
                    delay_ms = 400,
                },
            },
        },
    },
}
```

`remember_numlock_state` controls whether GNOME remembers the Num Lock LED
state between sessions. `numlock_state` temporarily overrides that state while
Gnoblin's config is active. Omitted fields keep their current GNOME/Mutter
values. Every field also accepts `"inherit"` to restore that value when an
earlier config file supplied an override.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    input = {
        keyboard = {
            ["repeat"] = boolean?,
            delay = integer?, -- 1–10000 ms
            repeat_interval = integer?, -- 1–10000 ms
            remember_numlock_state = boolean?,
            numlock_state = boolean?,
            xkb_options = {string, ...} | {}?,
            accessibility = {
                shortcuts_enabled = boolean?,
                beep_on_feature_state_change = boolean?,
                bounce_keys = {
                    enabled = boolean?, delay_ms = integer?, beep_on_reject = boolean?,
                }?,
                mouse_keys = {
                    enabled = boolean?, max_speed = integer?,
                    acceleration_time_ms = integer?, initial_delay_ms = integer?,
                }?,
                slow_keys = {
                    enabled = boolean?, delay_ms = integer?, beep_on_press = boolean?,
                    beep_on_accept = boolean?, beep_on_reject = boolean?,
                }?,
                sticky_keys = {
                    enabled = boolean?, two_key_off = boolean?, beep_on_modifier = boolean?,
                }?,
                toggle_keys = {enabled = boolean?}?,
            }?,
        },
    },
}
```
