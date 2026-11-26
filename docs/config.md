# Configuration

Gnoblin is configured with one shell file: `~/.config/gnoblin/shell.ts`, `shell.py`, or a Rust shell library. The file is a
program: it runs from top to bottom against live objects such as `workspaces`, `input` and `clients`, and what it sets is
what the desktop does. There is no config object and nothing is layered under the file.

A file replaces the stock setup, and a statement you delete is gone after the next reload.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import { FloatingLayout, Workspace } from "gnoblin-std/generated/objects";

gnoblin.workspaces.add(new Workspace("main", { layout: new FloatingLayout() }));
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin import gnoblin
from gnoblin.objects.std import Workspace, Workspaces

Workspaces.get().add(Workspace("main"))  # floating is the default layout


@gnoblin.bind("<Super>Return")
def terminal():
    gnoblin.spawn(["foot"])
```

```rust
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    Workspace::builder_in(&g.std()?, "main").add()?; // floating is the default layout

    g.bind("<Super>Return", move || {
        if let Err(error) = g.spawn("foot") {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;
    Ok(())
}
```

:::

## Where to go

- [Shell files](/typescript-shell): write your first config, where it lives and how it reloads (TypeScript; see also [Python](/python-shell) and [Rust](/rust-shell)).
- [Start Gnoblin and choose a shell](/starting-gnoblin): run a shell from a file, a name or `config.toml`, and list the
  shells Gnoblin can see. The shell can also be a [Python script](/python-shell) or a [Rust library](/rust-shell).
- [Settings in the shell file](/js-settings): what you can set on each object, with an example, and the settings that have no
  home in the shell API yet.
- [Settings reference](/config/configure): the pages for devices, monitors, dialogs and the other settings.
- [Shortcuts](/guides/shortcuts): bind keys and gestures.
- [Files and load order](/guides/files_and_load_order): split a config over several files.
- [`gnoblinctl`](/gnoblinctl): check, reload and inspect the running configuration.

## First config

Write a commented starter shell in the language you pick. All three are the stock desktop as statements against the shell's
objects, and none needs a build step except Rust:

```sh
gnoblinctl config init                        # TypeScript: shell.ts and the editor files
gnoblinctl config init --language python      # Python: shell.py
gnoblinctl config init --language rust        # Rust: a Cargo project in ~/.config/gnoblin/rust-shell
```

`--language` takes `typescript` (the default), `python` or `rust`. Then reload the session. A Rust shell is built first: the
command prints the `cargo build` line and how to make `gnoblin` find the library.

```sh
gnoblinctl config reload
```

`config init` never replaces a config. A running shell also reloads the file when you save it. See
[configuration recipes](/recipes/) for complete examples and the [guides](/guides/shortcuts) for task-based
instructions.
