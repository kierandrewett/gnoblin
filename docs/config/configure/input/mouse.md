# Mouse settings

A `Mouse` stands for a mouse, a trackball or a pointing stick, and has three settings. They are accepted and checked,
and no device reads them: the compositor logs `not applied: ...` for the `input.mouse` group.

```ts
import { Mouse } from "gnoblin";

for (const device of gnoblin.input.devices) {
    if (device instanceof Mouse) {
        device.sensitivity = -0.2;
        device.acceleration = "flat";
        device.naturalScroll = true;
    }
}
```

| Property        | Values                                | Default      | Written to                   |
| --------------- | ------------------------------------- | ------------ | ---------------------------- |
| `sensitivity`   | Number from `-1` (slowest) to `1`     | The system's | `input.mouse.speed`          |
| `acceleration`  | `"default"`, `"flat"` or `"adaptive"` | The system's | `input.mouse.accel-profile`  |
| `naturalScroll` | Boolean                               | The system's | `input.mouse.natural-scroll` |

`undefined` takes a property back to the system's value. A custom acceleration curve, the double-click time, the drag
threshold, middle-click emulation and the left-handed layout have no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).

GNOME's [pointer-speed guide](https://help.gnome.org/gnome-help/mouse-sensitivity.html) explains the user-facing
speed setting.
