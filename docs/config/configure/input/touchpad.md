# Touchpad settings

A `Touchpad` has four settings. The hardware backend gives them to every touchpad through libinput when the session
starts, when you plug one in and after every reload; a nested session leaves its host's devices alone
([input device settings](/backends#input-device-settings)). This example enables tap-to-click and natural scrolling:

```ts
import { Touchpad } from "gnoblin-core";

for (const device of gnoblin.input.devices) {
    if (device instanceof Touchpad) {
        device.tapToClick = true;
        device.naturalScroll = true;
    }
}
```

| Property        | Values                                | Default      | Written to                      |
| --------------- | ------------------------------------- | ------------ | ------------------------------- |
| `sensitivity`   | Number from `-1` (slowest) to `1`     | The system's | `input.touchpad.speed`          |
| `acceleration`  | `"default"`, `"flat"` or `"adaptive"` | The system's | `input.touchpad.accel-profile`  |
| `naturalScroll` | Boolean                               | The system's | `input.touchpad.natural-scroll` |
| `tapToClick`    | Boolean                               | The system's | `input.touchpad.tap-to-click`   |

`device.reset(name)` takes a property back to the system's value. `name` is the property in snake case, such as
`"tap_to_click"`.

The scroll speed, tap-and-drag, disable-while-typing, click
method and the other touchpad fields have no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).

A swipe or a pinch of several fingers is a binding, not a setting: see [touchpad gestures](/config/configure/touchpad_gestures).
GNOME's [touchpad guide](https://help.gnome.org/gnome-help/mouse-touchpad-click.html) explains tap, click and scroll
behaviour.
