# Input settings

Input settings go in the `input` key of `defineConfig`. Write each name in camelCase: `leftHanded` for the document's
`left-handed`. Groups and fields are optional. Gnoblin leaves anything you omit at its current system setting, so
start with a small override and add only what you need.

Only the `keyboard` group is applied. These settings are accepted and checked, and no device reads them, so the
compositor logs `not applied: ...` for each:

- `mouse`, `touchpad`, `trackball` and `pointingStick`
- `tablets` and `styluses`
- `orientationLock`

Keyboard repeat, delay and xkb options apply on a reload.

`"inherit"` has the same effect for an individual field. Use it to say that you want the system setting, even where
an earlier config set one. For example, this keeps the GNOME pointer speed:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({ input: { mouse: { speed: "inherit" } } });
```

GNOME Settings changes to inherited fields continue to take effect while the session is running. An explicit Gnoblin
value takes precedence until removed or set to `"inherit"`.

For the common pointer and keyboard settings, see:

- [Mouse](/config/configure/input/mouse)
- [Touchpad](/config/configure/input/touchpad)
- [Trackball and pointing stick](/config/configure/input/trackball-pointing-stick)
- [Keyboard](/config/configure/input/keyboard)

## Tablets

Tablet overrides use a four-digit hexadecimal vendor and product ID key, such as `"1234:5678"`. Unlisted tablets keep
their current system settings.

Find the device path with `libinput list-devices`, then inspect its IDs with
`udevadm info --query=property --name=/dev/input/eventN`. The
[libinput tools guide](https://wayland.freedesktop.org/libinput/doc/latest/tools.html) and
[`udevadm` manual](https://man7.org/linux/man-pages/man8/udevadm.8.html) explain the commands and their output.

| Field        | Accepted values                    | Meaning                                                                                     |
| ------------ | ---------------------------------- | ------------------------------------------------------------------------------------------- |
| `mapping`    | `"absolute"` or `"relative"`       | Absolute maps pen position to a fixed tablet area; relative moves the pointer like a mouse. |
| `output`     | `"auto"` or a monitor connector ID | Maps the tablet to that active output; `"auto"` asks the compositor to choose.              |
| `leftHanded` | Boolean                            | Reverses the tablet's button orientation.                                                   |
| `keepAspect` | Boolean                            | Preserves proportions when tablet and display have different shapes.                        |
| `area`       | Four fractions                     | Crops the tablet's active area.                                                             |
| `padButtons` | List of button action objects      | Sets actions for tablet-pad buttons.                                                        |

Set `area` in left, right, top, bottom order. Each value is a fraction of the tablet dimension: `0.1` means 10%. Values
must be at least `0` and below `1`, and opposing edges must add to less than `1`.

If you omit `area` or set it to `"inherit"`, the system preference is used. Calibration applies only to integrated
tablets that expose a libinput calibration matrix; other devices keep their system behaviour.

If you omit `output` or set it to `"inherit"`, Gnoblin follows the system tablet mapping, including its saved output
or automatic selection. Set `output: "auto"` to explicitly use automatic selection.

To choose an output, use its connector ID from `gnoblinctl output list`, such as `"DP-1"`. The ID can change when you
move the display to another port. A disconnected configured output leaves the tablet unmapped until it returns. Cloned
outputs map to their shared logical monitor.

Tablet-pad button numbers start at `0`. Each entry needs a unique `button` from `0` to `255` and an `action`:

| Action             | Effect                                                                                |
| ------------------ | ------------------------------------------------------------------------------------- |
| `"default"`        | Use the current system action and keybinding for this button.                         |
| `"none"`           | Do not handle the button in the compositor; clients may receive the tablet-pad event. |
| `"help"`           | Accepted. The compositor sends the shell no help request, so nothing is shown.        |
| `"switch-monitor"` | Cycle the tablet's mapped monitor.                                                    |
| `"keybinding"`     | Send the accelerator in `keybinding`.                                                 |

Buttons you omit keep their current system action. Add `keybinding` only with the `"keybinding"` action. The list can
contain up to 256 entries. Device IDs identify vendor and product, so identical tablets share the same overrides. Set
`padButtons: "inherit"` to clear an earlier list and return every button to its system action.

A `keybinding` value is an accelerator in the compositor's own spelling, such as `<Super>e`. It is not the `Super+e`
trigger of the [`keys`](/guides/shortcuts) table.

Enable `keepAspect` when you want a drawn circle to stay circular across different display and tablet shapes.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        tablets: {
            "1234:5678": {
                mapping: "absolute",
                output: "DP-1", // an ID from `gnoblinctl output list`
                keepAspect: true,
                area: [0.03, 0.03, 0.05, 0.05],
                padButtons: [
                    { button: 0, action: "keybinding", keybinding: "<Super>e" },
                    { button: 1, action: "switch-monitor" },
                    { button: 2, action: "none" },
                ],
            },
        },
    },
});
```

## Styluses

Stylus overrides use either a hexadecimal device serial or a vendor and product ID prefixed by `default-`, such as
`"default-1234:5678"`. Use the device event path with `udevadm info --query=property` to inspect available serial
properties. See the [`udevadm` reference](https://man7.org/linux/man-pages/man8/udevadm.8.html) and
[libinput tools](https://wayland.freedesktop.org/libinput/doc/latest/tools.html).

Set an action to `"default"` to leave the button unchanged. To assign a shortcut, use `"keybinding"` and provide the
matching keybinding field. The eraser button uses `eraserButtonMode` to choose between its default behaviour and
acting as a button with its own action.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        styluses: {
            "default-1234:5678": {
                eraserButtonMode: "button",
                eraserButtonAction: "keybinding",
                eraserButtonKeybinding: "<Super>e",
                secondaryButtonAction: "keybinding",
                secondaryButtonKeybinding: "<Super>r",
            },
        },
    },
});
```

| Action                | Effect                                  |
| --------------------- | --------------------------------------- |
| `"default"`           | Keep the system's button behaviour.     |
| `"middle"`, `"right"` | Send a middle or right click.           |
| `"back"`, `"forward"` | Send a navigation button click.         |
| `"switch-monitor"`    | Switch the stylus to another monitor.   |
| `"keybinding"`        | Run the matching configured keybinding. |

| `eraserButtonMode` | Effect                                                        |
| ------------------ | ------------------------------------------------------------- |
| `"default"`        | Keep the eraser button's default behaviour.                   |
| `"button"`         | Use `eraserButtonAction` and its matching keybinding, if any. |

Each button has an action field and a keybinding field:

- The primary button uses `buttonAction` and `buttonKeybinding`.
- Secondary and tertiary buttons put a prefix first, as in `secondaryButtonAction` and `tertiaryButtonKeybinding`.
- The eraser button uses `eraserButtonAction` and `eraserButtonKeybinding`.

An empty keybinding has no effect unless its action is `"keybinding"`.

## Orientation lock

| `orientationLock`      | Behaviour                            |
| ---------------------- | ------------------------------------ |
| `true`                 | Lock the current screen orientation. |
| `false`                | Allow automatic rotation.            |
| `"inherit"` or omitted | Follow the system setting.           |

On config reload, Gnoblin reapplies a configured boolean. An omitted value or `"inherit"` clears the override and
follows the system setting. Use this override when the system rotation preference should not change screen
orientation for this config.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: { orientationLock: true },
});
```

The keyboard `numlockState` override is kept in memory while Gnoblin's config is active; removing it or setting it to
`"inherit"` restores GNOME's saved state and normal Num Lock persistence.
