# Combine window rules

Set shared defaults first, then override them for selected windows. Later statements win, so put the general one first.

Put the script in the file for your language: `shell.ts`, `shell.py`, or a shell library written in Rust (see [Write a shell in Rust](/rust-shell)).
Each one sets `gnoblin.clients.style`, the style of every window, and adds a listener that runs when a window is added.

::: tabs

```typescript
import { ClientCollectionListener } from "gnoblin-core";
import "gnoblin-std";
import type { Client } from "gnoblin-core";

gnoblin.clients.style.opacity = 1;

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            client.maximised = false;
            if (client.appId === "org.gnome.TextEditor") {
                client.maximised = true;
            }
        }
    })(),
);
```

```python
from gnoblin import gnoblin
from gnoblin.objects.model import Client, ClientCollection, ClientCollectionListener


gnoblin.clients.style.opacity = 1.0


class MaximiseTextEditor(ClientCollectionListener):
    def on_added(self, client: Client, collection: ClientCollection) -> None:
        client.maximised = False
        if client.app_id == "org.gnome.TextEditor":
            client.maximised = True


gnoblin.clients.add_listener(MaximiseTextEditor())
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{Client, ClientCollection, ClientCollectionListenerHandlers, ModelRoot};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    clients.style()?.set_opacity(1.0)?;
    let listener = ClientCollectionListenerHandlers {
        on_added: Some(Box::new(|client: Client, _collection: ClientCollection| {
            client.set_maximised(false)?;
            if client.app_id()? == "org.gnome.TextEditor" {
                client.set_maximised(true)?;
            }
            Ok(())
        })),
        ..ClientCollectionListenerHandlers::default()
    }
    .implement(&objects)?;
    clients.add_listener(&listener)?;
    Ok(())
}
```

:::

The first statement and the first line of the listener apply to every window. The `if` changes only the editor: the
default is set first and the exception after it. See [window rules](/guides/window_rules#order).
