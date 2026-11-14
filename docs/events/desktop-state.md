# Window, workspace and output events

Windows, workspaces and outputs are objects in `gnoblin.clients`, `gnoblin.workspaces` and `gnoblin.outputs`. A listener
attached to one of them is told what happens to it and receives the object itself. Use a listener for these;
`gnoblin.on` also raises the matching signals, with object identities in place of objects.

## Listen to windows

Attach a `ClientListener` to `gnoblin.clients` to hear every window, or to one window's `addListener` to hear that one.
Every method is optional.

```ts
import { ClientListener, gnoblin } from "gnoblin";
import type { Client } from "gnoblin";

class Titles extends ClientListener {
    onTitleChanged(client: Client, title: string) {
        gnoblin.spawn(["logger", "-t", "titles", `${client.appId}: ${title}`]);
    }
    onClosed(client: Client) {
        gnoblin.spawn(["logger", "-t", "titles", `${client.appId} closed`]);
    }
}

gnoblin.clients.addListener(new Titles());
```

Save it as `init.ts`. Open a terminal and read the lines with `journalctl --user -t titles -f`.

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
| `onRequest(client, request)`          | The client asked for a change of its window. See below.                     |

A pointer `event` has `x` and `y` in the window's own coordinates and `button`, which is 0 for every event but a press and
a release.

### Answer a request yourself

A client asks the compositor to maximise, fullscreen, minimise, close, raise or move its window, and the compositor does
nothing until the shell answers. The stock shell answers every kind. A listener's `onRequest` hears the request first,
and `request.handle()` answers it: the stock answer does not run. The kinds and the `windowState` policy are in
[Windows](/shell-api/windows#window-requests).

## Listen to membership

A `CollectionListener` on a collection hears members join and leave:

- `onAdded(member)` runs when a window, workspace or output joins.
- `onRemoved(member)` runs when one leaves.

```ts
import { CollectionListener, gnoblin, toast } from "gnoblin";
import type { Client } from "gnoblin";

class Count extends CollectionListener<Client> {
    onAdded(client: Client) {
        toast(`${client.appId} opened: ${gnoblin.clients.length} windows`);
    }
}

gnoblin.clients.addListener(new Count());
```

## Listen to outputs

An `OutputListener` attached to `gnoblin.outputs`, or to one output, has these methods:

| Method                                | Called when                                       |
| ------------------------------------- | ------------------------------------------------- |
| `onConnected(output)`                 | An output was plugged in while the shell runs.    |
| `onDisconnected(output)`              | An output is gone. Its last values stay readable. |
| `onModeChanged(output, mode)`         | The mode changed. `mode` is `WxH@Hz`.             |
| `onScaleChanged(output, scale)`       | The scale changed.                                |
| `onGeometryChanged(output, geometry)` | The output's position or size changed.            |

A listener on the collection does not hear `onConnected` for an output that was there when the shell started. See
[Outputs](/shell-api/outputs#listen-for-plugging-and-unplugging).

## Signals of `gnoblin.on`

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
