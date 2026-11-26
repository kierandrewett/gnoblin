# Event handlers

Put live compositor behaviour in a module your shell file imports. Write it in TypeScript (`shell.ts`), Python
(`shell.py`) or Rust (a shell library built with cargo). A handler reacts to something that happens, such as a window
opening, and runs your code.

Create `~/.config/gnoblin/scripts/window-log` in the language your shell uses, then import it from the shell file:

- TypeScript: `scripts/window-log.ts` and `import "./scripts/window-log.ts";` in `shell.ts`.
- Python: `scripts/window_log.py` and `import scripts.window_log` in `shell.py`.
- Rust: add the module to the crate and call its setup from `main`.

The module logs each new window through `journalctl`:

::: tabs

```typescript
import { ClientCollectionListener, gnoblin } from "gnoblin-core/generated/objects";
import type { Client } from "gnoblin-core/generated/objects";

class WindowLog extends ClientCollectionListener {
    onAdded(client: Client) {
        gnoblin.spawn(["logger", "-t", "windows", `${client.appId}: ${client.title}`]);
    }
}

gnoblin.clients.addListener(new WindowLog());
```

```python
from gnoblin.objects.model import ClientCollectionListener, Gnoblin


class WindowLog(ClientCollectionListener):
    def on_added(self, client, collection):
        Gnoblin.get().process.spawn(
            ["logger", "-t", "windows", f"{client.app_id}: {client.title}"]
        )


Gnoblin.get().clients.add_listener(WindowLog())
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let root = ModelRoot::get(&objects)?;
    let (Some(clients), Some(process)) = (root.clients()?, root.process()?) else {
        return Ok(());
    };
    let listener = clients.on_added(move |client, _collection| {
        let (Ok(app_id), Ok(title)) = (client.app_id(), client.title()) else {
            return;
        };
        let line = format!("{app_id}: {title}");
        let _ = process.spawn(&["logger".into(), "-t".into(), "windows".into(), line], None);
    })?;
    let _listener = listener;
    Ok(())
}
```

:::

Save the shell file. The shell reloads on save, or run `gnoblinctl config reload`. Open a window and read the line with
`journalctl --user -t windows -n 1`.

A reload detaches the handlers of the old configuration and attaches the ones your files attach again. A failed reload
leaves the running config in place and reports the error.

## Choose a kind of handler

- A listener, attached to `gnoblin.clients`, `gnoblin.workspaces` or `gnoblin.outputs` (`clients`, `workspaces` and
  `outputs` in Python and Rust), receives the objects themselves. Use it for windows, workspaces and outputs:
  [Window, workspace and output events](/events/desktop-state).
- A session or runtime listener, attached to `session` or `runtime`, hears the session and the configuration, such as
  the bell or a reload: [Events](/events/).
- A function bound with `input.primary.bind` runs when a trigger is pressed: [Keyboard shortcuts](/shell-api/keyboard-shortcuts).
- `timers.after(ms, fn)` runs a function later: [Operations, errors and timers](/shell-api/errors#timers).

## What a handler can do

A handler reads and changes windows, workspaces and outputs through the [runtime API](/shell-api/), and it can ask the
session to lock or log out. It cannot read arbitrary files, open network connections or load native modules. To run a
program, call `spawn` on the process object: `gnoblin.spawn` in TypeScript, `Gnoblin.get().process.spawn` in Python,
and `process.spawn` in Rust.

Panels, launchers, notifications and other visible interface belong either in the shell's
own [UI toolkit](/js-ui) or in separate Wayland clients that you start with `spawn`.

Keep a handler short. Each call has a time budget, and a call that runs past it is stopped and reported: see
[Write your own shell](/user-shell#time-and-memory-limits).
