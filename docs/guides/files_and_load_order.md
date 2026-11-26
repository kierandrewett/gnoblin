# Files and load order

[Configuration reference](/config/configure)

Start with one shell file: `shell.ts`, `shell.py`, or a Rust shell library. Split it into modules when that makes it
easier to read. A module is an ordinary file of the same language that the shell imports.

## Which file is loaded?

Gnoblin runs one shell, chosen when it starts. `gnoblin` looks in seven places in order and takes the first that names
exactly one shell:

1. The command line: `gnoblin ./shell.ts`, `gnoblin ./shell.py`, or `gnoblin --shell NAME`.
2. The `GNOBLIN_SHELL` environment variable: a path when it has a `/`, else the name of an installed shell.
3. `[shell]` in `config.toml`: `default = "NAME"` or `path = "..."`.
4. `shell.so`, `shell.py`, `shell.js`, `shell.mjs`, `shell.ts` or `shell.mts` in the config directory.
5. An installed shell in `~/.local/share/gnoblin/shells/`.
6. An installed shell in `gnoblin/shells/` under each `XDG_DATA_DIRS` directory.
7. The packaged default, `<prefix>/share/gnoblin/default-shell/shell.ts`.

The config directory is `~/.config/gnoblin`, or `$XDG_CONFIG_HOME/gnoblin` when that variable is an absolute path. Put
your shell file there, and `gnoblinctl config init` writes a TypeScript starter. A Python or Rust shell is a file you
write yourself.

Two shell files in the config directory, such as `shell.ts` and `shell.py`, leave nothing to choose by, so the start
stops with a message that lists both. An already-running compositor does not see a variable you set in a terminal.

[Start Gnoblin and choose a shell](/starting-gnoblin) shows an example of each place, the `config.toml` keys, and the
messages for no shell and for several. `gnoblin --list-shells` prints what Gnoblin can see and which one would run.

When no user config exists, the session runs the stock config described in the
[TypeScript shell](/typescript-shell#with-no-shell-ts). There are no system drop-in directories: `/etc/gnoblin/conf.d` and
`gnoblin/conf.d` under `XDG_DATA_DIRS` are not read by the shell. Put a setting you want from a package in your shell
file.

## Split a config into files

An import runs the module. Put each concern in its own file as a function that takes what it needs and does its calls,
and call the functions from the shell file:

::: tabs

```typescript
// keys.ts
export function bindKeys(input: typeof gnoblin.input.primary): void {
    input.bind("Super+Return", () => {
        gnoblin.spawn(["foot"]);
    });
    input.bind("Super+d", () => {
        gnoblin.spawn(["fuzzel"]);
    });
}
```

```python
# keys.py
from gnoblin import gnoblin


def bind_keys():
    @gnoblin.bind("Super+Return")
    def terminal():
        gnoblin.spawn(["foot"])

    @gnoblin.bind("Super+d")
    def launcher():
        gnoblin.spawn(["fuzzel"])
```

```rust
// keys.rs
use gnoblin_core::Gnoblin;

pub fn bind_keys(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let term = g.clone();
    g.bind("Super+Return", move || term.spawn("foot"))?;
    let launcher = g.clone();
    g.bind("Super+d", move || launcher.spawn("fuzzel"))?;
    Ok(())
}
```

:::

The `appearance` module styles windows. It needs the `gnoblin-std` library, which a TypeScript file loads with
`import "gnoblin-std";` and a Python file imports as `gnoblin.std`.

::: tabs

```typescript
// appearance.ts
import "gnoblin-std";

export function styleWindows(): void {
    gnoblin.clients.style.cornerRadius = 12;
}
```

```python
# appearance.py
from gnoblin import gnoblin
from gnoblin.objects.std import ClientStyle, CornerRadius


def style_windows():
    style = ClientStyle.of_clients()
    style.corner_radius = CornerRadius(all=12.0)
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
    Ok(())
}
```

:::

Then the shell file imports the modules and calls them in order:

::: tabs

```typescript skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
// shell.ts
import { styleWindows } from "./appearance.ts";
import { bindKeys } from "./keys.ts";

styleWindows();
bindKeys(gnoblin.input.primary);
```

```python skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
# shell.py
from gnoblin import gnoblin
from appearance import style_windows
from keys import bind_keys

style_windows()
bind_keys()
```

```rust skip="the sibling files the import names are not part of this page, so the file does not resolve on its own"
// lib.rs
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

mod appearance;
mod keys;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    appearance::style_windows(g)?;
    keys::bind_keys(g)?;
    Ok(())
}
```

:::

- A TypeScript path starts with `./` or `../` and names the file with its extension. It resolves under the config
  directory, and a path outside it is refused.
- A Python module in the config directory is imported by its file name, as in `from keys import bind_keys`.
- A Rust module is declared with `mod keys;` and lives in its own file in the crate's `src` directory.
- There is no glob. List each file you import. The order of the calls in the shell file is the order they run.
- A module that is imported twice runs once.
- A module is a program, so it can compute a value or read the `gnoblin` object like the shell file can. Keep it quick:
  the whole load runs inside one time budget.
- An import that does not resolve, a syntax error or a throw in any file fails the load at the line you wrote, in that
  file's own coordinates. The running config stays.

## Replace or add?

Your shell file replaces the stock default as a whole: nothing is layered under it, so a stock key or workspace you want
is a line you keep. Bind a key once: a second `bind` of a trigger that is already bound is refused, so bind it from one
file only.

::: tabs

```typescript
const common = ["Super+Return", "Super+d"];
const laptop = ["XF86MonBrightnessUp"];

for (const trigger of [...common, ...laptop]) {
    gnoblin.input.primary.bind(trigger, () => {
        gnoblin.spawn(["notify-send", trigger]);
    });
}
```

```python
from gnoblin import gnoblin

common = ["Super+Return", "Super+d"]
laptop = ["XF86MonBrightnessUp"]

for trigger in common + laptop:
    def notify(trigger=trigger):
        gnoblin.spawn(["notify-send", trigger])

    gnoblin.bind(trigger, notify)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    for trigger in ["Super+Return", "Super+d", "XF86MonBrightnessUp"] {
        let g2 = g.clone();
        g.bind(trigger, move || g2.spawn_with("notify-send", [trigger]))?;
    }
    Ok(())
}
```

:::

- Later statements run later. A style set later on the same scope overwrites an earlier one.
- Several modules can add to the same collection: `gnoblin.workspaces.add` and `gnoblin.clients.addListener` accumulate.

## What a config file can do

A shell file and its modules run inside the shell. They can use their language's own features and the `gnoblin` object.
Three limits apply:

- They cannot read files, and they cannot run programs while loading. Start a program with a key or `spawn` from a
  function that runs after the load.
- An endless loop is cut off by the time budget.
- Each reload evaluates the files again, so a variable does not survive it. A Rust shell is compiled, so a change to any
  of its modules needs a rebuild before the reload.

## Reload

A save reloads a running session, and so does `gnoblinctl config reload`. The shell watches the config directory, and
any module file in it that changes triggers one reload. `GNOBLIN_JS_WATCH=0` turns that off, for a file system that
cannot be watched.

A reload commits the whole document your files describe, so a setting you delete is back to its default. A failed reload
keeps the running config, and `gnoblinctl config reload` prints the error with its file, line and column.

Check a set of files without a session:

```sh
gnoblinctl check shell.ts
```
