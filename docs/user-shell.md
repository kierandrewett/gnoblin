# Write your own shell

The stock shell places, focuses and arranges windows, draws the titlebars and the window menu, and binds the keys. It is
TypeScript, and it is the same library you import.

Your `shell.ts` replaces the default: it is the whole configuration, and what it does not say the shell does not do. Run
`gnoblinctl config init` to start from the full default as code you can read and cut down. This page shows how to grow an
`shell.ts` from a few lines into your own shell.

New to the file? Start with the [TypeScript shell](typescript-shell.md), which writes a first `shell.ts`.

## The two packages

The library is two packages:

- `gnoblin-core` is the host bridge and the core objects: `gnoblin`, `Client`, `Output`, collections, listeners, input
  contexts and bindings, `process`, `session`, and the dialog, idle and authentication services.
- `gnoblin-std` is the desktop policy built on it: workspaces, layouts, the move and resize bindings, focus, snapping,
  the window frame and menus, styles, transitions, scenes, the toolkit and XR.

`gnoblin-std` imports `gnoblin-core`, never the reverse. Import from the package a name lives in, or from a submodule
such as `gnoblin-std/layouts/dwindle`. There is no bare `gnoblin` import.

`gnoblin-core` runs on its own. A config that imports only it loads no `gnoblin-std` module. Such a shell can:

- watch windows and outputs, and place windows itself by writing their geometry;
- claim keys, gestures and devices through `gnoblin.input`;
- turn the compositor's own window motion off (`gnoblin.input.primary.builtinMotion`);
- control each window's frame callbacks (`framePolicy`, `frameRate`, `reportPresented`, `presented`);
- start programs.

It has no workspaces, layouts, styles, transitions or window frame, and no lock screen of its own: the compositor's card
stays in charge. Importing `gnoblin-std` adds all of those, and members such as `client.workspace` and
`gnoblin.workspaces`, to the same objects. A config that imports neither is a core-only shell with no workspace.

## Run your own code

The file is a program, and it runs when the shell loads it. The `gnoblin` object is a global, and every statement reads or
changes a live object.

```ts
import { ClientListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

class FocusLog extends ClientListener {
    onFocusChanged(client: Client, focused: boolean) {
        if (focused) {
            gnoblin.spawn(["logger", "-t", "focus", client.appId]);
        }
    }
}

gnoblin.clients.addListener(new FocusLog());
```

Save it as `~/.config/gnoblin/shell.ts`. Focus a window, then read the line with `journalctl --user -t focus -n 1`.

A listener is a class whose methods the shell calls when something happens: [Events](/events/desktop-state) lists them. The
objects you read and change are in the [runtime API](/shell-api/).

## Split it into modules

A config grows past one file. Put code in other files under the config directory and import them.

```ts
// ~/.config/gnoblin/focus-log.ts
import { ClientListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

export class FocusLog extends ClientListener {
    onFocusChanged(client: Client, focused: boolean) {
        if (focused) {
            gnoblin.spawn(["logger", "-t", "focus", client.appId]);
        }
    }
}
```

```ts
// ~/.config/gnoblin/shell.ts
import { FocusLog } from "./focus-log.ts";

gnoblin.clients.addListener(new FocusLog());
```

- An import is a relative path (`./x.ts`, `../x.ts`, or `./x` with no extension), `"gnoblin-core"` or `"gnoblin-std"`, or a path
  under one of them. There are no packages to install and no glob imports.
- A file is `.ts`, `.mts`, `.js` or `.mjs`. The shell strips the types as it loads a TypeScript file, so there is no build
  step. Write `./x.js` and it falls back to `./x.ts` when only that exists.
- Imports stay inside the config directory. A path that climbs out of it, or a symlink that points out, is refused.
- A module that must run, with nothing to export, is `import "./rules.ts";`.
- `import { ... } from "gnoblin-core"` gives you the classes. The `gnoblin` object is also a global, so a module needs no import to
  use it.

## Save and see it

Saving `shell.ts`, or any file it imports, reloads the shell after a short pause. The windows you have open stay where they
are, and the new code is told about each of them. `gnoblinctl config reload` reloads on request and prints the revision it
committed, or the error with its file, line and column.

A file that fails to load leaves the running config in place. A syntax error, an import that does not resolve, a throw and
a script that runs away are each reported once as `file:line:column: message`, in your `.ts` coordinates.

Check a file before you save it:

```sh
gnoblinctl check --types ~/.config/gnoblin/shell.ts
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

| Entry                                   | Limit      | When it goes past                                    |
| --------------------------------------- | ---------- | ---------------------------------------------------- |
| Loading the file and its imports        | 30 seconds | The load fails and the old config keeps running.     |
| One listener, handler, binding or timer | 1 second   | It is stopped and reported. The frame goes on.       |
| One frame, all its handlers together    | 5 seconds  | The call that passes it is interrupted and reported. |
| Memory of the config's script engine    | 64 MiB     | The allocation fails and the call is reported.       |

A handler that throws is reported once by name and with its line, and the other handlers still run. After several stops in
a short time the shell stops starting the config again until a cool-down has passed or you reload on purpose. The
compositor keeps running whatever your code does.

## Where to change what

| To change                                          | Go to                                                                                                |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Where windows open, by app or title                | [Window policy](/js-window-policy), a listener on `gnoblin.clients`.                                 |
| How windows are arranged                           | [Workspaces](/shell-api/workspaces#layouts), the `layout` of each workspace.                         |
| Keys, pointer drags and gestures                   | [Keyboard shortcuts](/shell-api/keyboard-shortcuts) and [Input handlers](/shell-api/input-handlers). |
| The window menu and the not-responding alert       | [Window menu and alerts](/guides/stock-window-menu).                                                 |
| The consent, passphrase and authentication dialogs | [Requests and dialogs](/shell-api/requests).                                                         |
| Titlebars, bars and your own interface             | [UI toolkit](/js-ui).                                                                                |
| Borders, shadows and blur                          | [Window style](/js-style).                                                                           |
