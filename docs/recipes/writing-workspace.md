# Set up a writing workspace

Give a writing workspace a stable ID, send new editor windows there, and bind keys that show a workspace or move the
focused window to one. Put this in your shell file: `~/.config/gnoblin/shell.ts`, `~/.config/gnoblin/shell.py`, or a
Rust shell crate's `lib.rs`. The three do the same thing:

::: tabs

```typescript
import { ClientState, FloatingLayout, MasterStackLayout, MonocleLayout, Workspace, Workspaces } from "gnoblin-std";

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
    gnoblin.input.primary?.bind(`Super+${index + 1}`, () => {
        workspaces.byId(id)?.show();
    });
});

gnoblin.clients?.addListener({
    onAdded(client) {
        const write = workspaces.byId("write");
        if (client.appId === "org.gnome.TextEditor" && write !== undefined) {
            ClientState.of(client).moveToWorkspace(write);
        }
    },
});

gnoblin.input.primary?.bind("Super+Shift+w", () => {
    const write = workspaces.byId("write");
    const focused = gnoblin.input.primary?.focusedClient;
    if (write !== undefined && focused !== undefined) {
        ClientState.of(focused).moveToWorkspace(write);
    }
});
```

```python
from gnoblin import gnoblin
from gnoblin.objects.std import ClientState, FloatingLayout, MasterStackLayout, MonocleLayout, Workspace, Workspaces

workspaces = Workspaces.get()
layouts = {
    "write": MonocleLayout(),
    "research": MasterStackLayout(),
    "review": FloatingLayout(),
    "other": FloatingLayout(),
}
names = {"write": "Write", "research": "Research", "review": "Review", "other": "Other"}
ids = ["write", "research", "review", "other"]

for index, id in enumerate(ids):
    workspaces.add(Workspace(id, name=names[id], layout=layouts[id]))


@gnoblin.bind("<Super>1")
def show_write() -> None:
    workspaces.by_id("write").show()


@gnoblin.bind("<Super>2")
def show_research() -> None:
    workspaces.by_id("research").show()


@gnoblin.bind("<Super>3")
def show_review() -> None:
    workspaces.by_id("review").show()


@gnoblin.bind("<Super>4")
def show_other() -> None:
    workspaces.by_id("other").show()


@gnoblin.on_client_created
def place_editor(client) -> None:
    write = workspaces.by_id("write")
    if client.app_id == "org.gnome.TextEditor" and write is not None:
        ClientState.of(client).move_to_workspace(write)


@gnoblin.bind("<Super><Shift>w")
def send_to_write() -> None:
    write = workspaces.by_id("write")
    focused = gnoblin.input.primary.focused_client()
    if write is not None and focused is not None:
        ClientState.of(focused).move_to_workspace(write)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{
    ClientState, FloatingLayout, MasterStackLayout, MonocleLayout, Workspace, WorkspaceOptions, Workspaces,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.std()?.objects()?;
    let workspaces = Workspaces::get(&objects)?;

    let write = Workspace::new_with(
        &objects,
        "write",
        WorkspaceOptions {
            name: Some("Write".to_owned()),
            layout: Some(MonocleLayout::new(&objects)?.as_layout()?),
            ..Default::default()
        },
    )?;
    let research = Workspace::new_with(
        &objects,
        "research",
        WorkspaceOptions {
            name: Some("Research".to_owned()),
            layout: Some(MasterStackLayout::new(&objects)?.as_layout()?),
            ..Default::default()
        },
    )?;
    let review = Workspace::new_with(
        &objects,
        "review",
        WorkspaceOptions {
            name: Some("Review".to_owned()),
            layout: Some(FloatingLayout::new(&objects)?.as_layout()?),
            ..Default::default()
        },
    )?;
    let other = Workspace::new_with(
        &objects,
        "other",
        WorkspaceOptions {
            name: Some("Other".to_owned()),
            layout: Some(FloatingLayout::new(&objects)?.as_layout()?),
            ..Default::default()
        },
    )?;
    workspaces.add(&write)?;
    workspaces.add(&research)?;
    workspaces.add(&review)?;
    workspaces.add(&other)?;

    let input = g.input()?.and_then(|manager| manager.primary().ok().flatten());
    let Some(input) = input else {
        return Ok(());
    };
    input.bind("Super+Shift+w", Some(move |_event| {
        let focused = input.focused_client()?;
        if let (Some(focused), Some(write)) = (focused, workspaces.by_id("write")?) {
            ClientState::of(&objects, &focused)?.move_to_workspace(&write)?;
        }
        Ok(true)
    }), None)?;
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

`ClientState.of(client)` moves the window and answers whether it moved. It does not move a fullscreen client or a
dialog, so check the result if the window must end up on `write`.

See [workspaces](../shell-api/workspaces.md) for the full member list.
