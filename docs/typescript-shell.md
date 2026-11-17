# TypeScript shell

Configure the shell with one TypeScript file, `shell.ts`. You install nothing and run no build step: the shell reads the
file as it is, and reloads it when you save.

The session runs this shell unless you pick another: see [Where the config lives](#where-the-config-lives). A directory with only an `init.lua` runs the stock config (see [Which shell runs](#which-shell-runs)).

## Write your first config

A `shell.ts` is a program. It runs top to bottom, and each line reads or changes an object of the shell. This one is
short:

```ts
import { MasterStackLayout, Workspace, startDecorations } from "gnoblin-std";

startDecorations();

const code = new Workspace({ id: "code", name: "Code", layout: new MasterStackLayout({ ratio: 0.6, gap: 12 }) });
gnoblin.workspaces.add(code);

gnoblin.clients.style.cornerRadius = 12;
gnoblin.clients.where({ appId: "firefox" }).style.cornerRadius = 18;

const input = gnoblin.input.primary;
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
input.bind("Super+q", () => {
    input.focusedClient?.close();
});
```

The file you write is the whole configuration. Nothing is layered under it, so a file with only the lines above has one
workspace, no volume keys and no launcher. Run this once to start from the full default instead:

```sh
gnoblinctl config init
```

It writes a commented `shell.ts` to `~/.config/gnoblin` (`$XDG_CONFIG_HOME/gnoblin` when that is set) and the files your
editor needs for completion:

```text
ok  shell.ts written: /home/me/.config/gnoblin/shell.ts
ok  library refreshed: /home/me/.config/gnoblin/.gnoblin/lib/gnoblin (200 files, the real sources of `gnoblin`)
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/.gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json

Next:
  1. Open /home/me/.config/gnoblin/shell.ts in your editor.
  2. Check it with `gnoblinctl check --types /home/me/.config/gnoblin/shell.ts`.
  3. Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request.
```

`config init` never replaces a config. It stops and says so when a shell file is already there, because two shell
files in one directory leave Gnoblin with no way to choose.

- A `shell.ts`, `shell.js` or any other `shell.*` file stops it. Add `--force` to write the starter over `shell.ts`.
- An `init.lua` does not stop it: the new `shell.ts` runs, and nothing reads the old file.

Open the file. As written it does exactly what the shell does when there is no `shell.ts`, so nothing changes until you
edit it. The lines that start with `// ` are examples. Remove the `// ` to switch one on:

```ts
gnoblin.workspaces.add(new Workspace({ id: "work", name: "Work", layout: new MasterStackLayout() }));
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

Save the file. The shell reloads it after a short pause, and the windows you have open stay where they are. Press
`Super+2` and you are on the next workspace. `gnoblinctl check --types shell.ts` checks the file before you save it, and
`gnoblinctl console` opens a live prompt where you can try any line first.

## Where the config lives

Gnoblin runs one shell, chosen once at start from seven places in order. For a TypeScript shell the ones you use are:

1. **The command line:** `gnoblin ./shell.ts` runs that file, and `gnoblin --shell tiler` runs the installed shell called
   `tiler`.
2. **`GNOBLIN_SHELL`:** a value with a `/` is a path (relative to the directory you start Gnoblin from), anything else is the
   name of an installed shell.
3. **`config.toml`:** `[shell] path = "~/dev/tiler/shell.ts"` names a file, and `[shell] default = "tiler"` names an
   installed shell. Set one of them, not both.
4. **A file in the config directory:** `shell.ts`, `shell.mts`, `shell.js` or `shell.mjs` in `~/.config/gnoblin` (or
   `$XDG_CONFIG_HOME/gnoblin` when that is an absolute path). `shell.py` and `shell.so` are found the same way, for a
   [Python](python-shell.md) and a [Rust](rust-shell.md) shell.
5. **Installed for your user:** a file such as `tiler.ts`, a directory that holds a `shell.ts`, or a launcher file
   `tiler.toml`, in `~/.local/share/gnoblin/shells/`.
6. **Installed for the system:** the same, in `gnoblin/shells/` under each `XDG_DATA_DIRS` directory.
7. **The packaged default:** `<prefix>/share/gnoblin/default-shell/shell.ts`, the stock config that a fresh install runs.

The first place that gives exactly one shell wins.

- A shell you named that is missing stops the start.
- Two equal shells in one place stop it with a message that lists them.
- With no shell at all, `gnoblin` says where it looked and exits 1. No compositor starts.

[Start Gnoblin and choose a shell](/starting-gnoblin) has an example for each place, the exact messages, and the
installed-shell file formats. `GNOBLIN_SHELL` with a `.lua` file is refused with the migrate command.

Two commands show the result without starting a session:

```sh
gnoblin --list-shells      # every shell Gnoblin can see; `yes` marks the one that would run
gnoblin --watch            # run the selected shell and reload it when its files change
```

The shell runs the file from top to bottom against the live objects. What the file does not say, the session does not
do: a `shell.ts` replaces the starter entirely. A statement you delete from `shell.ts` is gone after the next reload or
restart.

Programs you start with `gnoblin.spawn` at the top level of the file start when the file loads, once: a reload keeps the
one that runs and stops one you took out of the file. A program a key binding or a listener started is left alone by a
reload. To start something once per session, use your session's own autostart.

A save reloads the file. So does `gnoblinctl config reload`, which prints the revision it committed or the error it
refused, with the file, line and column. `GNOBLIN_JS_WATCH=0` turns the reload on save off, and then only the command
reloads.

## The bytecode cache

The shell keeps what the stock library and your config compile to, so a start loads bytecode instead of compiling it
again: about 6 ms in place of about 40 ms for the stock library and a small `shell.ts`.

- **Where.** `$XDG_CACHE_HOME/gnoblin/js/`, or `~/.cache/gnoblin/js/` without the variable. The files are private to
  you.
- **Why it is never stale.** Each file is named by a hash of the module's text, and the directory it is in by a hash of
  the shell's build: the QuickJS and type stripper versions and, when the runtime is a library, its version string. An edit
  or an upgrade reads nothing old. A file that is damaged is compiled again and replaced.
- **Clearing and turning it off.** Delete the directory, or set `GNOBLIN_JS_CACHE=0`.

## Which shell runs

This build runs TypeScript and JavaScript shells, and [Where the config lives](#where-the-config-lives) says which
one. `gnoblin ./shell.ts` runs a file you name, in the compositor's own process. `GNOBLIN_SHELL_VM` is not read.

### How a script is run

A `.js`, `.mjs`, `.ts` or `.mts` shell is run by the JavaScript runtime. That is a library, `libgnoblin-runtime-js.so`, in
`<libdir>/gnoblin/`, which the shell loader opens when it is asked to run such a file.

The loader finds the library by the file's extension, opens it once, and asks it for a new generation of your file each time
the file loads. The loader is the one every language shares. A script is staged beside the running one, takes over only when
it evaluated cleanly, and logs `generation N active` as a native shell does.

- **The runtime is optional.** Without `libgnoblin-runtime-js.so` a `.ts` file is refused with the library's name and the
  directories searched. `GNOBLIN_RUNTIME_DIR` names a directory to look in first.
- **The engine is in the library.** QuickJS and the type stripper are not part of the `gnoblin` binary, so a session that
  runs no script never loads them.
- **The shell is in the compositor's process.** The runtime reaches the compositor's tree through the core library and keeps
  its own copy of it, as the script runtime always has. A script that hangs is interrupted at its budget, as before.
- **The runtime does what the old `runtime` role did.** It owns the configuration document: what the device objects write is
  committed to the compositor as one revision, and the guardian is told each revision with its `autostart` entries.
  `gnoblinctl config reload` is answered by the generation the reload makes, with the revision it committed or the reason
  it was refused, and a refused reload leaves the revision in force. There is no separate `runtime` process any more.

### The stock library is installed data

`gnoblin-core` and `gnoblin-std` are TypeScript files, installed under `share/gnoblin/js/gnoblin-core/` and
`share/gnoblin/js/gnoblin-std/`. Nothing of them is built into the runtime library. The runtime looks for them, in order, in:

1. the one directory `GNOBLIN_JS_STOCK_DIR` names, when it is set and not empty (to work on the stock library, point it at
   `data/js` of a source tree; a build tree's `cargo` environment does);
2. `gnoblin/js` under `$XDG_DATA_HOME`, then under each directory of `$XDG_DATA_DIRS`;
3. `share/gnoblin/js` (or `data/js` of a source tree) of the prefix the core library and the executable were installed in.

A package is the one of the first directory that holds it. A prefix with no `gnoblin-std` directory runs a shell that
imports only `gnoblin-core`, and refuses one that imports `gnoblin-std` with the text of the loader, which says where it
looked:

```text
gnoblin-std is not installed (looked in: /home/you/.local/share/gnoblin/js, /usr/local/share/gnoblin/js, /usr/share/gnoblin/js)
```

The starter is the packaged default shell, `share/gnoblin/default-shell/shell.ts`: it is what `gnoblin` runs on a fresh
install and what `gnoblinctl config init` writes.

### If you still have an `init.lua`

Gnoblin does not read Lua. When the config directory has an `init.lua` and nothing selects a shell, the session
starts on the stock config and logs one warning:

```text
/home/you/.config/gnoblin/init.lua: this is a Lua config and Gnoblin no longer runs Lua, so the stock desktop runs instead;
run `gnoblinctl config migrate` to convert it to shell.ts (see docs/migrating-from-lua.md)
```

[Migrating from Lua](migrating-from-lua.md) maps each Lua construct to its TypeScript equivalent. Run
`gnoblinctl config migrate` to convert your `init.lua` to a `shell.ts`, or `gnoblinctl config init` to write a starter.
When a `shell.ts` exists, nothing reads the `init.lua` beside it.

## Check a file

```sh
gnoblinctl check --types ~/.config/gnoblin/shell.ts
```

`check` loads the file the way the shell does. `gnoblinctl config validate FILE` runs the same check for a `.ts` or `.js` file. It reports a syntax error, an import that does not resolve or a throw at
the line you wrote.

With `--types` it also runs a TypeScript compiler when one is installed. That catches a key that does not exist or a value
of the wrong type. Without a compiler it says so and exits 0. See [Check and types](gnoblinctl.md#check-and-types).

## Editor support

`gnoblinctl config init` runs `gnoblinctl types init`. It writes:

- the library's own TypeScript source and the generated declarations, into a `.gnoblin` directory beside `shell.ts`,
- a `tsconfig.json` that extends the one inside `.gnoblin`.

An editor that understands TypeScript then completes every key and underlines a wrong one. Go-to-definition on an import
such as `gnoblin-std/scene` opens the file the shell runs.

- Commit `shell.ts` and `tsconfig.json`. Do not commit `.gnoblin`: it holds a copy of the library that the command writes
  again, and it ignores itself in a Git repository.
- Run `gnoblinctl types init` again after an upgrade. It rewrites `.gnoblin` to the release you run and never replaces a
  `tsconfig.json` you have.

See [Editor files](gnoblinctl.md#editor-files) for what is written where.

## What the file can do

Everything is an object in the [shell API](/shell-api/). The ones the starter uses are these:

- `gnoblin.workspaces` holds the workspaces, and each `Workspace` has a layout.
- `gnoblin.input.primary.bind(trigger, fn)` binds a key, a gesture or a pointer chord.
- `gnoblin.clients` is every window, and `gnoblin.clients.style` is how they all look.
- `gnoblin.outputs.profiles` holds the settings of your monitors.
- `startDecorations()`, `startWindowMenu()` and `startNotRespondingAlert()` start the parts of the stock desktop that
  you may want to leave out.

A name that does not exist is a type error under `gnoblinctl check --types`, and a throw at load. A reload that throws
leaves the running file in place.

Some settings of the older configuration have no object in the API, so a file cannot set them. [Settings with no
object](js-settings.md) lists them.

## The native std binding: `gnoblin-std/native`

`import { ... } from "gnoblin-std/native"` is the **generated** binding of `libgnoblin-std.so.1`: the same collections, workspaces,
layouts, bindings, transitions, styles and scene nodes the Rust and Python SDKs have, written by `cargo xtask gen-sdks` from the
library's function table ([the generator](sdk-generator.md)) and calling it through `__gnoblinStd`, which the JavaScript runtime
library installs. A shell that imports it fails at the import, naming `libgnoblin-std.so.1`, when the library is not installed.

```ts
import { EventKind, GridLayout, clients, createWorkspace, insetArea, tile } from "gnoblin-std/native";
import type { Layout, LayoutContext } from "gnoblin-std/native";

class Columns implements Layout {
    compute(cx: LayoutContext) {
        const area = insetArea(cx);
        const width = area.width / Math.max(cx.children.length, 1);
        return cx.children.map((client, i) => tile(client, area.x + i * width, area.y, width, area.height));
    }
}

const main = createWorkspace("main", { layout: new Columns(), gaps: [10, 20] });
clients().listen(EventKind.ADDED, 0, (event) => console.log(`client ${event.member} opened`));

const other = createWorkspace("other");
new GridLayout({ columns: 3 }).attach(other); // a built-in layout: the algorithm is the library's
```

Records use camelCase (`gapInner`). A record you give may leave out the fields it wants zero, and a layout may leave out the
hooks it does not need.

`examples/03-typescript-std/` has the shells the tests run. The hand-written part of the binding is `gnoblin-std/native`
itself (`clients()`, `tile`, `createWorkspace`).

## With no `shell.ts`

A session with no `shell.ts` is not a bare shell. It runs the starter, the same file `gnoblinctl config init`
writes:

- four floating workspaces, `1` to `4` (named `Workspace 1` to `Workspace 4`);
- `Super+1` to `Super+9` show the workspace at that position, sliding over a quarter of a second, and do nothing when
  there is none, so `Super+1` to `Super+4` work until you declare more;
- `Super+Left` and `Super+Right` move focus between the columns of a `StripLayout` workspace;
- a three-finger swipe slides between workspaces, and the windows follow the fingers;
- the volume, microphone and media keys run `wpctl` and `playerctl`;
- a titlebar on every window, with a window menu on a right click;
- an alert with Wait and Force quit for a window that stops answering;
- `Super+space` opens a launcher entry that starts what you type.

A `shell.ts` that fails to load when the shell starts is a failed start: the loader's text names the file, line and column in
your `.ts` coordinates, and the session runs without a shell, on the compositor's own window management, until the file is
fixed. Nothing is lost by waiting and nothing needs the session restarted:

- the guardian logs `the shell failed to start:` and the text, `gnoblinctl shell status` prints `state: failed` with the
  same text, and `gnoblin ./shell.ts` shows it on your terminal;
- saving the file loads it (the watcher of a script covers a file that failed to start too; `GNOBLIN_JS_WATCH=0` turns it
  off), and `gnoblinctl shell reload` loads it on request, exiting 1 with the new text while it still fails.

## What it looks like

A session with no `shell.ts` shows each window under its titlebar on an empty screen. The stock config has no wallpaper
and no bar.

![A terminal under the shell's titlebar on an empty screen](images/js-shell/default-desktop.png)

A workspace with a layout tiles its windows. This file tiles three terminals with a master on the left and a gap of 12
pixels. It calls `startDecorations()` because a file that leaves it out draws no titlebars:

```ts
import { MasterStackLayout, Workspace, startDecorations } from "gnoblin-std";

startDecorations();
gnoblin.workspaces.add(new Workspace({ id: "dev", layout: new MasterStackLayout({ ratio: 0.6, gap: 12 }) }));
```

![Three terminals tiled with a master on the left and a gap of 12 pixels](images/js-shell/master-stack.png)

`DwindleLayout({ gap: 12 })` splits the space of the focused window in two for each new window:

![Three terminals tiled by a dwindle layout](images/js-shell/dwindle.png)

A split goes side by side or top to bottom by the shape of its cell, as in Hyprland's dwindle layout; the options and the
actions that bind to keys are in [Dwindle like Hyprland](/recipes/dwindle-like-hyprland).

A client that draws its own title bar, as `foot` does when decorations are off, keeps it. The window is its window
geometry, the bar included, so a window the shell places at the top-left corner has its bar on the screen and is the
size the shell asked for:

![A terminal that draws its own title bar, at the top-left corner of the screen](images/js-shell/csd-own.png)

The window menu opens on a right click on a titlebar, and with `Alt+Space`:

![The window menu open under a titlebar](images/js-shell/window-menu.png)

## Window requests

A client asks the compositor for a change of its window: fullscreen, minimise, maximise, raise, activate, move, resize.
The compositor changes nothing until the shell answers, and the stock shell answers every kind:

| Request          | What the shell does                                                                                              |
| ---------------- | ---------------------------------------------------------------------------------------------------------------- |
| `maximize`       | Fills the area the bars leave and keeps the floating rectangle. `unmaximize` puts the window back.               |
| `fullscreen`     | Covers the whole output, bars included, and goes on top. `unfullscreen` returns the rectangle.                   |
| `minimize`       | Hides the window and hands the keyboard focus to the topmost window left, or to nobody.                          |
| `unminimize`     | Shows the window, and focuses it only when nothing holds the focus.                                              |
| `activate`       | On the shown workspace: shows a minimised window, focuses it and raises it. On another workspace: nothing.       |
| `raise`, `lower` | Puts the window on top of its siblings or under them. A fullscreen window stays on top.                          |
| `move`, `resize` | Starts the drag of `Super` with the left or right button. A tiled, maximised or fullscreen window is left alone. |
| `close`          | Asks the client to close.                                                                                        |

A fullscreen window hides the bars on its output while it shows on the workspace that output shows, and they return when
it leaves, minimises or closes. A tiling layout leaves a fullscreen or maximised window where it is.

### Answering a request yourself

A `ClientListener` with an `onRequest(client, request)` method hears every request first. Call `request.handle()` to
answer it yourself: the built-in answer does not run, and no later listener hears it.

```ts
import { ClientListener, gnoblin } from "gnoblin-core";
import { windowState } from "gnoblin-std";

class KeepOnTop extends ClientListener {
    onRequest(client, request) {
        if (request.kind === "minimize" && client.appId === "org.example.Clock") {
            request.handle(); // this window never minimises
        } else if (request.kind === "maximize") {
            request.handle();
            windowState.enterFullscreen(client); // answer a maximise as a fullscreen
        }
    }
}
gnoblin.clients.addListener(new KeepOnTop());
```

The listeners attached to the client itself are asked first, then those of the collections that hold it, each in the
order it was attached. The first one to call `request.handle()` wins. A listener that throws is reported and skipped.
`request.kind` is one of:

- `maximize` and `unmaximize`
- `fullscreen` and `unfullscreen`
- `minimize` and `unminimize`
- `close`, `raise`, `lower` and `activate`
- `move`, `resize` and `menu`

`windowState` is the policy the built-in answer calls. Its methods return whether they ran, so a listener can answer a
request with the stock behaviour of another kind:

- `maximise` and `unmaximise`
- `enterFullscreen` and `leaveFullscreen`
- `minimise` and `unminimise`
- `present`, `raise` and `lower`

## Outputs

With more than one output each shows its own workspace, bars sit on one output, a window moves between outputs with
`client.moveToOutput(output)`, and the windows of an unplugged output move to another output or wait hidden for the next.
A dialog stays centred in its parent and goes wherever the parent goes. The page is
[Outputs](/shell-api/outputs).

[Window policy in TypeScript](/js-window-policy).

[Animation in TypeScript](/js-transitions): `gnoblin.requestTransition` draws a change of state as one animation.

## Reloading

Saving `shell.ts`, or any module it imports, reloads the shell after a short pause. `gnoblinctl config reload` reloads it on
request: see [`gnoblinctl config reload`](/gnoblinctl#config). Windows keep their identity through a reload, and the new
config is told about every window that exists.

Your keys keep working through a reload. The shortcuts, touchpad gestures, pointer chords and binding-only devices of the
old config are given up and the new config's are taken in one step, so a key is never refused for being held.

A key you removed from the file stops working at that reload and reaches the focused application again.

A file that fails to load (a syntax error, a throw, a script that runs away) leaves the running config in place. The failure
is logged once as `file:line:column: message`, in your `.ts` coordinates, and `gnoblinctl config reload` exits 1 with the
same text.

Set `GNOBLIN_JS_WATCH=0` to turn the watcher off, for a config directory on a file system that cannot be watched. The shell
then reloads only when asked.

When the JavaScript runtime library runs the file, it watches every file the running generation loaded and asks the shell
loader for a reload when one changes.

The loader evaluates the new generation beside the old one and swaps it in only when it loaded. An edit that does not load
leaves the running shell and reports the file, line and column.

## Debugging

`console` is a global, so `console.log("hi")` works anywhere in your config, in a listener and in the
[console](/js-console). `log` and `info` write at info level, `warn` at warn, `error` at error and `debug` at debug:

```ts
import { ClientListener, gnoblin } from "gnoblin-core";
import type { Client } from "gnoblin-core";

class Trace extends ClientListener {
    onFocusChanged(client: Client, focused: boolean) {
        console.log("%s focus is %j", client.appId, focused);
        if (!focused) console.warn("lost focus:", client.title);
    }
}
gnoblin.clients.addListener(new Trace());
```

Each call is one record in the log, component `js` and field `area=script`, and its message starts with where the call
was, as `shell.ts:6:53` (the file relative to your config directory, the line and column in your `.ts`):

```text
INFO  compositor/js  shell.ts:6:53: foot focus is true  area=script
```

Read it with `gnoblinctl log --follow` (see [Logging](/guides/logging)). The console prints what an expression printed
before the value, so you can try a call at the prompt.

What a call prints:

- Strings print as they are. `%s`, `%d`, `%i`, `%f`, `%j`, `%o`, `%O`, `%c` and `%%` in a first string format the arguments
  after it, as in Node; the arguments that are left follow, separated by spaces.
- An object prints as one line of JSON, with `[Circular]` where it contains itself and `[Function name]` for a function.
- An `Error` prints its message and its stack, with every frame in your `.ts` coordinates.
- `console.assert(condition, ...)` logs at error level, starting with `Assertion failed`, when the condition is false. It
  never throws.
- `console.table(data)` logs the data as one line of JSON. It does not draw a table.

How much a script can print:

- A call logs at most 4 KiB. A longer text is cut and says how many bytes it dropped.
- A generation logs at most 200 calls in a second, so a loop cannot flood the log. The rest are dropped, and when the next
  second begins one line says how many: `801 console lines dropped: a generation logs at most 200 a second`.

The line is the line of the call. The column is the position the engine reports, rounded to the nearest token of your
source: the last argument when it is a name, else the method (`log`). A call that spans lines reports the line it ends on.

## Session, processes and timers

The shell has more than windows:

- `gnoblin.session` logs out, suspends, powers off and locks.
- `gnoblin.process` starts programs and reports how they end.
- `gnoblin.timers` runs a function later, and a `SessionListener` or `RuntimeListener` hears the session and the configuration.
- `gnoblin.find`, `gnoblin.focus.history` and `gnoblin.version()` read what the shell holds.

See [Session and runtime](/shell-api/session-runtime), [Processes](/shell-api/processes) and [Events](/events/).

### Verbs and signals by name

`gnoblin-core/generated/verbs` is the compositor's `gnoblin` namespace as the schema spells it (generated, like the node-class
types), over the core's own `call` and `subscribe`. Each object of the namespace is a member of `verbs`:

```ts
import { verbs } from "gnoblin-core/generated/verbs";

verbs.idle.inhibit("my-shell", (answer) => console.log(answer.error ?? "held"));
verbs.session.onBell((count) => console.log(`the bell has rung ${count} time(s)`));
```

A verb sends the call and returns. The optional answer function runs once on the shell thread with `{ value, error }`:

- `value` is the operation's value parsed from its JSON, or `undefined` for a bare "done".
- `error` is the compositor's error code, or `null`.

A signal method returns a `VerbSubscription` whose `unsubscribe()` removes it, and a reload removes what a generation
registered. A wrong argument is a `TypeError` that names the parameter. The functions need the JavaScript runtime library; a
shell host without it says so at the first call.

## Console and inspector

`gnoblinctl console` opens a live prompt into the running shell, and `inspect.clients()`, `inspect.workspaces()`
and `inspect.bindings()` return what the shell holds as plain data. See
[Console and inspector](/js-console).

## Idle, authentication and launches

`gnoblin.idle.watch(ms, { onIdle, onResume })` runs code when the session has had no input for a while,
`gnoblin.authentication.addListener(listener)` watches polkit authentication requests and may refuse one (the compositor
draws the password dialog and checks the password itself), and
`gnoblin.launches.begin(token, application)` tracks the busy state between a launch and its window. See
[Idle watches](/shell-api/idle), [Authentication prompts](/shell-api/authentication) and [Launch feedback](/shell-api/launch-feedback).

## XR

`gnoblin.extensions.xr` is `null` until the XR program has a session, so a config guards it and works without a headset.
The stock shell builds the room and places the panels when the session starts. See
[XR mode](/xr#scripting-xr-in-typescript) for the room, the viewport and the bindings that move and resize a panel.

## When the shell stops

The shell is a client of the compositor. If it exits or is killed, the compositor keeps running and answers every client.
With no shell holding the tree:

- a window that maps is placed by the compositor's own policy, a cascade from 32, 32;
- windows the shell had moved go back to the places the compositor gave them.

The shell runs on a thread of the compositor's process, so the compositor ends no session for it; the guardian is told, and
the log says why. Starting the session again runs the file again, so what it starts, such as a `gnoblin.spawn` call, starts
again, and no window changes its identity.
