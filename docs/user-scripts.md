# Event handlers

Put live compositor behaviour in TypeScript modules that your `init.ts` imports. A handler reacts to something that
happens, such as a window opening, and runs your code.

Create `~/.config/gnoblin/scripts/window-log.ts`:

```ts
import { CollectionListener, gnoblin } from "gnoblin";
import type { Client } from "gnoblin";

class WindowLog extends CollectionListener<Client> {
    onAdded(client: Client) {
        gnoblin.spawn(["logger", "-t", "windows", `${client.appId}: ${client.title}`]);
    }
}

gnoblin.clients.addListener(new WindowLog());
```

Import the module from `~/.config/gnoblin/init.ts`:

```ts
import "./scripts/window-log.ts";
```

Save `init.ts`. The shell reloads on save, or run `gnoblinctl config reload`. Open a window and read the line with
`journalctl --user -t windows -n 1`.

A reload detaches the handlers of the old configuration and attaches the ones your files attach again. A failed reload
leaves the running config in place and reports the error.

## Choose a kind of handler

- A listener, attached to `gnoblin.clients`, `gnoblin.workspaces` or `gnoblin.outputs`, receives the objects themselves.
  Use it for windows, workspaces and outputs: [Window, workspace and output events](/events/desktop-state).
- `gnoblin.on(name, handler)` hears the compositor's other signals, such as a reload or the bell:
  [Events](/events/).
- A key action in `keys` runs when a trigger is pressed: [Keyboard shortcuts](/shell-api/keyboard-shortcuts).
- `gnoblin.timers.after(ms, fn)` runs a function later: [Operations, errors and timers](/shell-api/errors#timers).

## What a handler can do

A handler reads and changes windows, workspaces and outputs through the [runtime API](/shell-api/), and it can ask the
session to lock or log out. It cannot read arbitrary files, open network connections or load native modules. To run a
program, call `gnoblin.spawn`.

Panels, launchers, notifications and other visible interface belong either in the shell's
own [UI toolkit](/js-ui) or in separate Wayland clients that you start with `autostart`.

Keep a handler short. Each call has a time budget, and a call that runs past it is stopped and reported: see
[Write your own shell](/user-shell#time-and-memory-limits).
