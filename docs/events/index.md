# Events

A script hears the compositor in two ways. Attach a listener to a window, a workspace or an output to hear what happens to
it. Call `gnoblin.on` for everything else: the session, a reload, a finished operation, a dialog request.

```ts
import { defineConfig, toast } from "gnoblin";

defineConfig({
    setup: (gnoblin) => {
        gnoblin.on("gnoblin.session.bell", (event) => toast(`Bell number ${event.count}`));
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. Ring a terminal bell and a message appears in the top right corner. A reload
detaches every handler the old file attached, so the new file attaches its own.

## `gnoblin.on`, `once` and `off`

| Call                          | What it does                                                    |
| ----------------------------- | --------------------------------------------------------------- |
| `gnoblin.on(name, handler)`   | Calls `handler(event)` for every signal called `name`.          |
| `gnoblin.once(name, handler)` | The same, for the first signal only.                            |
| `gnoblin.off(subscription)`   | Detaches what `on` or `once` returned.                          |
| `gnoblin.off(name, handler)`  | Detaches that `handler` from every subscription to `name`.      |
| `subscription.unsubscribe()`  | The same as `off(subscription)`. Calling it twice does nothing. |

A name is `namespace.object.signal`, such as `gnoblin.window.created`. Hyphens may stand for underscores, so
`gnoblin.session.reload-requested` names `gnoblin.session.reload_requested`. An unknown name throws a `TypeError` that
lists the near matches. At most 1024 handlers attach at once.

The handler receives one object:

- `name`: the signal's name, with underscores.
- `object`: the numeric identity of the object that raised it.
- The signal's own fields, spelled in camelCase: `not_applied` is `notApplied`.

A field that names an object, such as `window` on `gnoblin.window.created`, holds that object's numeric identity.
To get the object itself, read it from `gnoblin.clients`, `gnoblin.workspaces` or `gnoblin.outputs`, or use a listener,
which receives the object.

Handlers run as queued calls under the same time budget as every listener. A handler that throws is reported once in the
log, and the next handler still runs. The generated `gnoblin.d.ts` types each event by its name.

## Which events exist

- [Lifecycle events](lifecycle): the session, the configuration, finished operations and dialog requests.
- [Window, workspace and output events](desktop-state): what happens to the things on screen, with the listeners that
  receive the objects themselves.

Seat chords and gestures have no event. A shortcut is a binding that runs your function: see
[Keyboard shortcuts](/shell-api/keyboard-shortcuts). The `tree` namespace holds signals about nodes of the compositor's
tree, for the stock library's own use: a config uses listeners and bindings instead.
