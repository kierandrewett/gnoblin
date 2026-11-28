# Window rules

A window rule is code in your shell file that runs for each window that matches. Gnoblin has no rule table. You attach a
listener to the clients and decide in a function what to do with each window. The file runs against live objects, and a
save reloads it.

The examples are shown in TypeScript (`shell.ts`), Python (`shell.py`) and Rust (a shell built with cargo). Each one
does the same thing.

## Add a rule

This moves every editor window to workspace `dev` and maximises it:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";

gnoblin.clients.on("added", (client) => {
    if (client.appId === "org.example.Editor") {
        client.maximised = true;
    }
});
```

```python
from gnoblin import Gnoblin


@Gnoblin.get().clients.on("added")
def rule(client):
    if client.app_id == "org.example.Editor":
        client.maximised = True
```

```rust
use gnoblin_core::{Client, ClientCollection, Gnoblin};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let clients = Gnoblin::get(&g.objects()?)?.clients()?;
    clients
        .on_added(|client: Client, _clients: ClientCollection| {
            if client.app_id().unwrap_or_default() == "org.example.Editor" {
                let _ = client.set_maximised(true);
            }
        })?
        .keep();
    Ok(())
}
```

:::

`added` runs when a window opens. It does not run for windows already open when the file loads, so a save applies a
rule only to windows opened after it.

## Choose what to match

A rule is an `if` on a property of the window:

| Property | TypeScript     | Python          | Rust               |
| -------- | -------------- | --------------- | ------------------ |
| App ID   | `client.appId` | `client.app_id` | `client.app_id()?` |
| Title    | `client.title` | `client.title`  | `client.title()?`  |

Use the app ID to match an application across all its windows. Use the title to target one document or page. Titles
change as the document changes, so a title match can stop matching later.

To list the app IDs and titles of the windows you want to match, run `gnoblinctl window list --json`.

## Workspaces

Declare a workspace and move a window to it. Look the workspace up by id, and handle the case where it is missing:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { Workspace, Workspaces } from "gnoblin-std";

Workspaces.get().add(new Workspace("code"));

gnoblin.clients.on("added", (client) => {
    if (client.appId !== "org.example.Editor") {
        return;
    }
    const code = Workspaces.get().byId("code");
    if (code !== undefined) {
        client.moveToWorkspace(code);
    }
});
```

```python
from gnoblin import Gnoblin
from gnoblin.std import Workspace, Workspaces

code = Workspace("code")
Workspaces.get().add(code)


@Gnoblin.get().clients.on("added")
def rule(client):
    if client.app_id == "org.example.Editor":
        client.move_to_workspace(code)
```

```rust
use gnoblin_core::{Client, ClientCollection, Gnoblin};
use gnoblin_std::{Workspace, Workspaces};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let code = Workspace::new(&objects, "code")?;
    Workspaces::get(&objects)?.add(&code)?;
    let clients = Gnoblin::get(&g.objects()?)?.clients()?;
    clients
        .on_added(move |client: Client, _clients: ClientCollection| {
            if client.app_id().unwrap_or_default() == "org.example.Editor" {
                let _ = client.move_to_workspace(&code);
            }
        })?
        .keep();
    Ok(())
}
```

:::

`Workspaces.by_id` in Python and `workspaces.by_id("code")?` in Rust return `None` for a workspace you did not add, so
check it before you move a window to it.

## Order

Code runs in the order you wrote it. A later statement overrides what an earlier one set, so put the general rule first
and the rule for one app after it.

## Floating and secure windows

A listener can mark a window floating, so a tiling layout leaves it where it is, and secure, so it is kept out of
screen captures:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";

gnoblin.clients.on("added", (client) => {
    if (client.appId === "org.example.Player") {
        client.secure = true;
        client.floating = true;
    }
});
```

```python
from gnoblin import Gnoblin


@Gnoblin.get().clients.on("added")
def rule(client):
    if client.app_id == "org.example.Player":
        client.secure = True
        client.floating = True
```

```rust
use gnoblin_core::{Client, ClientCollection, Gnoblin};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let clients = Gnoblin::get(&g.objects()?)?.clients()?;
    clients
        .on_added(|client: Client, _clients: ClientCollection| {
            if client.app_id().unwrap_or_default() == "org.example.Player" {
                let _ = client.set_secure(true);
                let _ = client.set_floating(true);
            }
        })?
        .keep();
    Ok(())
}
```

:::

To react when a title or app ID changes after the window opens, attach a listener for that change to the window itself.
