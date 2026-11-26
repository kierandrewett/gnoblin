# Route new app windows to named workspaces

Give workspaces stable IDs in your shell, then place each matching new window on its workspace when it opens. The
shell can be `shell.ts`, `shell.py` or a Rust shell library; the code is the same in each. Replace the example app IDs
with the values reported by `gnoblinctl window list --json`.

::: tabs

```typescript
import { ClientCollectionListener } from "gnoblin-core";
import { FloatingLayout, Workspace } from "gnoblin-std";
import type { Client } from "gnoblin-core";

for (const id of ["code", "web", "chat"]) {
    gnoblin.workspaces.add(new Workspace(id, { layout: new FloatingLayout() }));
}

const homes: Record<string, string> = {
    "org.gnome.TextEditor": "code",
    "org.mozilla.firefox": "web",
    "org.gnome.Fractal": "chat",
};

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            const id = homes[client.appId];
            const target = id === undefined ? undefined : gnoblin.workspaces.byId(id);
            if (target !== undefined) {
                client.moveToWorkspace(target);
            }
        }
    })(),
);
```

```python
from gnoblin.objects.model import Client, ClientCollectionListener, Gnoblin
from gnoblin.objects.std import FloatingLayout, Workspace, Workspaces

homes = {
    "org.gnome.TextEditor": "code",
    "org.mozilla.firefox": "web",
    "org.gnome.Fractal": "chat",
}

for id in ["code", "web", "chat"]:
    Workspaces.get().add(Workspace(id, layout=FloatingLayout()))


class Homes(ClientCollectionListener):
    def on_added(self, client: Client, collection) -> None:
        id = homes.get(client.app_id)
        target = None if id is None else Workspaces.get().by_id(id)
        if target is not None:
            client.move_to_workspace(target)


Gnoblin.get().clients.add_listener(Homes())
```

```rust
use std::collections::HashMap;

use gnoblin_core::objects::{Client, ModelRoot};
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{FloatingLayout, Workspace, WorkspaceOptions, Workspaces};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let homes = [
        ("org.gnome.TextEditor", "code"),
        ("org.mozilla.firefox", "web"),
        ("org.gnome.Fractal", "chat"),
    ];

    let mut workspaces: HashMap<String, Workspace> = HashMap::new();
    for id in ["code", "web", "chat"] {
        let layout = FloatingLayout::new(&objects)?.as_layout()?;
        let workspace = Workspace::new_with(
            &objects,
            id,
            WorkspaceOptions { name: None, gaps: None, layout: Some(layout) },
        )?;
        Workspaces::get(&objects)?.add(&workspace)?;
        workspaces.insert(id.to_string(), workspace);
    }

    let clients = ModelRoot::get(&objects)?.clients()?.expect("the model has clients");
    clients
        .on_added(move |client: Client, _collection| {
            let app_id = client.app_id().unwrap_or_default();
            if let Some((_, id)) = homes.iter().find(|(app, _)| *app == app_id) {
                if let Some(target) = workspaces.get(*id) {
                    let _ = client.move_to_workspace(target);
                }
            }
        })?
        .keep();
    Ok(())
}
```

:::

Each workspace has a unique non-empty `id`. `Workspaces.get().by_id(id)` in Python, `gnoblin.workspaces.byId(id)` in
TypeScript and `Workspaces::get(&objects)?.by_id(id)?` in Rust find one you added, and return nothing for an ID you did
not add. The first workspace added is shown when the shell starts.

| Part                        | Values and default                  | Effect                                                                                                          |
| --------------------------- | ----------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `client.appId`              | The application ID of the window    | Selects windows by application ID; the example matches it exactly.                                              |
| `client.moveToWorkspace(w)` | A `Workspace` in the workspace list | Moves the window. Returns false when it cannot move it: a fullscreen client, a dialog, a window already on `w`. |

An app ID matches as a whole string, so `"org.mozilla.firefox"` matches only that app. To match every app with one prefix,
test the prefix with `startsWith` in TypeScript, `startswith` in Python or `starts_with` in Rust.

A `ClientCollectionListener` also hears the windows that are already open when it is added, so they move too.

Placement happens when a window is added. Moving a window afterwards is allowed; the rule does not run again for it.

- If an app reuses an existing window, the listener does not run again for that window.

See [window rules](/guides/window_rules#workspaces) and the [workspaces](/shell-api/workspaces) reference.
