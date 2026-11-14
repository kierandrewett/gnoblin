# Keyboard settings

The `input.keyboard` group sets key repeat, XKB options and keyboard accessibility. It is the one input group this
version applies, and changes apply on a config reload. This example turns Caps Lock into Escape:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        keyboard: {
            xkbOptions: ["caps:escape"],
        },
    },
});
```

| Field                  | Values                     | Default                    | Effect                                                               |
| ---------------------- | -------------------------- | -------------------------- | -------------------------------------------------------------------- |
| `repeat`               | Boolean                    | Current GNOME/Mutter value | Enables or disables key repeat.                                      |
| `delay`                | Integer, 1–10000 ms        | Current GNOME/Mutter value | Time a key must be held before repeating starts.                     |
| `repeatInterval`       | Integer, 1–10000 ms        | Current GNOME/Mutter value | Time between repeated key presses.                                   |
| `rememberNumlockState` | Boolean                    | Current GNOME/Mutter value | Remembers the Num Lock LED state between sessions.                   |
| `numlockState`         | Boolean                    | Current GNOME/Mutter value | Temporarily sets the Num Lock LED state while this config is active. |
| `xkbOptions`           | List of XKB option strings | Current option list        | Replaces the active option list.                                     |

Set `repeat: false` to turn repeat off; the delay fields then have no effect.

`xkbOptions` replaces the active option list, so include every option you want to keep. Common values include:

- `"caps:escape"`: make Caps Lock an additional Escape key.
- `"compose:ralt"`: use Right Alt as the Compose key.
- `"grp:alt_shift_toggle"`: switch layouts with Alt+Shift.

You can combine options in the list. The upstream
[XKB configuration guide](https://xkeyboard-config.freedesktop.org/doc/config/) explains how they work;
`/usr/share/X11/xkb/rules/evdev.lst` lists the options available on your system.

## Accessibility

Set `accessibility` to configure the keyboard accessibility features. Omitted fields keep their current system
setting. On a system using the schema defaults, booleans are `false`, delay values are `300` ms, and Mouse Keys max
speed is `10` pixels per second. A system preference can change those fallback values. Use `"inherit"` on a field or
feature group to remove a Gnoblin override.

`shortcutsEnabled` controls keyboard shortcuts that toggle accessibility features. It does not turn all the features
on or off. Each feature's `enabled` field controls that feature.

| Field                          | Accepted values                    | Effect                                                          |
| ------------------------------ | ---------------------------------- | --------------------------------------------------------------- |
| `shortcutsEnabled`             | Boolean                            | Enables keyboard shortcuts for toggling accessibility features. |
| `beepOnFeatureStateChange`     | Boolean                            | Beeps when an accessibility feature changes state.              |
| `bounceKeys.enabled`           | Boolean                            | Ignores repeated presses of a key during the bounce delay.      |
| `bounceKeys.delayMs`           | Integer, 0–10000 ms                | Minimum interval between accepted presses of the same key.      |
| `bounceKeys.beepOnReject`      | Boolean                            | Beeps when a repeated key press is rejected.                    |
| `mouseKeys.enabled`            | Boolean                            | Moves the pointer with the keyboard.                            |
| `mouseKeys.maxSpeed`           | Integer, 1–10000 pixels per second | Maximum pointer speed when using Mouse Keys.                    |
| `mouseKeys.accelerationTimeMs` | Integer, 1–10000 ms                | Time to accelerate from rest to maximum pointer speed.          |
| `mouseKeys.initialDelayMs`     | Integer, 0–10000 ms                | Delay before Mouse Keys starts moving the pointer.              |
| `slowKeys.enabled`             | Boolean                            | Accepts a key press only after the key is held for the delay.   |
| `slowKeys.delayMs`             | Integer, 0–10000 ms                | Time a key must be held before the press is accepted.           |
| `slowKeys.beepOnPress`         | Boolean                            | Beeps when a key is first pressed.                              |
| `slowKeys.beepOnAccept`        | Boolean                            | Beeps when a held key press is accepted.                        |
| `slowKeys.beepOnReject`        | Boolean                            | Beeps when a key press is rejected.                             |
| `stickyKeys.enabled`           | Boolean                            | Lets modifier keys apply to one key press at a time.            |
| `stickyKeys.twoKeyOff`         | Boolean                            | Disables Sticky Keys when two keys are pressed together.        |
| `stickyKeys.beepOnModifier`    | Boolean                            | Beeps when a modifier key is pressed.                           |
| `toggleKeys.enabled`           | Boolean                            | Beeps when a lock key such as Caps Lock changes state.          |

A dotted name is a field of the nested object: `slowKeys.delayMs` is `slowKeys: { delayMs: 400 }`. For example, this
enables Slow Keys with a 400 ms hold and leaves the other accessibility settings at their system values:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        keyboard: {
            accessibility: {
                slowKeys: {
                    enabled: true,
                    delayMs: 400,
                },
            },
        },
    },
});
```

`rememberNumlockState` controls whether GNOME remembers the Num Lock LED state between sessions. `numlockState`
temporarily overrides that state while Gnoblin's config is active. Omitted fields keep their current GNOME/Mutter
values. Every field also accepts `"inherit"` to restore that value.
