# gnoblin.configure.input.trackball and pointing_stick

Use these groups to configure trackballs and pointing sticks. Each group
applies to every device of that type. Settings apply at startup and when the
configuration reloads. An omitted field keeps its current system value. Set a
field to `"inherit"` to clear an earlier Gnoblin override.

The tables list the schema defaults. A saved system setting takes precedence
when the corresponding Gnoblin field is omitted.

```lua
gnoblin.configure {
    input = {
        trackball = {
            accel_profile = "adaptive",
            scroll_wheel_emulation_button = 2,
        },
        pointing_stick = {
            speed = 0.2,
            scroll_method = "on-button-down",
        },
    },
}
```

## Trackball

| Field                                | Type and accepted values                    | Schema default | What it changes                                                                |
| ------------------------------------ | ------------------------------------------- | -------------- | ------------------------------------------------------------------------------ |
| `accel_profile`                      | String: `"default"`, `"flat"`, `"adaptive"` | `"default"`    | Selects the device's pointer acceleration curve.                               |
| `middle_click_emulation`             | Boolean                                     | `false`        | Emulates middle-click when both mouse buttons are pressed.                     |
| `scroll_wheel_emulation_button`      | Integer from `0` to `24`                    | `0`            | Hold this button and move the trackball to scroll. `0` disables this behavior. |
| `scroll_wheel_emulation_button_lock` | Boolean                                     | `false`        | Toggle scrolling with button clicks instead of holding the button.             |

`"default"` uses the device's default acceleration profile. The other profiles
select a flat or adaptive curve. Button lock takes effect only when
`scroll_wheel_emulation_button` is nonzero.

## Pointing stick

| Field           | Type and accepted values                          | Schema default | What it changes                                                                          |
| --------------- | ------------------------------------------------- | -------------- | ---------------------------------------------------------------------------------------- |
| `speed`         | Number from `-1` to `1`                           | `0`            | Adjusts pointer speed. `-1` is slowest, `1` is fastest, and `0` uses the device default. |
| `accel_profile` | String: `"default"`, `"flat"`, `"adaptive"`       | `"default"`    | Selects the device's pointer acceleration curve.                                         |
| `scroll_method` | String: `"default"`, `"none"`, `"on-button-down"` | `"default"`    | Use device behavior, disable scrolling, or scroll while holding the middle button.       |

These settings require matching hardware. When a field is omitted or set to
`"inherit"`, Mutter keeps using its current preference. The defaults and enum
values follow the [GNOME desktop peripheral schemas](https://github.com/GNOME/gsettings-desktop-schemas/blob/main/schemas/org.gnome.desktop.peripherals.gschema.xml.in).
