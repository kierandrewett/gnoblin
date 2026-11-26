# Keyboard settings

A `Keyboard` has two settings, the key repeat rate and the delay before a key repeats. This example repeats a held key
40 times a second after 250 ms:

::: tabs

```typescript
import { Keyboard } from "gnoblin-core";

for (const device of gnoblin.input?.devices ?? []) {
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
        device.repeatDelay = 250;
    }
}
```

```python
from gnoblin import gnoblin
from gnoblin.objects.model import Keyboard

for device in gnoblin.input.devices:
    if isinstance(device, Keyboard):
        device.repeat_rate = 40
        device.repeat_delay = 250
```

```rust skip="the reference does not document how a shell gets the Objects handle that Keyboard::new_with takes"
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{Keyboard, KeyboardOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    // Keyboard repeat settings belong to every keyboard, so one handle sets them all.
    let _keyboard = Keyboard::new_with(
        &objects,
        KeyboardOptions {
            enabled: None,
            context: None,
            binding_only: None,
            repeat_rate: Some(40.0),
            repeat_delay: Some(250),
        },
    )?;
    Ok(())
}
```

:::

| Property      | Values                   | Default      | Effect                                        |
| ------------- | ------------------------ | ------------ | --------------------------------------------- |
| `repeatRate`  | Number, 0.1 to 1000      | The system's | Key repeats per second.                       |
| `repeatDelay` | Whole number, 1 to 10000 | The system's | Milliseconds a key is held before it repeats. |

In Python the same properties are `repeat_rate` and `repeat_delay`. In Rust they are read and set with `repeat_rate()`
and `set_repeat_rate(..)`, and `repeat_delay()` and `set_repeat_delay(..)`, and given as `repeat_rate` and
`repeat_delay` in `KeyboardOptions`.

`device.reset("repeat_rate")` and `device.reset("repeat_delay")` take a property back to the system's value, in every
language. In the document the repeat rate is held as the time between two repeats, in whole milliseconds, as
`input.keyboard.repeat-interval`, and the delay as `input.keyboard.delay`.

The XKB options (`caps:escape`, `compose:ralt`), Num Lock state and the keyboard accessibility features (slow keys,
sticky keys, bounce keys, mouse keys) have no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).
