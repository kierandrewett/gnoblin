# Set up a laptop keyboard and touchpad

This setup makes a held key repeat faster and turns on tap to click and natural scrolling for the touchpad. Each device
is an object in `gnoblin.input.devices`, and a setting is a property you write on it.

```ts
import { CollectionListener, Keyboard, Touchpad } from "gnoblin";
import type { InputDevice } from "gnoblin";

gnoblin.input.devices.addListener(
    new (class extends CollectionListener<InputDevice> {
        onAdded(device: InputDevice) {
            if (device instanceof Keyboard) {
                device.repeatRate = 40;
                device.repeatDelay = 250;
            }
            if (device instanceof Touchpad) {
                device.tapToClick = true;
                device.naturalScroll = true;
                device.sensitivity = -0.25;
            }
        }
    })(),
);
```

`onAdded` runs for each device that is present when the file loads and for each one plugged in later, so an external
keyboard gets the same settings.

| Property                 | Type and accepted values               | Effect                                       |
| ------------------------ | -------------------------------------- | -------------------------------------------- |
| `Keyboard.repeatRate`    | Number, 0.1 to 1000 repeats per second | How fast a held key repeats.                 |
| `Keyboard.repeatDelay`   | Whole milliseconds, 1 to 10000         | How long a held key waits before it repeats. |
| `Touchpad.tapToClick`    | Boolean                                | Treats a tap as a click.                     |
| `Touchpad.naturalScroll` | Boolean                                | Reverses the scroll direction.               |
| `Touchpad.sensitivity`   | Number, -1 (slowest) to 1              | Pointer speed.                               |

A setting you do not write keeps the session value. The settings are written to the `input` section of the configuration
document and applied when the compositor commits it.

Keyboard layouts, XKB options and the other touchpad settings (tap and drag, disable while typing, two-finger scrolling)
cannot be set from `init.ts`. See [dropped settings](/js-settings#dropped-settings).
