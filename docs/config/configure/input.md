# gnoblin.configure.input

Input settings go in `gnoblin.configure {input = {...}}` and apply on config
reload. Groups and fields are optional. Gnoblin leaves anything you omit at its
current GNOME/Mutter setting, so start with a small override and add only what
you need.

In the standalone native compositor preview, these settings apply at startup.
Restart that compositor after changing the file. The full Shell session applies
valid changes on config reload.

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
- [`gnoblin.configure.input.keyboard`](/config/configure/input/keyboard)

## Tablets

Tablet overrides use a four-digit hexadecimal vendor and product ID key, such
as `"1234:5678"`. Unlisted tablets keep their current system settings.

Find the device path with `libinput list-devices`, then inspect its IDs with
`udevadm info --query=property --name=/dev/input/eventN`. The [libinput tools
guide](https://wayland.freedesktop.org/libinput/doc/latest/tools.html) and
[`udevadm` manual](https://man7.org/linux/man-pages/man8/udevadm.8.html) explain
the commands and their output.

| Field         | Accepted values              | Meaning                                                                                     |
| ------------- | ---------------------------- | ------------------------------------------------------------------------------------------- |
| `mapping`     | `"absolute"` or `"relative"` | Absolute maps pen position to a fixed tablet area; relative moves the pointer like a mouse. |
| `left_handed` | Boolean                      | Reverses the tablet's button orientation.                                                   |
| `keep_aspect` | Boolean                      | Preserves proportions when tablet and display have different shapes.                        |

Enable `keep_aspect` when you want a drawn circle to stay circular across
different display and tablet shapes.

```lua
gnoblin.configure {
    input = {
        tablets = {
            ["1234:5678"] = {
                mapping = "absolute",
                keep_aspect = true,
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
shortcut, use `"keybinding"` and provide the matching keybinding field.

```lua
gnoblin.configure {
    input = {
        styluses = {
            ["default-1234:5678"] = {
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

The primary button uses `button_action` and `button_keybinding`. Secondary and
tertiary buttons use the corresponding `secondary_` and `tertiary_` fields.
An empty keybinding has no effect unless its action is `"keybinding"`.

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
                left_handed = boolean?,
                keep_aspect = boolean?,
            }, ...,
        }?,
        styluses = {
            ["serial-or-default-vvvv:pppp"] = {
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
```
