# Input settings

Input settings are properties of the device objects in the input manager's device list. A device is a `Keyboard`, a
`Mouse`, a `Touchpad`, a `Touchscreen` or a `Tablet`.

Use the device's type to tell them apart (`instanceof` in TypeScript, `isinstance` in Python), and call `reset(name)` to
take a setting back to the system's value. In Rust, a settings handle is made with `new_with` and its options.

::: tabs

```typescript
import { Keyboard, Mouse } from "gnoblin-core";

const input = gnoblin.input;
if (input) {
    for (const device of input.devices) {
        if (device instanceof Keyboard) {
            device.repeatRate = 40;
        }
        if (device instanceof Mouse) {
            device.sensitivity = 0.5;
        }
    }
}
```

```python
from gnoblin import gnoblin
from gnoblin.objects.model import Keyboard, Mouse

for device in gnoblin.input.devices:
    if isinstance(device, Keyboard):
        device.repeat_rate = 40
    if isinstance(device, Mouse):
        device.sensitivity = 0.5
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{Keyboard, KeyboardOptions, Mouse, MouseOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    // Each handle sets the setting of every device of its kind.
    let _keyboard = Keyboard::new_with(
        &objects,
        KeyboardOptions {
            enabled: None,
            context: None,
            binding_only: None,
            repeat_rate: Some(40.0),
            repeat_delay: None,
        },
    )?;
    let _mouse = Mouse::new_with(
        &objects,
        MouseOptions {
            enabled: None,
            context: None,
            binding_only: None,
            sensitivity: Some(0.5),
            acceleration: None,
            natural_scroll: None,
        },
    )?;
    Ok(())
}
```

:::

The compositor keeps one value for each kind of device, so a write on one mouse is the setting of every mouse, and a
read on another mouse shows it. A device that is plugged in later reads the same value. A value outside its range is
refused, and the error names the device and the property.

The mouse, touchpad and keyboard settings are applied by the hardware backend. It gives them to every device of that
kind when the session starts, to a device you plug in later, and again after every reload.

A nested session leaves its host's devices alone. See [input device settings](/backends#input-device-settings) for what
each backend does and the settings that are not applied yet.

- [Keyboard](/config/configure/input/keyboard): `repeatRate` and `repeatDelay`, `repeat_rate` and `repeat_delay`.
- [Mouse](/config/configure/input/mouse): `sensitivity`, `acceleration` and `naturalScroll`.
- [Touchpad](/config/configure/input/touchpad): `sensitivity`, `acceleration`, `naturalScroll` and `tapToClick`.

A `Touchscreen` and a `Tablet` have no settings of their own. They have only `enabled`, `context` and `binding_only`,
which every device has.

A device also belongs to one `InputContext` and can hold its own bindings. See
[keyboard shortcuts](/shell-api/keyboard-shortcuts).

Tablets, styluses, the orientation lock, the XKB options and the keyboard accessibility features have no shell API home
yet. See [settings with no object yet](/js-settings#dropped-settings).
