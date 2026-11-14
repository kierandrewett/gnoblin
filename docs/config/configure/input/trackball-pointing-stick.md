# Trackball and pointing stick settings

Use the `trackball` and `pointingStick` groups of `input` to configure trackballs and pointing sticks. Each group
applies to every device of that type. An omitted field keeps its current system value. Set a field to `"inherit"` to
say that you want the system value.

These settings are accepted and checked, and no device reads them: the compositor logs `not applied: ...` for these
groups.

The tables list the schema defaults. A saved system setting takes precedence when the corresponding Gnoblin field is
omitted.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        trackball: {
            accelProfile: "adaptive",
            scrollWheelEmulationButton: 2,
        },
        pointingStick: {
            speed: 0.2,
            scrollMethod: "on-button-down",
        },
    },
});
```

## Trackball

| Field                            | Type and accepted values                    | Schema default | What it changes                                                                 |
| -------------------------------- | ------------------------------------------- | -------------- | ------------------------------------------------------------------------------- |
| `accelProfile`                   | String: `"default"`, `"flat"`, `"adaptive"` | `"default"`    | Selects the device's pointer acceleration curve.                                |
| `middleClickEmulation`           | Boolean                                     | `false`        | Emulates middle-click when both mouse buttons are pressed.                      |
| `scrollWheelEmulationButton`     | Integer from `0` to `24`                    | `0`            | Hold this button and move the trackball to scroll. `0` disables this behaviour. |
| `scrollWheelEmulationButtonLock` | Boolean                                     | `false`        | Toggle scrolling with button clicks instead of holding the button.              |

`"default"` uses the device's default acceleration profile. The other profiles select a flat or adaptive curve. Button
lock takes effect only when `scrollWheelEmulationButton` is nonzero.

## Pointing stick

| Field          | Type and accepted values                          | Schema default | What it changes                                                                          |
| -------------- | ------------------------------------------------- | -------------- | ---------------------------------------------------------------------------------------- |
| `speed`        | Number from `-1` to `1`                           | `0`            | Adjusts pointer speed. `-1` is slowest, `1` is fastest, and `0` uses the device default. |
| `accelProfile` | String: `"default"`, `"flat"`, `"adaptive"`       | `"default"`    | Selects the device's pointer acceleration curve.                                         |
| `scrollMethod` | String: `"default"`, `"none"`, `"on-button-down"` | `"default"`    | Use device behaviour, disable scrolling, or scroll while holding the middle button.      |

These settings require matching hardware. When a field is omitted or set to `"inherit"`, the current system
preference is used. The defaults and enum values follow the
[GNOME desktop peripheral schemas](https://github.com/GNOME/gsettings-desktop-schemas/blob/main/schemas/org.gnome.desktop.peripherals.gschema.xml.in).
