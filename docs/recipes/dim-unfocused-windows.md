# Dim unfocused windows

Set the opacity of windows that do not have the keyboard. Put the script in the file for your language: `shell.ts`,
`shell.py`, or a shell library written in Rust (see [Write a shell in Rust](/rust-shell)).

::: tabs

```typescript
import { ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

class DimUnfocused extends ClientCollectionListener {
    onFocusChanged(client: Client, focused: boolean) {
        client.opacity = focused ? 1 : 0.95;
    }
}

gnoblin.clients.addListener(new DimUnfocused());
```

```python
from gnoblin import gnoblin
from gnoblin.objects.model import Client, ClientCollectionListener


class DimUnfocused(ClientCollectionListener):
    def on_focus_changed(self, client: Client, focused: bool) -> None:
        client.opacity = 1.0 if focused else 0.95


gnoblin.clients.add_listener(DimUnfocused())
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{ClientCollectionListenerHandlers, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    let listener = ClientCollectionListenerHandlers {
        on_added: None,
        on_removed: None,
        on_title_changed: None,
        on_app_id_changed: None,
        on_decoration_changed: None,
        on_pointer_enter: None,
        on_pointer_motion: None,
        on_pointer_leave: None,
        on_pointer_press: None,
        on_pointer_release: None,
        on_focus_changed: Some(Box::new(|client, focused| {
            client.set_opacity(if focused { 1.0 } else { 0.95 })?;
            Ok(())
        })),
        on_geometry_changed: None,
        on_closed: None,
        on_ready: None,
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
import { ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

class FadeUnfocused extends ClientCollectionListener {
    onFocusChanged(client: Client, focused: boolean) {
        gnoblin.requestTransition(
            () => {
                client.opacity = focused ? 1 : 0.95;
            },
            { duration: 180 },
        );
    }
}

gnoblin.clients.addListener(new FadeUnfocused());
```

```python
from gnoblin import gnoblin
from gnoblin.objects.model import Client, ClientCollectionListener
from gnoblin.objects.std import TransitionOptions


class FadeUnfocused(ClientCollectionListener):
    def on_focus_changed(self, client: Client, focused: bool) -> None:
        def set_opacity() -> None:
            client.opacity = 1.0 if focused else 0.95

        gnoblin.request_transition(set_opacity, TransitionOptions(duration=180))


gnoblin.clients.add_listener(FadeUnfocused())
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{ClientCollectionListenerHandlers, ModelRoot};
use gnoblin_std::objects::{Easing, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = Rc::new(ModelRoot::get(&objects)?);
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    let transitioner = Rc::clone(&model);
    let listener = ClientCollectionListenerHandlers {
        on_added: None,
        on_removed: None,
        on_title_changed: None,
        on_app_id_changed: None,
        on_decoration_changed: None,
        on_pointer_enter: None,
        on_pointer_motion: None,
        on_pointer_leave: None,
        on_pointer_press: None,
        on_pointer_release: None,
        on_focus_changed: Some(Box::new(move |client, focused| {
            let options = TransitionOptions {
                duration: 180.0,
                easing: Easing::Linear,
                delay: 0.0,
                autoplay: true,
            };
            transitioner.request_transition(
                move || client.set_opacity(if focused { 1.0 } else { 0.95 }),
                Some(&options),
                None::<fn() -> gnoblin::Result<()>>,
            )?;
            Ok(())
        })),
        on_geometry_changed: None,
        on_closed: None,
        on_ready: None,
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
