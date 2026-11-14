# Write your own shell

The stock shell places, focuses and arranges windows, draws the titlebars and the window menu, and binds the keys. It is
TypeScript, and it is the same library you import.

Your `init.ts` is laid over it: a key you leave out keeps the stock
behaviour, and code you add runs beside it. This page shows how to grow an `init.ts` from a few settings into your own shell.

New to the file? Start with the [TypeScript shell](typescript-shell.md), which writes a first `init.ts`.

## Run your own code

`defineConfig` takes declarative keys, and a `setup` function for everything they cannot say. `setup` runs when the config
loads, with the `gnoblin` object as its argument.

```ts
import { ClientListener, defineConfig } from "gnoblin";
import type { Client } from "gnoblin";

class FocusLog extends ClientListener {
    onFocusChanged(client: Client, focused: boolean) {
        if (focused) {
            gnoblin.spawn(["logger", "-t", "focus", client.appId]);
        }
    }
}

defineConfig({
    setup: (gnoblin) => gnoblin.clients.addListener(new FocusLog()),
});
```

Save it as `~/.config/gnoblin/init.ts`. Focus a window, then read the line with `journalctl --user -t focus -n 1`.

A listener is a class whose methods the shell calls when something happens: [Events](/events/desktop-state) lists them. The
objects you read and change are in the [runtime API](/shell-api/).

## Split it into modules

A config grows past one file. Put code in other files under the config directory and import them.

```ts
// ~/.config/gnoblin/focus-log.ts
import { ClientListener } from "gnoblin";
import type { Client } from "gnoblin";

export class FocusLog extends ClientListener {
    onFocusChanged(client: Client, focused: boolean) {
        if (focused) {
            gnoblin.spawn(["logger", "-t", "focus", client.appId]);
        }
    }
}
```

```ts
// ~/.config/gnoblin/init.ts
import { defineConfig } from "gnoblin";
import { FocusLog } from "./focus-log.ts";

defineConfig({
    setup: (gnoblin) => gnoblin.clients.addListener(new FocusLog()),
});
```

- An import is a relative path (`./x.ts`, `../x.ts`, or `./x` with no extension) or `"gnoblin"`. There are no packages to
  install and no glob imports.
- A file is `.ts`, `.mts`, `.js` or `.mjs`. The shell strips the types as it loads a TypeScript file, so there is no build
  step. Write `./x.js` and it falls back to `./x.ts` when only that exists.
- Imports stay inside the config directory. A path that climbs out of it, or a symlink that points out, is refused.
- A module that must run, with nothing to export, is `import "./rules.ts";`.
- `import { ... } from "gnoblin"` gives you the classes. The `gnoblin` object is also a global, so a module needs no import to
  use it.

## Save and see it

Saving `init.ts`, or any file it imports, reloads the shell after a short pause. The windows you have open stay where they
are, and the new code is told about each of them. `gnoblinctl config reload` reloads on request and prints the revision it
committed, or the error with its file, line and column.

A file that fails to load leaves the running config in place. A syntax error, an import that does not resolve, a throw and
a script that runs away are each reported once as `file:line:column: message`, in your `.ts` coordinates.

Check a file before you save it:

```sh
gnoblinctl check --types ~/.config/gnoblin/init.ts
```

## Try things live

`gnoblinctl console` is a prompt in the running shell. It sees the `gnoblin` object and the windows as they are now, so you
can read `gnoblin.clients.focused?.title` or call a method before you write it into a file. See
[Console and inspector](/js-console).

## What your code can reach

Your code runs in the shell's own JavaScript engine, not in a browser or Node.

- It has the language's objects: `Date`, `JSON`, `RegExp`, `Map`, `Set`, `Promise`, typed arrays and the rest of ECMAScript.
- It has `console`, which writes to the shell's log: see [Debugging](/typescript-shell#debugging).
- It has no files, no network, no `setTimeout` and no packages. Start a program with `gnoblin.spawn`, wait with
  `gnoblin.timers.after`, and show a message with `toast(text)`.
- Output of a program you start goes where its `stdout` option says: see [Processes](/shell-api/processes).

## Time and memory limits

Every call into your code has a time budget, and the first limit reached stops it.

| Entry                                   | Limit     | When it goes past                                    |
| --------------------------------------- | --------- | ---------------------------------------------------- |
| Loading the file and its imports        | 5 seconds | The load fails and the old config keeps running.     |
| One listener, handler, binding or timer | 100 ms    | It is stopped and reported. The frame goes on.       |
| One frame, all its handlers together    | 500 ms    | The call that passes it is interrupted and reported. |
| Memory of the config's script engine    | 64 MiB    | The allocation fails and the call is reported.       |

A handler that throws is reported once by name and with its line, and the other handlers still run. After several stops in
a short time the shell stops starting the config again until a cool-down has passed or you reload on purpose. The
compositor keeps running whatever your code does.

## Where to change what

| To change                                          | Go to                                                                                                |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Where windows open, by app or title                | [Window rules](/config/window_rule), the `rules` key.                                                |
| How windows are arranged                           | [Workspaces](/shell-api/workspaces#layouts), the `layout` of each workspace.                         |
| Groups of workspaces shown together                | [Workspace sets](/shell-api/workspaces#workspace-sets).                                              |
| Keys, pointer drags and gestures                   | [Keyboard shortcuts](/shell-api/keyboard-shortcuts) and [Input handlers](/shell-api/input-handlers). |
| The window menu and the not-responding alert       | [Window menu and alerts](/guides/stock-window-menu).                                                 |
| The consent, passphrase and authentication dialogs | [Requests and dialogs](/shell-api/requests).                                                         |
| Titlebars, bars and your own interface             | [UI toolkit](/js-ui).                                                                                |
| Borders, shadows and blur                          | [Window style](/js-style).                                                                           |
