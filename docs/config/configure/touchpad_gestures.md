# Touchpad gestures

A touchpad gesture is a swipe or a pinch with a finger count, such as `Swipe3` or `Pinch2In`. A gesture binding is an
object that answers the gesture as it runs. Give it to the input context:

- TypeScript: `bindGesture`
- Python and Rust: `bind_gesture`

Its hooks are `onBegin` (required), `onUpdate`, `onEnd` and `onCancel`.

The stock three-finger slide between workspaces is `WorkspaceSlideBinding`. It follows the fingers and commits when they
lift past `commit`, a fraction from 0 to 1 that defaults to 0.25.

::: tabs

```typescript
import { GestureBinding, gnoblin } from "gnoblin-core/generated/objects";
import { WorkspaceSlideBinding } from "gnoblin-std/generated/objects";

class CloseOnSwipe extends GestureBinding {
    onBegin(): boolean {
        return true;
    }

    onEnd(): void {
        gnoblin.input.primary?.focusedClient?.close();
    }
}

gnoblin.input.primary?.bindGesture("Swipe4Up", new CloseOnSwipe());

new WorkspaceSlideBinding({ trigger: "Swipe3" }).install();
```

```python
from gnoblin.objects.model import Gnoblin, GestureBinding
from gnoblin.objects.std import WorkspaceSlideBinding


class CloseOnSwipe(GestureBinding):
    def on_begin(self, event):
        return True

    def on_end(self, event):
        primary = Gnoblin.get().input.primary
        if primary is not None and primary.focused_client is not None:
            primary.focused_client.close()


Gnoblin.get().input.primary.bind_gesture("Swipe4Up", CloseOnSwipe())

WorkspaceSlideBinding(trigger="Swipe3").install()
```

```rust
use gnoblin_core::objects::{GestureBindingHandlers, ModelRoot};
use gnoblin_std::objects::{WorkspaceSlideBinding, WorkspaceSlideBindingOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?;
    let std_objects = g.std()?.objects()?;
    let model = ModelRoot::get(&objects)?;
    let input = model.input()?;
    let context = input.primary()?;

    let close = GestureBindingHandlers {
        on_end: Some(Box::new(|_| Ok(()))),
        ..GestureBindingHandlers::new(|_| Ok(true))
    }
    .implement(&objects)?;
    context.bind_gesture("Swipe4Up", &close, None)?;

    let slide = WorkspaceSlideBinding::new_with(
        &std_objects,
        WorkspaceSlideBindingOptions {
            priority: None,
            trigger: Some("Swipe3".to_string()),
            travel: None,
            commit: None,
            duration: None,
        },
    )?;
    slide.install(None)?;
    Ok(())
}
```

:::

`WorkspaceSlideBinding` takes `trigger`, `priority`, `travel` (the distance in touchpad units for a whole slide),
`commit` and `duration` (in milliseconds). Installing a second binding with the same trigger and priority is refused.
