# Mouse settings

A `Mouse` stands for every pointer that is not a touchpad: a mouse, a trackball or a pointing stick. Its settings apply to
all of them. The hardware backend gives them to each device when the session starts, when you plug a device in and after
every reload; a nested session leaves its host's devices alone ([input device settings](/backends#input-device-settings)).

::: tabs

```typescript
import { Mouse } from "gnoblin-core/generated/objects";

const mouse = new Mouse({
    sensitivity: -0.2,
    acceleration: "flat",
    naturalScroll: true,
});
```

```python
from gnoblin.objects.model import AccelerationProfile, Mouse

mouse = Mouse(
    sensitivity=-0.2,
    acceleration=AccelerationProfile.FLAT,
    natural_scroll=True,
)
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

| Setting           | TypeScript      | Python           | Rust             | Values                                                                     |
| ----------------- | --------------- | ---------------- | ---------------- | -------------------------------------------------------------------------- |
| Speed             | `sensitivity`   | `sensitivity`    | `sensitivity`    | A number from `-1` (slowest) to `1`, or none for the system's              |
| Acceleration      | `acceleration`  | `acceleration`   | `acceleration`   | `AccelerationProfile`: default, flat or adaptive, or none for the system's |
| Natural scrolling | `naturalScroll` | `natural_scroll` | `natural_scroll` | Boolean, or none for the system's                                          |

The acceleration values are `"default"`, `"flat"` and `"adaptive"` in TypeScript; `AccelerationProfile.DEFAULT`, `FLAT` and
`ADAPTIVE` in Python (a plain string is also accepted); and `AccelerationProfile::Default`, `Flat` and `Adaptive` in Rust.

Each handle also has `reset(name)`, which takes one setting back to the system's value. `name` is `"sensitivity"`,
`"acceleration"` or `"natural_scroll"`, and any other name is refused. The double-click time and drag threshold are not
applied yet: see [input device settings](/backends#input-device-settings).
