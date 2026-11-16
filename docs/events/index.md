# Events

A script hears the compositor through listeners. A listener is a class with optional `on…` methods, attached to the
object the event is about. Its methods receive that object first and the event's own values after it.

```ts
import { SessionListener } from "gnoblin-core";
import { toast } from "gnoblin-std";

class Bell extends SessionListener {
    onBell(session, count) {
        toast(`Bell number ${count}`);
    }
}

gnoblin.session.addListener(new Bell());
```

Put the file in `~/.config/gnoblin/shell.ts`. Ring a terminal bell and a message appears in the top right corner. A reload
detaches every listener the old file attached, so the new file attaches its own.

## The listeners

| Listener             | Attach it with                                               | It hears                                                     |
| -------------------- | ------------------------------------------------------------ | ------------------------------------------------------------ |
| `ClientListener`     | `client.addListener(l)`, or `gnoblin.clients.addListener(l)` | A window: title, focus, geometry, pointer, close, requests.  |
| `CollectionListener` | `gnoblin.clients.addListener(l)`, and the other collections  | An object joining or leaving a collection.                   |
| `OutputListener`     | `output.addListener(l)`, or `gnoblin.outputs.addListener(l)` | An output: connected, mode, scale, applied.                  |
| `SessionListener`    | `gnoblin.session.addListener(l)`                             | The session: ready, bell, reload asked for, lock state.      |
| `RuntimeListener`    | `gnoblin.runtime.addListener(l)`                             | A configuration reload that was accepted or refused.         |
| `ProcessListener`    | `gnoblin.process.addListener(l)`                             | A process that ended. See [Processes](/shell-api/processes). |

- Every method of a listener is optional.
- Each method of a listener is also a named subscription, which [Subscribe to one event](#subscribe-to-one-event) describes.
- A listener added twice is called once, and `removeListener` detaches it.
- A method runs as a queued call under the same time budget as every other listener.
- A method that throws is reported once in the log, and the next listener still runs.
- A listener that is not an object, or has an `on…` member that is not a function, throws a `TypeError` when it is added.

## Subscribe to one event

Every method of `SessionListener`, `RuntimeListener`, `ClientListener`, `CollectionListener` and `OutputListener` is also a
named subscription: a function on the objects that take that listener, which takes just the handler and returns the
function that detaches it. The handler receives what the method receives.

```ts
const stop = gnoblin.session.onBell((session, count) => {
    gnoblin.spawn(["notify-send", `Bell number ${count}`]);
});

gnoblin.clients.onTitleChanged((client, title) => {
    gnoblin.spawn(["logger", "-t", "titles", `${client.appId}: ${title}`]);
});

stop(); // the bell is not heard any more
```

Put the file in `~/.config/gnoblin/shell.ts`. The first handler runs for every bell until `stop()` is called, and the second
for every title change of every window.

| Object                             | Subscriptions                                                                            |
| ---------------------------------- | ---------------------------------------------------------------------------------------- |
| `gnoblin.session`                  | The methods of `SessionListener`: `onReady`, `onBell`, `onReloadRequested`, ...          |
| `gnoblin.runtime`                  | The methods of `RuntimeListener`: `onConfigReloaded` and `onConfigReloadFailed`.         |
| a `Client`                         | The methods of `ClientListener`: `client.onTitleChanged(handler)`.                       |
| an `Output`                        | The methods of `OutputListener`: `output.onModeChanged(handler)`.                        |
| a collection                       | `onAdded` and `onRemoved`, and the methods of the listener of its members.               |
| `gnoblin`, for windows and outputs | `onClientCreated`, an `onClient<Event>` and an `onOutput<Event>` for each method, below. |

`gnoblin` forwards a subscription to `gnoblin.clients` or to `gnoblin.outputs`, so a script that watches windows does not
reach for the collection first:

```ts
gnoblin.onClientCreated((client) => {
    gnoblin.spawn(["logger", "-t", "windows", `${client.appId} appeared`]);
});
```

- `gnoblin.onClientCreated(handler)` runs for each window that appears, and once for each window that is there when it is
  called, as `onAdded` on a collection does.
- An `onClient<Event>` function exists on `gnoblin` for each method of `ClientListener`: `gnoblin.onClientTitleChanged`,
  `gnoblin.onClientFocusChanged`, `gnoblin.onClientClosed`, `gnoblin.onClientReady` and the rest. It hears every window.
- `gnoblin.onOutputAdded` and `gnoblin.onOutputRemoved` are the membership of `gnoblin.outputs`, and an `onOutput<Event>`
  function exists on `gnoblin` for each method of `OutputListener`: `gnoblin.onOutputConnected`,
  `gnoblin.onOutputModeChanged` and the rest.

A subscription and `addListener` are one list. A handler and a listener that hear the same event are called in the
order they were attached, with the same arguments.

- A handler that is not a function throws a `TypeError` that names the subscription.
- Calling the function a subscription returned twice is harmless.
- A reload detaches every handler the old file attached, as it does every listener.

## Which events exist

- [Lifecycle events](lifecycle): the session, the configuration, finished operations and dialog requests.
- [Window, workspace and output events](desktop-state): what happens to the things on screen.

Seat chords and gestures have no listener. A shortcut is a binding that runs your function: see
[Keyboard shortcuts](/shell-api/keyboard-shortcuts). The `tree` namespace holds signals about nodes of the compositor's
tree, for the stock library's own use: a config uses listeners and bindings instead.
