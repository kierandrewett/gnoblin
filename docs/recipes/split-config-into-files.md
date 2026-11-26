# Split a config into files

Import functions from your shell file and call them in the order you want them applied. Each file below does one job:
`appearance` styles windows and `bindings` binds keys. The shell file calls them. Each example uses the same two files.

In `appearance`, which uses the `gnoblin-std` library for `gnoblin.clients.style`:

::: tabs

```typescript
// appearance.ts
import { gnoblin } from "gnoblin-core/generated/objects";
import "gnoblin-std/generated/objects";

export function styleWindows(): void {
    gnoblin.clients.style.cornerRadius = 12;
    gnoblin.clients.style.opacity = 1;
}
```

```python
# appearance.py
from gnoblin import gnoblin
from gnoblin.objects.std import ClientStyle, CornerRadius


def style_windows():
    style = ClientStyle.of_clients()
    style.corner_radius = CornerRadius(all=12.0)
    style.opacity = 1.0
```

```rust
// appearance.rs
use gnoblin_std::objects::{ClientStyle, CornerRadius};
use gnoblin_std::prelude::*;

pub fn style_windows(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let style = ClientStyle::of_clients(&objects)?;
    style.set_corner_radius(&CornerRadius {
        all: Some(12.0),
        ..CornerRadius::default()
    })?;
    style.set_opacity(1.0)?;
    Ok(())
}
```

:::

In `bindings`:

::: tabs

```typescript
// bindings.ts
import { gnoblin } from "gnoblin-core/generated/objects";

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

pub fn bind_keys(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let launcher = g.clone();
    g.bind("Super+d", move || launcher.spawn("fuzzel"))?;
    Ok(())
}
```

:::

In the shell file:

::: tabs

```typescript skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
// shell.ts
import { styleWindows } from "./appearance.ts";
import { bindKeys } from "./bindings.ts";

styleWindows();
bindKeys();
```

```python skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
# shell.py
from appearance import style_windows
from bindings import bind_keys

style_windows()
bind_keys()
```

```rust skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
// lib.rs
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

mod appearance;
mod bindings;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    appearance::style_windows(g)?;
    bindings::bind_keys(g)?;
    Ok(())
}
```

:::

Modules share the same `gnoblin` object, and a module that exports nothing still runs when it is imported. There is no
glob. How a module is named depends on the language:

- A TypeScript path starts with `./`, names the file with its extension and resolves under the config directory.
- A Python module is imported by its file name.
- A Rust module is declared with `mod` and lives beside `lib.rs`.

See [files and load order](/guides/files_and_load_order) for how a reload runs several files.
