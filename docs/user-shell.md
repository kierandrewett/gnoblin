# Write your own shell

The stock shell places, focuses and arranges windows, draws the titlebars and the window menu, and binds the keys. It is
written against the same objects you import, so your own shell can do all of it and more.

A shell is a program in one of three languages, and the three reach the same objects:

| Language   | You write                                 | Start with                                                                    |
| ---------- | ----------------------------------------- | ----------------------------------------------------------------------------- |
| TypeScript | `shell.ts`, run as it is, no build step   | The [TypeScript shell](typescript-shell.md), which writes a first `shell.ts`. |
| Python     | `shell.py`, run as it is, no build step   | The [Python shell](python-shell.md), which writes a first `shell.py`.         |
| Rust       | A library crate that you build with Cargo | The [Rust shell](rust-shell.md), which builds a first `libmyshell.so`.        |

Your shell replaces the default: it is the whole configuration, and what it does not say the shell does not do. Run
`gnoblinctl config init` (add `--language python` or `--language rust` for those) to start from the full default as code
you can read and cut down. This page shows how to grow a shell from a few lines into your own, in whichever language you
picked.

## The two libraries

The library is two parts, in every language:

- `gnoblin-core` is the host bridge and the core objects: `gnoblin`, `Client`, `Output`, collections, listeners, input
  contexts and bindings, `process`, `session`, and the dialog, idle and authentication services. It is the package
  `gnoblin-core` in TypeScript, the module `gnoblin.objects.model` in Python and the crate `gnoblin-core` in Rust.
- `gnoblin-std` is the desktop policy built on it: workspaces, layouts, the move and resize bindings, focus, snapping,
  the window frame and menus, styles, transitions, scenes, the toolkit and XR. It is the package `gnoblin-std`, the module
  `gnoblin.objects.std` and the crate `gnoblin-std`.

`gnoblin-std` builds on `gnoblin-core`, never the reverse. Import from the part a name lives in.

`gnoblin-core` runs on its own. A shell that uses only it loads no `gnoblin-std` code. Such a shell can:

- watch windows and outputs, and place windows itself by writing their geometry;
- claim keys, gestures and devices through `gnoblin.input`;
- turn the compositor's own window motion off (`gnoblin.input.primary.builtinMotion`, `builtin_motion` in Python and Rust);
- control each window's frame callbacks (`framePolicy`, `frameRate`, `reportPresented`, `presented`);
- start programs.

It has no workspaces, layouts, styles, transitions or window frame, and no lock screen of its own: the compositor's card
stays in charge. Using `gnoblin-std` adds all of those, and members such as `client.workspace` and
`gnoblin.workspaces`, to the same objects. A shell that uses neither is a core-only shell with no workspace.

## Run your own code

The file is a program, and it runs when the shell loads it. Every statement reads or changes a live object. This shell
logs the application of each window that takes the keyboard:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";

gnoblin.clients.on("focusChanged", (client, focused) => {
    if (focused) {
        gnoblin.spawn(["logger", "-t", "focus", client.appId]);
    }
});
```

```python
from gnoblin.objects.model import Client, Gnoblin

gnoblin = Gnoblin.get()


def focused(client: Client, has_focus: bool) -> None:
    if has_focus:
        gnoblin.spawn(["logger", "-t", "focus", client.app_id])


gnoblin.clients.on("focus_changed", focused)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let clients = model.clients()?;
    let shell = Rc::clone(&model);
    clients
        .on_focus_changed(move |client, focused| {
            if focused {
                if let Ok(app_id) = client.app_id() {
                    let argv = ["logger".to_owned(), "-t".to_owned(), "focus".to_owned(), app_id];
                    let _ = shell.spawn(&argv, None);
                }
            }
        })?
        .keep();
    Ok(())
}
```

:::

Save the TypeScript as `~/.config/gnoblin/shell.ts` or the Python as `~/.config/gnoblin/shell.py`, or build the Rust and
run `gnoblin ./target/debug/libmyshell.so`. Focus a window, then read the line with `journalctl --user -t focus -n 1`.

`on(event, handler)` is one way to hear something. The other is a listener: an object whose methods the shell calls when
something happens. TypeScript and Python extend `ClientCollectionListener` and define the methods they want, and Rust fills
in `ClientCollectionListenerHandlers`. [Events](/events/desktop-state) lists them. The objects you read and change are in the
[runtime API](/shell-api/).

## Split it into modules

A shell grows past one file. Put code in other files and use them from the shell file. The three languages do it their own way.

**TypeScript.** Put code in other files under the config directory and import them:

```ts
// ~/.config/gnoblin/focus-log.ts
import type { Client } from "gnoblin-core";

export function logFocus(client: Client, focused: boolean) {
    if (focused) {
        gnoblin.spawn(["logger", "-t", "focus", client.appId]);
    }
}
```

```ts
// ~/.config/gnoblin/shell.ts
import { logFocus } from "./focus-log.ts";

gnoblin.clients.on("focusChanged", logFocus);
```

- An import is a relative path (`./x.ts`, `../x.ts`, or `./x` with no extension), `"gnoblin-core"` or `"gnoblin-std"`, or a path
  under one of them. There are no packages to install and no glob imports.
- A file is `.ts`, `.mts`, `.js` or `.mjs`. The shell strips the types as it loads a TypeScript file, so there is no build
  step. Write `./x.js` and it falls back to `./x.ts` when only that exists.
- Imports stay inside the config directory. A path that climbs out of it, or a symlink that points out, is refused.
- A module that must run, with nothing to export, is `import "./rules.ts";`.
- `import { ... } from "gnoblin-core"` gives you the classes. The `gnoblin` object is also a global, so a module needs no import to
  use it.

**Python.** Put modules beside the script and import them by name:

```python
# ~/.config/gnoblin/focus_log.py
from gnoblin.objects.model import Client, Gnoblin


def log_focus(client: Client, has_focus: bool) -> None:
    if has_focus:
        Gnoblin.get().spawn(["logger", "-t", "focus", client.app_id])
```

```python
# ~/.config/gnoblin/shell.py
from focus_log import log_focus
from gnoblin.objects.model import Gnoblin

Gnoblin.get().clients.on("focus_changed", log_focus)
```

```rust
// src/focus_log.rs
use std::rc::Rc;

use gnoblin_core::objects::{Client, ModelRoot};

pub fn log_focus(shell: &Rc<ModelRoot>, client: Client, has_focus: bool) {
    if has_focus {
        if let Ok(app_id) = client.app_id() {
            let argv = ["logger".to_owned(), "-t".to_owned(), "focus".to_owned(), app_id];
            let _ = shell.spawn(&argv, None);
        }
    }
}
```

```rust
// src/lib.rs
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

mod focus_log;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let clients = model.clients()?;
    let shell = Rc::clone(&model);
    clients
        .on_focus_changed(move |client, focused| focus_log::log_focus(&shell, client, focused))?
        .keep();
    Ok(())
}
```

- A module beside the script, or in a package directory beside it, is read again from its source at every reload, so an edit
  of `focus_log.py` takes effect without a restart.
- Libraries installed for your Python import as usual, and modules from `site-packages` are shared by every generation.
- The `gnoblin` package needs no install: the runtime serves it.

**Rust.** Put code in modules of the library crate, with `mod focus_log;` in `lib.rs` and a file `src/focus_log.rs`.
Everything is compiled into the one `cdylib` that the loader runs, so there is nothing to import at run time.

Another crate is an ordinary Cargo dependency; the SDK crates themselves are `path` or `git` dependencies. See
[Rust shell](rust-shell.md#imports-and-libraries).

## Examples

Small shells in `examples/` show the pieces, and the tests in `crates/gnoblin/tests/modular/` run each one as it is in a
real session (`gnoblin ./shell.js`, `gnoblin ./shell.ts`, `gnoblin ./shell.py`, `gnoblin ./target/debug/libnative_object.so`),
with a Wayland client that stays connected:

- `examples/04-js-esm` is a JavaScript shell of ES modules. It imports `gnoblin-core` and `gnoblin-std`, adds one workspace
  with the stock master-stack layout, and binds its launcher keys in a module of its own (`./keys.js`).
- `examples/05-ts-types` is a TypeScript shell of three files: the types, the geometry and the shell. The types are erased
  when each file loads, and an error is reported at the line of the `.ts` file that wrote it, not at the stripped
  JavaScript.
- `examples/07-script-binding` is a custom input binding written in TypeScript (`shell.ts`) and in Python (`shell.py`): a
  key binding that moves a window when its chord is pressed.
- `examples/03-typescript-std`, `examples/03-python-std` and `examples/06-rust-layout` are the same idea in each language: a
  workspace with a layout of your own.
- `examples/20-native-object` to `examples/39-native-errors` are one directory each, with a Rust crate, a `shell.ts` and a
  `shell.py` that log the same lines, so you can compare the three line by line.

## Save and see it

Saving the shell file, or any file it imports, reloads a TypeScript or Python shell after a short pause. A Rust shell
reloads when the library file is rebuilt and `gnoblin --watch` is running. The windows you have open stay where they are,
and the new code is told about each of them.

`gnoblinctl config reload` or `gnoblinctl shell reload` reloads on request and prints the revision it committed, or the
error with its file, line and column.

A file that fails to load leaves the running config in place. A syntax error, an import that does not resolve, a throw and
a script that runs away are each reported once as `file:line:column: message`, in your own coordinates.

Check a file before you save it:

```sh
gnoblinctl check --types ~/.config/gnoblin/shell.ts
gnoblinctl check ~/.config/gnoblin/shell.py
gnoblinctl check ./target/debug/libmyshell.so
```

`--types` runs a TypeScript compiler and is for `.ts` files. Python's types are checked by pointing `pyright` at the stubs
(see [Python shell](python-shell.md#types-and-editor-support)), and Rust's by `cargo check`.

## Try things live

In a TypeScript or JavaScript shell, `gnoblinctl console` is a prompt in the running shell. It sees the `gnoblin` object and
the windows as they are now, so you can read `gnoblin.clients.focused?.title` or call a method before you write it into a
file. See [Console and inspector](/js-console).

The console is a JavaScript tool: a Python or Rust shell has none. The commands `gnoblinctl OBJECT list`, `get`, `watch`
and `call` read and change the same objects from a terminal in any language.

## What your code can reach

What your code can use depends on the language it runs in.

- **TypeScript** runs in the shell's own JavaScript engine, not in a browser or Node. It has the language's objects: `Date`,
  `JSON`, `RegExp`, `Map`, `Set`, `Promise`, typed arrays and the rest of ECMAScript, and `console`, which writes to the
  shell's log: see [Debugging](/typescript-shell#debugging). It has no files, no network, no `setTimeout` and no packages.
- **Python** runs in a system CPython 3.10 or newer. It has the standard library and any package installed for that Python,
  and `print` and `logging` write to the shell's log. Threads you start may only log and call `post`: see
  [Python shell](python-shell.md#threads).
- **Rust** is native code in a library of your own, so it has the whole of Rust and its crates. A `Gnoblin` is not `Send`:
  hand work to the shell thread with a `Poster`. See [Rust shell](rust-shell.md#what-the-project-needs).

In every language, start a program with `gnoblin.spawn`, wait with `gnoblin.timers.after`, and show a message with
`Toast.show(text)`. Output of a program you start goes where its `stdout` option says: see
[Processes](/shell-api/processes).

## Time and memory limits

Every call into a TypeScript or JavaScript shell has a time budget, and the first limit reached stops it. A Python or Rust
callback has no such budget: one that never returns holds the shell thread, which costs the shell its own events and
nothing else, because the compositor keeps running whatever your code does.

| Entry                                   | Limit      | When it goes past                                    |
| --------------------------------------- | ---------- | ---------------------------------------------------- |
| Loading the file and its imports        | 30 seconds | The load fails and the old config keeps running.     |
| One listener, handler, binding or timer | 1 second   | It is stopped and reported. The frame goes on.       |
| One frame, all its handlers together    | 5 seconds  | The call that passes it is interrupted and reported. |
| Memory of the config's script engine    | 64 MiB     | The allocation fails and the call is reported.       |

A handler that throws is reported once by name and with its line, and the other handlers still run. After several stops in
a short time the shell stops starting the config again until a cool-down has passed or you reload on purpose.

A Python callback that raises and a Rust callback that panics or returns an error are reported the same way, and the next
event is delivered.

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
