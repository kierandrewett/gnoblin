# Migrating from Lua

The TypeScript shell is the default shell. It reads `init.ts` and does not read `init.lua`. The Lua shell stays for one
more release so that an update never replaces your configuration with the stock one without telling you.

## What happens to an `init.lua` today

- If your config directory has an `init.lua` and no `init.ts` or `init.js`, and you have not set `GNOBLIN_SHELL_VM`, the
  session runs the Lua shell as before and logs one warning that names this page.
- Once `init.ts` exists, the TypeScript shell runs. The `init.lua` beside it is ignored and the log says so.
- `GNOBLIN_SHELL_VM=lua` runs the Lua shell whatever is on disk. It is deprecated and is removed after this release.

See [Which shell runs](typescript-shell.md#which-shell-runs) for the full table.

## Move your config

1. Convert it: `gnoblinctl config migrate`. It reads your `init.lua` and the files it loads, writes `init.ts` and the
   editor files beside it, and changes nothing else. See [the converter](#the-converter).
2. Review `init.ts`. Everything the converter could not carry over is a comment that starts with `UNMAPPED`, and the
   command lists each one with its line. Move those across by hand using the table below. Then run
   `gnoblinctl check --types init.ts`: it reports a mistake with its line, and a retired key is refused with the key to
   use instead.
3. Log out and back in. A running Lua session keeps reading `init.lua`; from the next session the TypeScript shell runs and
   `init.ts` takes precedence.

## The converter

`gnoblinctl config migrate [FILE] [-o OUT] [--force] [--dry-run]` turns a Lua config into a TypeScript one. It reads the
Lua as text and never runs it, so it needs no session and no Lua.

```sh
gnoblinctl config migrate
gnoblinctl config migrate --dry-run
gnoblinctl config migrate ~/old/init.lua -o ~/new/init.ts
```

- `FILE` is the `init.lua` the Lua shell reads. The `shell.lua` beside an `init.lua` is read too.
- It writes `init.ts` beside `FILE`, or `OUT`. It stops instead of replacing a file, and instead of writing an `init.ts`
  beside an `init.js`, unless you add `--force`.
- It runs `gnoblinctl types init` on the directory it wrote to, and loads the result once with the shell's own
  `defineConfig` before it writes, so a file that would not load is reported and not found at the next login.
- `--dry-run` prints the TypeScript on standard output and the report on standard error, and writes nothing.

The exit code is 0 when everything converted, 1 when the file was written and something was left out, and 2 when nothing
was written: a wrong command line, a file it could not read, or Lua that does not parse. See the
[gnoblinctl reference](gnoblinctl.md#convert-an-initlua).

### What the converter converts

It converts the data a config holds, exactly, and each file that `gnoblin.load` names, in the order Lua loaded them.
The files become one `init.ts`, with a comment that names the file each part came from.

| In Lua                                                       | In `init.ts`                                                                                                                     |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.configure { ... }` and `gnoblin.configure({ ... })` | One `defineConfig({ ... })`. Several calls and several files merge the way the document's layers do: tables by key, lists whole. |
| `["window-management"] = { ["focus-mode"] = "hover" }`       | `windowManagement: { focusMode: "hover" }`. A key is camel case when `defineConfig` reads it back as the same key.               |
| `shortcuts`, `keybindings` with `binding` and `command`      | `keys: { "Super+Return": spawn("foot") }`. `<Super><Shift>x` is `Super+Shift+x`, and `command = "foot"` is `spawn("foot")`.      |
| `autostart = { name = { command = { ... } } }`               | `autostart: ["waybar", ["foot", "--server"]]`, in name order. An entry with `enable = false` is left out.                        |
| `workspaces = { { id = "1", name = "One" } }`                | `workspaces: [{ id: "1", name: "One" }]`.                                                                                        |
| `touchpad-gestures = { ... }`                                | `touchpadGestures: [ ... ]`.                                                                                                     |
| `gnoblin.load("config/*.lua")`                               | The files are read in place, sorted, relative to the file that makes the call, and their settings are part of the one `init.ts`. |
| `local x = 5 * 100`, `"a" .. "b"`                            | The value, when it is a number, a string, a boolean or a table with only such values.                                            |
| `seat.data.window_rules = { ... }` in `shell.lua`            | `rules: [ ... ]`. See [window rules](#window-rules).                                                                             |
| `seat.data.workspace_sets`, `seat.data.alerts`               | `workspaceSets`, `alerts`.                                                                                                       |
| `seat.edge_tiling`, `seat.drag_constraint` in `shell.lua`    | `edgeTiling`, `dragConstraint`.                                                                                                  |

A shortcut converts when its chord has an exact trigger:

- `<Primary>`, `<Ctrl>` and `<Ctl>` are `Ctrl`, `<Mod1>` is `Alt` and `<Shft>` is `Shift`.
- A media key written without its `XF86` prefix gets it back.

A shortcut is left out, with the reason, when it acts on key release, when its chord is on a pointer button, a hardware
key code or `Above_Tab`, when its key is an upper-case letter, when it binds a trigger that is bound already, or when its
command is one string with spaces in it.

### Window rules

A rule's `match` becomes its query and its other fields become its actions:

- `app_id` is `appId`, and `workspace_id` is `onWorkspace`.
- `workspace = { id = "3" }` is `workspace: "3"`, and `opacity = 0.9` is `style: { opacity: 0.9 }`.
- `title`, `focused`, `maximised`, `fullscreen` and `secure` keep their names.
- A Lua rule ran again when the window changed, as a `defineConfig` rule does, so no option is needed.

A Lua pattern is searched anywhere in the text. A pattern that is plain text becomes a glob, which matches the whole text,
so the stars say "anywhere". Any other pattern becomes a RegExp, with `s` when it has a `.` because a Lua `.` matches a
newline.

| Lua pattern                              | In the rule                               |
| ---------------------------------------- | ----------------------------------------- |
| `^foot$`                                 | `"foot"`                                  |
| `^org%.mozilla%.`                        | `"org.mozilla.*"`                         |
| `fire`                                   | `"*fire*"`                                |
| `^org%.x%.(%w+)$`                        | `/^org\.x\.[A-Za-z0-9]+$/`                |
| `%d+ unread`                             | `/[0-9]+ unread/`                         |
| `^[a-z%d_]+$`                            | `/^[a-z0-9_]+$/`                          |
| `Picture.in.Picture`                     | `/Picture.in.Picture/s`                   |
| `%bxy`, `%f[%w]`, `%1`, `%p`, `%c`, `%g` | Not translated, and the rule is left out. |

These translate:

- The anchors `^` and `$`, `.`, and a `%` before a punctuation character.
- The classes `%a %d %s %w %l %u %x` and their upper-case complements.
- The quantifiers `*`, `+`, `-` and `?`.
- Character sets and ranges. Capture parentheses are dropped, because a rule reads no capture.

Anything else leaves the whole rule out, because a looser match would act on windows you never named. So does a `match`
key other than `app_id`, `title`, `focused` and `workspace_id`, and a rule that names a workspace the config does not
declare. A rule field the converter cannot carry over is left out alone, so the rule still applies without it.

A Lua `app_id` pattern also matched the WM class when the app id was empty. `appId` does not.

### What the converter cannot do

Anything it cannot evaluate without running the file is left out of `init.ts`, written there as a comment, and listed
with its line:

```ts
// UNMAPPED (init.lua:6): gnoblin.on("gnoblin.window.created", function(w)
//     w:maximise()
// end)
// -> an event handler runs code: write `on: { clientAdded(client) {} }` or a `ClientListener`
```

What is left out:

- Every function, `if`, loop, `require`, method call, `seat:bind`, `gnoblin.on` and timer.
- A value that comes from a call, or from an operator the converter does not evaluate, and a name it does not know.
  One computed value in a table costs only its own field: the settings around it still convert.
- `animations` and `portals`, which the document carries and nothing reads, and a section it does not know.
- `seat.data.prompts`, which has no switch because the compositor draws prompts.
- Comments in your Lua.
- A `gnoblin.load` of a glob outside the config directory.

The shell files, `shell.lua` and `shell/*.lua`, are arbitrary Lua over the object model, and cannot be translated
mechanically. These are the parts that need a hand port:

- `Node:extend`, reactive properties, `go`, `await`, `every` and selectors.
- A subclass of `Workspace`, and `Window:on_ready`.

The converter reads the `seat.data` assignments in the table above from `shell.lua` and reports every other statement,
so a shell file becomes a list of what to port.

## What each construct becomes

| In Lua                                                                | In TypeScript                                                                                                                      |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.configure { ["window-management"] = { ... } }`               | `defineConfig({ windowManagement: { ... } })`. Kebab and camel case keys both work, and an unknown key fails with a suggestion.    |
| `shortcuts`, `keybindings`                                            | `keys: { "Super+Return": spawn("foot") }`. A Lua `<Super>Return` chord is `Super+Return`. A shortcut on key release is not ported. |
| `workspaces = { { id, name } }`                                       | `workspaces: [{ id, name, layout }]`.                                                                                              |
| `autostart = { ... }`                                                 | `autostart: ["waybar", ["foot", "--server"]]`. The session starts them and stops one you remove.                                   |
| `input`, `cursor`, `compositor`, `protocols` and the other sections   | The same names in camel case, as `defineConfig` keys. See the [defineConfig reference](typescript-config.md).                      |
| `gnoblin.load("config/*.lua")`, `require`                             | `import "./config/10-windows.ts"`. There is no glob. The converter reads a `gnoblin.load` in place and has no `require`.           |
| `gnoblin.on("gnoblin.window.created", fn)`                            | `on: { clientAdded(client) {} }`, or a `ClientListener`. Only the window events are mapped.                                        |
| `seat.data.window_rules = { { match = { app_id = "^x$" }, ... } }`    | `rules: [{ appId: "x", ... }]`. Write `\.` where Lua wrote `%.` in a RegExp. Every rule re-applies when a window changes.          |
| `seat:bind({ ["<Super>x"] = fn })`, `Swipe3`                          | `keys: { "Super+x": ({ gnoblin }) => ... }`, and `gnoblin.input.primary.bind` for gestures.                                        |
| `seat.data.prompts = false`, `seat.data.alerts = false`               | `alerts: false`, `menus: false`. Prompts are drawn by the compositor and have no switch.                                           |
| `gnoblin.timers.after`, `gnoblin.process.spawn`                       | `gnoblin.after`, `gnoblin.spawn`.                                                                                                  |
| Reactive properties, `go`, `await`, `every`, `Node:extend`, selectors | None. Rewrite them as listeners and `gnoblin.after`.                                                                               |

Settings that were in the Lua document are in the TypeScript document with the same meaning, and each page under
`config/configure` says how it is applied.
