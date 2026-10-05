# gnoblin.configure.input

Input settings go in `gnoblin.configure {input = {...}}` and apply on config
reload. Groups and fields are optional. Gnoblin leaves anything you omit at its
current system setting, so start with a small override and add only what you
need.

`"inherit"` has the same effect for an individual field. Use it to clear a
value supplied by an earlier loaded config file. For example, this keeps the
GNOME pointer speed even if another file set a Gnoblin speed:

```lua
gnoblin.configure {input = {mouse = {speed = "inherit"}}}
```

GNOME Settings changes to inherited fields continue to take effect while the
session is running. An explicit Gnoblin value takes precedence until removed
or set to `"inherit"`.

For the common pointer and keyboard settings, see:

- [`gnoblin.configure.input.mouse`](/config/configure/input/mouse)
- [`gnoblin.configure.input.touchpad`](/config/configure/input/touchpad)
- [`gnoblin.configure.input.trackball` and `pointing_stick`](/config/configure/input/trackball-pointing-stick)
- [`gnoblin.configure.input.keyboard`](/config/configure/input/keyboard)

## Tablets

Tablet overrides use a four-digit hexadecimal vendor and product ID key, such
as `"1234:5678"`. Unlisted tablets keep their current system settings.

Find the device path with `libinput list-devices`, then inspect its IDs with
`udevadm info --query=property --name=/dev/input/eventN`. The [libinput tools
guide](https://wayland.freedesktop.org/libinput/doc/latest/tools.html) and
[`udevadm` manual](https://man7.org/linux/man-pages/man8/udevadm.8.html) explain
the commands and their output.

| Field         | Accepted values                    | Meaning                                                                                     |
| ------------- | ---------------------------------- | ------------------------------------------------------------------------------------------- |
| `mapping`     | `"absolute"` or `"relative"`       | Absolute maps pen position to a fixed tablet area; relative moves the pointer like a mouse. |
| `output`      | `"auto"` or a monitor connector ID | Maps the tablet to that active output; `"auto"` asks Mutter to choose.                      |
| `left_handed` | Boolean                            | Reverses the tablet's button orientation.                                                   |
| `keep_aspect` | Boolean                            | Preserves proportions when tablet and display have different shapes.                        |
| `area`        | Four fractions                     | Crops the tablet's active area.                                                             |
| `pad_buttons` | Array of button action tables      | Sets actions for tablet-pad buttons.                                                        |

Set `area` in left, right, top, bottom order. Each value is a fraction of the
tablet dimension: `0.1` means 10%. Values must be at least `0` and below `1`,
and opposing edges must add to less than `1`.

If you omit `area` or set it to `"inherit"`, Mutter uses the system preference.
Calibration applies only to integrated tablets that expose a libinput
calibration matrix; other devices keep their system behavior.

If you omit `output` or set it to `"inherit"`, Gnoblin follows the system
tablet mapping, including its saved output or automatic selection. Set
`output = "auto"` to explicitly use Mutter's automatic selection.

To choose an output, use its connector ID from `gnoblin.monitors.list()`, such
as `"DP-1"`. The ID can change when you move the display to another port. A
disconnected configured output leaves the tablet unmapped until it returns.
Cloned outputs map to their shared logical monitor.

The `"switch-monitor"` pad or stylus action keeps its system behavior when
`output` is omitted. With an explicit Lua output, it changes the tablet's
mapping for the current session; reloading the Lua config restores the
configured output. This session override does not change the system setting.

Tablet-pad button numbers start at `0`. Each entry needs a unique `button` from
`0` to `255` and an `action`:

| Action             | Effect                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------------ |
| `"default"`        | Use the current system action and keybinding for this button.                              |
| `"none"`           | Do not handle the button in the compositor; clients may receive the tablet-pad event.      |
| `"help"`           | Emit `gnoblin.input.pad-help-requested`. The shell displays and dismisses its own overlay. |
| `"switch-monitor"` | Cycle the tablet's mapped monitor.                                                         |
| `"keybinding"`     | Send the accelerator in `keybinding`.                                                      |

Buttons you omit keep their current system action. Add `keybinding` only with
the `"keybinding"` action. The list can contain up to 256 entries. Device IDs
identify vendor and product, so identical tablets share the same overrides.
Set `pad_buttons = "inherit"` to clear an earlier list and return every button
to its system action.

The `"help"` action has no built-in overlay. Subscribe to the
[pad-help event](/config/runtime-api#tablet-pad-help) in your shell.

Enable `keep_aspect` when you want a drawn circle to stay circular across
different display and tablet shapes.

```lua
gnoblin.configure {
    input = {
        tablets = {
            ["1234:5678"] = {
                mapping = "absolute",
                output = "DP-1", -- use an ID from gnoblin.monitors.list()
                keep_aspect = true,
                area = {0.03, 0.03, 0.05, 0.05},
                pad_buttons = {
                    {button = 0, action = "keybinding", keybinding = "<Super>e"},
                    {button = 1, action = "switch-monitor"},
                    {button = 2, action = "none"},
                },
            },
        },
    },
}
```

## Styluses

Stylus overrides use either a hexadecimal device serial or a vendor and
product ID prefixed by `default-`, such as `"default-1234:5678"`. Use the
device event path with `udevadm info --query=property` to inspect available
serial properties. See the [`udevadm` reference](https://man7.org/linux/man-pages/man8/udevadm.8.html)
and [libinput tools](https://wayland.freedesktop.org/libinput/doc/latest/tools.html).

Set an action to `"default"` to leave the button unchanged. To assign a
shortcut, use `"keybinding"` and provide the matching keybinding field. The
eraser button uses `eraser_button_mode` to choose between its default behavior
and acting as a button with its own action.

```lua
gnoblin.configure {
    input = {
        styluses = {
            ["default-1234:5678"] = {
                eraser_button_mode = "button",
                eraser_button_action = "keybinding",
                eraser_button_keybinding = "<Super>e",
                secondary_button_action = "keybinding",
                secondary_button_keybinding = "<Super>r",
            },
        },
    },
}
```

| Action                | Effect                                  |
| --------------------- | --------------------------------------- |
| `"default"`           | Keep the system's button behavior.      |
| `"middle"`, `"right"` | Send a middle or right click.           |
| `"back"`, `"forward"` | Send a navigation button click.         |
| `"switch-monitor"`    | Switch the stylus to another monitor.   |
| `"keybinding"`        | Run the matching configured keybinding. |

| `eraser_button_mode` | Effect                                                          |
| -------------------- | --------------------------------------------------------------- |
| `"default"`          | Keep the eraser button's default behavior.                      |
| `"button"`           | Use `eraser_button_action` and its matching keybinding, if any. |

The primary button uses `button_action` and `button_keybinding`. Secondary and
tertiary buttons use the corresponding `secondary_` and `tertiary_` fields.
The eraser button uses `eraser_button_action` and
`eraser_button_keybinding`. An empty keybinding has no effect unless its action
is `"keybinding"`.

## Orientation lock

| `orientation_lock`     | Behavior                             |
| ---------------------- | ------------------------------------ |
| `true`                 | Lock the current screen orientation. |
| `false`                | Allow automatic rotation.            |
| `"inherit"` or omitted | Follow the system setting.           |

On config reload, Gnoblin reapplies a configured boolean. An omitted value or
`"inherit"` clears the override and follows the system setting.

Use this override when the system rotation preference should not change screen
orientation for this config.

```lua
gnoblin.configure {
    input = {
        orientation_lock = true,
    },
}
```

The keyboard `numlock_state` override is kept in memory while Gnoblin's config
is active; removing it or setting it to `"inherit"` restores GNOME's saved
state and normal Num Lock persistence.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.
Mouse, touchpad, and keyboard fields are listed on their linked pages.
Every optional input field can also be set to `"inherit"`.

```lua
gnoblin.configure {
    input = {
        mouse = {...}?,
        touchpad = {...}?,
        keyboard = {...}?,
        orientation_lock = boolean | "inherit"?,
        tablets = {
            ["vvvv:pppp"] = {
                mapping = "absolute" | "relative"?,
                output = string?, -- "auto" or a monitor connector ID
                left_handed = boolean?,
                keep_aspect = boolean?,
                area = {number, number, number, number} | "inherit"?,
                pad_buttons = TabletPadButtonAction[] | "inherit"?,
            }, ...,
        }?,
        styluses = {
            ["serial-or-default-vvvv:pppp"] = {
                eraser_button_mode = StylusEraserMode?,
                eraser_button_action = StylusAction?,
                eraser_button_keybinding = string?,
                button_action = StylusAction?,
                button_keybinding = string?,
                secondary_button_action = StylusAction?,
                secondary_button_keybinding = string?,
                tertiary_button_action = StylusAction?,
                tertiary_button_keybinding = string?,
            }, ...,
        }?,
    },
}

-- StylusAction = "default" | "middle" | "right" | "back" | "forward"
--             | "switch-monitor" | "keybinding"
-- TabletPadButtonAction = {button = integer 0..255, action = TabletPadAction,
--                          keybinding = string?}
-- TabletPadAction = "default" | "none" | "help" | "switch-monitor" | "keybinding"
-- StylusEraserMode = "default" | "button"
```
