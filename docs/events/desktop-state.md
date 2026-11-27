# Window, workspace and output events

Windows, workspaces and outputs are objects in `clients`, `workspaces` and `outputs`. A listener attached to one of them
is told what happens to it and receives the object itself.

## Listen to windows

Attach a `ClientCollectionListener` to the window collection to hear every window, or a `ClientListener` to one window to
hear that one. The two have the same methods, and a collection listener adds `on_added` and `on_removed`. Every method is
optional.

::: tabs

```typescript
import { Gnoblin, ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

const gnoblin = Gnoblin.get();

class Titles extends ClientCollectionListener {
    onTitleChanged(client: Client, title: string) {
        gnoblin.spawn(["logger", "-t", "titles", `${client.appId}: ${title}`]);
    }

    onClosed(client: Client) {
        gnoblin.spawn(["logger", "-t", "titles", `${client.appId} closed`]);
    }
}

gnoblin.clients.addListener(new Titles());
```

```python
from gnoblin.objects.model import Client, ClientCollectionListener, Gnoblin


class Titles(ClientCollectionListener):
    def on_title_changed(self, client: Client, title: str) -> None:
        Gnoblin.get().spawn(["logger", "-t", "titles", f"{client.app_id}: {title}"])

    def on_closed(self, client: Client) -> None:
        Gnoblin.get().spawn(["logger", "-t", "titles", f"{client.app_id} closed"])


Gnoblin.get().clients.add_listener(Titles())
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

fn watch(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let clients = ModelRoot::get(&g.objects()?)?.clients()?;
    clients
        .on_title_changed(|client, title| {
            println!("{}: {title}", client.app_id().unwrap_or_default());
        })?
        .keep();
    clients
        .on_closed(|client| {
            println!("{} closed", client.app_id().unwrap_or_default());
        })?
        .keep();
    Ok(())
}
```

:::

Save the TypeScript as `shell.ts`, the Python as `shell.py`, and put the Rust in a shell library. Read the lines with
`journalctl --user -t titles -f`.

| Method                                          | Called when                                                                 |
| ----------------------------------------------- | --------------------------------------------------------------------------- |
| `onTitleChanged` / `on_title_changed`           | The window's title changed.                                                 |
| `onAppIdChanged` / `on_app_id_changed`          | The window's application id changed.                                        |
| `onFocusChanged` / `on_focus_changed`           | The window gained or lost the keyboard focus.                               |
| `onGeometryChanged` / `on_geometry_changed`     | The window's rectangle changed.                                             |
| `onClosed` / `on_closed`                        | The window closed. The object keeps its last values and `alive` is `false`. |
| `onPointerEnter` / `on_pointer_enter`           | The pointer crossed into the window.                                        |
| `onPointerMotion` / `on_pointer_motion`         | The pointer moved over the window.                                          |
| `onPointerLeave` / `on_pointer_leave`           | The pointer left the window.                                                |
| `onPointerPress` / `on_pointer_press`           | A button went down on the window.                                           |
| `onPointerRelease` / `on_pointer_release`       | A button came up on the window.                                             |
| `onDecorationChanged` / `on_decoration_changed` | Who draws the window's frame changed: `client`, `server` or `none`.         |
| `onReady` / `on_ready`                          | The window finished its first setup. A window there at boot hears it once.  |

A pointer event has `x` and `y` in the window's own coordinates and `button`, which is 0 for every event but a press and
a release.

Each method is also available as a single subscription that returns a `Listener`, whose `stop()` detaches it:

- A window has them as `client.on("titleChanged", handler)`.
- Every collection has them as `clients.on("titleChanged", handler)`, which hears every window.
- Rust has them as `client.on_title_changed(handler)` and `clients.on_title_changed(handler)`.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";

const gnoblin = Gnoblin.get();

const focus = gnoblin.clients.on("focusChanged", (client, focused) => {
    if (focused) {
        gnoblin.spawn(["logger", "-t", "focus", client.appId]);
    }
});

focus.stop(); // focus is not logged any more
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()


def log_focus(client, focused):
    if focused:
        gnoblin.spawn(["logger", "-t", "focus", client.app_id])


focus = gnoblin.clients.on("focus_changed", log_focus)
focus.stop()  # focus is not logged any more
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

fn watch_focus(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let clients = ModelRoot::get(&g.objects()?)?.clients()?;
    let focus = clients.on_focus_changed(|client, focused| {
        if focused {
            println!("{}", client.app_id().unwrap_or_default());
        }
    })?;
    // Dropping the Listener stops the subscription.
    drop(focus); // focus is not logged any more
    Ok(())
}
```

:::

### Answer a request yourself

A client asks the compositor to maximise, fullscreen, minimise, close, raise or move its window, and the compositor waits
for the shell to answer. The kinds and the policy are in [Windows](/shell-api/windows#window-requests).

## Listen to membership

A `ClientCollectionListener` on the window collection hears windows join and leave, and an `OutputCollectionListener` on
the output collection does the same for outputs:

- `onAdded` / `on_added` runs when a window or an output joins.
- `onRemoved` / `on_removed` runs when one leaves.

The workspace collection has `added` and `removed` events of its own: `workspaces.on("added", handler)`. See
[Workspaces](/shell-api/workspaces).

::: tabs

```typescript
import { Gnoblin, ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";
import { Toast } from "gnoblin-std";

const gnoblin = Gnoblin.get();

class Count extends ClientCollectionListener {
    onAdded(client: Client) {
        Toast.show(`${client.appId} opened`);
    }
}

gnoblin.clients.addListener(new Count());

gnoblin.clients.on("added", (client) => {
    gnoblin.spawn(["logger", "-t", "windows", `${client.appId} appeared`]);
});
```

```python
from gnoblin.objects.model import Client, ClientCollectionListener, Gnoblin
from gnoblin.objects.std import Toast


class Count(ClientCollectionListener):
    def on_added(self, client: Client, collection) -> None:
        Toast.show(f"{client.app_id} opened")


Gnoblin.get().clients.add_listener(Count())
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

fn count(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let clients = ModelRoot::get(&g.objects()?)?.clients()?;
    clients
        .on_added(|client, _collection| {
            println!("{} opened", client.app_id().unwrap_or_default());
        })?
        .keep();
    Ok(())
}
```

:::

The shell reads what the compositor published a batch at a time:

- A window that opens and closes inside one batch is gone when the batch is read. The shell is told of it neither way.
- A window the shell has been told of is always told closed later.
- A script that acts on a window it has just opened, such as closing it, listens for the window to appear first and acts
  from there.

## Listen to outputs

An `OutputCollectionListener` on the output collection, or an `OutputListener` on one output, has these methods:

| Method                                      | Called when                                                                  |
| ------------------------------------------- | ---------------------------------------------------------------------------- |
| `onConnected` / `on_connected`              | An output was plugged in while the shell runs.                               |
| `onDisconnected` / `on_disconnected`        | An output is gone. Its last values stay readable.                            |
| `onModeChanged` / `on_mode_changed`         | The mode changed. The mode is `WxH@Hz`.                                      |
| `onScaleChanged` / `on_scale_changed`       | The scale changed.                                                           |
| `onGeometryChanged` / `on_geometry_changed` | The output's position or size changed.                                       |
| `onApplied` / `on_applied`                  | An `apply` or `mirror` was answered. The error is empty when it took effect. |

A listener on the collection does not hear `onConnected` for an output that was there when the shell started. See
[Outputs](/shell-api/outputs#listen-for-plugging-and-unplugging).

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";

const gnoblin = Gnoblin.get();

gnoblin.outputs.on("connected", (output) => {
    gnoblin.spawn(["notify-send", `${output.connector} connected`]);
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()

gnoblin.outputs.on(
    "connected",
    lambda output: gnoblin.spawn(["notify-send", f"{output.connector} connected"]),
)
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

fn watch_outputs(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let outputs = ModelRoot::get(&g.objects()?)?.outputs()?;
    outputs
        .on_connected(|output| {
            println!("{} connected", output.connector().unwrap_or_default());
        })?
        .keep();
    Ok(())
}
```

:::

`onAdded` runs for the outputs there are now as well as for each that joins. `onConnected` runs only for an output that is
plugged in later.

## Signals the compositor raises

These are the compositor's own signals behind the listeners above. A script hears them through the listener methods, which
receive the object itself. The signals carry object identities.

| Event                           | Fields                                        | When                                              |
| ------------------------------- | --------------------------------------------- | ------------------------------------------------- |
| `gnoblin.window.created`        | `window`                                      | A window came into existence.                     |
| `gnoblin.window.changed`        | `windowId`, `changed`, `window`               | A property other than `focused` changed.          |
| `gnoblin.window.closed`         | none                                          | The window is gone.                               |
| `gnoblin.window.menu_requested` | `x`, `y`, `width`, `height`, `source`, `seat` | The user or a client asked for the window's menu. |
| `gnoblin.workspace.created`     | `workspace`                                   | A workspace was created.                          |
| `gnoblin.workspace.removed`     | `workspaceId`, `last`                         | A workspace was removed.                          |
| `gnoblin.output.added`          | `outputId`, `output`                          | A display was connected.                          |
| `gnoblin.output.changed`        | `outputId`, `output`, `changed`               | A property of the output changed.                 |
| `gnoblin.output.removed`        | `outputId`, `last`                            | A display was disconnected.                       |
| `gnoblin.output.closed`         | none                                          | The output object is gone.                        |

- `changed` is the list of property names that changed.
- `window`, `output`, `workspace` and `last` hold the object's numeric identity, not the object.
- `x`, `y`, `width` and `height` of the menu request are an anchor in the compositor's coordinates. A point has no size.
- `source` is `client` for a program's own request, `keyboard` for the window menu key and `extension` for a script that
  asked. `seat` names the seat that made the request.

`gnoblin.window.changed` does not report a focus change. A focus change reaches `onFocusChanged` on a `ClientListener`.
