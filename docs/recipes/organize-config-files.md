# Organize a growing config

Keep the entry point short and group statements by the kind of change you make. For example, let the shell file import a
module that places windows and one that binds your shortcuts, and call them in the order you want.

In `windows` (`windows.ts`, `windows.py` or `windows.rs`):

::: tabs

```typescript
// windows.ts
import { gnoblin } from "gnoblin-core";

export function placeWindows(): void {
    gnoblin.clients.on("added", (client) => {
        if (client.appId === "org.gnome.TextEditor") {
            client.maximised = true;
        }
    });
}
```

```python
# windows.py
from gnoblin.objects.model import Gnoblin


def place_windows():
    @Gnoblin.get().clients.on("added")
    def maximise_editor(client):
        if client.app_id == "org.gnome.TextEditor":
            client.maximised = True
```

```rust
// windows.rs
use gnoblin_core::{Client, Gnoblin};
use gnoblin_std::prelude::*;

pub fn place_windows(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?;
    let clients = Gnoblin::get(&objects)?.clients()?;
    clients
        .on_added(|client: Client, _collection| {
            if client.app_id().unwrap_or_default() == "org.gnome.TextEditor" {
                let _ = client.set_maximised(true);
            }
        })?
        .keep();
    Ok(())
}
```

:::

In `bindings` (`bindings.ts`, `bindings.py` or `bindings.rs`):

::: tabs

```typescript
// bindings.ts
import { gnoblin } from "gnoblin-core";

export function bindKeys(): void {
    gnoblin.input.primary.bind("Super+d", () => {
        gnoblin.spawn(["fuzzel"]);
    });
}
```

```python
# bindings.py
from gnoblin import gnoblin


def bind_keys():
    @gnoblin.bind("Super+d")
    def launcher():
        gnoblin.spawn(["fuzzel"])
```

```rust
// bindings.rs
use gnoblin_core::Gnoblin;

pub fn bind_keys(g: &Gnoblin) -> gnoblin_core::Res<()> {
    let launcher = g.clone();
    g.bind("Super+d", move || launcher.spawn(&["fuzzel".to_owned()], None))?;
    Ok(())
}
```

:::

In the shell file (`shell.ts`, `shell.py` or `lib.rs`):

::: tabs

```typescript skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
// shell.ts
import { bindKeys } from "./bindings.ts";
import { placeWindows } from "./windows.ts";

placeWindows();
bindKeys();
```

```python skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
# shell.py
from gnoblin import gnoblin
from windows import place_windows
from bindings import bind_keys

place_windows()
bind_keys()
```

```rust skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
// lib.rs
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

mod bindings;
mod windows;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    windows::place_windows(g)?;
    bindings::bind_keys(g)?;
    Ok(())
}
```

:::

Imported files share the same `gnoblin` object and need no registration. Keep the calls in one place, the shell file, so
you can see which file supplies each part, and in the order they run. A trigger is bound once, so bind each key from one
file only. `gnoblin.workspaces.add` and `gnoblin.clients.on("added", ...)` accumulate across files.

Check the files with `gnoblinctl check shell.ts`, `gnoblinctl check shell.py` or `gnoblinctl check` in a Rust shell crate,
and read [file loading and order](/guides/files_and_load_order) before adding overlapping files.
