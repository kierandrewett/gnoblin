# Combine window rules

Set shared defaults first, then override them for selected windows. Later statements win, so put the general one first.

Put the script in the file for your language: `shell.ts`, `shell.py`, or a shell library written in Rust (see [Write a shell in Rust](/rust-shell)).
Each one sets `gnoblin.clients.style`, the style of every window, and runs a function when a window is added.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import "gnoblin-std";

gnoblin.clients.style.opacity = 1;

gnoblin.clients.on("added", (client) => {
    client.maximised = false;
    if (client.appId === "org.gnome.TextEditor") {
        client.maximised = true;
    }
});
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import ClientCollectionAugment


ClientCollectionAugment.get().style(Gnoblin.get().clients).opacity = 1.0


@Gnoblin.get().clients.on("added")
def rule(client):
    client.maximised = False
    if client.app_id == "org.gnome.TextEditor":
        client.maximised = True
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{Client, ClientCollection, ModelRoot};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let clients = ModelRoot::get(&g.objects()?)?.clients()?;
    clients.style()?.set_opacity(1.0)?;
    clients
        .on_added(|client: Client, _clients: ClientCollection| {
            let _ = client.set_maximised(false);
            if client.app_id().unwrap_or_default() == "org.gnome.TextEditor" {
                let _ = client.set_maximised(true);
            }
        })?
        .keep();
    Ok(())
}
```

:::

The first statement and the first line of the listener apply to every window. The `if` changes only the editor: the
default is set first and the exception after it. See [window rules](/guides/window_rules#order).
