# Migrating from Lua

Gnoblin runs the TypeScript shell. It reads `shell.ts` (or `shell.js`) and does not read `init.lua`. The Lua shell,
the Lua API and `GNOBLIN_SHELL_VM=lua` are gone.

## What happens to an init.lua today

- If your config directory has an `init.lua` and no `shell.ts` or `shell.js`, the session starts on the stock config and
  logs one warning that names `gnoblinctl config migrate` and this page. Your `init.lua` is not touched.
- Once `shell.ts` exists, it is the config, and nothing reads the `init.lua` beside it.
- `GNOBLIN_SHELL_VM` is not read.

## Move your config

1. Convert it: `gnoblinctl config migrate`. It reads your `init.lua` and the files it loads, writes `shell.ts` and the
   editor files beside it, and changes nothing else. See [the converter](#the-converter).
2. Review `shell.ts`. Everything the converter could not carry over is a comment that starts with `UNMAPPED`, and the
   command lists each one with its line. Port what has a home by hand using the table below, and read
   [the settings with no TypeScript home yet](#settings-with-no-typescript-home-yet). Then run
   `gnoblinctl check --types shell.ts`: it runs the file the way the shell does and reports a mistake with its line.
3. Log out and back in. `shell.ts` takes precedence over the `init.lua` beside it.

## The converter

`gnoblinctl config migrate [FILE] [-o OUT] [--force] [--dry-run]` turns a Lua config into a TypeScript one. It reads the
Lua as text and never runs it, so it needs no session and no Lua.

```sh
gnoblinctl config migrate
gnoblinctl config migrate --dry-run
gnoblinctl config migrate ~/old/init.lua -o ~/new/shell.ts
```

- `FILE` is the `init.lua` the Lua shell reads. The `shell.lua` beside an `init.lua` is read too.
- It writes `shell.ts` beside `FILE`, or `OUT`. It stops instead of replacing a file, and instead of writing a `shell.ts`
  beside another `shell.*` file, unless you add `--force`. `--force` replaces that one file and nothing else: the Lua
  files are only ever read.
- It runs `gnoblinctl types init` on the directory it wrote to. Before it writes, it runs the result once in the dry-run
  shell, the evaluation `gnoblinctl check` makes, so a file that would not load is reported and not found at the next
  login.
- `--dry-run` prints the TypeScript on standard output and the report on standard error, and writes nothing.

The exit code is 0 when everything converted, 1 when the file was written and something was left out, and 2 when nothing
was written: a wrong command line, a file it could not read, or Lua that does not parse. When something was left out,
the first line of `shell.ts` says so:

```ts
// INCOMPLETE: 19 unmapped settings, see comments
```

See the [gnoblinctl reference](gnoblinctl.md#convert-an-init-lua).

### What the converter writes

The output is statements against the objects of the shell, not a config object. The shell runs the file from top to
bottom, so each statement does what it says. See the [shell API](shell-api/index.md) for each object.

The file starts with the imports it needs, then writes out the stock desktop, then your Lua in the order it was met,
with a comment that names the file each part came from. The stock desktop is written out because a `shell.ts` replaces
the stock default instead of layering over it:

- Four floating workspaces, unless your Lua declares workspaces. Yours then stand in their place.
- A titlebar on every window, the window menu and the not-responding alert.
- `Super+1` to `Super+9`, `Super+Left` and `Super+Right`, the three-finger workspace swipe, the media keys and
  `Super+space`. A key your Lua binds takes the place of the stock key on the same trigger.

Delete a statement and that part of the desktop is gone. The stock desktop is the same statements as the starter that
[`gnoblinctl config init`](gnoblinctl.md#write-a-starter-config) writes.

### What the converter converts

It converts what has an object in the shell, exactly, and each file that `gnoblin.load` names, in the order Lua loaded
them.

| In Lua                                                  | In `shell.ts`                                                                                   |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `workspaces = { { id = "1", name = "One" } }`           | `gnoblin.workspaces.add(new Workspace({ id: "1", name: "One", layout: new FloatingLayout() }))` |
| `shortcuts`, `keybindings` with `binding` and `command` | `input.bind("Super+Return", () => { gnoblin.spawn(["foot"]); })`                                |
| `gnoblin.load("config/*.lua")`                          | The files are read in place, sorted, relative to the file that makes the call.                  |
| `local x = 5 * 100`, `"a" .. "b"`                       | The value, when it is a number, a string, a boolean or a table with only such values.           |
| `seat.data.window_rules = { ... }` in `shell.lua`       | A `where(...).style` or a listener. See [window rules](#window-rules).                          |
| `seat.data.alerts = false` in `shell.lua`               | `startNotRespondingAlert()` is left out of the stock desktop.                                   |

A workspace list replaces the one before it, as it did in Lua, so the last list that was loaded stands. A workspace
with no `layout` is a floating one. `input` is `gnoblin.input.primary`, which the file declares once at the top.

A shortcut converts when its chord has an exact trigger:

- `<Primary>`, `<Ctrl>` and `<Ctl>` are `Ctrl`, `<Mod1>` is `Alt` and `<Shft>` is `Shift`.
- A media key written without its `XF86` prefix gets it back.
- `<Super><Shift>x` is `Super+Shift+x`, and `command = "foot"` is `gnoblin.spawn(["foot"])`.

A shortcut is left out, with the reason, when:

- it acts on key release;
- its chord is on a pointer button, a hardware key code or `Above_Tab`;
- its key is an upper-case letter;
- it binds a trigger that is bound already;
- its command is one string with spaces in it.

`input.bind` refuses a second binding of one trigger, so the stock key on a trigger your Lua binds is not written.

### Window rules

A rule in `shell.lua` has a `match` and the things it does to the windows that match:

| In Lua rule                             | In `shell.ts`                                         |
| --------------------------------------- | ----------------------------------------------------- |
| `match = { app_id = ... }`              | A condition on `client.appId`                         |
| `match = { title = ... }`               | A condition on `client.title`                         |
| `workspace = { id = "3" }`, `"3"`       | `gnoblin.workspaces.get("3")?.clients.add(client)`    |
| `maximised = true`, `fullscreen = true` | `client.maximised = true`, `client.fullscreen = true` |
| `opacity = 0.9`                         | `client.style.opacity = 0.9`                          |

A rule whose only action is opacity, on windows of an exact app id or title, is a style on a selection instead, which
the shell keeps up to date as windows come and go:

```ts
gnoblin.clients.where({ appId: "foot" }).style.opacity = 0.9;
```

Every other rule is one block of a listener attached to `gnoblin.clients`:

```ts
gnoblin.clients.addListener(
    new (class extends CollectionListener {
        onAdded(client: Client): void {
            // shell.lua:2
            if (client.appId === "org.example.Editor") {
                gnoblin.workspaces.get("dev")?.clients.add(client);
                client.maximised = true;
            }
        }
    })(),
);
```

A listener hears a window once, when it is added. A Lua rule ran again whenever the window's title or focus changed, so
a rule that depends on a title set after the window opens may not match.

The rules of one file run in the order Lua listed them, so a later rule wins where two write the same property.

A Lua pattern is searched anywhere in the text. A pattern that is plain text becomes a comparison of the whole text, and
the stars of the Lua anchors say where it may continue. Any other pattern becomes a RegExp, with `s` when it has a `.`
because a Lua `.` matches a newline.

| Lua pattern                              | In the condition                              |
| ---------------------------------------- | --------------------------------------------- |
| `^foot$`                                 | `client.appId === "foot"`                     |
| `^org%.mozilla%.`                        | `client.appId.startsWith("org.mozilla.")`     |
| `Notes$`                                 | `client.title.endsWith("Notes")`              |
| `fire`                                   | `client.title.includes("fire")`               |
| `^org%.x%.(%w+)$`                        | `/^org\.x\.[A-Za-z0-9]+$/.test(client.appId)` |
| `%d+ unread`                             | `/[0-9]+ unread/.test(client.title)`          |
| `Picture.in.Picture`                     | `/Picture.in.Picture/s.test(client.title)`    |
| `%bxy`, `%f[%w]`, `%1`, `%p`, `%c`, `%g` | Not translated, and the rule is left out.     |

These translate:

- The anchors `^` and `$`, `.`, and a `%` before a punctuation character.
- The classes `%a %d %s %w %l %u %x` and their upper-case complements.
- The quantifiers `*`, `+`, `-` and `?`.
- Character sets and ranges. Capture parentheses are dropped, because a rule reads no capture.

Anything else leaves the whole rule out, because a looser match would act on windows you never named. So does:

- a `match` key other than `app_id` and `title`;
- a rule that names a workspace the config does not declare;
- a rule that matches on `focused` or `workspace_id`, which has to be checked again whenever the focus or the workspace
  changes, while a listener hears a window once.

A rule field the converter cannot carry over is left out alone, so the rule still applies without it. `secure` is one:
a `Client` has no property to write it to yet.

A Lua `app_id` pattern also matched the WM class when the app id was empty. `client.appId` does not.

### Settings with no TypeScript home yet

The shell's objects are the whole API, and no object carries these settings, so the converter does not carry them over.
The shell keeps its default for every one.

Each setting is an `UNMAPPED` comment in `shell.ts` and a line in the report that says what it would need. The
[settings page](js-settings.md#dropped-settings) lists the same settings from the TypeScript side.

| Lua setting                                                          | What it would need                                                                                                   |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `autostart`                                                          | A session-scoped startup list                                                                                        |
| `touchpad-gestures`                                                  | A path gesture trigger                                                                                               |
| `workspace_sets` in `shell.lua`                                      | Nothing: the shell has no workspace sets                                                                             |
| `window-management`: `focus-*`, `raise-on-click`, `auto-raise*`      | A focus policy on the seat                                                                                           |
| `window-management`: `edge-tiling`, `constrain-drag-to-work-area`    | A snapping policy on the seat or the layout                                                                          |
| `seat.edge_tiling`, `seat.drag_constraint` in `shell.lua`            | The same snapping policy                                                                                             |
| `keyring`                                                            | A setting read before the session starts                                                                             |
| `dialogs.style`                                                      | A style object on the dialog manager                                                                                 |
| `protocols`                                                          | A compositor-wide settings object                                                                                    |
| `compositor`, `cursor`, `appearance`, `accessibility`, `layer-shell` | A compositor-wide settings object                                                                                    |
| `auth`, `location`, `compat`, `capture`, `monitors`, `xwayland`      | A compositor-wide settings object                                                                                    |
| `logging`, and the other `window-management` settings                | A compositor-wide settings object                                                                                    |
| `input`, `input-sources`                                             | Device settings are `Keyboard`, `Mouse` and `Touchpad` properties; the converter does not translate the Lua sections |

The last line of each `UNMAPPED` comment says the same, so a setting that gains a home can be found by searching
`shell.ts` for the word `UNMAPPED`. A section with no setting in it, such as `cursor = {}`, loses nothing and is not
reported.

An `autostart` entry with `enable = false` started nothing in Lua, so it is a note in the report and not a loss.

### What the converter cannot do

Anything it cannot evaluate without running the file is left out of `shell.ts`, written there as a comment, and listed
with its line:

```ts
// UNMAPPED (init.lua:6): gnoblin.on("gnoblin.window.created", function(w)
//     w:maximise()
// end)
// -> an event handler runs code: write a `ClientListener` and attach it with `gnoblin.clients.addListener`
```

What is left out:

- Every function, `if`, loop, `require`, method call, `seat:bind`, `gnoblin.on` and timer.
- A value that comes from a call, or from an operator the converter does not evaluate, and a name it does not know.
  One computed value in a table costs only its own setting: the settings around it still convert.
- `animations` and `portals`, which the document carries and nothing reads, and a section it does not know.
- `seat.data.prompts`, which has no switch because the compositor draws prompts.
- Comments in your Lua.
- A `gnoblin.load` of a glob outside the config directory.

The shell files, `shell.lua` and `shell/*.lua`, are arbitrary Lua over the object model, and cannot be translated
mechanically. These are the parts that need a hand port:

- `Node:extend`, reactive properties, `go`, `await`, `every` and selectors.
- A subclass of `Workspace`, and `Window:on_ready`.

The converter reads the `seat.data` assignments named above from `shell.lua` and reports every other statement, so a
shell file becomes a list of what to port.

## What each construct becomes

| In Lua                                                                | In TypeScript                                                                                                            |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `shortcuts`, `keybindings`                                            | `input.bind("Super+Return", () => { gnoblin.spawn(["foot"]); })`. A key release is not ported.                           |
| `workspaces = { { id, name } }`                                       | `gnoblin.workspaces.add(new Workspace({ id, name, layout: new FloatingLayout() }))`.                                     |
| `autostart = { ... }`                                                 | No home yet. Start the command from your session.                                                                        |
| `input`, `cursor`, `compositor`, `protocols` and the other sections   | No home yet, except the device settings. See [the table above](#settings-with-no-typescript-home-yet).                   |
| `gnoblin.load("config/*.lua")`, `require`                             | `import "./config/10-windows.ts"`. There is no glob. The converter reads a `gnoblin.load` in place and has no `require`. |
| `gnoblin.on("gnoblin.window.created", fn)`                            | A `ClientListener` or a `CollectionListener` on `gnoblin.clients`. Only the window events are mapped.                    |
| `seat.data.window_rules = { { match = { app_id = "^x$" }, ... } }`    | A listener or a `where(...).style`. See [window rules](#window-rules).                                                   |
| `seat:bind({ ["<Super>x"] = fn })`, `Swipe3`                          | `input.bind("Super+x", () => ...)`, and `input.bind("Swipe3", ...)` for gestures.                                        |
| `seat.data.prompts = false`, `seat.data.alerts = false`               | `alerts = false` leaves `startNotRespondingAlert()` out. Prompts are drawn by the compositor and have no switch.         |
| `gnoblin.timers.after`, `gnoblin.process.spawn`                       | `gnoblin.timers.after`, `gnoblin.spawn`.                                                                                 |
| Reactive properties, `go`, `await`, `every`, `Node:extend`, selectors | None. Rewrite them as listeners and `gnoblin.timers.after`.                                                              |

Each page of the [shell API](shell-api/index.md) shows the objects these statements use, and the
[settings page](js-settings.md) shows the settings that have one.
