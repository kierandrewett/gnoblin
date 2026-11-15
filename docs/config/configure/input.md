# Input settings

Input settings are properties of the device objects in `gnoblin.input.devices`. A device is a `Keyboard`, a `Mouse`, a
`Touchpad`, a `Touchscreen` or a `Tablet`.

Use `instanceof` to tell them apart, and set a property to `undefined` to take it back to the system's value.

```ts
import { Keyboard, Mouse } from "gnoblin";

for (const device of gnoblin.input.devices) {
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
    }
    if (device instanceof Mouse) {
        device.sensitivity = 0.5;
    }
}
```

The compositor keeps one value for each kind of device, so a write on one mouse is the setting of every mouse, and a
read on another mouse shows it. A device that is plugged in later reads the same value. A value outside its range
throws an error that names the device and the property, as in `Keyboard#1.repeatRate: must be a number of repeats per
second from 0.1 to 1000, got 0`.

Only the keyboard properties are applied. The mouse and touchpad properties are accepted and checked, and no device
reads them, so the compositor logs `not applied: ...` for each.

- [Keyboard](/config/configure/input/keyboard): `repeatRate` and `repeatDelay`.
- [Mouse](/config/configure/input/mouse): `sensitivity`, `acceleration` and `naturalScroll`.
- [Touchpad](/config/configure/input/touchpad): `sensitivity`, `acceleration`, `naturalScroll` and `tapToClick`.

`Touchscreen` and `Tablet` have no settings. A device also belongs to one `InputContext` and can hold its own
bindings: see [keyboard shortcuts](/shell-api/keyboard-shortcuts).

Tablets, styluses, trackballs, pointing sticks, the orientation lock, the XKB options and the keyboard accessibility
features have no TypeScript home yet: see [settings with no TypeScript home yet](/js-settings#dropped-settings).
