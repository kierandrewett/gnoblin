# Touchpad settings

A `Touchpad` has four settings. The hardware backend gives them to every touchpad through libinput when the session
starts, when you plug one in and after every reload; a nested session leaves its host's devices alone
([input device settings](/backends#input-device-settings)). This example enables tap-to-click and natural scrolling:

::: tabs

```typescript
import { Touchpad } from "gnoblin-core";

for (const device of gnoblin.input.devices) {
    if (device instanceof Touchpad) {
        device.tapToClick = true;
        device.naturalScroll = true;
    }
}
```

```python
from gnoblin.objects.model import Gnoblin, Touchpad

for device in Gnoblin.get().input.devices:
    if isinstance(device, Touchpad):
        device.tap_to_click = True
        device.natural_scroll = True
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::{Touchpad, TouchpadOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    // Touchpad settings belong to every touchpad, so one handle sets them all.
    let _touchpad = Touchpad::new_with(
        &objects,
        TouchpadOptions {
            enabled: None,
            context: None,
            binding_only: None,
            sensitivity: None,
            acceleration: None,
            natural_scroll: Some(true),
            tap_to_click: Some(true),
        },
    )?;
    Ok(())
}
```

:::

| Setting                            | Values                                | Default      | Written to                      |
| ---------------------------------- | ------------------------------------- | ------------ | ------------------------------- |
| `sensitivity`                      | Number from `-1` (slowest) to `1`     | The system's | `input.touchpad.speed`          |
| `acceleration`                     | `"default"`, `"flat"` or `"adaptive"` | The system's | `input.touchpad.accel-profile`  |
| `naturalScroll` / `natural_scroll` | Boolean                               | The system's | `input.touchpad.natural-scroll` |
| `tapToClick` / `tap_to_click`      | Boolean                               | The system's | `input.touchpad.tap-to-click`   |

The first name in the table is the TypeScript property; the second is the Python and Rust name. Rust sets a property
with `set_` in front of its name, such as `set_natural_scroll`.

`reset(name)` takes a setting back to the system's value. `name` is the setting in snake case, such as `"tap_to_click"`,
in every language.

These four are the only touchpad settings the shell API has. Settings that a shell cannot set are listed on
[settings with no shell API home yet](/js-settings#dropped-settings).

A swipe or a pinch of several fingers is a binding, not a setting: see [touchpad gestures](/config/configure/touchpad_gestures).
GNOME's [touchpad guide](https://help.gnome.org/gnome-help/mouse-touchpad-click.html) explains tap, click and scroll
behaviour.
