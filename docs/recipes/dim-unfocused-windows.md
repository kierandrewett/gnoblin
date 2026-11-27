# Dim unfocused windows

Set the opacity of windows that do not have the keyboard. Put the script in the file for your language: `shell.ts`,
`shell.py`, or a shell library written in Rust (see [Write a shell in Rust](/rust-shell)).

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";

gnoblin.clients.addListener({
    onFocusChanged(client, focused) {
        client.opacity = focused ? 1 : 0.95;
    },
});
```

```python
from gnoblin.objects.model import Client, ClientCollectionListener, Gnoblin


class DimUnfocused(ClientCollectionListener):
    def on_focus_changed(self, client: Client, focused: bool) -> None:
        client.opacity = 1.0 if focused else 0.95


Gnoblin.get().clients.add_listener(DimUnfocused())
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{ClientCollectionListenerHandlers, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let clients = model.clients()?;
    let listener = ClientCollectionListenerHandlers {
        on_focus_changed: Some(Box::new(|client, focused| {
            client.set_opacity(if focused { 1.0 } else { 0.95 })?;
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    clients.add_listener(&listener)?;
    Ok(())
}
```

:::

`on_focus_changed` runs each time a window gains or loses the keyboard. `opacity` is a number from `0` (transparent) to `1`
(opaque); every example uses `0.95` for an unfocused window. The listener hears windows, not layer surfaces.

To fade the change instead of cutting it, run the assignment inside a transition that lasts 180 ms.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import type {} from "gnoblin-std/generated/objects";

gnoblin.clients.addListener({
    onFocusChanged(client, focused) {
        gnoblin.requestTransition(
            () => {
                client.opacity = focused ? 1 : 0.95;
            },
            { duration: 180 },
        );
    },
});
```

```python
from gnoblin.objects.model import Client, ClientCollectionListener, Gnoblin
from gnoblin.objects.std import GnoblinAugment, TransitionOptions


class FadeUnfocused(ClientCollectionListener):
    def on_focus_changed(self, client: Client, focused: bool) -> None:
        def set_opacity() -> None:
            client.opacity = 1.0 if focused else 0.95

        GnoblinAugment.get().request_transition(set_opacity, TransitionOptions(duration=180))


Gnoblin.get().clients.add_listener(FadeUnfocused())
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{ClientCollectionListenerHandlers, ModelRoot};
use gnoblin_std::objects::{Easing, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?;
    let model = Rc::new(ModelRoot::get(&objects)?);
    let clients = model.clients()?;
    let transitioner = Rc::clone(&model);
    let listener = ClientCollectionListenerHandlers {
        on_focus_changed: Some(Box::new(move |client, focused| {
            let options = TransitionOptions {
                duration: 180.0,
                easing: Easing::Linear,
                delay: 0.0,
                autoplay: true,
            };
            transitioner.request_transition(
                move || {
                    client.set_opacity(if focused { 1.0 } else { 0.95 })?;
                    Ok(())
                },
                Some(&options),
                Some(|| Ok(())),
            )?;
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    clients.add_listener(&listener)?;
    Ok(())
}
```

:::

See [transitions](/js-transitions) for what the work callback may change and how `rollback` is used.

Opacity affects text and controls too. For a translucent panel with opaque text, use transparency in the client's
background instead. Remove the listener and save to go back to full opacity.

See [window style](/js-style) for the style properties.
