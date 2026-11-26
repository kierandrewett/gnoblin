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
import { GestureBinding } from "gnoblin-core";
import { WorkspaceSlideBinding } from "gnoblin-std";

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
from gnoblin import gnoblin
from gnoblin.objects.model import GestureBinding
from gnoblin.objects.std import WorkspaceSlideBinding


class CloseOnSwipe(GestureBinding):
    def on_begin(self, event):
        return True

    def on_end(self, event):
        primary = gnoblin.input.primary
        if primary is not None and primary.focused_client is not None:
            primary.focused_client.close()


gnoblin.input.primary.bind_gesture("Swipe4Up", CloseOnSwipe())

WorkspaceSlideBinding(trigger="Swipe3").install()
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{GestureBindingHandlers, ModelRoot};
use gnoblin_std::objects::{WorkspaceSlideBinding, WorkspaceSlideBindingOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let Some(input) = model.input()? else {
        return Ok(());
    };
    let Some(context) = input.primary()? else {
        return Ok(());
    };

    let close = GestureBindingHandlers {
        on_begin: Box::new(|_| Ok(true)),
        on_update: None,
        on_end: Some(Box::new(|_| Ok(()))),
        on_cancel: None,
    }
    .implement(&objects)?;
    context.bind_gesture("Swipe4Up", &close, None)?;

    let slide = WorkspaceSlideBinding::new_with(
        &objects,
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
