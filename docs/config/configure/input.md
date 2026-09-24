# gnoblin.configure.input

Input settings go in `gnoblin.configure {input = {...}}` and apply on config
reload. Groups and fields are optional. Gnoblin leaves anything you omit at its
current GNOME/Mutter setting, so start with a small override and add only what
you need.

For the common pointer and keyboard settings, see:

- [`gnoblin.configure.input.mouse`](/config/configure/input/mouse)
- [`gnoblin.configure.input.touchpad`](/config/configure/input/touchpad)
- [`gnoblin.configure.input.keyboard`](/config/configure/input/keyboard)

## Tablets

Tablet overrides are keyed by vendor and product ID, for example `"1234:5678"`.
Add only devices you want to change; unlisted tablets keep their current system
settings. `mapping` accepts `"absolute"` or `"relative"`, while
`left_handed` and `keep_aspect` are booleans.

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

Absolute mapping ties pen position to a fixed area of the tablet; relative
mapping moves the pointer like a mouse. Keep aspect ratio when the tablet and
display have different shapes and you want a drawn circle to stay circular.

## Styluses

Stylus overrides use a hexadecimal device serial, or `default-` followed by a
vendor and product ID such as `"default-1234:5678"`. Use `"default"` to leave a
button's action unchanged. Set an action to `"keybinding"` and provide the
matching keybinding field to assign a shortcut.

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

Each button action accepts `"default"`, `"middle"`, `"right"`, `"back"`,
`"forward"`, `"switch-monitor"`, or `"keybinding"`. The primary button uses
`button_action` and `button_keybinding`; the other buttons use the corresponding
`secondary_` and `tertiary_` fields. An empty keybinding has no effect unless
its action is set to `"keybinding"`.

## Orientation lock

Set `orientation_lock = true` to lock the current screen orientation, or
`false` to allow automatic rotation. Leave the field out to follow GNOME's
setting. Use the override when the system rotation preference should not
change the screen orientation for this config.

```lua
gnoblin.configure {
    input = {
        orientation_lock = true,
    },
}
```

The keyboard `numlock_state` override is kept in memory while Gnoblin's config
is active; removing it restores the system setting.
