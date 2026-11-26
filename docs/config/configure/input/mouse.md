# Mouse settings

A `Mouse` stands for every pointer that is not a touchpad: a mouse, a trackball or a pointing stick. The seat reports each
device, and TypeScript and Python set the settings on the `Mouse` objects of `gnoblin.input.devices`.

The hardware backend gives the settings to each device when the session starts, when you plug a device in and after every
reload. A nested session leaves its host's devices alone ([input device settings](/backends#input-device-settings)).

::: tabs

```typescript
import { gnoblin, Mouse } from "gnoblin-core/generated/objects";

for (const device of gnoblin.input.devices) {
    if (device instanceof Mouse) {
        device.sensitivity = -0.2;
        device.acceleration = "flat";
        device.naturalScroll = true;
    }
}
```

```python
from gnoblin.objects.model import AccelerationProfile, Gnoblin, Mouse

for device in Gnoblin.get().input.devices:
    if isinstance(device, Mouse):
        device.sensitivity = -0.2
        device.acceleration = AccelerationProfile.FLAT
        device.natural_scroll = True
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{AccelerationProfile, Mouse, MouseOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let _mouse = Mouse::new_with(
        &objects,
        MouseOptions {
            enabled: None,
            context: None,
            binding_only: None,
            sensitivity: Some(-0.2),
            acceleration: Some(AccelerationProfile::Flat),
            natural_scroll: Some(true),
        },
    )?;
    Ok(())
}
```

:::

| Setting           | TypeScript      | Python           | Rust             | Values                                           | Default      |
| ----------------- | --------------- | ---------------- | ---------------- | ------------------------------------------------ | ------------ |
| Speed             | `sensitivity`   | `sensitivity`    | `sensitivity`    | A number from `-1` (slowest) to `1`              | The system's |
| Acceleration      | `acceleration`  | `acceleration`   | `acceleration`   | `AccelerationProfile`: default, flat or adaptive | The system's |
| Natural scrolling | `naturalScroll` | `natural_scroll` | `natural_scroll` | Boolean                                          | The system's |

A setting you leave out keeps the system's value, and a change takes effect on the next reload.

Acceleration values by language:

- TypeScript: `"default"`, `"flat"` and `"adaptive"`.
- Python: `AccelerationProfile.DEFAULT`, `FLAT` and `ADAPTIVE`. A plain string is also accepted.
- Rust: `AccelerationProfile::Default`, `Flat` and `Adaptive`.

`reset(name)` takes one setting back to the system's value. `name` is `"sensitivity"`, `"acceleration"` or
`"natural_scroll"`, and any other name is refused.

The double-click time and drag threshold are not applied yet: see [input device settings](/backends#input-device-settings).
