# Set up a writing workspace

Give a writing workspace a stable ID, send new editor windows there, and bind keys that show a workspace or move the
focused window to one. Put this in your shell file: `~/.config/gnoblin/shell.ts`, `~/.config/gnoblin/shell.py`, or a
Rust shell crate's `lib.rs`. The three do the same thing:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import { FloatingLayout, MasterStackLayout, MonocleLayout, Workspace, Workspaces } from "gnoblin-std/generated/objects";

const workspaces = Workspaces.get();
const layouts = {
    write: new MonocleLayout(),
    research: new MasterStackLayout(),
    review: new FloatingLayout(),
    other: new FloatingLayout(),
};
const names = { write: "Write", research: "Research", review: "Review", other: "Other" };
const ids = ["write", "research", "review", "other"] as const;

ids.forEach((id, index) => {
    workspaces.add(new Workspace(id, { name: names[id], layout: layouts[id] }));
    gnoblin.input.primary.bind(`Super+${index + 1}`, () => {
        workspaces.byId(id)?.show();
    });
});

gnoblin.clients.on("added", (client) => {
    const write = workspaces.byId("write");
    if (client.appId === "org.gnome.TextEditor" && write !== undefined) {
        client.moveToWorkspace(write);
    }
});

gnoblin.input.primary.bind("Super+Shift+w", () => {
    const write = workspaces.byId("write");
    const focused = gnoblin.input.focusedClient;
    if (write !== undefined && focused !== undefined) {
        focused.moveToWorkspace(write);
    }
});
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import FloatingLayout, MasterStackLayout, MonocleLayout, Workspace, Workspaces

gnoblin = Gnoblin.get()
workspaces = Workspaces.get()
layouts = {
    "write": MonocleLayout(),
    "research": MasterStackLayout(),
    "review": FloatingLayout(),
    "other": FloatingLayout(),
}
names = {"write": "Write", "research": "Research", "review": "Review", "other": "Other"}
ids = ["write", "research", "review", "other"]


def show(id):
    def run(_event) -> None:
        workspaces.show_by_id(id)

    return run


for index, id in enumerate(ids):
    workspaces.add(Workspace(id, name=names[id], layout=layouts[id]))
    gnoblin.input.primary.bind(f"Super+{index + 1}", show(id))


def move_to(client, workspace) -> bool:
    return client.move_to_workspace(workspace)


@gnoblin.clients.on("added")
def place_editor(client) -> None:
    write = workspaces.by_id("write")
    if client.app_id == "org.gnome.TextEditor" and write is not None:
        move_to(client, write)


def send_to_write(_event) -> None:
    write = workspaces.by_id("write")
    focused = gnoblin.input.focused_client
    if write is not None and focused is not None:
        move_to(focused, write)


gnoblin.input.primary.bind("Super+Shift+w", send_to_write)
```

```rust
use gnoblin_core::objects::{Client, ModelRoot};
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{FloatingLayout, MasterStackLayout, MonocleLayout, Workspace, WorkspaceOptions, Workspaces};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let workspaces = Workspaces::get(&objects)?;
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(input) = model.input()? else {
        return Ok(());
    };
    let Some(seat) = input.primary()? else {
        return Ok(());
    };

    let layouts = [
        ("write", "Write", MonocleLayout::new(&objects)?.as_layout()?),
        ("research", "Research", MasterStackLayout::new(&objects)?.as_layout()?),
        ("review", "Review", FloatingLayout::new(&objects)?.as_layout()?),
        ("other", "Other", FloatingLayout::new(&objects)?.as_layout()?),
    ];
    for (index, (id, name, layout)) in layouts.into_iter().enumerate() {
        let workspace = Workspace::new_with(
            &objects,
            id,
            WorkspaceOptions { name: Some(name.to_owned()), layout: Some(layout), ..Default::default() },
        )?;
        workspaces.add(&workspace)?;
        seat.bind(
            &format!("Super+{}", index + 1),
            Some(move |_event| {
                workspace.show(None)?;
                Ok(true)
            }),
            None,
        )?;
    }

    if let Some(clients) = model.clients()? {
        let workspaces = Workspaces::get(&objects)?;
        clients
            .on_added(move |client: Client, _collection| {
                let write = workspaces.by_id("write").ok().flatten();
                if let Some(write) = write
                    && client.app_id().is_ok_and(|app_id| app_id == "org.gnome.TextEditor")
                {
                    let _ = client.move_to_workspace(&write);
                }
            })?
            .keep();
    }

    seat.bind(
        "Super+Shift+w",
        Some(move |_event| {
            if let (Some(focused), Some(write)) = (input.focused_client()?, workspaces.by_id("write")?) {
                focused.move_to_workspace(&write)?;
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Each workspace declares a stable id. The id is what keys and code refer to, and `byId` answers nothing for an id you
did not declare.

| Argument or option | Accepted value                    | Meaning                                                                                      |
| ------------------ | --------------------------------- | -------------------------------------------------------------------------------------------- |
| id                 | Required, unique non-empty string | Stable workspace identity used by code and keys.                                             |
| `name`             | A string; defaults to the id      | Label shown for the workspace.                                                               |
| `layout`           | A layout object                   | How the windows of the workspace are arranged. See [workspaces](../shell-api/workspaces.md). |

Adding a workspace whose id is already in the list is refused, so keep the ids unique. Your file replaces the stock
workspaces, so keep at least one. `Super+1` to `Super+4` show the four workspaces, and `Super+Shift+w` moves the focused
window to `write` without showing it.

`client.moveToWorkspace(write)` (`move_to_workspace` in Python and Rust) moves the window and answers whether it moved. It does not move a fullscreen client or a
dialog, so check the result if the window must end up on `write`.

See [workspaces](../shell-api/workspaces.md) for the full member list.
