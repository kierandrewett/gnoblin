# Set up a laptop keyboard and touchpad

This setup makes a held key repeat faster and turns on tap to click and natural scrolling for the touchpad. Each device
is an object in `gnoblin.input.devices`, and a setting is a property you write on it. The same setup is written in each
language: `shell.ts`, `shell.py`, or a Rust shell library. Pick the one you write.

::: tabs

```typescript
import { Keyboard, Touchpad } from "gnoblin-core";

for (const device of gnoblin.input.devices) {
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
```

```python
from gnoblin import Gnoblin, Keyboard, Touchpad

for device in Gnoblin.get().input.devices:
    if isinstance(device, Keyboard):
        device.repeat_rate = 40
        device.repeat_delay = 250
    if isinstance(device, Touchpad):
        device.tap_to_click = True
        device.natural_scroll = True
        device.sensitivity = -0.25
```

```rust skip="a listed InputDevice cannot yet be turned into a Keyboard or Touchpad handle"
use gnoblin_core::InputManager;

let objects = g.objects()?.std()?;
let input = InputManager::get(&objects)?;
for device in input.devices()? {
    println!("{}: {:?}", device.name()?, device.capabilities()?);
}
```

:::

The loop runs over the devices that are present when the file loads. A device plugged in later is not covered until the
file reloads: save it again after plugging one in.

| Setting           | TypeScript               | Python                    | Rust                 | Type and accepted values               | Effect                                       |
| ----------------- | ------------------------ | ------------------------- | -------------------- | -------------------------------------- | -------------------------------------------- |
| Key repeat rate   | `Keyboard.repeatRate`    | `Keyboard.repeat_rate`    | `set_repeat_rate`    | Number, 0.1 to 1000 repeats per second | How fast a held key repeats.                 |
| Key repeat delay  | `Keyboard.repeatDelay`   | `Keyboard.repeat_delay`   | `set_repeat_delay`   | Whole milliseconds, 1 to 10000         | How long a held key waits before it repeats. |
| Tap to click      | `Touchpad.tapToClick`    | `Touchpad.tap_to_click`   | `set_tap_to_click`   | Boolean                                | Treats a tap as a click.                     |
| Natural scrolling | `Touchpad.naturalScroll` | `Touchpad.natural_scroll` | `set_natural_scroll` | Boolean                                | Reverses the scroll direction.               |
| Pointer speed     | `Touchpad.sensitivity`   | `Touchpad.sensitivity`    | `set_sensitivity`    | Number, -1 (slowest) to 1              | Pointer speed.                               |

A setting you do not write keeps the session value. The settings are written to the `input` section of the configuration
document and applied when the compositor commits it.

Keyboard layouts, XKB options and the other touchpad settings (tap and drag, disable while typing, two-finger scrolling)
cannot be set from a shell. See [settings with no object yet](/js-settings#dropped-settings).
