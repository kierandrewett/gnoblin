# Events

A script hears the compositor through listeners. A listener is a class with optional `on…` methods, attached to the
object the event is about. Its methods receive that object first and the event's own values after it.

```ts
import { SessionListener, toast } from "gnoblin";

class Bell extends SessionListener {
    onBell(session, count) {
        toast(`Bell number ${count}`);
    }
}

gnoblin.session.addListener(new Bell());
```

Put the file in `~/.config/gnoblin/init.ts`. Ring a terminal bell and a message appears in the top right corner. A reload
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
- A listener added twice is called once, and `removeListener` detaches it.
- A method runs as a queued call under the same time budget as every other listener.
- A method that throws is reported once in the log, and the next listener still runs.
- A listener that is not an object, or has an `on…` member that is not a function, throws a `TypeError` when it is added.

## Which events exist

- [Lifecycle events](lifecycle): the session, the configuration, finished operations and dialog requests.
- [Window, workspace and output events](desktop-state): what happens to the things on screen.

Seat chords and gestures have no listener. A shortcut is a binding that runs your function: see
[Keyboard shortcuts](/shell-api/keyboard-shortcuts). The `tree` namespace holds signals about nodes of the compositor's
tree, for the stock library's own use: a config uses listeners and bindings instead.
