# Window, workspace and output events

Windows, workspaces and outputs are objects in `gnoblin.clients`, `gnoblin.workspaces` and `gnoblin.outputs`. A listener
attached to one of them is told what happens to it and receives the object itself.

## Listen to windows

Attach a `ClientCollectionListener` to `gnoblin.clients` to hear every window, or a `ClientListener` to one window's
`addListener` to hear that one.

The two have the same methods, and a collection listener adds `onAdded` and `onRemoved`. Every method is optional.

```ts
import { ClientCollectionListener, gnoblin } from "gnoblin-core";
import type { Client } from "gnoblin-core";

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

Save it as `shell.ts`. Open a terminal and read the lines with `journalctl --user -t titles -f`.

| Method                                | Called when                                                                 |
| ------------------------------------- | --------------------------------------------------------------------------- |
| `onTitleChanged(client, title)`       | The window's title changed.                                                 |
| `onAppIdChanged(client, appId)`       | The window's application id changed.                                        |
| `onFocusChanged(client, focused)`     | The window gained or lost the keyboard focus.                               |
| `onGeometryChanged(client, geometry)` | The window's rectangle changed. `geometry` is `{ x, y, width, height }`.    |
| `onClosed(client)`                    | The window closed. The object keeps its last values and `alive` is `false`. |
| `onPointerEnter(client, event)`       | The pointer crossed into the window.                                        |
| `onPointerMotion(client, event)`      | The pointer moved over the window.                                          |
| `onPointerLeave(client, event)`       | The pointer left the window.                                                |
| `onPointerPress(client, event)`       | A button went down on the window.                                           |
| `onPointerRelease(client, event)`     | A button came up on the window.                                             |
| `onDecorationChanged(client, mode)`   | Who draws the window's frame changed: `client`, `server` or `none`.         |
| `onReady(client)`                     | The window finished its first setup. A window there at boot hears it once.  |

A pointer `event` has `x` and `y` in the window's own coordinates and `button`, which is 0 for every event but a press and
a release.

Each method is also an event that `on` takes with just the handler, and `on` returns a `Listener` whose `stop()` detaches
it. See [Events](/events/#subscribe-to-one-event).

- A window has them: `client.on("titleChanged", handler)`.
- Every collection of windows has them: `gnoblin.clients.on("titleChanged", handler)` hears every window.
- `gnoblin` has them under a longer name: `gnoblin.onClientTitleChanged(handler)`.

```ts
const focus = gnoblin.onClientFocusChanged((client, focused) => {
    if (focused) {
        gnoblin.spawn(["logger", "-t", "focus", client.appId]);
    }
});

focus.stop(); // focus is not logged any more
```

### Answer a request yourself

A client asks the compositor to maximise, fullscreen, minimise, close, raise or move its window, and the compositor does
nothing until the shell answers. `WindowState.get().start()` makes the shell answer every kind. A `RequestListener` added
with `WindowState.get().addListener(listener)` hears the request first, and `request.handle()` answers it: the stock
answer does not run. The kinds and the `WindowState` policy are in [Windows](/shell-api/windows#window-requests).

## Listen to membership

A `ClientCollectionListener` on `gnoblin.clients` hears windows join and leave, and an `OutputCollectionListener` on
`gnoblin.outputs` does the same for outputs:

- `onAdded(member, collection)` runs when a window or an output joins.
- `onRemoved(member, collection)` runs when one leaves.

`gnoblin.workspaces` has `added` and `removed` events for workspaces: `gnoblin.workspaces.on("added", handler)`.

```ts
import { ClientCollectionListener, gnoblin } from "gnoblin-core";
import { Toast } from "gnoblin-std";
import type { Client } from "gnoblin-core";

class Count extends ClientCollectionListener {
    onAdded(client: Client) {
        Toast.show(`${client.appId} opened: ${gnoblin.clients.length} windows`);
    }
}

gnoblin.clients.addListener(new Count());
```

Each is also an event of the collection: `gnoblin.clients.on("added", handler)` runs the handler for every window there is
now and every one that joins, and `gnoblin.onClientCreated(handler)` is the same for windows from `gnoblin`.

```ts
gnoblin.onClientCreated((client) => {
    gnoblin.spawn(["logger", "-t", "windows", `${client.appId} appeared`]);
});
```

The shell reads what the compositor published a batch at a time:

- A window that opens and closes inside one batch is gone when the batch is read. The shell is told of it neither way: no
  `onAdded` or `clientCreated`, and no `onClosed`.
- A window the shell has been told of is always told closed later.
- A script that acts on a window it has just opened, such as closing it, listens for the window to appear first and acts
  from there.

## Listen to outputs

An `OutputCollectionListener` attached to `gnoblin.outputs`, or an `OutputListener` attached to one output, has these
methods:

| Method                                | Called when                                                                |
| ------------------------------------- | -------------------------------------------------------------------------- |
| `onConnected(output)`                 | An output was plugged in while the shell runs.                             |
| `onDisconnected(output)`              | An output is gone. Its last values stay readable.                          |
| `onModeChanged(output, mode)`         | The mode changed. `mode` is `WxH@Hz`.                                      |
| `onScaleChanged(output, scale)`       | The scale changed.                                                         |
| `onGeometryChanged(output, geometry)` | The output's position or size changed.                                     |
| `onApplied(output, error)`            | An `apply` or `mirror` was answered. `error` is empty when it took effect. |

A listener on the collection does not hear `onConnected` for an output that was there when the shell started. See
[Outputs](/shell-api/outputs#listen-for-plugging-and-unplugging).

Each method is also an event that `on` takes with just the handler:

- `output.on("modeChanged", handler)` hears one output, and `gnoblin.outputs.on("modeChanged", handler)` hears every output.
- `gnoblin` has them as `gnoblin.onOutputModeChanged`, `gnoblin.onOutputConnected` and so on.
- `gnoblin.onOutputAdded` and `gnoblin.onOutputRemoved` follow the membership of `gnoblin.outputs`.

`onAdded` runs for the outputs there are now as well as for each that joins. `onConnected` runs only for an output that is
plugged in later.

```ts
gnoblin.onOutputConnected((output) => {
    gnoblin.spawn(["notify-send", `${output.connector} connected`]);
});
```

## Signals the compositor raises

These are the compositor's own signals behind the listeners above. A script hears them through the listener methods, which
receive the object itself; the signals carry object identities.

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
- `closed` is raised by the object that is gone, so its `object` field says which one.

`gnoblin.window.changed` does not report a focus change. A focus change reaches `ClientListener.onFocusChanged`.
