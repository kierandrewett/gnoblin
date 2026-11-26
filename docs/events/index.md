# Events

A script hears the compositor through listeners. A listener is a class with optional `on…` methods, attached to the
object the event is about. Its methods receive that object first and the event's own values after it.

```ts
import { SessionListener } from "gnoblin-core";
import type { Session } from "gnoblin-core";
import { Toast } from "gnoblin-std";

class Bell extends SessionListener {
    onBell(_session: Session, count: number) {
        Toast.show(`Bell number ${count}`);
    }
}

gnoblin.session.addListener(new Bell());
```

Put the file in `~/.config/gnoblin/shell.ts`. Ring a terminal bell and a message appears in the top right corner. A reload
detaches every listener the old file attached, so the new file attaches its own.

## The listeners

| Listener                   | Attach it with                                               | It hears                                                     |
| -------------------------- | ------------------------------------------------------------ | ------------------------------------------------------------ |
| `ClientListener`           | `client.addListener(l)`                                      | One window: title, focus, geometry, pointer, close.          |
| `ClientCollectionListener` | `gnoblin.clients.addListener(l)`, and the other window lists | Every window of the collection, and one joining or leaving.  |
| `OutputListener`           | `output.addListener(l)`                                      | One output: connected, mode, scale, applied.                 |
| `OutputCollectionListener` | `gnoblin.outputs.addListener(l)`                             | Every output of the collection, and one joining or leaving.  |
| `SessionListener`          | `gnoblin.session.addListener(l)`                             | The session: ready, bell, reload asked for, lock state.      |
| `RuntimeListener`          | `gnoblin.runtime.addListener(l)`                             | A configuration reload that was accepted or refused.         |
| `ProcessListener`          | `gnoblin.process.addListener(l)`                             | A process that ended. See [Processes](/shell-api/processes). |

- Every method of a listener is optional.
- Each method of a listener is also a named subscription, which [Subscribe to one event](#subscribe-to-one-event) describes.
- A listener added twice is called once, and `removeListener` detaches it.
- A method runs as a queued call under the same time budget as every other listener.
- A method that throws is reported once in the log, and the next listener still runs.
- A listener that is not an object, or has an `on…` member that is not a function, throws a `TypeError` when it is added.

## Subscribe to one event

Every object that takes a listener also has `on(event, handler)`. The event is the name of a listener method without the
`on`, starting with a lower-case letter.

- `onBell` is `"bell"`, and `onTitleChanged` is `"titleChanged"`.
- The handler receives what the method receives.
- `on` returns a `Listener` whose `stop()` detaches the handler.

```ts
const bell = gnoblin.session.on("bell", (session, count) => {
    gnoblin.spawn(["notify-send", `Bell number ${count}`]);
});

gnoblin.clients.on("titleChanged", (client, title) => {
    gnoblin.spawn(["logger", "-t", "titles", `${client.appId}: ${title}`]);
});

bell.stop(); // the bell is not heard any more
```

Put the file in `~/.config/gnoblin/shell.ts`. The first handler runs for every bell until `stop()` is called, and the second
for every title change of every window.

| Object                             | Events                                                                                       |
| ---------------------------------- | -------------------------------------------------------------------------------------------- |
| `gnoblin.session`                  | The methods of `SessionListener`: `ready`, `bell`, `reloadRequested` and `lockStateChanged`. |
| `gnoblin.runtime`                  | The methods of `RuntimeListener`: `configReloaded` and `configReloadFailed`.                 |
| a `Client`                         | The methods of `ClientListener`: `client.on("titleChanged", handler)`.                       |
| an `Output`                        | The methods of `OutputListener`: `output.on("modeChanged", handler)`.                        |
| a collection                       | `added` and `removed`, and the methods of the listener of its members.                       |
| `gnoblin`, for windows and outputs | `clientCreated`, a `client<Event>` and an `output<Event>` for each method, below.            |

Every property of an object has a `<property>Changed` event too: `client.on("maximisedChanged", handler)` hears the window
maximise and unmaximise. The declarations of each class list its events.

`gnoblin` forwards a subscription to `gnoblin.clients` or to `gnoblin.outputs`, so a script that watches windows does not
reach for the collection first. It has a named function for each of its events, such as `gnoblin.onClientCreated(handler)`,
which is the same as `gnoblin.on("clientCreated", handler)`:

```ts
gnoblin.onClientCreated((client) => {
    gnoblin.spawn(["logger", "-t", "windows", `${client.appId} appeared`]);
});
```

- `gnoblin.onClientCreated(handler)` runs for each window that appears, and once for each window that is there when it is
  called, as `added` on a collection does.
- A `client<Event>` event exists on `gnoblin` for each method of `ClientListener`: `clientTitleChanged`,
  `clientFocusChanged`, `clientClosed`, `clientReady` and the rest, with a function of the same name that starts with
  `on`, such as `gnoblin.onClientTitleChanged`. It hears every window.
- `outputAdded` and `outputRemoved` are the membership of `gnoblin.outputs`, and an `output<Event>` event exists for each
  method of `OutputListener`: `outputConnected`, `outputModeChanged` and the rest, with `gnoblin.onOutputConnected` and the
  like.

A subscription and `addListener` are one list. A handler and a listener that hear the same event are called in the
order they were attached, with the same arguments.

- A handler that is not a function throws a `TypeError`.
- Calling `stop()` twice is harmless.
- A reload detaches every handler the old file attached, as it does every listener.

## Which events exist

- [Lifecycle events](lifecycle): the session, the configuration, finished operations and dialog requests.
- [Window, workspace and output events](desktop-state): what happens to the things on screen.

Seat chords and gestures have no listener. A shortcut is a binding that runs your function: see
[Keyboard shortcuts](/shell-api/keyboard-shortcuts). The `tree` namespace holds signals about nodes of the compositor's
tree, for the stock library's own use: a config uses listeners and bindings instead.
